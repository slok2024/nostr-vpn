#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
PROJECT_NAME="nostr-vpn-e2e-wireguard-exit-node"
COMPOSE=(docker compose -p "$PROJECT_NAME" -f "$ROOT_DIR/docker-compose.exit-node-e2e.yml")

REFLECTOR_ADDR="198.51.100.3:3478"
CONFIG_PATH="/root/.config/nvpn/config.toml"
PUBLIC_INTERNET_TARGET="${NVPN_EXIT_NODE_E2E_PUBLIC_IP:-198.51.100.100}"

cleanup() {
  "${COMPOSE[@]}" down -v --remove-orphans >/dev/null 2>&1 || true
  docker network rm \
    "${PROJECT_NAME}_internet" \
    "${PROJECT_NAME}_private-b" >/dev/null 2>&1 || true
}

dump_debug() {
  set +e
  echo "wireguard exit-node docker e2e failed, collecting debug output..."
  "${COMPOSE[@]}" ps || true
  for service in reflector internet-target wireguard-upstream nat-b node-a node-b; do
    echo "--- logs: $service ---"
    "${COMPOSE[@]}" logs --no-color --tail 120 "$service" || true
  done
  for node in wireguard-upstream node-a node-b; do
    echo "--- $node routes ---"
    "${COMPOSE[@]}" exec -T "$node" sh -lc "ip route; ip rule; ip addr" || true
    echo "--- $node wireguard ---"
    "${COMPOSE[@]}" exec -T "$node" sh -lc "wg show || true" || true
    echo "--- $node iptables ---"
    "${COMPOSE[@]}" exec -T "$node" sh -lc "iptables -S || true; iptables -t nat -S || true" || true
  done
  for node in node-a node-b; do
    echo "--- $node status ---"
    "${COMPOSE[@]}" exec -T "$node" nvpn status --json --discover-secs 0 || true
    echo "--- $node daemon.log ---"
    "${COMPOSE[@]}" exec -T "$node" sh -lc "tail -n 200 /root/.config/nvpn/daemon.log 2>/dev/null || true" || true
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

  echo "wireguard exit-node docker e2e failed: service '$service' did not reach running state" >&2
  exit 1
}

ping_until_success() {
  local node="$1"
  local target="$2"
  local log_path="$3"
  for _ in $(seq 1 8); do
    if "${COMPOSE[@]}" exec -T "$node" ping -c 3 -W 2 "$target" >"$log_path"; then
      return 0
    fi
    sleep 2
  done

  return 1
}

configure_wireguard_upstream() {
  local server_private="$1"
  local client_public="$2"
  "${COMPOSE[@]}" exec -T wireguard-upstream sh -lc "umask 077; cat > /tmp/server.key" <<<"$server_private"
  "${COMPOSE[@]}" exec -T wireguard-upstream sh -lc "
    set -e
    public_iface=\"\$(ip -o -4 addr show | awk '\$4 == \"198.51.100.20/24\" { print \$2; exit }')\"
    test -n \"\$public_iface\"
    ip link add wg-upstream type wireguard
    ip address add 10.200.0.1/24 dev wg-upstream
    wg set wg-upstream private-key /tmp/server.key listen-port 51830 peer '$client_public' allowed-ips 10.200.0.2/32
    ip link set wg-upstream up
    sysctl -w net.ipv4.ip_forward=1 >/dev/null 2>&1 || true
    iptables -P FORWARD ACCEPT
    iptables -t nat -A POSTROUTING -o \"\$public_iface\" -s 10.200.0.0/24 -j MASQUERADE
  "
}

cleanup

"${COMPOSE[@]}" build >/dev/null
"${COMPOSE[@]}" up -d reflector internet-target wireguard-upstream node-a nat-b >/dev/null

for service in reflector internet-target wireguard-upstream node-a nat-b; do
  wait_for_service "$service"
done

"${COMPOSE[@]}" up -d node-b >/dev/null
wait_for_service node-b

NODE_B_PRIVATE_IFACE="$(private_iface_for_ip node-b 172.30.242.3/24)"
[[ -n "$NODE_B_PRIVATE_IFACE" ]]

"${COMPOSE[@]}" exec -T node-b sh -lc \
  "ip route del default >/dev/null 2>&1 || true; ip route add default via 172.30.242.2 dev $NODE_B_PRIVATE_IFACE"

WG_SERVER_PRIVATE="$("${COMPOSE[@]}" exec -T wireguard-upstream wg genkey | tr -d '\r')"
WG_SERVER_PUBLIC="$(printf '%s\n' "$WG_SERVER_PRIVATE" | "${COMPOSE[@]}" exec -T wireguard-upstream wg pubkey | tr -d '\r')"
WG_CLIENT_PRIVATE="$("${COMPOSE[@]}" exec -T wireguard-upstream wg genkey | tr -d '\r')"
WG_CLIENT_PUBLIC="$(printf '%s\n' "$WG_CLIENT_PRIVATE" | "${COMPOSE[@]}" exec -T wireguard-upstream wg pubkey | tr -d '\r')"
configure_wireguard_upstream "$WG_SERVER_PRIVATE" "$WG_CLIENT_PUBLIC"

for node in node-a node-b; do
  "${COMPOSE[@]}" exec -T "$node" nvpn init --force >/dev/null
done

ALICE_NPUB="$(nostr_pubkey_from_config node-a)"
BOB_NPUB="$(nostr_pubkey_from_config node-b)"

