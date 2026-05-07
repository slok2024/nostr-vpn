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
  autoDetectWindowsVmName,
  buildReleaseManifest,
  buildReleaseManifestFiles,
  linuxReleaseTargetsForDockerPlatform,
  normalizeTag,
  parseEnvFile,
  readWorkspaceVersionTag,
  renderReleaseNotes,
  splitCsv,
  validateReleaseAssetSet,
} from './local-release-lib.mjs'

const __dirname = dirname(fileURLToPath(import.meta.url))
const repoRoot = resolve(__dirname, '..')
const rootCargoToml = join(repoRoot, 'Cargo.toml')
const changelogPath = join(repoRoot, 'CHANGELOG.md')
const distDir = join(repoRoot, 'dist')
const defaultEnvFiles = [join(repoRoot, '.env.release.local')]
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
  --only <csv>             Limit steps to verify,macos,linux,windows
  --skip <csv>             Skip steps by name
  --allow-partial          Stage/publish even if a selected platform build fails
  --help                   Show this help

The script auto-loads .env.release.local when present. Shell environment
variables override values from that file.`)
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
    `nvpn - Nostr-signaled WireGuard control plane
============================================

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

function defaultSharedWindowsRepoPath() {
  if (process.platform !== 'darwin') {
    return null
  }

  const homeDir = os.homedir()
  if (!repoRoot.startsWith(`${homeDir}/`)) {
    return null
  }

  const relative = repoRoot.slice(homeDir.length + 1).split('/').join('\\')
  return `C:\\Mac\\Home\\${relative}`
}

function psQuote(value) {
  return `'${String(value).replace(/'/g, "''")}'`
}

function encodePowerShellScript(script) {
  return Buffer.from(script, 'utf16le').toString('base64')
}

