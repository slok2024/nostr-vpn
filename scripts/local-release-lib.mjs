import { statSync } from 'node:fs'
import { basename } from 'node:path'

export function parseEnvFile(text) {
  const values = {}
  for (const rawLine of text.split(/\r?\n/)) {
    const line = rawLine.trim()
    if (!line || line.startsWith('#')) {
      continue
    }

    const separator = line.indexOf('=')
    if (separator <= 0) {
      continue
    }

    const key = line.slice(0, separator).trim()
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(key)) {
      continue
    }

    let value = line.slice(separator + 1).trim()
    if (
      (value.startsWith('"') && value.endsWith('"')) ||
      (value.startsWith("'") && value.endsWith("'"))
    ) {
      value = value.slice(1, -1)
    }

    value = value
      .replace(/\\n/g, '\n')
      .replace(/\\r/g, '\r')
      .replace(/\\t/g, '\t')

    values[key] = value
  }

  return values
}

export function splitCsv(value) {
  return (value || '')
    .split(',')
    .map((part) => part.trim())
    .filter(Boolean)
}

export function linuxReleaseTargetsForDockerPlatform(platform) {
  const normalized = String(platform || '').trim()
  const match = normalized.match(/^linux\/([^/]+)(?:\/[^/]+)?$/)
  if (!match) {
    throw new Error(`Unsupported Linux Docker platform: ${normalized || '<empty>'}`)
  }

  const dockerArch = match[1]
  if (dockerArch === 'arm64' || dockerArch === 'aarch64') {
    return {
      linuxArchSuffix: 'arm64',
      muslTriple: 'aarch64-unknown-linux-musl',
    }
  }

  if (dockerArch === 'amd64' || dockerArch === 'x86_64') {
    return {
      linuxArchSuffix: 'x64',
      muslTriple: 'x86_64-unknown-linux-musl',
    }
  }

  throw new Error(`Unsupported Linux Docker architecture: ${dockerArch}`)
}

export function shouldBlockLocalLinuxAmd64Qemu({ platform, hostPlatform, hostArch, allowQemu = false }) {
  if (allowQemu) {
    return false
  }

  return platform === 'linux/amd64' && hostPlatform === 'darwin' && hostArch === 'arm64'
}

export function validateReleaseAssetSet(
  assetNames,
  { allowLinuxArm64DesktopOnly = false, requireCompleteAppRelease = false } = {},
) {
  const names = [...assetNames]
  const hasMacosZip = names.some((name) => /^nostr-vpn-.*-macos-arm64\.zip$/.test(name))
  const hasMacosDmg = names.some((name) => /^nostr-vpn-.*-macos-arm64\.dmg$/.test(name))
  const hasMacosUpdater = names.some((name) => /^nostr-vpn-.*-macos-arm64\.app\.tar\.gz$/.test(name))
  const hasLinuxX64Desktop = names.some((name) => /^nostr-vpn-.*-linux-x64\.(AppImage|deb)$/.test(name))
  const hasLinuxArm64Desktop = names.some((name) => /^nostr-vpn-.*-linux-arm64\.(AppImage|deb)$/.test(name))
  const hasWindowsX64Setup = names.some((name) => /^nostr-vpn-.*-windows-x64-setup\.exe$/.test(name))
  const hasSignedAndroidApk = names.some((name) => /^nostr-vpn-.*-android-arm64\.apk$/.test(name))
  const hasUnsignedAndroid = names.some((name) => /^nostr-vpn-.*-android-arm64-unsigned\.(apk|aab)$/.test(name))

  if (hasMacosZip) {
    throw new Error(
      'Release includes a macOS .zip app archive. Ship a signed/notarized .dmg for users and a signed/notarized .app.tar.gz for the updater instead.',
    )
  }

  if (hasMacosDmg && !hasMacosUpdater) {
    throw new Error(
      'Release includes a macOS .dmg but no macOS .app.tar.gz updater archive.',
    )
  }

  if (hasLinuxArm64Desktop && !hasLinuxX64Desktop && !allowLinuxArm64DesktopOnly) {
    throw new Error(
      'Release has Linux ARM64 desktop artifacts but no Linux x64 desktop artifacts. Build Linux x64 on a native amd64 builder, remove the ARM64 desktop artifacts, or set NVPN_ALLOW_LINUX_ARM64_DESKTOP_ONLY=1.',
    )
  }

  if (hasUnsignedAndroid) {
    throw new Error(
      'Release includes unsigned Android artifacts. Configure Android signing for public releases.',
    )
  }

  if (requireCompleteAppRelease) {
    const missing = []
    if (!hasMacosDmg) {
      missing.push('macOS DMG')
    }
    if (!hasMacosUpdater) {
      missing.push('macOS updater archive')
    }
    if (!hasLinuxX64Desktop) {
      missing.push('Linux x64 desktop package')
    }
    if (!hasWindowsX64Setup) {
      missing.push('Windows x64 installer')
    }
    if (!hasSignedAndroidApk) {
      missing.push('signed Android APK')
    }
    if (missing.length > 0) {
      throw new Error(`Release is missing required app artifact(s): ${missing.join(', ')}.`)
    }
  }
}

