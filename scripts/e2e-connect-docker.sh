#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
PROJECT_NAME="nostr-vpn-e2e-connect"
COMPOSE=(docker compose -p "$PROJECT_NAME" -f "$ROOT_DIR/docker-compose.e2e.yml")
NETWORK_ID="${NVPN_CONNECT_E2E_NETWORK_ID:-docker-connect}"
IDLE_CPU_MAX_PERCENT="${NVPN_E2E_IDLE_CPU_MAX_PERCENT:-80}"

cleanup() {
  "${COMPOSE[@]}" down -v --remove-orphans >/dev/null 2>&1 || true
  docker network rm "${PROJECT_NAME}_e2e" >/dev/null 2>&1 || true
  for _ in $(seq 1 20); do
    docker network inspect "${PROJECT_NAME}_e2e" >/dev/null 2>&1 || break
    sleep 1
  done
}
trap cleanup EXIT

wait_for_service() {
  local service="$1"
  local container_id=""
  for _ in $(seq 1 30); do
    container_id="$("${COMPOSE[@]}" ps -q "$service" 2>/dev/null || true)"
    if [[ -n "$container_id" ]] \
      && [[ "$(docker inspect -f '{{.State.Running}}' "$container_id" 2>/dev/null || true)" == "true" ]]; then
      return 0
    fi
    sleep 1
  done

  echo "connect e2e failed: service '$service' did not reach running state" >&2
  exit 1
}

nostr_pubkey_from_config() {
  local service="$1"
  local config_path="${2:-/root/.config/nvpn/config.toml}"
  "${COMPOSE[@]}" exec -T "$service" sh -lc "
    awk '
      /^\\[nostr\\]$/ { in_nostr = 1; next }
      /^\\[/ { in_nostr = 0 }
      in_nostr && /^public_key[[:space:]]*=/ {
        print \$3;
        exit
      }
    ' '$config_path'
  " | tr -d '\r\"'
}

assert_idle_cpu_below() {
  local service="$1"
  local pids
  pids="$("${COMPOSE[@]}" exec -T "$service" sh -lc 'pgrep -d, -x nvpn || true' | tr -d '\r')"
  if [[ -z "$pids" ]]; then
    echo "connect e2e failed: no nvpn process found on $service for idle CPU guard" >&2
    exit 1
  fi

  local max_cpu
  max_cpu="$("${COMPOSE[@]}" exec -T "$service" sh -lc \
    "top -b -n 3 -d 1 -p '$pids' | awk '\$12 == \"nvpn\" && \$9 + 0 > max { max = \$9 + 0 } END { printf \"%.1f\", max + 0 }'" \
    | tr -d '\r')"
  echo "--- $service idle nvpn CPU max: ${max_cpu}% ---"
  if awk -v max="$max_cpu" -v limit="$IDLE_CPU_MAX_PERCENT" 'BEGIN { exit !(max > limit) }'; then
    echo "connect e2e failed: $service idle nvpn CPU ${max_cpu}% exceeded ${IDLE_CPU_MAX_PERCENT}%" >&2
    exit 1
  fi
}

cleanup

"${COMPOSE[@]}" build >/dev/null
"${COMPOSE[@]}" up -d node-a node-b >/dev/null
for service in node-a node-b; do
  wait_for_service "$service"
done

"${COMPOSE[@]}" exec -T node-a nvpn init --force >/dev/null
"${COMPOSE[@]}" exec -T node-b nvpn init --force >/dev/null
ALICE_NPUB="$(nostr_pubkey_from_config node-a)"
BOB_NPUB="$(nostr_pubkey_from_config node-b)"

if [[ -z "$ALICE_NPUB" || -z "$BOB_NPUB" ]]; then
  echo "connect e2e failed: unable to resolve node npubs from config" >&2
  exit 1
fi

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

"${COMPOSE[@]}" exec -d node-a sh -lc "nvpn connect > /tmp/connect.log 2>&1"
"${COMPOSE[@]}" exec -d node-b sh -lc "nvpn connect > /tmp/connect.log 2>&1"

for _ in $(seq 1 20); do
  ALICE_CONNECT_LOGS="$("${COMPOSE[@]}" exec -T node-a sh -lc 'cat /tmp/connect.log 2>/dev/null || true')"
  BOB_CONNECT_LOGS="$("${COMPOSE[@]}" exec -T node-b sh -lc 'cat /tmp/connect.log 2>/dev/null || true')"

  if grep -q "mesh: 1/1 peers connected" <<<"$ALICE_CONNECT_LOGS" \
    && grep -q "mesh: 1/1 peers connected" <<<"$BOB_CONNECT_LOGS"; then
    break
  fi

  sleep 1
done

ALICE_CONNECT_LOGS="$("${COMPOSE[@]}" exec -T node-a sh -lc 'cat /tmp/connect.log 2>/dev/null || true')"
BOB_CONNECT_LOGS="$("${COMPOSE[@]}" exec -T node-b sh -lc 'cat /tmp/connect.log 2>/dev/null || true')"

if ! grep -q "mesh: 1/1 peers connected" <<<"$ALICE_CONNECT_LOGS"; then
  echo "connect e2e failed: alice mesh did not reach 1/1" >&2
  echo "$ALICE_CONNECT_LOGS"
  exit 1
fi

if ! grep -q "mesh: 1/1 peers connected" <<<"$BOB_CONNECT_LOGS"; then
  echo "connect e2e failed: bob mesh did not reach 1/1" >&2
  echo "$BOB_CONNECT_LOGS"
  exit 1
fi

sleep 2

BOB_TUNNEL_IP="$("${COMPOSE[@]}" exec -T node-a nvpn ip --peer --discover-secs 0 | head -n1 | tr -d '\r')"
ALICE_TUNNEL_IP="$("${COMPOSE[@]}" exec -T node-b nvpn ip --peer --discover-secs 0 | head -n1 | tr -d '\r')"

if [[ -z "$BOB_TUNNEL_IP" || -z "$ALICE_TUNNEL_IP" ]]; then
  echo "connect e2e failed: unable to resolve FIPS peer tunnel IPs" >&2
  echo "$ALICE_CONNECT_LOGS"
  echo "$BOB_CONNECT_LOGS"
  exit 1
fi

if ! "${COMPOSE[@]}" exec -T node-a ping -c 3 -W 2 "$BOB_TUNNEL_IP" >/tmp/ping-a.log; then
  echo "connect e2e failed: ping A -> B failed" >&2
  echo "$ALICE_CONNECT_LOGS"
  echo "$BOB_CONNECT_LOGS"
  exit 1
fi

if ! "${COMPOSE[@]}" exec -T node-b ping -c 3 -W 2 "$ALICE_TUNNEL_IP" >/tmp/ping-b.log; then
  echo "connect e2e failed: ping B -> A failed" >&2
  echo "$ALICE_CONNECT_LOGS"
  echo "$BOB_CONNECT_LOGS"
  exit 1
fi

assert_idle_cpu_below node-a
assert_idle_cpu_below node-b

echo "--- Alice connect log ---"
echo "$ALICE_CONNECT_LOGS"
echo "--- Bob connect log ---"
echo "$BOB_CONNECT_LOGS"
echo "--- Ping A -> B ---"
cat /tmp/ping-a.log
echo "--- Ping B -> A ---"
cat /tmp/ping-b.log

echo "connect docker e2e passed: config-driven nvpn connect established a FIPS private mesh and passed tunnel pings"
