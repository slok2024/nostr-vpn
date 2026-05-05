#!/usr/bin/env node

import { existsSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { resolve } from 'node:path'

import {
  normalizeTag,
  renderReleaseNotes,
  splitCsv,
} from './local-release-lib.mjs'

function usage() {
  console.log(`Usage: node scripts/render-release-notes.mjs [options]

Options:
  --tag <tag>              Release tag
  --commit <sha>           Release commit
  --asset-dir <path>       Directory containing release assets
  --asset-base-url <url>   Base URL for asset links
  --changelog <path>       Changelog path
  --built-line <text>      Build summary line (repeatable)
  --skipped-line <text>    Skipped/missing line (repeatable)
  --out <path>             Output file (defaults to stdout)
  --help                   Show this help`)
}

function parseArgs(argv) {
  const options = {
    tag: '',
    commit: '',
    assetDir: '',
    assetBaseUrl: '',
    changelog: '',
    builtLines: [],
    skippedLines: [],
    out: '',
  }

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    switch (arg) {
      case '--help':
      case '-h':
        usage()
        process.exit(0)
      case '--tag':
        options.tag = normalizeTag(argv[++index] ?? '')
        break
      case '--commit':
        options.commit = argv[++index] ?? ''
        break
      case '--asset-dir':
        options.assetDir = resolve(argv[++index] ?? '')
        break
      case '--asset-base-url':
        options.assetBaseUrl = String(argv[++index] ?? '').replace(/\/+$/, '')
        break
      case '--changelog':
        options.changelog = resolve(argv[++index] ?? '')
        break
      case '--built-line':
        options.builtLines.push(argv[++index] ?? '')
        break
      case '--skipped-line':
        options.skippedLines.push(argv[++index] ?? '')
        break
      case '--skipped-lines':
        options.skippedLines.push(...splitCsv(argv[++index] ?? ''))
        break
      case '--out':
        options.out = resolve(argv[++index] ?? '')
        break
      default:
        throw new Error(`Unknown argument: ${arg}`)
    }
  }

  if (!options.tag) {
    throw new Error('--tag is required')
  }
  if (!options.assetDir) {
    throw new Error('--asset-dir is required')
  }

  return options
}

function readAssetNames(assetDir) {
  if (!existsSync(assetDir)) {
    throw new Error(`Asset directory does not exist: ${assetDir}`)
  }

  return readdirSync(assetDir, { withFileTypes: true })
    .filter((entry) => entry.isFile())
    .map((entry) => entry.name)
}

function main() {
  const options = parseArgs(process.argv.slice(2))
  const notes = renderReleaseNotes({
    tag: options.tag,
    commit: options.commit,
    assetNames: readAssetNames(options.assetDir),
    assetBaseUrl: options.assetBaseUrl,
    changelogText: options.changelog && existsSync(options.changelog)
      ? readFileSync(options.changelog, 'utf8')
      : '',
    builtLines: options.builtLines.filter(Boolean),
    skippedLines: options.skippedLines.filter(Boolean),
  })

  if (options.out) {
    writeFileSync(options.out, notes)
  } else {
    process.stdout.write(notes)
  }
}

try {
  main()
} catch (error) {
  console.error(error.message)
  process.exit(1)
}
