#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
PROJECT_NAME="nostr-vpn-e2e-fips-routed-udp"
COMPOSE=(docker compose -p "$PROJECT_NAME" -f "$ROOT_DIR/docker-compose.e2e.yml")

NETWORK_ID="docker-fips-routed-udp"
CONFIG_PATH="/root/.config/nvpn/config.toml"
UDP_PORT=42424

cleanup() {
  "${COMPOSE[@]}" down -v --remove-orphans >/dev/null 2>&1 || true
  docker network rm "${PROJECT_NAME}_e2e" >/dev/null 2>&1 || true
  for _ in $(seq 1 20); do
    docker network inspect "${PROJECT_NAME}_e2e" >/dev/null 2>&1 || break
    sleep 1
  done
}

dump_debug() {
  set +e
  echo "fips routed udp e2e failed, collecting debug output..."
  "${COMPOSE[@]}" ps || true
  for service in node-a node-b node-c; do
    echo "--- logs: $service ---"
    "${COMPOSE[@]}" logs --no-color --tail 120 "$service" || true
  done
  for node in node-a node-b node-c; do
    echo "--- $node status ---"
    "${COMPOSE[@]}" exec -T "$node" nvpn status --json --discover-secs 0 || true
    echo "--- $node daemon.state.json ---"
    "${COMPOSE[@]}" exec -T "$node" sh -lc "cat /root/.config/nvpn/daemon.state.json 2>/dev/null || true" || true
    echo "--- $node daemon.log ---"
    "${COMPOSE[@]}" exec -T "$node" sh -lc "tail -n 240 /root/.config/nvpn/daemon.log 2>/dev/null || true" || true
    echo "--- $node routes ---"
    "${COMPOSE[@]}" exec -T "$node" sh -lc "ip route || true" || true
    echo "--- $node utun100 ---"
    "${COMPOSE[@]}" exec -T "$node" sh -lc "ip addr show utun100 || true" || true
    echo "--- $node iptables ---"
    "${COMPOSE[@]}" exec -T "$node" sh -lc "iptables -S || true" || true
  done
}

on_exit() {
  local exit_code=$?
  if [[ $exit_code -ne 0 ]]; then
    dump_debug
  fi
  cleanup
  exit "$exit_code"
}
trap on_exit EXIT

compact_json() {
  tr -d '\n\r\t '
}

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

  echo "fips routed udp e2e failed: service '$service' did not reach running state" >&2
  exit 1
}

