#!/usr/bin/env node

import { spawnSync } from 'node:child_process'
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs'
import os from 'node:os'
import { basename, dirname, join, resolve } from 'node:path'
import process from 'node:process'
import { fileURLToPath } from 'node:url'

import {
  androidReleaseAssetName,
  buildReleaseManifest,
  buildReleaseManifestFiles,
  linuxReleaseTargetsForDockerPlatform,
  normalizeTag,
  parseEnvFile,
  readWorkspaceVersionTag,
  splitCsv,
  validateReleaseAssetSet,
} from './local-release-lib.mjs'

const __dirname = dirname(fileURLToPath(import.meta.url))
const repoRoot = resolve(__dirname, '..')
const rootCargoToml = join(repoRoot, 'Cargo.toml')
const changelogPath = join(repoRoot, 'CHANGELOG.md')
const distDir = join(repoRoot, 'dist')
const defaultEnvFiles = [join(repoRoot, '.env.release.local'), join(repoRoot, '.env.zapstore.local')]
const versionlessCliAssets = new Map([
  ['nvpn-aarch64-apple-darwin.tar.gz', 'nvpn-{tag}-aarch64-apple-darwin.tar.gz'],
  ['nvpn-x86_64-unknown-linux-musl.tar.gz', 'nvpn-{tag}-x86_64-unknown-linux-musl.tar.gz'],
  ['nvpn-aarch64-unknown-linux-musl.tar.gz', 'nvpn-{tag}-aarch64-unknown-linux-musl.tar.gz'],
])

class SkipStepError extends Error {}

function usage() {
  console.log(`Usage: node scripts/local-release.mjs [options]

Build local Rust/native release artifacts, stage a hashtree release directory,
and optionally publish it.

Options:
  --publish                 Publish the staged release tree with htree
  --dry-run                 Print the plan without running build or publish commands
  --skip-verify            Skip fmt/clippy/test verification
  --tag <tag>              Release tag (defaults to workspace version, for example v4.0.0)
  --release-tree <name>    htree release tree name (default: releases/nostr-vpn)
  --stage-dir <path>       Directory used for staged release metadata
  --env-file <path>        Extra dotenv file to load (repeatable)
  --only <csv>             Limit steps to verify,macos,linux,windows,android
  --skip <csv>             Skip steps by name
  --allow-partial          Stage/publish even if a selected platform build fails
  --help                   Show this help

The script auto-loads .env.release.local and .env.zapstore.local when present.
Shell environment variables override values from those files.`)
}

function parseArgs(argv) {
  const options = {
    dryRun: false,
    publish: false,
    skipVerify: false,
    releaseTree: null,
    stageDir: null,
    tag: null,
    envFiles: [],
    only: null,
    skip: new Set(),
    allowPartial: false,
  }

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    switch (arg) {
      case '--help':
      case '-h':
        usage()
        process.exit(0)
      case '--publish':
        options.publish = true
        break
      case '--dry-run':
        options.dryRun = true
        break
      case '--skip-verify':
        options.skipVerify = true
        break
      case '--tag':
        options.tag = normalizeTag(argv[++index] ?? '')
        break
      case '--release-tree':
        options.releaseTree = argv[++index] ?? ''
        break
      case '--stage-dir':
        options.stageDir = argv[++index] ?? ''
        break
      case '--env-file':
        options.envFiles.push(resolve(repoRoot, argv[++index] ?? ''))
        break
      case '--only':
        options.only = new Set(splitCsv(argv[++index] ?? ''))
        break
      case '--skip':
        for (const value of splitCsv(argv[++index] ?? '')) {
          options.skip.add(value)
        }
        break
      case '--allow-partial':
        options.allowPartial = true
        break
      default:
        throw new Error(`Unknown argument: ${arg}`)
    }
  }

  return options
}

function readOptionalEnvFiles(envFiles) {
  const loaded = {}
  const loadedPaths = []

  for (const envFile of envFiles) {
    if (!existsSync(envFile)) {
      continue
    }
    Object.assign(loaded, parseEnvFile(readFileSync(envFile, 'utf8')))
    loadedPaths.push(envFile)
  }

  return { loaded, loadedPaths }
}

function commandExists(command) {
  const result =
    process.platform === 'win32'
      ? spawnSync('where', [command], { stdio: 'ignore' })
      : spawnSync('sh', ['-lc', `command -v "${command}"`], { stdio: 'ignore' })

  return result.status === 0
}