if [[ -z "$ALICE_NPUB" || -z "$BOB_NPUB" ]]; then
  echo "wireguard exit-node docker e2e failed: unable to resolve node npubs" >&2
  exit 1
fi

"${COMPOSE[@]}" exec -T node-a nvpn set \
  --participant "$BOB_NPUB" \
  --endpoint "198.51.100.10:51820" \
  --listen-port 51820 \
  --fips-advertise-endpoint true \
  --fips-peer-endpoint "$BOB_NPUB=198.51.100.11:51820" \
  --advertise-exit-node \
  --wireguard-exit-enabled \
  --wireguard-exit-interface nvpn-wg-exit \
  --wireguard-exit-address 10.200.0.2/32 \
  --wireguard-exit-private-key "$WG_CLIENT_PRIVATE" \
  --wireguard-exit-peer-public-key "$WG_SERVER_PUBLIC" \
  --wireguard-exit-endpoint "198.51.100.20:51830" \
  --wireguard-exit-allowed-ips "0.0.0.0/0" >/dev/null
"${COMPOSE[@]}" exec -T node-b nvpn set \
  --participant "$ALICE_NPUB" \
  --endpoint "198.51.100.11:51820" \
  --listen-port 51820 \
  --fips-advertise-endpoint true \
  --fips-peer-endpoint "$ALICE_NPUB=198.51.100.10:51820" \
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
for _ in $(seq 1 90); do
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
    && grep -q '"mesh_ready":true' <<<"$ALICE_COMPACT" \
    && grep -q '"mesh_ready":true' <<<"$BOB_COMPACT" \
    && grep -q '"wireguard_exit":{"enabled":true' <<<"$ALICE_COMPACT" \
    && grep -q '"configured":true' <<<"$ALICE_COMPACT" \
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
grep -q '"mesh_ready":true' <<<"$ALICE_COMPACT"
grep -q '"mesh_ready":true' <<<"$BOB_COMPACT"
grep -q '"wireguard_exit":{"enabled":true' <<<"$ALICE_COMPACT"
grep -q '"configured":true' <<<"$ALICE_COMPACT"

if grep -q 'FIPS route refresh failed' <<<"$ALICE_STATUS$BOB_STATUS"; then
  echo "wireguard exit-node docker e2e failed: daemon reported FIPS route refresh failure" >&2
  exit 1
fi

REFLECTOR_ROUTE="$("${COMPOSE[@]}" exec -T node-b sh -lc "ip route get 198.51.100.3 | tr -d '\r'")"
BOB_DEFAULT_ROUTE="$("${COMPOSE[@]}" exec -T node-b sh -lc "ip route show default | head -n1 | tr -d '\r'")"
ALICE_DEFAULT_ROUTE="$("${COMPOSE[@]}" exec -T node-a sh -lc "ip route show default | head -n1 | tr -d '\r'")"
ALICE_FORWARD_ROUTE="$("${COMPOSE[@]}" exec -T node-a sh -lc "ip route show table 51888 | tr -d '\r'")"

if grep -q 'dev utun100' <<<"$REFLECTOR_ROUTE"; then
  echo "wireguard exit-node docker e2e failed: reflector route unexpectedly points into the tunnel" >&2
  echo "$REFLECTOR_ROUTE"
  exit 1
fi

if ! grep -q 'dev utun100' <<<"$BOB_DEFAULT_ROUTE"; then
  echo "wireguard exit-node docker e2e failed: client default route did not switch to FIPS tunnel" >&2
  echo "$BOB_DEFAULT_ROUTE"
  exit 1
fi

if grep -q 'dev nvpn-wg-exit' <<<"$ALICE_DEFAULT_ROUTE"; then
  echo "wireguard exit-node docker e2e failed: provider host default route moved into WireGuard" >&2
  echo "$ALICE_DEFAULT_ROUTE"
  exit 1
fi

if ! grep -q 'default dev nvpn-wg-exit' <<<"$ALICE_FORWARD_ROUTE"; then
  echo "wireguard exit-node docker e2e failed: provider did not install WireGuard exit policy table" >&2
  echo "$ALICE_FORWARD_ROUTE"
  exit 1
fi

if ! ping_until_success node-b "$PUBLIC_INTERNET_TARGET" /tmp/nvpn-wireguard-exit-public-ping.log; then
  echo "wireguard exit-node docker e2e failed: unable to reach public target through WireGuard-backed exit node" >&2
  cat /tmp/nvpn-wireguard-exit-public-ping.log 2>/dev/null || true
  exit 1
fi

WG_SHOW="$("${COMPOSE[@]}" exec -T node-a sh -lc "wg show nvpn-wg-exit | tr -d '\r'")"
grep -q 'latest handshake' <<<"$WG_SHOW"

echo "--- Client reflector route ---"
echo "$REFLECTOR_ROUTE"
echo "--- Client default route ---"
echo "$BOB_DEFAULT_ROUTE"
echo "--- Provider default route ---"
echo "$ALICE_DEFAULT_ROUTE"
echo "--- Provider forwarded route ---"
echo "$ALICE_FORWARD_ROUTE"
echo "--- Provider WireGuard ---"
echo "$WG_SHOW"
echo "--- Public internet ping ---"
cat /tmp/nvpn-wireguard-exit-public-ping.log

echo "wireguard exit-node docker e2e passed: FIPS members selected a normal exit node while the provider sent forwarded exit traffic through its WireGuard upstream"
