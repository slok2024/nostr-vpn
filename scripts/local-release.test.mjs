import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { tmpdir } from 'node:os'

import {
  androidReleaseAssetName,
  androidVersionCode,
  autoDetectWindowsVmName,
  buildReleaseManifestFiles,
  buildReleaseManifest,
  bumpAndroidGradleVersion,
  bumpCargoPackageVersion,
  bumpPbxprojMarketingVersion,
  describeAsset,
  extractChangelogSection,
  linuxReleaseTargetsForDockerPlatform,
  parseEnvFile,
  readWorkspaceVersionTag,
  renderReleaseNotes,
  semverFromTag,
  shouldBlockLocalLinuxAmd64Qemu,
  splitCsv,
  validateReleaseAssetSet,
} from './local-release-lib.mjs'

test('parseEnvFile reads basic dotenv syntax', () => {
  const parsed = parseEnvFile(`
# comment
NVPN_RELEASE_TREE=releases/nostr-vpn
NVPN_WINDOWS_VM_NAME="Windows 11"
NVPN_NOTE='line one'
INVALID KEY=nope
`)

  assert.deepEqual(parsed, {
    NVPN_RELEASE_TREE: 'releases/nostr-vpn',
    NVPN_WINDOWS_VM_NAME: 'Windows 11',
    NVPN_NOTE: 'line one',
  })
})

test('splitCsv trims and drops empties', () => {
  assert.deepEqual(splitCsv('verify, windows,android ,, macos'), [
    'verify',
    'windows',
    'android',
    'macos',
  ])
})

test('readWorkspaceVersionTag reads the workspace package version', () => {
  const tag = readWorkspaceVersionTag(`
[workspace]
members = []

[workspace.package]
version = "0.2.27"
`)

  assert.equal(tag, 'v0.2.27')
})

test('linuxReleaseTargetsForDockerPlatform maps Docker platforms to release targets', () => {
  assert.deepEqual(linuxReleaseTargetsForDockerPlatform('linux/arm64'), {
    linuxArchSuffix: 'arm64',
    muslTriple: 'aarch64-unknown-linux-musl',
  })
  assert.deepEqual(linuxReleaseTargetsForDockerPlatform('linux/arm64/v8'), {
    linuxArchSuffix: 'arm64',
    muslTriple: 'aarch64-unknown-linux-musl',
  })
  assert.deepEqual(linuxReleaseTargetsForDockerPlatform('linux/amd64'), {
    linuxArchSuffix: 'x64',
    muslTriple: 'x86_64-unknown-linux-musl',
  })
  assert.throws(
    () => linuxReleaseTargetsForDockerPlatform('linux/arm/v7'),
    /Unsupported Linux Docker architecture/,
  )
})

test('shouldBlockLocalLinuxAmd64Qemu protects Apple Silicon Docker Desktop releases', () => {
  assert.equal(
    shouldBlockLocalLinuxAmd64Qemu({
      platform: 'linux/amd64',
      hostPlatform: 'darwin',
      hostArch: 'arm64',
    }),
    true,
  )
  assert.equal(
    shouldBlockLocalLinuxAmd64Qemu({
      platform: 'linux/amd64',
      hostPlatform: 'linux',
      hostArch: 'x64',
    }),
    false,
  )
  assert.equal(
    shouldBlockLocalLinuxAmd64Qemu({
      platform: 'linux/arm64',
      hostPlatform: 'darwin',
      hostArch: 'arm64',
    }),
    false,
  )
})

test('validateReleaseAssetSet rejects ARM64-only Linux desktop releases', () => {
  assert.throws(
    () =>
      validateReleaseAssetSet([
        'nostr-vpn-v0.3.23-linux-arm64.AppImage',
        'nostr-vpn-v0.3.23-linux-arm64.deb',
      ]),
    /no Linux x64 desktop artifacts/,
  )
  assert.doesNotThrow(() =>
    validateReleaseAssetSet([
      'nostr-vpn-v0.3.23-linux-x64.AppImage',
      'nostr-vpn-v0.3.23-linux-arm64.AppImage',
    ]),
  )
  assert.doesNotThrow(() =>
    validateReleaseAssetSet(['nostr-vpn-v0.3.23-linux-arm64.AppImage'], {
      allowLinuxArm64DesktopOnly: true,
    }),
  )
})