function quote(arg) {
  const value = String(arg)
  return /[^\w./:-]/.test(value) ? JSON.stringify(value) : value
}

function envFlagEnabled(value) {
  return /^(1|true|yes|on)$/i.test(String(value ?? '').trim())
}

function cargoTargetDir(env = process.env) {
  const configured = String(env.CARGO_TARGET_DIR ?? '').trim()
  if (configured.length === 0) {
    return join(repoRoot, 'target')
  }
  return resolve(repoRoot, configured)
}

function findFirstFile(root, matcher) {
  if (!existsSync(root)) {
    return null
  }

  const entries = readdirSync(root).sort()
  const match = entries.find((entry) => matcher(entry))
  return match ? join(root, match) : null
}

function run(command, args, { cwd = repoRoot, env = process.env, capture = false, dryRun = false } = {}) {
  const rendered = [command, ...args].map(quote).join(' ')
  console.log(`$ ${rendered}`)
  if (dryRun) {
    return ''
  }

  const result = spawnSync(command, args, {
    cwd,
    env,
    encoding: 'utf8',
    stdio: capture ? 'pipe' : 'inherit',
  })
  if (result.status !== 0) {
    const stderr = capture ? result.stderr.trim() : ''
    throw new Error(stderr || `${command} exited with status ${result.status ?? 'unknown'}`)
  }
  return capture ? result.stdout.trim() : ''
}

function writeUnixInstallScript(path) {
  writeFileSync(
    path,
    `#!/bin/bash
set -e

path_contains() {
  case ":\${PATH}:" in
    *":$1:"*) return 0 ;;
    *) return 1 ;;
  esac
}

default_install_dir() {
  if [ "$(uname -s)" = "Darwin" ] && { [ -d /opt/homebrew/bin ] || path_contains /opt/homebrew/bin; }; then
    printf '%s\\n' /opt/homebrew/bin
  else
    printf '%s\\n' /usr/local/bin
  fi
}

INSTALL_DIR="\${1:-$(default_install_dir)}"
install -d "\${INSTALL_DIR}"
install -m 755 nvpn "\${INSTALL_DIR}/"
`,
  )
}

function writeUnixReadme(path) {
  writeFileSync(
    path,
    `nvpn - FIPS private mesh CLI
============================

Binary included:
  nvpn  - CLI control plane

Quick install:
  ./install.sh
  ./install.sh ~/.local/bin
`,
  )
}

function packageUnixCliTarball({ binaryPath, targetTriple, tag, dryRun }) {
  const bundleDir = join(distDir, 'nvpn')
  if (!dryRun) {
    rmSync(bundleDir, { recursive: true, force: true })
    mkdirSync(bundleDir, { recursive: true })
    copyFileSync(binaryPath, join(bundleDir, 'nvpn'))
    writeUnixInstallScript(join(bundleDir, 'install.sh'))
    writeUnixReadme(join(bundleDir, 'README.txt'))
  }

  run('chmod', ['+x', join(bundleDir, 'install.sh')], { dryRun })

  const unversioned = join(distDir, `nvpn-${targetTriple}.tar.gz`)
  const versioned = join(distDir, `nvpn-${tag}-${targetTriple}.tar.gz`)
  run('tar', ['-czf', unversioned, '-C', distDir, 'nvpn'], { dryRun })
  if (!dryRun) {
    copyFileSync(unversioned, versioned)
  }
  return [unversioned, versioned]
}

function psQuote(value) {
  return `'${String(value).replace(/'/g, "''")}'`
}

function encodePowerShellScript(script) {
  return Buffer.from(script, 'utf16le').toString('base64')
}

/// Run a PowerShell snippet on the remote SSH host. We base64 the source so
/// quoting and newlines survive the SSH/PowerShell round-trip cleanly.
function runWindowsPowerShell(host, script, { capture = false, dryRun = false } = {}) {
  const encoded = encodePowerShellScript(script)
  return run(
    'ssh',
    [host, 'powershell.exe', '-NoProfile', '-EncodedCommand', encoded],
    { capture, dryRun },
  )
}

function windowsArtifactArch(targetTriple) {
  if (targetTriple.startsWith('x86_64-')) {
    return 'x64'
  }
  if (targetTriple.startsWith('aarch64-')) {
    return 'arm64'
  }
  return targetTriple
}

