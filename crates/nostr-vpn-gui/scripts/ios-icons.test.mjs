import assert from 'node:assert/strict'
import { readFile, readdir } from 'node:fs/promises'
import path from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const sourceDir = path.resolve(scriptDir, '../src-tauri/icons/ios')
const bundledDir = path.resolve(
  scriptDir,
  '../src-tauri/gen/apple/Assets.xcassets/AppIcon.appiconset',
)
const iosInfoPlistPath = path.resolve(
  scriptDir,
  '../src-tauri/gen/apple/nostr-vpn-gui_iOS/Info.plist',
)
const iosRustBuildScriptPath = path.resolve(
  scriptDir,
  '../src-tauri/gen/apple/scripts/build-rust-staticlib.sh',
)
const cargoManifestPath = path.resolve(scriptDir, '../src-tauri/Cargo.toml')

const listPngs = async (dir) =>
  (await readdir(dir))
    .filter((entry) => entry.endsWith('.png'))
    .sort((left, right) => left.localeCompare(right))

const hasPngAlphaChannel = (bytes) => {
  const pngSignature = '89504e470d0a1a0a'
  assert.equal(bytes.subarray(0, 8).toString('hex'), pngSignature, 'expected PNG image')
  assert.equal(bytes.subarray(12, 16).toString('ascii'), 'IHDR', 'expected PNG IHDR chunk')
  return bytes[25] === 4 || bytes[25] === 6
}

test('bundled iOS AppIcon catalog matches the committed iOS source icons', async () => {
  const [sourceFiles, bundledFiles] = await Promise.all([listPngs(sourceDir), listPngs(bundledDir)])

  assert.deepEqual(
    bundledFiles,
    sourceFiles,
    'generated iOS app icon filenames drifted from src-tauri/icons/ios',
  )

  await Promise.all(
    sourceFiles.map(async (file) => {
      const [sourceBytes, bundledBytes] = await Promise.all([
        readFile(path.join(sourceDir, file)),
        readFile(path.join(bundledDir, file)),
      ])

      assert.deepEqual(
        bundledBytes,
        sourceBytes,
        `generated iOS app icon ${file} differs from src-tauri/icons/ios/${file}`,
      )
      assert.equal(hasPngAlphaChannel(sourceBytes), false, `iOS app icon ${file} has an alpha channel`)
    }),
  )
})

test('generated iOS Info.plist registers nvpn invite links', async () => {
  const infoPlist = await readFile(iosInfoPlistPath, 'utf8')

  assert.match(infoPlist, /<key>CFBundleURLTypes<\/key>/)
  assert.match(infoPlist, /<string>nvpn<\/string>/)
})

test('iOS builds enable Tauri custom protocol assets', async () => {
  const [buildScript, cargoManifest] = await Promise.all([
    readFile(iosRustBuildScriptPath, 'utf8'),
    readFile(cargoManifestPath, 'utf8'),
  ])

  assert.match(cargoManifest, /\[features\][\s\S]*custom-protocol = \["tauri\/custom-protocol"\]/)
  assert.match(
    buildScript,
    /cargo_args=\(build -p nostr-vpn-gui --target "\$\{rust_target\}" --features custom-protocol\)/,
  )
  assert.match(
    buildScript,
    /cargo_args\+=\(--release\)/,
  )
  assert.match(buildScript, /cargo "\$\{cargo_args\[@\]\}"/)
})