nostr_pubkey_from_config() {
  local service="$1"
  "${COMPOSE[@]}" exec -T "$service" sh -lc "
    awk '
      /^\\[nostr\\]$/ { in_nostr = 1; next }
      /^\\[/ { in_nostr = 0 }
      in_nostr && /^public_key[[:space:]]*=/ {
        print \$3;
        exit
      }
    ' '$CONFIG_PATH'
  " | tr -d '\r\"'
}

replace_peer_aliases() {
  local service="$1"
  "${COMPOSE[@]}" exec -T "$service" sh -s -- \
    "$CONFIG_PATH" "$ALICE_NPUB" "$BOB_NPUB" "$CHARLIE_NPUB" <<'SH'
set -eu
config="$1"
alice="$2"
bob="$3"
charlie="$4"
tmp="$(mktemp)"
awk '
  /^\[peer_aliases\]$/ { skip = 1; next }
  /^\[/ { skip = 0 }
  !skip { print }
' "$config" > "$tmp"
{
  cat "$tmp"
  printf '\n[peer_aliases]\n'
  printf '"%s" = "alice"\n' "$alice"
  printf '"%s" = "bob"\n' "$bob"
  printf '"%s" = "charlie"\n' "$charlie"
} > "$config"
rm -f "$tmp"
SH
}

disable_nat_discovery() {
  local service="$1"
  "${COMPOSE[@]}" exec -T "$service" sh -lc '
    config=/root/.config/nvpn/config.toml
    tmp="$(mktemp)"
    awk "
      /^\\[nat\\]$/ { in_nat = 1; print; next }
      /^\\[/ { in_nat = 0 }
      in_nat && /^enabled[[:space:]]*=/ { print \"enabled = false\"; next }
      { print }
    " "$config" > "$tmp"
    mv "$tmp" "$config"
  '
}

block_direct_alice_bob_udp() {
  "${COMPOSE[@]}" exec -T node-a sh -lc '
    iptables -I OUTPUT -p udp -d 10.203.0.11 -j DROP
    iptables -I INPUT -p udp -s 10.203.0.11 -j DROP
  '
  "${COMPOSE[@]}" exec -T node-b sh -lc '
    iptables -I OUTPUT -p udp -d 10.203.0.10 -j DROP
    iptables -I INPUT -p udp -s 10.203.0.10 -j DROP
  '
}

wait_for_mesh() {
  local node="$1"
  local expected="$2"
  local status compact
  for _ in $(seq 1 100); do
    status="$("${COMPOSE[@]}" exec -T "$node" nvpn status --json --discover-secs 0 | tr -d '\r')"
    compact="$(printf '%s' "$status" | compact_json)"
    if grep -q '"status_source":"daemon"' <<<"$compact" \
      && grep -q '"running":true' <<<"$compact" \
      && grep -q '"mesh_ready":true' <<<"$compact" \
      && grep -q "\"connected_peer_count\":$expected" <<<"$compact"; then
      printf '%s\n' "$status"
      return 0
    fi
    sleep 1
  done

  printf '%s\n' "$status"
  return 1
}

resolve_magic_dns() {
  local node="$1"
  local name="$2"
  "${COMPOSE[@]}" exec -T "$node" sh -lc \
    "dig +short @127.0.0.1 -p 1053 '$name' A | tail -n1" | tr -d '\r'
}

send_udp_payload() {
  local source_node="$1"
  local target_ip="$2"
  local payload="$3"
  "${COMPOSE[@]}" exec -T "$source_node" sh -lc \
    "printf '%s\n' '$payload' | nc -u -w 2 '$target_ip' '$UDP_PORT'"
}

start_udp_listener() {
  local node="$1"
  local output="$2"
  "${COMPOSE[@]}" exec -T "$node" sh -lc \
    "rm -f '$output'; timeout 20 nc -u -l -p '$UDP_PORT' > '$output' & echo \$! > '${output}.pid'"
}

wait_for_payload() {
  local node="$1"
  local output="$2"
  local payload="$3"
  for _ in $(seq 1 20); do
    if "${COMPOSE[@]}" exec -T "$node" sh -lc "grep -q '$payload' '$output' 2>/dev/null"; then
      return 0
    fi
    sleep 1
  done
  return 1
}

cleanup

"${COMPOSE[@]}" build >/dev/null
"${COMPOSE[@]}" up -d node-a node-b node-c >/dev/null
for service in node-a node-b node-c; do
  wait_for_service "$service"
done

for node in node-a node-b node-c; do
  "${COMPOSE[@]}" exec -T "$node" nvpn init --force >/dev/null
done

ALICE_NPUB="$(nostr_pubkey_from_config node-a)"
BOB_NPUB="$(nostr_pubkey_from_config node-b)"
CHARLIE_NPUB="$(nostr_pubkey_from_config node-c)"

if [[ -z "$ALICE_NPUB" || -z "$BOB_NPUB" || -z "$CHARLIE_NPUB" ]]; then
  echo "fips routed udp e2e failed: unable to resolve node npubs from config" >&2
  exit 1
fi

"${COMPOSE[@]}" exec -T node-a nvpn set \
  --network-id "$NETWORK_ID" \
  --node-name alice \
  --participant "$ALICE_NPUB" \
  --participant "$BOB_NPUB" \
  --fips-peer-endpoint "$BOB_NPUB=10.203.0.11:51820" \
  --fips-peer-endpoint "$CHARLIE_NPUB=10.203.0.12:51820" \
  --endpoint "10.203.0.10:51820" \
  --listen-port 51820 \
  --fips-advertise-endpoint true >/dev/null

"${COMPOSE[@]}" exec -T node-b nvpn set \
  --network-id "$NETWORK_ID" \
  --node-name bob \
  --participant "$ALICE_NPUB" \
  --participant "$BOB_NPUB" \
  --fips-peer-endpoint "$ALICE_NPUB=10.203.0.10:51820" \
  --fips-peer-endpoint "$CHARLIE_NPUB=10.203.0.12:51820" \
  --endpoint "10.203.0.11:51820" \
  --listen-port 51820 \
  --fips-advertise-endpoint true >/dev/null

"${COMPOSE[@]}" exec -T node-c nvpn set \
  --network-id "$NETWORK_ID" \
  --node-name charlie \
  --fips-peer-endpoint "$ALICE_NPUB=10.203.0.10:51820" \
  --fips-peer-endpoint "$BOB_NPUB=10.203.0.11:51820" \
  --endpoint "10.203.0.12:51820" \
  --listen-port 51820 \
  --fips-advertise-endpoint true >/dev/null

for node in node-a node-b node-c; do
  replace_peer_aliases "$node"
  disable_nat_discovery "$node"
done

block_direct_alice_bob_udp

for node in node-a node-b node-c; do
  "${COMPOSE[@]}" exec -T "$node" nvpn start --daemon --connect >/dev/null
done

ALICE_STATUS="$(wait_for_mesh node-a 1)" || {
  echo "fips routed udp e2e failed: alice mesh did not reach 1/1" >&2
  echo "$ALICE_STATUS"
  exit 1
}
BOB_STATUS="$(wait_for_mesh node-b 1)" || {
  echo "fips routed udp e2e failed: bob mesh did not reach 1/1" >&2
  echo "$BOB_STATUS"
  exit 1
}
CHARLIE_STATUS="$("${COMPOSE[@]}" exec -T node-c nvpn status --json --discover-secs 0 | tr -d '\r')"

BOB_TUNNEL_IP="$(resolve_magic_dns node-a bob.nvpn)"
ALICE_TUNNEL_IP="$(resolve_magic_dns node-b alice.nvpn)"

if [[ -z "$BOB_TUNNEL_IP" || -z "$ALICE_TUNNEL_IP" ]]; then
  echo "fips routed udp e2e failed: magic dns did not resolve alice.nvpn/bob.nvpn" >&2
  exit 1
fi

ALICE_ROUTE="$("${COMPOSE[@]}" exec -T node-a sh -lc "ip route get '$BOB_TUNNEL_IP' | tr -d '\r'")"
BOB_ROUTE="$("${COMPOSE[@]}" exec -T node-b sh -lc "ip route get '$ALICE_TUNNEL_IP' | tr -d '\r'")"

if ! grep -q 'dev utun100' <<<"$ALICE_ROUTE"; then
  echo "fips routed udp e2e failed: alice route to bob.nvpn did not use utun100" >&2
  echo "$ALICE_ROUTE"
  exit 1
fi

if ! grep -q 'dev utun100' <<<"$BOB_ROUTE"; then
  echo "fips routed udp e2e failed: bob route to alice.nvpn did not use utun100" >&2
  echo "$BOB_ROUTE"
  exit 1
fi

start_udp_listener node-b /tmp/bob-udp.out
send_udp_payload node-a "$BOB_TUNNEL_IP" "alice-to-bob-fips-udp"
wait_for_payload node-b /tmp/bob-udp.out "alice-to-bob-fips-udp"

start_udp_listener node-a /tmp/alice-udp.out
send_udp_payload node-b "$ALICE_TUNNEL_IP" "bob-to-alice-fips-udp"
wait_for_payload node-a /tmp/alice-udp.out "bob-to-alice-fips-udp"

echo "--- Alice status ---"
echo "$ALICE_STATUS"
echo "--- Bob status ---"
echo "$BOB_STATUS"
echo "--- Charlie status ---"
echo "$CHARLIE_STATUS"
echo "--- Magic DNS ---"
echo "node-a bob.nvpn -> $BOB_TUNNEL_IP"
echo "node-b alice.nvpn -> $ALICE_TUNNEL_IP"
echo "--- Routes ---"
echo "$ALICE_ROUTE"
echo "$BOB_ROUTE"
echo "--- UDP payloads ---"
"${COMPOSE[@]}" exec -T node-b sh -lc 'cat /tmp/bob-udp.out'
"${COMPOSE[@]}" exec -T node-a sh -lc 'cat /tmp/alice-udp.out'

echo "fips routed udp docker e2e passed: alice.nvpn and bob.nvpn resolved to tunnel IPs and UDP crossed the FIPS overlay while direct Alice/Bob underlay UDP was blocked"