/// Run a tar pipe through ssh in either direction. We rely on macOS `tar`
/// and Windows 10+ `tar.exe` (bsdtar) sharing a wire format. The remote ssh
/// shell on win11-dev defaults to cmd.exe, which gladly forwards `tar` to
/// the Windows-side `tar.exe`. No extra tools are installed on the remote.
function runShellPipe(cmd, { dryRun = false, errMsg = 'pipe' } = {}) {
  if (dryRun) {
    console.log(`[dry] ${cmd}`)
    return
  }
  const result = spawnSync('bash', ['-c', cmd], { stdio: ['ignore', 'inherit', 'inherit'] })
  if (result.status !== 0) {
    throw new Error(`${errMsg} failed (exit ${result.status})`)
  }
}

function syncRepoToWindowsHost({ host, guestRepo, dryRun }) {
  // Translate the Windows path to a forward-slash form that tar accepts as
  // -C without backslash escaping.
  const guestRepoForward = guestRepo.replace(/\\/g, '/')
  // Skip the other platforms wholesale — they're not needed to build for
  // Windows, and shipping them across the SSH tunnel wastes minutes (and
  // risks the connection timing out on slow links). The cargo workspace
  // explicitly excludes `linux/`, and `crates/` + `windows/` are the only
  // things the Windows build touches.
  const tarExcludes = [
    '--exclude=./target',
    '--exclude=./dist',
    '--exclude=./.git',
    '--exclude=./artifacts',
    '--exclude=./node_modules',
    '--exclude=./.env.release.local',
    '--exclude=./.env.zapstore.local',
    '--exclude=./ios',
    '--exclude=./macos',
    '--exclude=./android',
    '--exclude=./linux',
    '--exclude=./umbrel',
  ].join(' ')

  // Ensure the destination exists first.
  runWindowsPowerShell(
    host,
    `New-Item -ItemType Directory -Force -Path ${psQuote(guestRepo)} | Out-Null`,
    { dryRun },
  )
  runShellPipe(
    `tar ${tarExcludes} -cf - -C ${quote(repoRoot)} . | ssh ${quote(host)} tar -xf - -C ${quote(guestRepoForward)}`,
    { dryRun, errMsg: `tar|ssh sync to ${host}` },
  )
}

function pullFileFromWindowsHost({ host, remotePath, localParent, name, dryRun }) {
  // remotePath is the directory containing `name` (a single file). Pull just
  // that file via tar so we don't need to know its size, etc.
  const remotePathForward = remotePath.replace(/\\/g, '/')
  runShellPipe(
    `ssh ${quote(host)} tar -cf - -C ${quote(remotePathForward)} ${quote(name)} | tar -xf - -C ${quote(localParent)}`,
    { dryRun, errMsg: `tar pull ${host}:${remotePath}/${name}` },
  )
}

