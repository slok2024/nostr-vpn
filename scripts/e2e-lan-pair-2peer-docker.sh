#!/usr/bin/env bash
# Two-container LAN-pairing e2e: Alice and Bob on a shared bridge network
# each spawn a LanPairingWorker (broadcast + listen) and emit JSON-line
# stdout. We assert each one received the other's npub before timing out.
#
# Run from repo root:
#   ./scripts/e2e-lan-pair-2peer-docker.sh

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
COMPOSE_FILE="$ROOT_DIR/docker-compose.lan-pair-e2e.yml"

cd "$ROOT_DIR"

cleanup() {
  docker compose -f "$COMPOSE_FILE" down --remove-orphans >/dev/null 2>&1 || true
}
trap cleanup EXIT

echo "==> Building image"
docker compose -f "$COMPOSE_FILE" build

echo "==> Starting alice + bob"
docker compose -f "$COMPOSE_FILE" up --abort-on-container-exit --exit-code-from alice alice bob \
  | tee /tmp/lan-pair-e2e.log

ALICE_LOG=$(grep -E '^alice(-[0-9]+)? +\|' /tmp/lan-pair-e2e.log || true)
BOB_LOG=$(grep -E '^bob(-[0-9]+)? +\|' /tmp/lan-pair-e2e.log || true)

extract_npub() {
  echo "$1" | sed -nE 's/.*\{"event":"ready","npub":"([^"]+)".*/\1/p' | head -n1
}

ALICE_NPUB=$(extract_npub "$ALICE_LOG")
BOB_NPUB=$(extract_npub "$BOB_LOG")

if [[ -z "$ALICE_NPUB" || -z "$BOB_NPUB" ]]; then
  echo "FAIL: could not parse ready npubs (alice=$ALICE_NPUB bob=$BOB_NPUB)" >&2
  exit 1
fi

echo "==> alice npub: $ALICE_NPUB"
echo "==> bob   npub: $BOB_NPUB"

if ! grep -q "\"npub\":\"$BOB_NPUB\"" <<<"$ALICE_LOG"; then
  echo "FAIL: alice never saw bob ($BOB_NPUB)" >&2
  exit 1
fi
if ! grep -q "\"npub\":\"$ALICE_NPUB\"" <<<"$BOB_LOG"; then
  echo "FAIL: bob never saw alice ($ALICE_NPUB)" >&2
  exit 1
fi

echo "PASS: alice and bob discovered each other over the bridge"
