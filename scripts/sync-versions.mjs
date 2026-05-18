#!/usr/bin/env node
// Single source of truth: [workspace.package].version in /Cargo.toml.
// Propagates that version to every other version-bearing file so all
// platforms stay in lockstep without manual bumps.
//
//   node scripts/sync-versions.mjs            # write (idempotent)
//   node scripts/sync-versions.mjs --check    # exit 1 if any file is stale

import { readFileSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), '..')

function readWorkspaceVersion() {
  const text = readFileSync(join(repoRoot, 'Cargo.toml'), 'utf8')
  const match = text.match(/^\[workspace\.package\][\s\S]*?^version\s*=\s*"([^"\n]+)"/m)
  if (!match) {
    throw new Error('Could not find [workspace.package] version in Cargo.toml')
  }
  return match[1].trim()
}

function androidVersionCode(version) {
  // Matches the historical packing in android/app/build.gradle.kts:
  // 4.0.2 -> 40002. Each component must fit in two digits.
  const core = version.split(/[-+]/, 1)[0]
  const parts = core.split('.').map((part) => parseInt(part, 10))
  if (parts.length === 0 || parts.some((value) => Number.isNaN(value))) {
    throw new Error(`Could not derive numeric version code from "${version}"`)
  }
  const [major = 0, minor = 0, patch = 0] = parts
  if (minor > 99 || patch > 99) {
    throw new Error(
      `versionCode formula needs an update for "${version}" (minor/patch > 99)`,
    )
  }
  return major * 10_000 + minor * 100 + patch
}

function versionTag(version) {
  return version.startsWith('v') ? version : `v${version}`
}

function makeTarget(relPath, transform) {
  return {
    relPath,
    apply(currentText, version) {
      return transform(currentText, version)
    },
  }
}

const targets = [
  makeTarget('linux/Cargo.toml', (text, version) =>
    text.replace(
      /^(version\s*=\s*")[^"\n]+(")/m,
      (_, prefix, suffix) => `${prefix}${version}${suffix}`,
    ),
  ),
  // ios/project.yml + macos/project.yml use plain ${NVPN_APP_VERSION_NAME} /
  // ${NVPN_APP_VERSION_CODE} substitution. Both env vars resolve from the
  // workspace version via release_common.sh's resolve_shared_build_metadata,
  // which is called by the entry-point scripts (tools/run-ios, scripts/ios-build,
  // scripts/macos-build) right before xcodegen runs. Nothing for sync-versions
  // to bump in project.yml itself — keeps a single source of truth.
  makeTarget('android/app/build.gradle.kts', (text, version) => {
    const code = androidVersionCode(version)
    return text
      .replace(
        /^(\s*versionCode\s*=\s*).+$/m,
        (_, prefix) => `${prefix}${code}`,
      )
      .replace(
        /^(\s*versionName\s*=\s*").+(")/m,
        (_, prefix, suffix) => `${prefix}${version}${suffix}`,
      )
  }),
  makeTarget('windows/NostrVpn.Windows/NostrVpn.Windows.csproj', (text, version) =>
    text.replace(
      /(<Version>)[^<]+(<\/Version>)/,
      (_, prefix, suffix) => `${prefix}${version}${suffix}`,
    ),
  ),
  makeTarget('umbrel/umbrel-app.yml', (text, version) =>
    text.replace(
      /^(version:\s*")[^"\n]+(")/m,
      (_, prefix, suffix) => `${prefix}${versionTag(version)}${suffix}`,
    ),
  ),
]

function main() {
  const checkOnly = process.argv.includes('--check')
  const version = readWorkspaceVersion()
  let stale = []
  let updated = []

  for (const target of targets) {
    const path = join(repoRoot, target.relPath)
    const before = readFileSync(path, 'utf8')
    const after = target.apply(before, version)
    if (after === before) continue
    if (checkOnly) {
      stale.push(target.relPath)
    } else {
      writeFileSync(path, after)
      updated.push(target.relPath)
    }
  }

  if (checkOnly) {
    if (stale.length === 0) {
      console.log(`Versions in sync at ${version}.`)
      return
    }
    console.error(
      `Versions out of sync with workspace ${version}:\n  - ${stale.join('\n  - ')}\n` +
        `Run \`node scripts/sync-versions.mjs\` to fix.`,
    )
    process.exit(1)
  }

  if (updated.length === 0) {
    console.log(`All version files already at ${version}.`)
  } else {
    console.log(`Synced ${updated.length} file(s) to ${version}:\n  - ${updated.join('\n  - ')}`)
  }
}

main()