function buildWindowsArtifacts({ env, tag, dryRun, builtLines }) {
  // Windows builds run on win11-dev — an x86_64 Windows VM reachable over the
  // Nostr VPN mesh (see CLAUDE.md). Set NVPN_WINDOWS_SSH_HOST to override.
  const host = env.NVPN_WINDOWS_SSH_HOST || 'win11-dev'

  // Probe SSH connectivity. Skip cleanly if the VM is unreachable rather
  // than aborting the whole release.
  if (!dryRun) {
    const probe = spawnSync(
      'ssh',
      ['-o', 'BatchMode=yes', '-o', 'ConnectTimeout=10', host, 'whoami'],
      { stdio: ['ignore', 'pipe', 'pipe'] },
    )
    if (probe.status !== 0) {
      throw new SkipStepError(
        `Skipping Windows artifacts because ssh ${host} is unreachable. ` +
          'Bring up the VM (e.g. on vader) and ensure VPN is connected, or set NVPN_WINDOWS_SSH_HOST.',
      )
    }
  }

  // Path to the working copy on the Windows host. Has to be a valid
  // PowerShell-quoted absolute path; cargo-target chains can get long, so
  // pick something short.
  const guestRepo = env.NVPN_WINDOWS_GUEST_REPO_PATH || 'C:\\src\\nostr-vpn'

  syncRepoToWindowsHost({ host, guestRepo, dryRun })

  const vmArchitecture = runWindowsPowerShell(
    host,
    '[System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()',
    { capture: true, dryRun },
  ).trim()
  if (!dryRun && vmArchitecture.toLowerCase() !== 'x64') {
    throw new SkipStepError(
      `Skipping Windows artifacts because ${host} is ${vmArchitecture}; Windows x64 release artifacts must be built on an x64 Windows runner.`,
    )
  }

  const llvmBin = env.NVPN_WINDOWS_LLVM_BIN || 'C:\\Program Files\\LLVM\\bin'
  const targets = splitCsv(
    env.NVPN_WINDOWS_CLI_TARGETS || 'x86_64-pc-windows-msvc',
  )
  const guestDist = `${guestRepo}\\dist`
  const guestRepoQuoted = psQuote(guestRepo)
  const guestDistQuoted = psQuote(guestDist)
  const pathSetup = `$env:PATH = ${psQuote(llvmBin)} + ';' + $env:PATH`

  // Make sure the guest's dist dir exists so we can write archives to it.
  runWindowsPowerShell(
    host,
    `New-Item -ItemType Directory -Force -Path ${guestDistQuoted} | Out-Null`,
    { dryRun },
  )

  for (const target of targets) {
    const archiveName = `nvpn-${tag}-${target}.zip`
    runWindowsPowerShell(
      host,
      `
${pathSetup}
Set-Location ${guestRepoQuoted}
cargo build --release --target ${psQuote(target)} -p nostr-vpn-cli
$cli = Join-Path ${guestRepoQuoted} ${psQuote(`target\\${target}\\release\\nvpn.exe`)}
if (!(Test-Path $cli)) { throw "Missing nvpn.exe for ${target}" }
$tempDir = Join-Path $env:TEMP ${psQuote(`nvpn-${target}-zip`)}
Remove-Item -Recurse -Force $tempDir -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $tempDir | Out-Null
Copy-Item $cli (Join-Path $tempDir 'nvpn.exe')
Compress-Archive -Path (Join-Path $tempDir '*') -DestinationPath ${psQuote(`${guestDist}\\${archiveName}`)} -Force
Remove-Item -Recurse -Force $tempDir
`,
      { dryRun },
    )
    pullFileFromWindowsHost({ host, remotePath: guestDist, localParent: distDir, name: archiveName, dryRun })
    builtLines.push(`Built Windows ${windowsArtifactArch(target)} CLI on ${host}.`)
  }

  const guiTargets = splitCsv(env.NVPN_WINDOWS_GUI_TARGETS || 'x86_64-pc-windows-msvc')
  for (const target of guiTargets) {
    const arch = windowsArtifactArch(target)
    if (arch !== 'x64') {
      throw new Error(`Windows desktop installer currently supports x64 only, got ${target}.`)
    }

    const installerName = `nostr-vpn-${tag}-windows-${arch}-setup.exe`
    runWindowsPowerShell(
      host,
      `
${pathSetup}
Set-Location ${guestRepoQuoted}
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\\scripts\\windows-build.ps1 -Configuration Release -Publish -Installer -Tag ${psQuote(tag)} -OutputDir ${guestDistQuoted}
$installer = ${psQuote(`${guestDist}\\${installerName}`)}
if (!(Test-Path $installer)) { throw "Missing Windows installer: $installer" }
`,
      { dryRun },
    )
    pullFileFromWindowsHost({ host, remotePath: guestDist, localParent: distDir, name: installerName, dryRun })
    builtLines.push(`Built Windows ${arch} desktop installer on ${host}.`)
  }
}

