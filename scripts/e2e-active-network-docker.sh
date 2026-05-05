#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
PROJECT_NAME="nostr-vpn-e2e-active-network"
COMPOSE=(docker compose -p "$PROJECT_NAME" -f "$ROOT_DIR/docker-compose.e2e.yml")

ACTIVE_NETWORK_ID="docker-active-home"
INACTIVE_NETWORK_ID="docker-saved-work"
RELAY_URL="ws://10.203.0.2:8080"

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

  echo "active network e2e failed: service '$service' did not reach running state" >&2
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

cleanup

"${COMPOSE[@]}" build >/dev/null
"${COMPOSE[@]}" up -d relay node-a node-b >/dev/null
for service in relay node-a node-b; do
  wait_for_service "$service"
done

"${COMPOSE[@]}" exec -T node-a nvpn init --force >/dev/null
"${COMPOSE[@]}" exec -T node-b nvpn init --force >/dev/null
"${COMPOSE[@]}" exec -T node-a sh -lc \
  "rm -f /tmp/phantom.toml && nvpn init --config /tmp/phantom.toml --force >/dev/null"
ALICE_NPUB="$(nostr_pubkey_from_config node-a)"
BOB_NPUB="$(nostr_pubkey_from_config node-b)"
PHANTOM_NPUB="$(nostr_pubkey_from_config node-a /tmp/phantom.toml)"

if [[ -z "$ALICE_NPUB" || -z "$BOB_NPUB" || -z "$PHANTOM_NPUB" ]]; then
  echo "active network e2e failed: unable to resolve participant npubs" >&2
  exit 1
fi

"${COMPOSE[@]}" exec -T node-a nvpn set \
  --network-id "$ACTIVE_NETWORK_ID" \
  --participant "$ALICE_NPUB" \
  --participant "$BOB_NPUB" \
  --relay "$RELAY_URL" >/dev/null

"${COMPOSE[@]}" exec -T node-b nvpn set \
  --network-id "$ACTIVE_NETWORK_ID" \
  --participant "$ALICE_NPUB" \
  --participant "$BOB_NPUB" \
  --relay "$RELAY_URL" >/dev/null

ALICE_TUNNEL_IP_BEFORE="$("${COMPOSE[@]}" exec -T node-a nvpn ip)"
BOB_TUNNEL_IP="$("${COMPOSE[@]}" exec -T node-b nvpn ip)"

if [[ -z "$ALICE_TUNNEL_IP_BEFORE" || -z "$BOB_TUNNEL_IP" ]]; then
  echo "active network e2e failed: auto tunnel IP lookup returned empty result" >&2
  exit 1
fi

"${COMPOSE[@]}" exec -T node-a sh -lc "
cfg=/root/.config/nvpn/config.toml
tmp=\$(mktemp)
awk '
  BEGIN { inserted = 0 }
  /^\\[peer_aliases\\]/ && !inserted {
    print \"[[networks]]\"
    print \"id = \\\"network-2\\\"\"
    print \"name = \\\"Saved Work\\\"\"
    print \"enabled = false\"
    print \"network_id = \\\"$INACTIVE_NETWORK_ID\\\"\"
    print \"participants = [\\\"$PHANTOM_NPUB\\\"]\"
    print \"\"
    inserted = 1
  }
  { print }
' \"\$cfg\" > \"\$tmp\" && mv \"\$tmp\" \"\$cfg\"
"

ALICE_TUNNEL_IP_AFTER="$("${COMPOSE[@]}" exec -T node-a nvpn ip)"

if [[ "$ALICE_TUNNEL_IP_BEFORE" != "$ALICE_TUNNEL_IP_AFTER" ]]; then
  echo "active network e2e failed: inactive saved network changed alice tunnel IP" >&2
  echo "before: $ALICE_TUNNEL_IP_BEFORE" >&2
  echo "after:  $ALICE_TUNNEL_IP_AFTER" >&2
  exit 1
fi

"${COMPOSE[@]}" exec -d node-a sh -lc "nvpn connect > /tmp/connect.log 2>&1"
"${COMPOSE[@]}" exec -d node-b sh -lc "nvpn connect > /tmp/connect.log 2>&1"

for _ in $(seq 1 30); do
  ALICE_CONNECT_LOGS="$("${COMPOSE[@]}" exec -T node-a sh -lc 'cat /tmp/connect.log 2>/dev/null || true')"
  BOB_CONNECT_LOGS="$("${COMPOSE[@]}" exec -T node-b sh -lc 'cat /tmp/connect.log 2>/dev/null || true')"

  if grep -q "mesh: 1/1 peers with presence" <<<"$ALICE_CONNECT_LOGS" \
    && grep -q "mesh: 1/1 peers with presence" <<<"$BOB_CONNECT_LOGS"; then
    break
  fi

  sleep 1
done

ALICE_CONNECT_LOGS="$("${COMPOSE[@]}" exec -T node-a sh -lc 'cat /tmp/connect.log 2>/dev/null || true')"
BOB_CONNECT_LOGS="$("${COMPOSE[@]}" exec -T node-b sh -lc 'cat /tmp/connect.log 2>/dev/null || true')"

if ! grep -q "mesh: 1/1 peers with presence" <<<"$ALICE_CONNECT_LOGS"; then
  echo "active network e2e failed: alice did not stay scoped to the active network" >&2
  echo "$ALICE_CONNECT_LOGS"
  exit 1
fi

if ! grep -q "mesh: 1/1 peers with presence" <<<"$BOB_CONNECT_LOGS"; then
  echo "active network e2e failed: bob did not reach 1/1 mesh state" >&2
  echo "$BOB_CONNECT_LOGS"
  exit 1
fi

PING_OK=false
for _ in $(seq 1 20); do
  if "${COMPOSE[@]}" exec -T node-a ping -c 1 -W 2 "$BOB_TUNNEL_IP" >/tmp/ping-a.log 2>&1 \
    && "${COMPOSE[@]}" exec -T node-b ping -c 1 -W 2 "$ALICE_TUNNEL_IP_AFTER" >/tmp/ping-b.log 2>&1; then
    PING_OK=true
    break
  fi

  sleep 1
done

if [[ "$PING_OK" != true ]]; then
  echo "active network e2e failed: active peers never established a tunnel" >&2
  echo "--- Alice connect log ---"
  echo "$ALICE_CONNECT_LOGS"
  echo "--- Bob connect log ---"
  echo "$BOB_CONNECT_LOGS"
  if [[ -f /tmp/ping-a.log ]]; then
    echo "--- Ping A -> B ---"
    cat /tmp/ping-a.log
  fi
  if [[ -f /tmp/ping-b.log ]]; then
    echo "--- Ping B -> A ---"
    cat /tmp/ping-b.log
  fi
  exit 1
fi

echo "--- Alice connect log ---"
echo "$ALICE_CONNECT_LOGS"
echo "--- Bob connect log ---"
echo "$BOB_CONNECT_LOGS"
echo "--- Alice tunnel IP ---"
echo "$ALICE_TUNNEL_IP_AFTER"
echo "--- Bob tunnel IP ---"
echo "$BOB_TUNNEL_IP"
echo "--- Ping A -> B ---"
cat /tmp/ping-a.log
echo "--- Ping B -> A ---"
cat /tmp/ping-b.log

echo "active network docker e2e passed: inactive saved networks did not alter the active mesh or its tunnel IP"
