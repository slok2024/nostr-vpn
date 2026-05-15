#!/usr/bin/env bash
# Real macOS launchd service install/uninstall e2e.
#
# Catches the regression class where service install puts the daemon
# binary at a stable path under /Library/PrivilegedHelperTools/, but the
# user-mode CLI's "is the daemon running?" heuristic misses it — leaving
# `nvpn status` reporting daemon.running=false, breaking pause/resume,
# and silently breaking the GUI VPN toggle on freshly-installed services.
#
# Requires root (writes /Library/LaunchDaemons + /Library/PrivilegedHelperTools).
# Gated by NVPN_RUN_MACOS_SERVICE_E2E=1 so it doesn't run on developer
# laptops by default — runs in the GitHub Release CI on macos-14 (which
# has passwordless sudo).

set -euo pipefail

case "$(uname -s)" in
  Darwin) ;;
  *)
    echo "macOS-only e2e; skipping on $(uname -s)"
    exit 0
    ;;
esac

case "${NVPN_RUN_MACOS_SERVICE_E2E:-0}" in
  1|true|TRUE|True|yes|YES|Yes|on|ON|On) ;;
  *)
    echo "Skipping macOS service e2e (set NVPN_RUN_MACOS_SERVICE_E2E=1 to run)."
    echo "It installs and uninstalls a real launchd daemon under a test"
    echo "config suffix, which mutates /Library/LaunchDaemons and"
    echo "/Library/PrivilegedHelperTools and needs root."
    exit 0
    ;;
esac

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NVPN_BIN="${NVPN_E2E_BINARY:-$ROOT/target/release/nvpn}"

if [ ! -x "$NVPN_BIN" ]; then
  echo "Building nvpn (release)..."
  (cd "$ROOT" && cargo build --release -p nvpn)
fi
if [ ! -x "$NVPN_BIN" ]; then
  echo "FAIL: $NVPN_BIN missing after build"
  exit 1
fi

# Use a config path with a unique suffix so the service label resolves to
# to.nostrvpn.nvpn.<suffix> and we don't touch a real user's service.
SUFFIX="e2e-$(date +%s)-$$"
TEST_DIR="$(mktemp -d -t nvpn-svc-e2e)"
TEST_CONFIG="$TEST_DIR/$SUFFIX.toml"
SERVICE_LABEL="to.nostrvpn.nvpn.$(printf '%s' "$TEST_DIR/$SUFFIX" \
  | sed -e 's:/:_:g' -e 's:^_*::' -e 's:^Users_:u_:' )"

cleanup() {
  echo "Cleaning up test service ($SERVICE_LABEL)..."
  sudo "$NVPN_BIN" service uninstall --config "$TEST_CONFIG" 2>/dev/null || true
  rm -rf "$TEST_DIR"
}
trap cleanup EXIT

"$NVPN_BIN" init --config "$TEST_CONFIG" >/dev/null

echo "Installing test service..."
sudo "$NVPN_BIN" service install --force --config "$TEST_CONFIG" >/dev/null

# Give launchd a moment to actually spawn the daemon.
sleep 3

DETECTED_RUNNING="$("$NVPN_BIN" status --json --discover-secs 0 --config "$TEST_CONFIG" \
  | python3 -c 'import sys,json; d=json.load(sys.stdin).get("daemon",{}); print(str(d.get("running")).lower())')"

if [ "$DETECTED_RUNNING" != "true" ]; then
  echo "FAIL: nvpn status reports daemon.running=$DETECTED_RUNNING after"
  echo "      `nvpn service install`. The launchd daemon should be visible."
  echo "Service status:"
  "$NVPN_BIN" service status --config "$TEST_CONFIG" || true
  exit 1
fi

echo "Pausing..."
"$NVPN_BIN" pause --config "$TEST_CONFIG"
echo "Resuming..."
"$NVPN_BIN" resume --config "$TEST_CONFIG"

echo "MACOS_SERVICE_E2E_OK"
echo "service install -> daemon detected -> pause/resume both succeeded"