function buildLinuxArtifacts({ env, tag, dryRun, builtLines }) {
  if (!commandExists('docker')) {
    throw new SkipStepError('Skipping Linux artifacts because docker is not on PATH.')
  }

  const platform = env.NVPN_LINUX_DOCKER_PLATFORM || 'linux/amd64'
  const { linuxArchSuffix, muslTriple } = linuxReleaseTargetsForDockerPlatform(platform)
  const imageName = 'nostr-vpn-linux-release'
  const linuxDebName = `nostr-vpn-${tag}-linux-${linuxArchSuffix}.deb`
  run('docker', ['build', '--platform', platform, '-f', 'Dockerfile.linux-release', '-t', imageName, '.'], {
    dryRun,
  })

  if (!dryRun) {
    mkdirSync(distDir, { recursive: true })
  }

  const innerScript = [
    'set -euo pipefail',
    `rustup target add ${muslTriple}`,
    'rsync -a --exclude target --exclude dist --exclude .git /work/ /build/',
    'cd /build',
    'cargo build --release --locked --manifest-path linux/Cargo.toml',
    'cd /build/linux',
    'cargo deb --no-build',
    `cp "$(ls -1t target/debian/*.deb | head -1)" "/work/dist/${linuxDebName}"`,
    'cd /build',
    `cargo build --release --target ${muslTriple} -p nostr-vpn-cli`,
    'rm -rf /work/dist/nvpn',
    'mkdir -p /work/dist/nvpn',
    `cp target/${muslTriple}/release/nvpn /work/dist/nvpn/`,
    "printf '%s\\n' '#!/bin/bash' 'set -e' 'install -d \"${1:-/usr/local/bin}\"' 'install -m 755 nvpn \"${1:-/usr/local/bin}/\"' > /work/dist/nvpn/install.sh",
    'chmod +x /work/dist/nvpn/install.sh',
    "printf '%s\\n' 'nvpn - FIPS private mesh CLI' > /work/dist/nvpn/README.txt",
    `tar -czf /work/dist/nvpn-${muslTriple}.tar.gz -C /work/dist nvpn`,
    `cp /work/dist/nvpn-${muslTriple}.tar.gz /work/dist/nvpn-${tag}-${muslTriple}.tar.gz`,
  ].join(' && ')

  run(
    'docker',
    [
      'run',
      '--rm',
      '--platform',
      platform,
      '-v',
      `${repoRoot}:/work`,
      '-w',
      '/work',
      imageName,
      'bash',
      '-c',
      innerScript,
    ],
    { dryRun },
  )

  builtLines.push(`Built Linux ${linuxArchSuffix} desktop Debian package in Docker (${platform}).`)
  builtLines.push(`Built Linux ${linuxArchSuffix} musl CLI in Docker (${platform}).`)
}

function ensureAndroidSdkEnv(env) {
  const updated = { ...env }
  if (!updated.ANDROID_SDK_ROOT) {
    const candidate = join(os.homedir(), 'Library', 'Android', 'sdk')
    if (existsSync(candidate)) {
      updated.ANDROID_SDK_ROOT = candidate
    }
  }
  if (!updated.ANDROID_HOME && updated.ANDROID_SDK_ROOT) {
    updated.ANDROID_HOME = updated.ANDROID_SDK_ROOT
  }
  if (!updated.ANDROID_NDK_HOME && updated.ANDROID_HOME) {
    const ndkRoot = join(updated.ANDROID_HOME, 'ndk')
    if (existsSync(ndkRoot)) {
      const versions = readdirSync(ndkRoot).sort((left, right) =>
        left.localeCompare(right, undefined, { numeric: true }),
      )
      const latest = versions.at(-1)
      if (latest) {
        updated.ANDROID_NDK_HOME = join(ndkRoot, latest)
      }
    }
  }
  if (!updated.NDK_HOME && updated.ANDROID_NDK_HOME) {
    updated.NDK_HOME = updated.ANDROID_NDK_HOME
  }
  return updated
}

function findAndroidBuildTool(env, toolName) {
  const sdkRoot = env.ANDROID_SDK_ROOT || env.ANDROID_HOME
  if (!sdkRoot) {
    return null
  }
  const buildToolsRoot = join(sdkRoot, 'build-tools')
  if (!existsSync(buildToolsRoot)) {
    return null
  }
  const versions = readdirSync(buildToolsRoot).sort((left, right) =>
    left.localeCompare(right, undefined, { numeric: true }),
  )
  for (const version of versions.reverse()) {
    const candidate = join(buildToolsRoot, version, toolName)
    if (existsSync(candidate)) {
      return candidate
    }
  }
  return null
}

function androidSigningIsComplete(env) {
  return Boolean(
    env.ANDROID_KEYSTORE_PATH &&
      env.ANDROID_KEYSTORE_PASSWORD &&
      env.ANDROID_KEY_ALIAS &&
      env.ANDROID_KEY_PASSWORD,
  )
}

