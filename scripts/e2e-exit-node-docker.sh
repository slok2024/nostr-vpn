#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
PROJECT_NAME="nostr-vpn-e2e-exit-node"
COMPOSE=(docker compose -p "$PROJECT_NAME" -f "$ROOT_DIR/docker-compose.exit-node-e2e.yml")

RELAY_URL="ws://198.51.100.2:8080"
REFLECTOR_ADDR="198.51.100.3:3478"
CONFIG_PATH="/root/.config/nvpn/config.toml"
PUBLIC_INTERNET_TARGET="${NVPN_EXIT_NODE_E2E_PUBLIC_IP:-198.51.100.100}"

cleanup() {
  "${COMPOSE[@]}" down -v --remove-orphans >/dev/null 2>&1 || true
  docker network rm \
    "${PROJECT_NAME}_internet" \
    "${PROJECT_NAME}_private-b" >/dev/null 2>&1 || true
  for network in "${PROJECT_NAME}_internet" "${PROJECT_NAME}_private-b"; do
    for _ in $(seq 1 20); do
      docker network inspect "$network" >/dev/null 2>&1 || break
      sleep 1
    done
  done
}

dump_debug() {
  set +e
  echo "exit-node docker e2e failed, collecting debug output..."
  "${COMPOSE[@]}" ps || true
  for service in relay reflector internet-target nat-b node-a node-b; do
    echo "--- logs: $service ---"
    "${COMPOSE[@]}" logs --no-color --tail 120 "$service" || true
  done
  for node in node-a node-b; do
    echo "--- $node status ---"
    "${COMPOSE[@]}" exec -T "$node" nvpn status --json --discover-secs 0 || true
    echo "--- $node daemon.state.json ---"
    "${COMPOSE[@]}" exec -T "$node" sh -lc "cat /root/.config/nvpn/daemon.state.json 2>/dev/null || true" || true
    echo "--- $node daemon.log ---"
    "${COMPOSE[@]}" exec -T "$node" sh -lc "tail -n 200 /root/.config/nvpn/daemon.log 2>/dev/null || true" || true
    echo "--- $node routes ---"
    "${COMPOSE[@]}" exec -T "$node" sh -lc "ip route || true" || true
    echo "--- $node utun100 ---"
    "${COMPOSE[@]}" exec -T "$node" sh -lc "ip addr show utun100 || true" || true
    echo "--- $node iptables ---"
    "${COMPOSE[@]}" exec -T "$node" sh -lc "iptables -S || true; iptables -t nat -S || true" || true
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

private_iface_for_ip() {
  local node="$1"
  local cidr="$2"
  "${COMPOSE[@]}" exec -T "$node" sh -lc \
    "ip -o -4 addr show | awk '\$4 == \"$cidr\" { print \$2; exit }'" | tr -d '\r'
}

nostr_pubkey_from_config() {
  local node="$1"
  "${COMPOSE[@]}" exec -T "$node" sh -lc "
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

  echo "exit-node docker e2e failed: service '$service' did not reach running state" >&2
  exit 1
}

ping_until_success() {
  local node="$1"
  local target="$2"
  local log_path="$3"
  for _ in $(seq 1 5); do
    if "${COMPOSE[@]}" exec -T "$node" ping -c 3 -W 2 "$target" >"$log_path"; then
      return 0
    fi
    sleep 2
  done

  return 1
}

cleanup

"${COMPOSE[@]}" build >/dev/null
"${COMPOSE[@]}" up -d relay reflector internet-target node-a nat-b >/dev/null

for service in relay reflector internet-target node-a nat-b; do
  wait_for_service "$service"
done

"${COMPOSE[@]}" up -d node-b >/dev/null
wait_for_service node-b

NODE_B_PRIVATE_IFACE="$(private_iface_for_ip node-b 172.30.242.3/24)"
[[ -n "$NODE_B_PRIVATE_IFACE" ]]

"${COMPOSE[@]}" exec -T node-b sh -lc \
  "ip route del default >/dev/null 2>&1 || true; ip route add default via 172.30.242.2 dev $NODE_B_PRIVATE_IFACE"

for node in node-a node-b; do
  "${COMPOSE[@]}" exec -T "$node" nvpn init --force >/dev/null
done

ALICE_NPUB="$(nostr_pubkey_from_config node-a)"
BOB_NPUB="$(nostr_pubkey_from_config node-b)"

if [[ -z "$ALICE_NPUB" || -z "$BOB_NPUB" ]]; then
  echo "exit-node docker e2e failed: unable to resolve node npubs" >&2
  exit 1
fi

"${COMPOSE[@]}" exec -T node-a nvpn set \
  --participant "$BOB_NPUB" \
  --relay "$RELAY_URL" \
  --advertise-exit-node >/dev/null
"${COMPOSE[@]}" exec -T node-b nvpn set \
  --participant "$ALICE_NPUB" \
  --relay "$RELAY_URL" \
  --exit-node "$ALICE_NPUB" >/dev/null

for node in node-a node-b; do
  "${COMPOSE[@]}" exec -T "$node" sh -lc \
    "sed -i 's|^reflectors = .*|reflectors = [\"$REFLECTOR_ADDR\"]|' '$CONFIG_PATH'"
  "${COMPOSE[@]}" exec -T "$node" sh -lc \
    "sed -i 's|^discovery_timeout_secs = .*|discovery_timeout_secs = 2|' '$CONFIG_PATH'"
done

"${COMPOSE[@]}" exec -T node-a nvpn start --daemon --connect >/dev/null
"${COMPOSE[@]}" exec -T node-b nvpn start --daemon --connect >/dev/null

ALICE_STATUS=""
BOB_STATUS=""
for _ in $(seq 1 80); do
  ALICE_STATUS="$("${COMPOSE[@]}" exec -T node-a nvpn status --json --discover-secs 0 | tr -d '\r')"
  BOB_STATUS="$("${COMPOSE[@]}" exec -T node-b nvpn status --json --discover-secs 0 | tr -d '\r')"
  ALICE_COMPACT="$(printf '%s' "$ALICE_STATUS" | compact_json)"
  BOB_COMPACT="$(printf '%s' "$BOB_STATUS" | compact_json)"
  ALICE_TUNNEL_IP="$("${COMPOSE[@]}" exec -T node-a nvpn ip | tr -d '\r')"
  BOB_TUNNEL_IP="$("${COMPOSE[@]}" exec -T node-b nvpn ip | tr -d '\r')"

  if grep -q '"status_source":"daemon"' <<<"$ALICE_COMPACT" \
    && grep -q '"status_source":"daemon"' <<<"$BOB_COMPACT" \
    && grep -q '"running":true' <<<"$ALICE_COMPACT" \
    && grep -q '"running":true' <<<"$BOB_COMPACT" \
    && grep -q '"private_data_plane":"fips"' <<<"$ALICE_COMPACT" \
    && grep -q '"private_data_plane":"fips"' <<<"$BOB_COMPACT" \
    && grep -q '"mesh_ready":true' <<<"$ALICE_COMPACT" \
    && grep -q '"mesh_ready":true' <<<"$BOB_COMPACT" \
    && grep -q '"endpoint":"fips"' <<<"$ALICE_COMPACT" \
    && grep -q '"endpoint":"fips"' <<<"$BOB_COMPACT" \
    && grep -q '"effective_advertised_routes":\["0.0.0.0/0","::/0"\]' <<<"$ALICE_COMPACT" \
    && [[ -n "$ALICE_TUNNEL_IP" ]] \
    && [[ -n "$BOB_TUNNEL_IP" ]]; then
    break
  fi
  sleep 1
done

printf 'ALICE STATUS\n%s\n' "$ALICE_STATUS"
printf 'BOB STATUS\n%s\n' "$BOB_STATUS"

ALICE_COMPACT="$(printf '%s' "$ALICE_STATUS" | compact_json)"
BOB_COMPACT="$(printf '%s' "$BOB_STATUS" | compact_json)"
grep -q '"status_source":"daemon"' <<<"$ALICE_COMPACT"
grep -q '"status_source":"daemon"' <<<"$BOB_COMPACT"
grep -q '"running":true' <<<"$ALICE_COMPACT"
grep -q '"running":true' <<<"$BOB_COMPACT"
grep -q '"private_data_plane":"fips"' <<<"$ALICE_COMPACT"
grep -q '"private_data_plane":"fips"' <<<"$BOB_COMPACT"
grep -q '"mesh_ready":true' <<<"$ALICE_COMPACT"
grep -q '"mesh_ready":true' <<<"$BOB_COMPACT"
grep -q '"endpoint":"fips"' <<<"$ALICE_COMPACT"
grep -q '"endpoint":"fips"' <<<"$BOB_COMPACT"
grep -q '"effective_advertised_routes":\["0.0.0.0/0","::/0"\]' <<<"$ALICE_COMPACT"

if [[ -z "$ALICE_TUNNEL_IP" || -z "$BOB_TUNNEL_IP" ]]; then
  echo "exit-node docker e2e failed: unable to resolve node tunnel IPs from status output" >&2
  exit 1
fi

REFLECTOR_ROUTE="$("${COMPOSE[@]}" exec -T node-b sh -lc "ip route get 198.51.100.3 | tr -d '\r'")"
DEFAULT_ROUTE="$("${COMPOSE[@]}" exec -T node-b sh -lc "ip route show default | head -n1 | tr -d '\r'")"

if grep -q 'dev utun100' <<<"$REFLECTOR_ROUTE"; then
  echo "exit-node docker e2e failed: reflector route unexpectedly points into the tunnel" >&2
  echo "$REFLECTOR_ROUTE"
  exit 1
fi

if ! grep -q 'dev utun100' <<<"$DEFAULT_ROUTE"; then
  echo "exit-node docker e2e failed: default route did not switch to the tunnel" >&2
  echo "$DEFAULT_ROUTE"
  exit 1
fi

if ! ping_until_success node-b 198.51.100.3 /tmp/nvpn-exit-node-reflector-ping.log; then
  echo "exit-node docker e2e failed: client could not reach reflector on the preserved underlay path" >&2
  if [[ -f /tmp/nvpn-exit-node-reflector-ping.log ]]; then
    cat /tmp/nvpn-exit-node-reflector-ping.log
  fi
  exit 1
fi

PUBLIC_ROUTE="$("${COMPOSE[@]}" exec -T node-b sh -lc "ip route get $PUBLIC_INTERNET_TARGET | tr -d '\r'")"

if ! grep -q 'dev utun100' <<<"$PUBLIC_ROUTE"; then
  echo "exit-node docker e2e failed: public internet route did not switch to the tunnel" >&2
  echo "$PUBLIC_ROUTE"
  exit 1
fi

if ! ping_until_success node-b "$PUBLIC_INTERNET_TARGET" /tmp/nvpn-exit-node-public-ping.log; then
  echo "exit-node docker e2e failed: unable to reach public internet target '$PUBLIC_INTERNET_TARGET' through exit node" >&2
  if [[ -f /tmp/nvpn-exit-node-public-ping.log ]]; then
    cat /tmp/nvpn-exit-node-public-ping.log
  fi
  exit 1
fi

echo "--- Reflector route ---"
echo "$REFLECTOR_ROUTE"
echo "--- Default route ---"
echo "$DEFAULT_ROUTE"
echo "--- Reflector ping ---"
cat /tmp/nvpn-exit-node-reflector-ping.log
echo "--- Public internet route ---"
echo "$PUBLIC_ROUTE"
echo "--- Public internet ping ---"
cat /tmp/nvpn-exit-node-public-ping.log

echo "exit-node docker e2e passed: tunnel traffic reached the selected exit node, the default route switched into the tunnel, and control-plane traffic stayed on the underlay"
