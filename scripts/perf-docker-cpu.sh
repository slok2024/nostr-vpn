#!/usr/bin/env bash
# Capture CPU usage for nvpn on both nodes during a sustained TCP iperf3.
# Tells us whether we're CPU-bound and which side hits the wall first.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
PROJECT_NAME="${PROJECT_NAME:-nvpn-perf}"
COMPOSE=(docker compose -p "$PROJECT_NAME" -f "$ROOT_DIR/docker-compose.e2e.yml")

NETWORK_ID="docker-perf"
DURATION="${DURATION:-20}"

cleanup() {
  if [[ -z "${KEEP:-}" ]]; then
    "${COMPOSE[@]}" down -v --remove-orphans >/dev/null 2>&1 || true
    docker network rm "${PROJECT_NAME}_e2e" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

wait_for_service() {
  local service="$1"
  for _ in $(seq 1 30); do
    cid="$("${COMPOSE[@]}" ps -q "$service" 2>/dev/null || true)"
    if [[ -n "$cid" ]] && [[ "$(docker inspect -f '{{.State.Running}}' "$cid" 2>/dev/null || true)" == "true" ]]; then
      return 0
    fi
    sleep 1
  done
  echo "perf: service '$service' did not start" >&2
  exit 1
}

nostr_pubkey_from_config() {
  local service="$1"
  "${COMPOSE[@]}" exec -T "$service" sh -lc "
    awk '
      /^\\[nostr\\]\$/ { in_nostr = 1; next }
      /^\\[/ { in_nostr = 0 }
      in_nostr && /^public_key[[:space:]]*=/ {
        print \$3;
        exit
      }
    ' /root/.config/nvpn/config.toml
  " | tr -d '\r"'
}

cleanup
"${COMPOSE[@]}" up -d node-a node-b >/dev/null
for service in node-a node-b; do
  wait_for_service "$service"
done

"${COMPOSE[@]}" exec -T node-a nvpn init --force >/dev/null
"${COMPOSE[@]}" exec -T node-b nvpn init --force >/dev/null
ALICE_NPUB="$(nostr_pubkey_from_config node-a)"
BOB_NPUB="$(nostr_pubkey_from_config node-b)"

"${COMPOSE[@]}" exec -T node-a nvpn set \
  --participant "$ALICE_NPUB" \
  --participant "$BOB_NPUB" >/dev/null

"${COMPOSE[@]}" exec -T node-b nvpn set \
  --participant "$ALICE_NPUB" \
  --participant "$BOB_NPUB" >/dev/null

"${COMPOSE[@]}" exec -T node-a nvpn set \
  --network-id "$NETWORK_ID" \
  --endpoint "10.203.0.10:51820" \
  --listen-port 51820 \
  --fips-advertise-endpoint true \
  --fips-peer-endpoint "$BOB_NPUB=10.203.0.11:51820" >/dev/null

"${COMPOSE[@]}" exec -T node-b nvpn set \
  --network-id "$NETWORK_ID" \
  --endpoint "10.203.0.11:51820" \
  --listen-port 51820 \
  --fips-advertise-endpoint true \
  --fips-peer-endpoint "$ALICE_NPUB=10.203.0.10:51820" >/dev/null

BOB_TUNNEL_IP="$("${COMPOSE[@]}" exec -T node-b nvpn ip | tr -d '\r')"

"${COMPOSE[@]}" exec -d node-a sh -lc "nvpn connect > /tmp/connect.log 2>&1"
"${COMPOSE[@]}" exec -d node-b sh -lc "nvpn connect > /tmp/connect.log 2>&1"

for _ in $(seq 1 30); do
  a="$("${COMPOSE[@]}" exec -T node-a sh -lc 'cat /tmp/connect.log 2>/dev/null || true')"
  b="$("${COMPOSE[@]}" exec -T node-b sh -lc 'cat /tmp/connect.log 2>/dev/null || true')"
  if grep -q "mesh: 1/1 peers connected" <<<"$a" \
    && grep -q "mesh: 1/1 peers connected" <<<"$b"; then
    break
  fi
  sleep 1
done

"${COMPOSE[@]}" exec -T node-a ping -c 3 -W 2 "$BOB_TUNNEL_IP" >/dev/null

# Kick off a 20s TCP iperf3 in background, then sample CPU usage
"${COMPOSE[@]}" exec -d node-b sh -lc "iperf3 -s -D --logfile /tmp/iperf3-server.log"
sleep 1

echo "Starting iperf3 (TCP 1 stream, ${DURATION}s) and sampling CPU..."
"${COMPOSE[@]}" exec -d node-a sh -lc "iperf3 -c $BOB_TUNNEL_IP -t $DURATION -i 0 -f m > /tmp/iperf3-client.log 2>&1"

# Wait a couple seconds for the bench to ramp up
sleep 3

# Sample top for both nodes (delay=2 count=3 → covers ~6s while bench runs)
echo "=== node-a (sender) top (3 samples, 2s apart) ==="
"${COMPOSE[@]}" exec -T node-a sh -lc 'top -b -n 3 -d 2 -p $(pgrep -d, -f "nvpn connect")' 2>&1 | grep -E "(top -|nvpn|^%Cpu|PID )" | tail -30

echo
echo "=== node-b (receiver) top (3 samples, 2s apart) ==="
"${COMPOSE[@]}" exec -T node-b sh -lc 'top -b -n 3 -d 2 -p $(pgrep -d, -f "nvpn connect")' 2>&1 | grep -E "(top -|nvpn|^%Cpu|PID )" | tail -30

# Wait for iperf3 to finish, then dump result
wait_secs=$((DURATION + 5))
sleep "$wait_secs"

echo
echo "=== iperf3 client result ==="
"${COMPOSE[@]}" exec -T node-a sh -lc 'cat /tmp/iperf3-client.log' 2>&1 | tail -10