test('validateReleaseAssetSet rejects macOS app zip releases', () => {
  assert.throws(
    () => validateReleaseAssetSet(['nostr-vpn-v4.0.1-macos-arm64.zip']),
    /macOS \.zip app archive/,
  )
  assert.throws(
    () => validateReleaseAssetSet(['nostr-vpn-v4.0.1-macos-arm64.dmg']),
    /no macOS \.app\.tar\.gz updater archive/,
  )
  assert.doesNotThrow(() =>
    validateReleaseAssetSet([
      'nostr-vpn-v4.0.1-macos-arm64.app.tar.gz',
      'nostr-vpn-v4.0.1-macos-arm64.dmg',
    ]),
  )
})

test('validateReleaseAssetSet rejects unsigned Android artifacts', () => {
  assert.throws(
    () => validateReleaseAssetSet(['nostr-vpn-v4.0.1-android-arm64-unsigned.apk']),
    /unsigned Android artifacts/,
  )
})

test('validateReleaseAssetSet can require complete app release artifacts', () => {
  assert.throws(
    () =>
      validateReleaseAssetSet([
        'nostr-vpn-v4.0.1-macos-arm64.app.tar.gz',
        'nostr-vpn-v4.0.1-macos-arm64.dmg',
      ], { requireCompleteAppRelease: true }),
    /Linux x64 desktop package, Windows x64 installer, signed Android APK/,
  )

  assert.doesNotThrow(() =>
    validateReleaseAssetSet([
      'nostr-vpn-v4.0.1-android-arm64.aab',
      'nostr-vpn-v4.0.1-android-arm64.apk',
      'nostr-vpn-v4.0.1-linux-x64.deb',
      'nostr-vpn-v4.0.1-macos-arm64.app.tar.gz',
      'nostr-vpn-v4.0.1-macos-arm64.dmg',
      'nostr-vpn-v4.0.1-windows-x64-setup.exe',
    ], { requireCompleteAppRelease: true }),
  )
})

test('autoDetectWindowsVmName returns the only running Windows VM', () => {
  const name = autoDetectWindowsVmName(`
UUID                                    STATUS       IP_ADDR         NAME
{1e553d3b-024e-4799-adb0-92127659f5dd}  running      -               Windows 11
`)

  assert.equal(name, 'Windows 11')
})

test('autoDetectWindowsVmName returns null when multiple Windows VMs match', () => {
  const name = autoDetectWindowsVmName(`
UUID                                    STATUS       IP_ADDR         NAME
{1}  running      -               Windows 11
{2}  running      -               Windows ARM
`)

  assert.equal(name, null)
})

test('describeAsset maps release filenames to readable labels', () => {
  assert.equal(
    describeAsset('nostr-vpn-v0.2.27-windows-x64-setup.exe'),
    'Windows x64 installer',
  )
  assert.equal(
    describeAsset('nvpn-v0.2.27-aarch64-pc-windows-msvc.zip'),
    'Windows ARM64 CLI',
  )
  assert.equal(
    describeAsset('nostr-vpn-v0.3.23-linux-arm64.AppImage'),
    'Linux ARM64 AppImage',
  )
  assert.equal(
    describeAsset('nostr-vpn-v0.3.23-linux-arm64.deb'),
    'Linux ARM64 Debian package',
  )
  assert.equal(
    describeAsset('nvpn-v0.3.23-aarch64-unknown-linux-musl.tar.gz'),
    'Linux ARM64 CLI (versioned)',
  )
})

test('androidReleaseAssetName formats signed and unsigned Android asset names', () => {
  assert.equal(androidReleaseAssetName('0.3.9'), 'nostr-vpn-v0.3.9-android-arm64.apk')
  assert.equal(
    androidReleaseAssetName('v0.3.9', { extension: 'aab', signed: false }),
    'nostr-vpn-v0.3.9-android-arm64-unsigned.aab',
  )
})