export function readWorkspaceVersionTag(cargoTomlText) {
  const match = cargoTomlText.match(
    /^\[workspace\.package\][\s\S]*?^version\s*=\s*"([^"\n]+)"/m,
  )
  if (!match) {
    throw new Error('Could not find [workspace.package] version in Cargo.toml')
  }

  return normalizeTag(match[1])
}

export function normalizeTag(value) {
  if (!value || !value.trim()) {
    throw new Error('Release tag must not be empty')
  }

  return value.startsWith('v') ? value : `v${value}`
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

export function extractChangelogSection(changelogText, tag) {
  const version = normalizeTag(tag).replace(/^v/, '')
  const headingPattern = new RegExp(`^##\\s+${escapeRegExp(version)}(?:\\s+-\\s+.*)?\\s*$`, 'm')
  const headingMatch = changelogText.match(headingPattern)
  if (!headingMatch || headingMatch.index == null) {
    return null
  }

  const sectionStart = headingMatch.index + headingMatch[0].length
  const remainder = changelogText.slice(sectionStart).replace(/^\r?\n/, '')
  const nextHeadingMatch = remainder.match(/^##\s+/m)
  const section = nextHeadingMatch ? remainder.slice(0, nextHeadingMatch.index) : remainder
  const trimmed = section.trim()
  return trimmed || null
}

/**
 * "v4.0.6" / "4.0.6" → "4.0.6". Throws on malformed input.
 */
export function semverFromTag(tag) {
  const stripped = normalizeTag(tag).replace(/^v/, '')
  if (!/^\d+\.\d+\.\d+$/.test(stripped)) {
    throw new Error(`Release tag must be a semver-shaped string, got "${tag}"`)
  }
  return stripped
}

/**
 * Encode a semver as a 5-digit-major-friendly Android versionCode:
 *   "4.0.6"  → 40006
 *   "4.10.6" → 40_10_06 = 41006
 * Each component is allotted two digits; minor/patch must be < 100.
 */
export function androidVersionCode(version) {
  const parts = semverFromTag(version).split('.').map(Number)
  if (parts.some((value, index) => index > 0 && value >= 100)) {
    throw new Error(`Android versionCode encoding requires minor/patch < 100, got ${version}`)
  }
  const [major, minor, patch] = parts
  return major * 10_000 + minor * 100 + patch
}

/**
 * Replace every `MARKETING_VERSION = X.Y.Z;` in an Xcode pbxproj. Returns the
 * updated text. Both Debug and Release configs share the same setting in our
 * project, so a global replace is the right scope.
 */
export function bumpPbxprojMarketingVersion(pbxprojText, version) {
  const semver = semverFromTag(version)
  return pbxprojText.replace(/(\bMARKETING_VERSION\s*=\s*)[^;]+(;)/g, `$1${semver}$2`)
}

/**
 * Sync `versionCode = N` and `versionName = "X.Y.Z"` in an Android
 * build.gradle.kts to match the workspace version.
 */
export function bumpAndroidGradleVersion(gradleText, version) {
  const semver = semverFromTag(version)
  const code = androidVersionCode(semver)
  return gradleText
    .replace(/(\bversionCode\s*=\s*)\d+/g, `$1${code}`)
    .replace(/(\bversionName\s*=\s*")[^"]+(")/g, `$1${semver}$2`)
}

/**
 * Replace `version = "X.Y.Z"` inside the first `[package]` table of a
 * Cargo.toml. Used for the `linux/` crate which is excluded from the workspace
 * (so workspace `[workspace.package].version` doesn't reach it).
 */
export function bumpCargoPackageVersion(cargoTomlText, version) {
  const semver = semverFromTag(version)
  const match = cargoTomlText.match(/^\[package\]\s*\n([\s\S]*?)(?=^\[)/m)
  if (!match) {
    throw new Error('Could not find [package] table in Cargo.toml')
  }
  const original = match[0]
  if (!/(\nversion\s*=\s*")[^"]+(")/.test(original)) {
    throw new Error('Could not find version field inside [package] table')
  }
  const replaced = original.replace(/(\nversion\s*=\s*")[^"]+(")/, `$1${semver}$2`)
  return cargoTomlText.replace(original, replaced)
}

export function autoDetectWindowsVmName(prlctlListOutput) {
  const candidates = []
  for (const line of prlctlListOutput.split(/\r?\n/)) {
    const trimmed = line.trim()
    if (!trimmed.startsWith('{')) {
      continue
    }

    const match = trimmed.match(/^\{[^}]+\}\s+(\S+)\s+\S+\s+(.+)$/)
    if (!match) {
      continue
    }

    const status = match[1].toLowerCase()
    const name = match[2].trim()
    if ((status === 'running' || status === 'suspended') && /windows/i.test(name)) {
      candidates.push(name)
    }
  }

  return candidates.length === 1 ? candidates[0] : null
}

export function describeAsset(name) {
  if (/^nostr-vpn-.*-macos-arm64\.zip$/.test(name)) {
    return 'macOS Apple Silicon legacy app archive'
  }
  if (/^nostr-vpn-.*-macos-arm64\.dmg$/.test(name)) {
    return 'macOS Apple Silicon disk image'
  }
  if (/^nostr-vpn-.*-macos-arm64\.app\.tar\.gz$/.test(name)) {
    return 'macOS Apple Silicon updater archive'
  }
  if (/^nostr-vpn-.*-linux-x64\.AppImage$/.test(name)) {
    return 'Linux x64 AppImage'
  }
  if (/^nostr-vpn-.*-linux-x64\.deb$/.test(name)) {
    return 'Linux x64 Debian package'
  }
  if (/^nostr-vpn-.*-linux-arm64\.AppImage$/.test(name)) {
    return 'Linux ARM64 AppImage'
  }
  if (/^nostr-vpn-.*-linux-arm64\.deb$/.test(name)) {
    return 'Linux ARM64 Debian package'
  }
  if (/^nostr-vpn-.*-windows-x64-setup\.exe$/.test(name)) {
    return 'Windows x64 installer'
  }
  if (/^nostr-vpn-.*-windows-arm64-setup\.exe$/.test(name)) {
    return 'Windows ARM64 installer'
  }
  if (/^nostr-vpn-.*-android-arm64(?:-unsigned)?\.apk$/.test(name)) {
    return name.includes('-unsigned.') ? 'Android arm64 APK (unsigned)' : 'Android arm64 APK'
  }
  if (/^nostr-vpn-.*-android-arm64(?:-unsigned)?\.aab$/.test(name)) {
    return name.includes('-unsigned.') ? 'Android arm64 AAB (unsigned)' : 'Android arm64 AAB'
  }
  if (/^nvpn-.*-aarch64-apple-darwin\.tar\.gz$/.test(name)) {
    return name.startsWith('nvpn-v') ? 'Apple Silicon CLI (versioned)' : 'Apple Silicon CLI'
  }
  if (/^nvpn-.*-x86_64-unknown-linux-musl\.tar\.gz$/.test(name)) {
    return name.startsWith('nvpn-v') ? 'Linux x64 CLI (versioned)' : 'Linux x64 CLI'
  }
  if (/^nvpn-.*-aarch64-unknown-linux-musl\.tar\.gz$/.test(name)) {
    return name.startsWith('nvpn-v') ? 'Linux ARM64 CLI (versioned)' : 'Linux ARM64 CLI'
  }
  if (/^nvpn-.*-x86_64-pc-windows-msvc\.zip$/.test(name)) {
    return 'Windows x64 CLI'
  }
  if (/^nvpn-.*-aarch64-pc-windows-msvc\.zip$/.test(name)) {
    return 'Windows ARM64 CLI'
  }

  return name
}

function firstMatchingAsset(assetNames, patterns) {
  for (const pattern of patterns) {
    const name = assetNames.find((assetName) => pattern.test(assetName))
    if (name) {
      return name
    }
  }
  return null
}

function assetReference(name, assetBaseUrl = '') {
  if (assetBaseUrl) {
    return `[${name}](${assetBaseUrl}/${encodeURIComponent(name)})`
  }
  return `[${name}](assets/${encodeURIComponent(name)})`
}

function pushAssetLine(lines, usedAssets, assetNames, label, patterns, assetBaseUrl = '') {
  const name = firstMatchingAsset(assetNames, patterns)
  if (!name) {
    return null
  }

  usedAssets.add(name)
  lines.push(`- ${label}: ${assetReference(name, assetBaseUrl)}`)
  return name
}

function markMatchingAssetsUsed(usedAssets, assetNames, patterns) {
  for (const name of assetNames) {
    if (patterns.some((pattern) => pattern.test(name))) {
      usedAssets.add(name)
    }
  }
}

function pushDownloadSections(lines, assetNames, assetBaseUrl = '') {
  const sortedNames = [...assetNames].sort((left, right) => left.localeCompare(right))
  const usedAssets = new Set()

  lines.push('## Downloads', '', '### Most People Will Want', '')

  pushAssetLine(lines, usedAssets, sortedNames, 'Nostr VPN for macOS (Apple Silicon)', [
    /^nostr-vpn-.*-macos-arm64\.dmg$/,
  ], assetBaseUrl)
  pushAssetLine(lines, usedAssets, sortedNames, 'Nostr VPN for Linux (AppImage)', [
    /^nostr-vpn-.*-linux-x64\.AppImage$/,
  ], assetBaseUrl)
  pushAssetLine(lines, usedAssets, sortedNames, 'Nostr VPN for Debian/Ubuntu (.deb)', [
    /^nostr-vpn-.*-linux-x64\.deb$/,
  ], assetBaseUrl)
  pushAssetLine(lines, usedAssets, sortedNames, 'Nostr VPN for Windows', [
    /^nostr-vpn-.*-windows-x64-setup\.exe$/,
  ], assetBaseUrl)
  pushAssetLine(lines, usedAssets, sortedNames, 'Nostr VPN for Android', [
    /^nostr-vpn-.*-android-arm64\.apk$/,
  ], assetBaseUrl)

  const cliLines = []
  const addCliAsset = (label, preferredPatterns, duplicatePatterns = preferredPatterns) => {
    const name = firstMatchingAsset(sortedNames, preferredPatterns)
    if (!name) {
      return
    }
    usedAssets.add(name)
    markMatchingAssetsUsed(usedAssets, sortedNames, duplicatePatterns)
    cliLines.push(`- ${label}: ${assetReference(name, assetBaseUrl)}`)
  }

  addCliAsset('macOS Apple Silicon CLI', [
    /^nvpn-aarch64-apple-darwin\.tar\.gz$/,
    /^nvpn-v.*-aarch64-apple-darwin\.tar\.gz$/,
  ], [/^nvpn(?:-v.*)?-aarch64-apple-darwin\.tar\.gz$/])
  addCliAsset('Linux x64 CLI', [
    /^nvpn-x86_64-unknown-linux-musl\.tar\.gz$/,
    /^nvpn-v.*-x86_64-unknown-linux-musl\.tar\.gz$/,
  ], [/^nvpn(?:-v.*)?-x86_64-unknown-linux-musl\.tar\.gz$/])
  addCliAsset('Linux ARM64 CLI', [
    /^nvpn-aarch64-unknown-linux-musl\.tar\.gz$/,
    /^nvpn-v.*-aarch64-unknown-linux-musl\.tar\.gz$/,
  ], [/^nvpn(?:-v.*)?-aarch64-unknown-linux-musl\.tar\.gz$/])
  addCliAsset('Windows x64 CLI', [/^nvpn-v.*-x86_64-pc-windows-msvc\.zip$/])
  addCliAsset('Windows ARM64 CLI', [/^nvpn-v.*-aarch64-pc-windows-msvc\.zip$/])

  if (cliLines.length > 0) {
    lines.push('', '### Command Line', '', ...cliLines)
  }

  const otherLines = []
  for (const name of sortedNames) {
    if (usedAssets.has(name)) {
      continue
    }
    otherLines.push(`- ${describeAsset(name)}: ${assetReference(name, assetBaseUrl)}`)
  }

  if (otherLines.length > 0) {
    lines.push('', '### Other Files', '', ...otherLines)
  }
}

export function androidReleaseAssetName(tag, { extension = 'apk', signed = true } = {}) {
  const normalizedTag = normalizeTag(tag)
  const suffix = signed ? '' : '-unsigned'
  return `nostr-vpn-${normalizedTag}-android-arm64${suffix}.${extension}`
}

export function buildReleaseManifest({ tag, commit, createdAt, assetPaths }) {
  const normalizedTag = normalizeTag(tag)
  const assets = [...assetPaths]
    .map((assetPath) => ({
      name: basename(assetPath),
      path: `assets/${basename(assetPath)}`,
      size: statSync(assetPath).size,
    }))
    .sort((left, right) => left.name.localeCompare(right.name))

  return {
    id: normalizedTag,
    title: normalizedTag,
    tag: normalizedTag,
    commit,
    created_at: createdAt,
    published_at: createdAt,
    draft: false,
    prerelease: normalizedTag.includes('-'),
    notes_file: 'notes.md',
    assets,
  }
}

export function buildReleaseManifestFiles(manifest) {
  const text = `${JSON.stringify(manifest, null, 2)}\n`
  return [
    ['release.json', text],
    // Older desktop updater builds used manifest.json during
    // install even though checks read release.json. Keep both names identical
    // so old installed apps can update into a fixed build.
    ['manifest.json', text],
  ]
}

export function renderReleaseNotes({
  tag,
  commit,
  assetNames,
  builtLines = [],
  skippedLines = [],
  changelogText = '',
  assetBaseUrl = '',
}) {
  const normalizedTag = normalizeTag(tag)
  const lines = []
  const changelogSection = extractChangelogSection(changelogText, normalizedTag)
  const visibleSkippedLines = skippedLines.filter((line) => !line.endsWith('skipped by CLI options.'))

  pushDownloadSections(lines, assetNames, assetBaseUrl)

  if (changelogSection) {
    lines.push('', '## Changes', '', ...changelogSection.split('\n'), '')
  }

  if (commit || builtLines.length > 0) {
    lines.push('', '## Release Build', '')
    if (commit) {
      lines.push(`- Built from commit \`${commit}\` for release \`${normalizedTag}\`.`)
    }
  }

  for (const line of builtLines) {
    lines.push(`- ${line}`)
  }

  if (visibleSkippedLines.length > 0) {
    lines.push('', '## Skipped or Not Built', '')
    for (const line of visibleSkippedLines) {
      lines.push(`- ${line}`)
    }
  }

  return `${lines.join('\n')}\n`
}