function buildAndroidArtifacts({ env, tag, dryRun, builtLines }) {
  const androidEnv = ensureAndroidSdkEnv(env)
  const sdkRoot = androidEnv.ANDROID_SDK_ROOT || androidEnv.ANDROID_HOME
  if (!sdkRoot) {
    throw new SkipStepError('Skipping Android artifacts because ANDROID_SDK_ROOT/ANDROID_HOME is not configured.')
  }
  if (!commandExists('cargo-ndk')) {
    throw new SkipStepError('Skipping Android artifacts because cargo-ndk is not on PATH.')
  }

  const installedTargets = run('rustup', ['target', 'list', '--installed'], {
    capture: true,
    dryRun,
  })
  if (!installedTargets.includes('aarch64-linux-android')) {
    run('rustup', ['target', 'add', 'aarch64-linux-android'], { dryRun })
  }

  const tempRoot = dryRun ? null : mkdtempSync(join(os.tmpdir(), 'nvpn-android-release-'))
  let wroteTempKeystore = false
  try {
    if (!androidEnv.ANDROID_KEYSTORE_PATH && androidEnv.ANDROID_KEYSTORE_B64 && tempRoot) {
      androidEnv.ANDROID_KEYSTORE_PATH = join(tempRoot, 'upload-keystore.jks')
      writeFileSync(androidEnv.ANDROID_KEYSTORE_PATH, Buffer.from(androidEnv.ANDROID_KEYSTORE_B64, 'base64'))
      wroteTempKeystore = true
    }

    const signed = androidSigningIsComplete(androidEnv)
    if (!signed && !envFlagEnabled(androidEnv.NVPN_ALLOW_UNSIGNED_ANDROID)) {
      throw new Error(
        'Android release signing is not configured. Set ANDROID_KEYSTORE_PATH, ANDROID_KEYSTORE_PASSWORD, ANDROID_KEY_ALIAS, and ANDROID_KEY_PASSWORD, or set NVPN_ALLOW_UNSIGNED_ANDROID=1 for an explicitly unsigned dev artifact.',
      )
    }

    run('bash', [join(repoRoot, 'tools', 'run-android'), ':app:assembleRelease', ':app:bundleRelease'], {
      env: androidEnv,
      dryRun,
    })

    const apkPath = findFirstFile(
      join(repoRoot, 'android', 'app', 'build', 'outputs', 'apk', 'release'),
      (entry) => entry.endsWith('.apk'),
    )
    const aabPath = findFirstFile(
      join(repoRoot, 'android', 'app', 'build', 'outputs', 'bundle', 'release'),
      (entry) => entry.endsWith('.aab'),
    )
    if (!dryRun && (!apkPath || !aabPath)) {
      throw new Error('Expected Android APK/AAB outputs were not produced.')
    }

    const apkDest = join(distDir, androidReleaseAssetName(tag, { extension: 'apk', signed }))
    const aabDest = join(distDir, androidReleaseAssetName(tag, { extension: 'aab', signed }))
    if (!dryRun) {
      mkdirSync(distDir, { recursive: true })
      copyFileSync(apkPath, apkDest)
      copyFileSync(aabPath, aabDest)
    }

    if (signed) {
      const apksigner = findAndroidBuildTool(androidEnv, 'apksigner')
      if (apksigner) {
        run(apksigner, ['verify', '--verbose', apkDest], { dryRun })
      }
    }

    builtLines.push(signed ? 'Built signed Android arm64 APK/AAB.' : 'Built unsigned Android arm64 APK/AAB.')
  } finally {
    if (wroteTempKeystore && androidEnv.ANDROID_KEYSTORE_PATH) {
      rmSync(androidEnv.ANDROID_KEYSTORE_PATH, { force: true })
    }
    if (tempRoot) {
      rmSync(tempRoot, { recursive: true, force: true })
    }
  }
}

function buildMacosArtifacts({ tag, dryRun, builtLines }) {
  if (process.platform !== 'darwin' || process.arch !== 'arm64') {
    throw new SkipStepError('Skipping macOS artifacts because the host is not Apple Silicon macOS.')
  }

  const env = {
    ...process.env,
    NVPN_MACOS_RUST_PROFILE: 'release',
    NVPN_MACOS_XCODE_CONFIGURATION: 'Release',
    NVPN_MACOS_RUST_TARGETS: 'aarch64-apple-darwin',
    NVPN_RELEASE_TAG: tag,
    NVPN_MACOS_REQUIRE_SIGNING: '1',
    NVPN_MACOS_REQUIRE_NOTARIZATION: '1',
  }
  if (!dryRun) {
    rmSync(join(distDir, `nostr-vpn-${tag}-macos-arm64.zip`), { force: true })
  }
  run('bash', [join(repoRoot, 'scripts', 'macos-build'), 'macos-release-artifacts'], { env, dryRun })

  packageUnixCliTarball({
    binaryPath: join(cargoTargetDir(env), 'aarch64-apple-darwin', 'release', 'nvpn'),
    targetTriple: 'aarch64-apple-darwin',
    tag,
    dryRun,
  })
  builtLines.push('Built Apple Silicon CLI locally.')
  builtLines.push('Built signed and notarized Apple Silicon macOS DMG and updater archive locally.')
}