test('buildReleaseManifest records staged assets with sizes', () => {
  const root = mkdtempSync(join(tmpdir(), 'nostr-vpn-release-test-'))
  const assetsDir = join(root, 'assets')
  mkdirSync(assetsDir)
  const installer = join(assetsDir, 'nostr-vpn-v0.2.27-windows-x64-setup.exe')
  const cliZip = join(assetsDir, 'nvpn-v0.2.27-x86_64-pc-windows-msvc.zip')
  writeFileSync(installer, 'installer')
  writeFileSync(cliZip, 'zip')

  const manifest = buildReleaseManifest({
    tag: 'v0.2.27',
    commit: 'abc123',
    createdAt: 1774523304,
    assetPaths: [installer, cliZip],
  })

  assert.equal(manifest.assets.length, 2)
  assert.equal(manifest.assets[0].name, 'nostr-vpn-v0.2.27-windows-x64-setup.exe')
  assert.equal(manifest.assets[1].name, 'nvpn-v0.2.27-x86_64-pc-windows-msvc.zip')
  assert.equal(manifest.assets[0].path, 'assets/nostr-vpn-v0.2.27-windows-x64-setup.exe')
})

test('buildReleaseManifestFiles writes legacy manifest alias', () => {
  const manifest = {
    id: 'v0.3.23',
    assets: [{ name: 'nostr-vpn-v0.3.23-macos-arm64.app.tar.gz' }],
  }

  const files = buildReleaseManifestFiles(manifest)
  assert.deepEqual(files.map(([name]) => name), ['release.json', 'manifest.json'])
  assert.equal(files[0][1], files[1][1])
  assert.deepEqual(JSON.parse(files[0][1]), manifest)
})

test('extractChangelogSection returns the matching version body', () => {
  const section = extractChangelogSection(`
# Changelog

## Unreleased

## 0.3.0 - 2026-03-31

Changes since v0.2.28.

### Added

- Admin-managed rosters.

## 0.2.28 - 2026-03-26

- Previous release.
`, 'v0.3.0')

  assert.equal(
    section,
    'Changes since v0.2.28.\n\n### Added\n\n- Admin-managed rosters.',
  )
})