function runWindowsPowerShell(vmName, script, { capture = false, dryRun = false } = {}) {
  const encoded = encodePowerShellScript(script)
  return run(
    'prlctl',
    ['exec', vmName, '--current-user', 'powershell.exe', '-NoProfile', '-EncodedCommand', encoded],
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

function syncRepoToWindowsVm({ vmName, sharedRepoPath, dryRun }) {
  runWindowsPowerShell(
    vmName,
    `
$sharedRepo = ${psQuote(sharedRepoPath)}
$guestRepo = Join-Path $env:USERPROFILE 'src\\nostr-vpn'
$guestRoot = Split-Path $guestRepo
New-Item -ItemType Directory -Force -Path $guestRoot | Out-Null
robocopy $sharedRepo $guestRepo /MIR /XD target dist .git artifacts /XF .env.release.local | Out-Null
`,
    { dryRun },
  )
}

function buildWindowsArtifacts({ env, tag, dryRun, builtLines }) {
  if (process.platform !== 'darwin') {
    throw new SkipStepError('Windows VM builds are only wired up for the macOS + Parallels workflow.')
  }
  if (!commandExists('prlctl')) {
    throw new SkipStepError('Skipping Windows CLI artifacts because prlctl is unavailable.')
  }

  const sharedRepoPath = env.NVPN_WINDOWS_SHARED_REPO_PATH || defaultSharedWindowsRepoPath()
  if (!sharedRepoPath) {
    throw new SkipStepError('Skipping Windows CLI artifacts because the shared repo path could not be derived; set NVPN_WINDOWS_SHARED_REPO_PATH.')
  }

  const vmName =
    env.NVPN_WINDOWS_VM_NAME ||
    autoDetectWindowsVmName(run('prlctl', ['list', '-a'], { capture: true, dryRun }))
  if (!vmName) {
    throw new SkipStepError('Skipping Windows CLI artifacts because no unique running Windows VM was detected; set NVPN_WINDOWS_VM_NAME.')
  }

  syncRepoToWindowsVm({ vmName, sharedRepoPath, dryRun })

  const llvmBin = env.NVPN_WINDOWS_LLVM_BIN || 'C:\\Program Files\\LLVM\\bin'
  const targets = splitCsv(
    env.NVPN_WINDOWS_CLI_TARGETS || 'x86_64-pc-windows-msvc,aarch64-pc-windows-msvc',
  )
  const guestRepo = "(Join-Path $env:USERPROFILE 'src\\nostr-vpn')"
  const distPath = `${sharedRepoPath}\\dist`
  const pathSetup = `$env:PATH = ${psQuote(llvmBin)} + ';' + $env:PATH`

  for (const target of targets) {
    const archiveName = `nvpn-${tag}-${target}.zip`
    runWindowsPowerShell(
      vmName,
      `
${pathSetup}
Set-Location ${guestRepo}
cargo build --release --target ${psQuote(target)} -p nostr-vpn-cli
$cli = Join-Path ${guestRepo} ${psQuote(`target\\${target}\\release\\nvpn.exe`)}
if (!(Test-Path $cli)) { throw "Missing nvpn.exe for ${target}" }
$tempDir = Join-Path $env:TEMP ${psQuote(`nvpn-${target}-zip`)}
Remove-Item -Recurse -Force $tempDir -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $tempDir | Out-Null
Copy-Item $cli (Join-Path $tempDir 'nvpn.exe')
Compress-Archive -Path (Join-Path $tempDir '*') -DestinationPath ${psQuote(`${distPath}\\${archiveName}`)} -Force
Remove-Item -Recurse -Force $tempDir
`,
      { dryRun },
    )
    builtLines.push(`Built Windows ${windowsArtifactArch(target)} CLI inside Parallels VM ${vmName}.`)
  }
}

function buildLinuxArtifacts({ env, tag, dryRun, builtLines }) {
  if (!commandExists('docker')) {
    throw new SkipStepError('Skipping Linux CLI artifacts because docker is not on PATH.')
  }

  const platform = env.NVPN_LINUX_DOCKER_PLATFORM || 'linux/amd64'
  const { linuxArchSuffix, muslTriple } = linuxReleaseTargetsForDockerPlatform(platform)
  const imageName = 'nostr-vpn-linux-release'
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
    `cargo build --release --target ${muslTriple} -p nostr-vpn-cli`,
    'rm -rf /work/dist/nvpn',
    'mkdir -p /work/dist/nvpn',
    `cp target/${muslTriple}/release/nvpn /work/dist/nvpn/`,
    "printf '%s\\n' '#!/bin/bash' 'set -e' 'install -d \"${1:-/usr/local/bin}\"' 'install -m 755 nvpn \"${1:-/usr/local/bin}/\"' > /work/dist/nvpn/install.sh",
    'chmod +x /work/dist/nvpn/install.sh',
    "printf '%s\\n' 'nvpn - Nostr-signaled WireGuard control plane' > /work/dist/nvpn/README.txt",
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

  builtLines.push(`Built Linux ${linuxArchSuffix} musl CLI in Docker (${platform}).`)
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
  }
  run('bash', [join(repoRoot, 'scripts', 'macos-build'), 'macos-build'], { env, dryRun })

  packageUnixCliTarball({
    binaryPath: join(cargoTargetDir(env), 'aarch64-apple-darwin', 'release', 'nvpn'),
    targetTriple: 'aarch64-apple-darwin',
    tag,
    dryRun,
  })
  builtLines.push('Built Apple Silicon CLI locally.')

  const productsDir = join(repoRoot, 'macos', '.build', 'DerivedData', 'Build', 'Products', 'Release')
  const appPath = existsSync(productsDir)
    ? readdirSync(productsDir)
        .map((entry) => join(productsDir, entry))
        .find((entry) => entry.endsWith('.app'))
    : null
  if (!dryRun && !appPath) {
    throw new Error(`No native macOS app bundle found under ${productsDir}.`)
  }

  if (appPath) {
    const zipPath = join(distDir, `nostr-vpn-${tag}-macos-arm64.zip`)
    rmSync(zipPath, { force: true })
    run('ditto', ['-c', '-k', '--sequesterRsrc', '--keepParent', appPath, zipPath], { dryRun })
  }
  builtLines.push('Built native Apple Silicon macOS app locally.')
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

function stageRelease({ tag, commit, stageDir, builtLines, skippedLines, dryRun }) {
  const assetPaths = collectReleaseAssetPaths(tag)
  const assetNames = assetPaths.map((assetPath) => basename(assetPath))
  validateReleaseAssetSet(assetNames)

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
  writeFileSync(
    join(stageDir, 'notes.md'),
    renderReleaseNotes({
      tag,
      commit,
      assetNames: stagedAssetPaths.map((assetPath) => basename(assetPath)),
      builtLines,
      skippedLines,
      changelogText: existsSync(changelogPath) ? readFileSync(changelogPath, 'utf8') : '',
    }),
  )

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