function runVerify({ dryRun, builtLines }) {
  run('cargo', ['fmt', '--check'], { dryRun })
  run('cargo', ['clippy', '--workspace', '--all-targets', '--', '-D', 'warnings'], { dryRun })
  run('cargo', ['test', '--workspace'], { dryRun })
  builtLines.push('Ran cargo fmt --check, cargo clippy, and cargo test.')
}

function shouldRunStep(step, options) {
  if (options.skipVerify && step === 'verify') {
    return false
  }
  if (options.only && !options.only.has(step)) {
    return false
  }
  return !options.skip.has(step)
}

function collectReleaseAssetPaths(tag) {
  if (!existsSync(distDir)) {
    return []
  }

  const versionedNames = new Set(
    readdirSync(distDir).filter((entry) => entry.includes(`-${tag}-`) || entry.includes(`${tag}-`)),
  )
  const paths = []

  for (const entry of readdirSync(distDir).sort()) {
    if (entry === `nostr-vpn-${tag}-macos-arm64.zip`) {
      continue
    }
    const fullPath = join(distDir, entry)
    if (!statSync(fullPath).isFile()) {
      continue
    }
    if (entry.includes(tag)) {
      paths.push(fullPath)
      continue
    }
    const companionPattern = versionlessCliAssets.get(entry)
    if (companionPattern && versionedNames.has(companionPattern.replace('{tag}', tag))) {
      paths.push(fullPath)
    }
  }

  return paths
}

function writeReleaseNotes({ tag, commit, stageDir, builtLines, skippedLines, dryRun }) {
  const args = [
    join(repoRoot, 'scripts', 'render-release-notes.mjs'),
    '--tag',
    tag,
    '--commit',
    commit,
    '--asset-dir',
    join(stageDir, 'assets'),
    '--changelog',
    changelogPath,
    '--out',
    join(stageDir, 'notes.md'),
  ]

  for (const line of builtLines) {
    args.push('--built-line', line)
  }
  for (const line of skippedLines) {
    args.push('--skipped-line', line)
  }

  run('node', args, { dryRun })
}

function stageRelease({
  tag,
  commit,
  stageDir,
  builtLines,
  skippedLines,
  dryRun,
  requireCompleteAppRelease,
}) {
  const assetPaths = collectReleaseAssetPaths(tag)
  const assetNames = assetPaths.map((assetPath) => basename(assetPath))
  validateReleaseAssetSet(assetNames, { requireCompleteAppRelease })

  if (dryRun) {
    console.log(`Would stage ${assetPaths.length} currently visible asset(s) into ${stageDir}`)
    return { assetPaths, stageDir }
  }

  if (assetPaths.length === 0) {
    throw new Error(`No dist assets found for ${tag}.`)
  }

  rmSync(stageDir, { recursive: true, force: true })
  mkdirSync(join(stageDir, 'assets'), { recursive: true })

  const stagedAssetPaths = []
  for (const assetPath of assetPaths) {
    const stagedPath = join(stageDir, 'assets', basename(assetPath))
    copyFileSync(assetPath, stagedPath)
    stagedAssetPaths.push(stagedPath)
  }

  const createdAt = Math.floor(Date.now() / 1000)
  const manifest = buildReleaseManifest({
    tag,
    commit,
    createdAt,
    assetPaths: stagedAssetPaths,
  })

  for (const [fileName, text] of buildReleaseManifestFiles(manifest)) {
    writeFileSync(join(stageDir, fileName), text)
  }
  writeReleaseNotes({ tag, commit, stageDir, builtLines, skippedLines, dryRun })

  return { assetPaths, stageDir }
}