test('renderReleaseNotes includes changelog, built, and skipped sections', () => {
  const notes = renderReleaseNotes({
    tag: 'v0.2.27',
    commit: 'abc123',
    assetNames: [
      'nostr-vpn-v0.2.27-macos-arm64.app.tar.gz',
      'nostr-vpn-v0.2.27-macos-arm64.dmg',
      'nvpn-v0.2.27-x86_64-pc-windows-msvc.zip',
    ],
    changelogText: `
# Changelog

## 0.2.27 - 2026-03-25

Changes since v0.2.26.

### Fixed

- Release note formatting.
`,
    builtLines: ['Built Windows x64 CLI on windows-dev.'],
    skippedLines: ['Linux musl CLI skipped because cross was unavailable.'],
  })

  assert.match(notes, /## Changes/)
  assert.match(notes, /Changes since v0\.2\.26\./)
  assert.match(notes, /### Fixed/)
  assert.match(notes, /### Most People Will Want/)
  assert.match(notes, /### Command Line/)
  assert.match(notes, /Windows x64 CLI/)
  assert.match(notes, /Built Windows x64 CLI on windows-dev\./)
  assert.match(notes, /Linux musl CLI skipped because cross was unavailable\./)
})

test('renderReleaseNotes omits CLI skip boilerplate and can link assets', () => {
  const notes = renderReleaseNotes({
    tag: 'v0.3.0',
    commit: 'abc123',
    assetNames: [
      'nostr-vpn-v0.3.0-macos-arm64.app.tar.gz',
      'nostr-vpn-v0.3.0-macos-arm64.dmg',
    ],
    assetBaseUrl: 'https://github.com/mmalmi/nostr-vpn/releases/download/v0.3.0',
    skippedLines: [
      'verify skipped by CLI options.',
      'windows skipped by CLI options.',
    ],
  })

  assert.match(
    notes,
    /\[nostr-vpn-v0\.3\.0-macos-arm64\.dmg\]\(https:\/\/github\.com\/mmalmi\/nostr-vpn\/releases\/download\/v0\.3\.0\/nostr-vpn-v0\.3\.0-macos-arm64\.dmg\)/,
  )
  assert.doesNotMatch(notes, /verify skipped by CLI options/)
  assert.doesNotMatch(notes, /windows skipped by CLI options/)
})

test('renderReleaseNotes groups common app downloads before advanced files', () => {
  const notes = renderReleaseNotes({
    tag: 'v0.3.23',
    commit: 'abc123',
    assetNames: [
      'nostr-vpn-v0.3.23-android-arm64.aab',
      'nostr-vpn-v0.3.23-android-arm64.apk',
      'nostr-vpn-v0.3.23-linux-x64.AppImage',
      'nostr-vpn-v0.3.23-linux-x64.deb',
      'nostr-vpn-v0.3.23-macos-arm64.app.tar.gz',
      'nostr-vpn-v0.3.23-macos-arm64.dmg',
      'nostr-vpn-v0.3.23-windows-x64-setup.exe',
      'nvpn-aarch64-apple-darwin.tar.gz',
      'nvpn-v0.3.23-aarch64-apple-darwin.tar.gz',
      'nvpn-v0.3.23-x86_64-pc-windows-msvc.zip',
      'nvpn-v0.3.23-x86_64-unknown-linux-musl.tar.gz',
      'nvpn-x86_64-unknown-linux-musl.tar.gz',
    ],
  })

  assert.match(notes, /### Most People Will Want[\s\S]*Nostr VPN for macOS \(Apple Silicon\)/)
  assert.match(notes, /### Most People Will Want[\s\S]*Nostr VPN for Linux \(AppImage\)/)
  assert.match(notes, /### Most People Will Want[\s\S]*Nostr VPN for Windows/)
  assert.match(notes, /### Command Line[\s\S]*macOS Apple Silicon CLI: \[nvpn-aarch64-apple-darwin\.tar\.gz\]\(assets\/nvpn-aarch64-apple-darwin\.tar\.gz\)/)
  assert.match(notes, /### Command Line[\s\S]*Linux x64 CLI: \[nvpn-x86_64-unknown-linux-musl\.tar\.gz\]\(assets\/nvpn-x86_64-unknown-linux-musl\.tar\.gz\)/)
  assert.match(notes, /### Other Files[\s\S]*Android arm64 AAB/)
  assert.match(notes, /### Other Files[\s\S]*macOS Apple Silicon updater archive/)
  assert.doesNotMatch(notes, /nvpn-v0\.3\.23-aarch64-apple-darwin\.tar\.gz/)
  assert.doesNotMatch(notes, /nvpn-v0\.3\.23-x86_64-unknown-linux-musl\.tar\.gz/)
})

test('semverFromTag strips an optional v prefix', () => {
  assert.equal(semverFromTag('v4.0.6'), '4.0.6')
  assert.equal(semverFromTag('4.0.6'), '4.0.6')
  assert.throws(() => semverFromTag('4.0'), /semver-shaped/)
  assert.throws(() => semverFromTag('4.0.6-alpha'), /semver-shaped/)
})

test('androidVersionCode encodes semver into a monotonic integer', () => {
  assert.equal(androidVersionCode('4.0.6'), 40_006)
  assert.equal(androidVersionCode('4.0.10'), 40_010)
  assert.equal(androidVersionCode('4.10.0'), 41_000)
  assert.equal(androidVersionCode('5.0.0'), 50_000)
  assert.throws(() => androidVersionCode('4.100.0'), /minor\/patch < 100/)
})

test('bumpPbxprojMarketingVersion replaces every MARKETING_VERSION setting', () => {
  const input = `
\t\t\t\tDEVELOPMENT_TEAM = ABC123;
\t\t\t\tMARKETING_VERSION = 4.0.2;
\t\t\t\tPRODUCT_NAME = Nostr VPN;
\t\t\t\tMARKETING_VERSION = 4.0.2;
`
  const next = bumpPbxprojMarketingVersion(input, 'v4.0.6')
  assert.equal(
    next,
    input.replaceAll('MARKETING_VERSION = 4.0.2;', 'MARKETING_VERSION = 4.0.6;'),
  )
})

test('bumpAndroidGradleVersion bumps both versionCode and versionName', () => {
  const input = `
android {
    defaultConfig {
        versionCode = 40002
        versionName = "4.0.2"
    }
}
`
  const next = bumpAndroidGradleVersion(input, '4.0.6')
  assert.match(next, /versionCode = 40006/)
  assert.match(next, /versionName = "4\.0\.6"/)
  assert.doesNotMatch(next, /4\.0\.2/)
})

test('bumpCargoPackageVersion only touches [package] version', () => {
  const input = `
[package]
name = "nostr-vpn-linux"
version = "4.0.2"
edition = "2021"

[dependencies]
adw = { package = "libadwaita", version = "0.7" }
`
  const next = bumpCargoPackageVersion(input, '4.0.6')
  assert.match(next, /\[package\][\s\S]*version = "4\.0\.6"/)
  assert.match(next, /adw = \{ package = "libadwaita", version = "0\.7" \}/)
})
