#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

usage() {
  cat >&2 <<'EOF'
usage: scripts/mobile-test-kit.sh [rust|fast|simulator|device]

Modes:
  rust       Run shared Rust mobile/core tests only.
  fast       Run Rust tests plus Android and iOS debug builds.
  simulator  Run fast checks, then iOS simulator and Android adb launch smokes.
  device     Run opt-in physical-device smokes using env-provided identifiers.

Device identifiers are intentionally not stored in the repo. Use environment
variables such as NVPN_ANDROID_SERIAL and NVPN_IOS_DEVICE when needed.
EOF
}

run_rust() {
  cargo test -p nostr-vpn-app-core
  cargo test -p nvpn platform_routing
}

run_android_build() {
  "$ROOT/tools/run-android" build
}

run_ios_build() {
  if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "Skipping iOS build on non-Darwin host"
    return
  fi
  "$ROOT/tools/run-ios" build
}

run_fast() {
  run_rust
  run_android_build
  run_ios_build
}

mode="${1:-fast}"
case "$mode" in
  rust)
    run_rust
    ;;
  fast)
    run_fast
    ;;
  simulator|sim)
    run_fast
    "$ROOT/scripts/mobile-ios-smoke.sh" simulator
    "$ROOT/scripts/mobile-android-smoke.sh" --no-build
    ;;
  device)
    "$ROOT/scripts/mobile-android-smoke.sh" --vpn-cycle
    "$ROOT/scripts/mobile-ios-smoke.sh" device --vpn-cycle
    ;;
  -h|--help|help)
    usage
    ;;
  *)
    usage
    exit 2
    ;;
esac