function publishRelease({ stageDir, releaseTree, tag, dryRun }) {
  if (dryRun) {
    console.log(`Would publish ${tag} from ${stageDir} into ${releaseTree}`)
    return 'dry-run'
  }

  const addOutput = run('htree', ['add', stageDir], { capture: true, dryRun })
  const match = addOutput.match(/^\s*url:\s*(\S+)/m)
  if (!match) {
    throw new Error('Could not parse htree add output for release CID.')
  }

  const cid = match[1]
  run('htree', ['release', 'publish', releaseTree, tag, cid], { dryRun })
  return cid
}

function resolveReleaseCommit(tag, { dryRun = false } = {}) {
  const normalizedTag = normalizeTag(tag)
  if (dryRun) {
    return normalizedTag
  }

  const taggedResult = spawnSync('git', ['rev-parse', '-q', '--verify', `${normalizedTag}^{commit}`], {
    cwd: repoRoot,
    encoding: 'utf8',
    stdio: 'pipe',
  })
  if (taggedResult.status === 0) {
    const taggedCommit = taggedResult.stdout.trim()
    if (taggedCommit) {
      return taggedCommit
    }
  }

  return run('git', ['rev-parse', 'HEAD'], { capture: true, dryRun }) || 'HEAD'
}

function main() {
  const options = parseArgs(process.argv.slice(2))
  const { loaded, loadedPaths } = readOptionalEnvFiles([...defaultEnvFiles, ...options.envFiles])
  const env = { ...loaded, ...process.env }

  const tag = options.tag || readWorkspaceVersionTag(readFileSync(rootCargoToml, 'utf8'))
  const releaseTree = options.releaseTree || env.NVPN_RELEASE_TREE || 'releases/nostr-vpn'
  const stageDir =
    options.stageDir || join(os.tmpdir(), `nostr-vpn-release-${tag.replace(/[^\w.-]/g, '_')}`)
  const allowPartial = options.allowPartial || envFlagEnabled(env.NVPN_RELEASE_ALLOW_PARTIAL)
  const builtLines = []
  const skippedLines = []

  console.log(`Release tag: ${tag}`)
  console.log(`Release tree: ${releaseTree}`)
  if (loadedPaths.length > 0) {
    console.log(`Loaded env files: ${loadedPaths.join(', ')}`)
  }
  if (options.dryRun) {
    console.log('Dry run mode: no build, copy, or publish commands will be executed.')
  }

  const steps = [
    ['verify', () => runVerify({ dryRun: options.dryRun, builtLines })],
    ['macos', () => buildMacosArtifacts({ tag, dryRun: options.dryRun, builtLines })],
    ['android', () => buildAndroidArtifacts({ env, tag, dryRun: options.dryRun, builtLines })],
    ['linux', () => buildLinuxArtifacts({ env, tag, dryRun: options.dryRun, builtLines })],
    ['windows', () => buildWindowsArtifacts({ env, tag, dryRun: options.dryRun, builtLines })],
  ]

  for (const [name, fn] of steps) {
    if (!shouldRunStep(name, options)) {
      skippedLines.push(`${name} skipped by CLI options.`)
      continue
    }

    try {
      fn()
    } catch (error) {
      if (error instanceof SkipStepError) {
        skippedLines.push(error.message)
        continue
      }
      if (name === 'verify') {
        throw error
      }
      const failure = `${name} build failed: ${error.message}`
      skippedLines.push(failure)
      if (!allowPartial) {
        throw new Error(`${failure}\nPass --allow-partial or set NVPN_RELEASE_ALLOW_PARTIAL=1 to stage/publish without this artifact.`)
      }
    }
  }

  const commit = resolveReleaseCommit(tag, { dryRun: options.dryRun })
  stageRelease({
    tag,
    commit,
    stageDir,
    builtLines,
    skippedLines,
    dryRun: options.dryRun,
    requireCompleteAppRelease: !allowPartial && !options.dryRun,
  })

  if (options.publish) {
    if (!commandExists('htree')) {
      throw new Error('Missing htree; cannot publish release.')
    }
    const cid = publishRelease({ stageDir, releaseTree, tag, dryRun: options.dryRun })
    console.log(`Published ${tag} to ${releaseTree} via ${cid}`)
  } else if (!options.dryRun) {
    console.log(`Staged ${tag} at ${stageDir}`)
  }
}

try {
  main()
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error))
  process.exit(1)
}
