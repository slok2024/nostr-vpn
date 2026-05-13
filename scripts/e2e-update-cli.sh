#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ARTIFACT_ROOT="${ARTIFACT_ROOT:-$ROOT/artifacts}"
FIXTURE_DIR="$ARTIFACT_ROOT/update-fixtures/cli"
INSTALL_DIR="$ARTIFACT_ROOT/update-downloads/cli"
TAG="${NVPN_UPDATE_E2E_TAG:-v999.0.0}"

case "$(uname -s):$(uname -m)" in
  Darwin:arm64)
    TARGET="aarch64-apple-darwin"
    ;;
  Linux:x86_64)
    TARGET="x86_64-unknown-linux-musl"
    ;;
  Linux:aarch64|Linux:arm64)
    TARGET="aarch64-unknown-linux-musl"
    ;;
  *)
    TARGET="unsupported"
    ;;
esac

ASSET_NAME="nvpn-${TAG}-${TARGET}.tar.gz"
ASSET_PATH="$FIXTURE_DIR/$ASSET_NAME"
MANIFEST_PATH="$FIXTURE_DIR/release.json"
INSTALL_PATH="$INSTALL_DIR/nvpn"

rm -rf "$FIXTURE_DIR" "$INSTALL_DIR"
mkdir -p "$FIXTURE_DIR/archive/bin" "$INSTALL_DIR"

cat >"$FIXTURE_DIR/archive/bin/nvpn" <<'SH'
#!/bin/sh
echo nostr vpn cli update fixture
SH
chmod +x "$FIXTURE_DIR/archive/bin/nvpn"
tar -czf "$ASSET_PATH" -C "$FIXTURE_DIR/archive" .

node - "$MANIFEST_PATH" "$TAG" "$ASSET_NAME" <<'NODE'
const fs = require('fs');
const [manifestPath, tag, assetName] = process.argv.slice(2);
fs.writeFileSync(manifestPath, JSON.stringify({
  tag,
  assets: [{ name: assetName, path: assetName }],
}, null, 2));
NODE

MANIFEST_URL="$(
  node -e 'const { pathToFileURL } = require("url"); console.log(pathToFileURL(process.argv[1]).href)' "$MANIFEST_PATH"
)"

cd "$ROOT"

CHECK_OUTPUT="$(
  NVPN_UPDATE_MANIFEST_URL="$MANIFEST_URL" \
    cargo run --quiet -p nvpn -- update --check
)"
if ! grep -Fq "update available:" <<<"$CHECK_OUTPUT"; then
  echo "CLI update e2e failed: update check did not report an available update" >&2
  echo "$CHECK_OUTPUT" >&2
  exit 1
fi
if ! grep -Fq "asset=$ASSET_NAME" <<<"$CHECK_OUTPUT"; then
  echo "CLI update e2e failed: update check selected the wrong asset" >&2
  echo "$CHECK_OUTPUT" >&2
  exit 1
fi

INSTALL_OUTPUT="$(
  NVPN_UPDATE_MANIFEST_URL="$MANIFEST_URL" \
    cargo run --quiet -p nvpn -- update --force --path "$INSTALL_PATH"
)"
if ! grep -Fq "updated nvpn at $INSTALL_PATH" <<<"$INSTALL_OUTPUT"; then
  echo "CLI update e2e failed: install did not report the expected destination" >&2
  echo "$INSTALL_OUTPUT" >&2
  exit 1
fi
if [[ ! -x "$INSTALL_PATH" ]]; then
  echo "CLI update e2e failed: installed fixture is not executable" >&2
  exit 1
fi

RUN_OUTPUT="$("$INSTALL_PATH")"
if [[ "$RUN_OUTPUT" != "nostr vpn cli update fixture" ]]; then
  echo "CLI update e2e failed: installed fixture produced unexpected output" >&2
  echo "$RUN_OUTPUT" >&2
  exit 1
fi

echo "CLI_UPDATE_E2E_OK"
echo "Installed fixture: $INSTALL_PATH"
