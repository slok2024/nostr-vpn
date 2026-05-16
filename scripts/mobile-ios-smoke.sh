#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUNDLE_ID="${NVPN_IOS_BUNDLE_ID:-to.iris.nvpn}"
SCREENSHOT="$ROOT/artifacts/nostr-vpn-ios.png"

usage() {
  cat >&2 <<'EOF'
usage: scripts/mobile-ios-smoke.sh [simulator|device] [--vpn-cycle]

simulator  Builds, installs, launches, and screenshots the simulator app.
device     Launches an already installed development build on a physical device.

Physical-device mode requires NVPN_IOS_DEVICE or NVPN_IOS_DEVICE_ID. Keep device
identifiers in environment variables or shell history, not committed files.
EOF
}

mode="${1:-simulator}"
if [[ $# -gt 0 ]]; then
  shift
fi
vpn_cycle=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --vpn-cycle)
      vpn_cycle=1
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      usage
      exit 2
      ;;
  esac
  shift
done

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "iOS smoke requires macOS with Xcode" >&2
  exit 1
fi

run_simulator() {
  "$ROOT/tools/run-ios" run
  if [[ ! -s "$SCREENSHOT" ]]; then
    echo "Expected simulator screenshot at $SCREENSHOT" >&2
    exit 1
  fi
  echo "iOS simulator smoke passed: $SCREENSHOT"
}

launch_device() {
  local device="$1"
  shift
  xcrun devicectl device process launch \
    --device "$device" \
    --terminate-existing \
    "$BUNDLE_ID" \
    "$@"
}

run_device() {
  local device="${NVPN_IOS_DEVICE:-${NVPN_IOS_DEVICE_ID:-}}"
  if [[ -z "$device" ]]; then
    echo "Set NVPN_IOS_DEVICE to the physical iOS device identifier for device smoke" >&2
    exit 1
  fi

  xcrun devicectl device info details --device "$device" >/dev/null
  if [[ "$vpn_cycle" -eq 1 ]]; then
    launch_device "$device" --nvpn-disconnect
    sleep 3
    launch_device "$device" --nvpn-connect
  else
    launch_device "$device"
  fi
  echo "iOS device smoke launched bundle $BUNDLE_ID"
}

case "$mode" in
  simulator|sim)
    run_simulator
    ;;
  device)
    run_device
    ;;
  -h|--help|help)
    usage
    ;;
  *)
    usage
    exit 2
    ;;
esac
