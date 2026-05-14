#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
PROJECT_NAME="nostr-vpn-e2e-fips-nat-safe-mtu"
COMPOSE=(docker compose -p "$PROJECT_NAME" -f "$ROOT_DIR/docker-compose.exit-node-e2e.yml")

NETWORK_ID="docker-fips-nat-safe-mtu"
CONFIG_PATH="/root/.config/nvpn/config.toml"
UDP_PORT=43434
SAFE_TUNNEL_MTU=1150
PING_PAYLOAD_SIZE=1000
CONTINUITY_DURATION_SECS="${NVPN_E2E_CONTINUITY_SECS:-90}"
CONTINUITY_INTERVAL_SECS="${NVPN_E2E_CONTINUITY_INTERVAL_SECS:-0.2}"

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
  echo "fips nat safe-mtu e2e failed, collecting debug output..."
  "${COMPOSE[@]}" ps || true
  for service in node-a nat-b node-b; do
    echo "--- logs: $service ---"
    "${COMPOSE[@]}" logs --no-color --tail 120 "$service" || true
  done
  for node in node-a node-b; do
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
    "${COMPOSE[@]}" exec -T "$node" sh -lc "iptables -S || true; iptables -t nat -S || true" || true
  done
  echo "--- nat-b iptables ---"
  "${COMPOSE[@]}" exec -T nat-b sh -lc "iptables -S || true; iptables -t nat -S || true" || true
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

  echo "fips nat safe-mtu e2e failed: service '$service' did not reach running state" >&2
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
    "$CONFIG_PATH" "$ALICE_NPUB" "$BOB_NPUB" <<'SH'
set -eu
config="$1"
alice="$2"
bob="$3"
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

block_docker_gateway_fips_shortcut() {
  "${COMPOSE[@]}" exec -T node-b sh -lc '
    iptables -I OUTPUT -p udp -d 172.30.242.1 --dport 51820 -j DROP
    iptables -I INPUT -p udp -s 172.30.242.1 --sport 51820 -j DROP
  '
}

start_nvpn_daemon() {
  local node="$1"
  "${COMPOSE[@]}" exec -T "$node" sh -lc \
    "NVPN_MESH_MTU_PROFILE=safe NVPN_MESH_UNDERLAY_UDP_MTU=1280 NVPN_MESH_TUNNEL_MTU=$SAFE_TUNNEL_MTU nvpn start --daemon --connect" >/dev/null
}

wait_for_mesh() {
  local node="$1"
  local status compact
  for _ in $(seq 1 100); do
    status="$("${COMPOSE[@]}" exec -T "$node" nvpn status --json --discover-secs 0 | tr -d '\r')"
    compact="$(printf '%s' "$status" | compact_json)"
    if grep -q '"status_source":"daemon"' <<<"$compact" \
      && grep -q '"running":true' <<<"$compact" \
      && grep -q '"mesh_ready":true' <<<"$compact" \
      && grep -q '"connected_peer_count":1' <<<"$compact"; then
      printf '%s\n' "$status"
      return 0
    fi
    sleep 1
  done

  printf '%s\n' "$status"
  return 1
}

assert_peer_online_via_fips() {
  local status="$1"
  local peer_key="$2"
  local label="$3"
  if ! jq -e --arg peer_key "$peer_key" '
    .daemon.state.peers
    | any(
      (.participant_pubkey == $peer_key or .fips_endpoint_npub == $peer_key)
      and (.endpoint == "fips" or .runtime_endpoint == "fips")
      and .reachable == true
    )
  ' >/dev/null <<<"$status"; then
    echo "fips nat safe-mtu e2e failed: $label did not show peer online via FIPS" >&2
    printf '%s\n' "$status" >&2
    exit 1
  fi
}

assert_no_docker_gateway_shortcut() {
  local status="$1"
  local label="$2"
  if jq -e '.daemon.state.peers | any((.fips_transport_addr? // "") | startswith("172.30.242.1:"))' >/dev/null <<<"$status"; then
    echo "fips nat safe-mtu e2e failed: $label reached its peer through the Docker bridge gateway instead of the NAT path" >&2
    printf '%s\n' "$status" >&2
    exit 1
  fi
}

resolve_magic_dns() {
  local node="$1"
  local name="$2"
  "${COMPOSE[@]}" exec -T "$node" sh -lc \
    "dig +short @127.0.0.1 -p 1053 '$name' A | tail -n1" | tr -d '\r'
}

assert_tunnel_mtu() {
  local node="$1"
  local actual
  actual="$("${COMPOSE[@]}" exec -T "$node" sh -lc \
    "ip -o link show utun100 | awk -F ' mtu ' '{ print \$2 }' | awk '{ print \$1 }'" | tr -d '\r')"
  if [[ "$actual" != "$SAFE_TUNNEL_MTU" ]]; then
    echo "fips nat safe-mtu e2e failed: $node utun100 MTU was '$actual', expected '$SAFE_TUNNEL_MTU'" >&2
    exit 1
  fi
}

ping_tunnel_payload() {
  local node="$1"
  local target_ip="$2"
  local log_path="$3"
  for _ in $(seq 1 10); do
    if "${COMPOSE[@]}" exec -T "$node" ping -M do -s "$PING_PAYLOAD_SIZE" -c 2 -W 2 "$target_ip" >"$log_path"; then
      return 0
    fi
    sleep 1
  done
  return 1
}

run_continuity_ping() {
  local node="$1"
  local target_ip="$2"
  local label="$3"
  local log_path="$4"
  "${COMPOSE[@]}" exec -T "$node" sh -lc "
    set -eu
    end=\$(( \$(date +%s) + $CONTINUITY_DURATION_SECS ))
    seq=0
    while [ \$(date +%s) -lt \$end ]; do
      seq=\$((seq + 1))
      printf 'sample=%s label=%s target=%s\n' \"\$seq\" '$label' '$target_ip'
      ping -M do -s '$PING_PAYLOAD_SIZE' -c 1 -W 2 '$target_ip'
      sleep '$CONTINUITY_INTERVAL_SECS'
    done
    printf 'continuity_samples=%s label=%s\n' \"\$seq\" '$label'
  " >"$log_path" 2>&1
}

assert_bidirectional_continuity() {
  local first_node="$1"
  local first_target_ip="$2"
  local first_label="$3"
  local first_log="$4"
  local second_node="$5"
  local second_target_ip="$6"
  local second_label="$7"
  local second_log="$8"
  local failed=0
  local first_pid second_pid

  run_continuity_ping "$first_node" "$first_target_ip" "$first_label" "$first_log" &
  first_pid=$!
  run_continuity_ping "$second_node" "$second_target_ip" "$second_label" "$second_log" &
  second_pid=$!

  if ! wait "$first_pid"; then
    failed=1
  fi
  if ! wait "$second_pid"; then
    failed=1
  fi

  if [[ "$failed" -ne 0 ]]; then
    echo "fips nat safe-mtu e2e failed: tunnel ping dropped during ${CONTINUITY_DURATION_SECS}s continuity check" >&2
    echo "--- $first_label continuity log ---" >&2
    cat "$first_log" >&2 2>/dev/null || true
    echo "--- $second_label continuity log ---" >&2
    cat "$second_log" >&2 2>/dev/null || true
    exit 1
  fi
}

start_udp_listener() {
  local node="$1"
  local output="$2"
  "${COMPOSE[@]}" exec -T "$node" sh -lc \
    "rm -f '$output'; timeout 20 nc -u -l -p '$UDP_PORT' > '$output' & echo \$! > '${output}.pid'"
}

send_udp_payload() {
  local source_node="$1"
  local target_ip="$2"
  local payload="$3"
  "${COMPOSE[@]}" exec -T "$source_node" sh -lc \
    "printf '%s' '$payload' | nc -u -w 2 '$target_ip' '$UDP_PORT'"
}

wait_for_payload_file() {
  local node="$1"
  local output="$2"
  local marker="$3"
  local min_bytes="$4"
  local bytes
  for _ in $(seq 1 20); do
    bytes="$("${COMPOSE[@]}" exec -T "$node" sh -lc "wc -c < '$output' 2>/dev/null || echo 0" | tr -d ' \r')"
    if [[ "$bytes" =~ ^[0-9]+$ ]] \
      && (( bytes >= min_bytes )) \
      && "${COMPOSE[@]}" exec -T "$node" sh -lc "grep -q '$marker' '$output' 2>/dev/null"; then
      return 0
    fi
    sleep 1
  done
  return 1
}

cleanup

"${COMPOSE[@]}" build >/dev/null
"${COMPOSE[@]}" up -d node-a nat-b >/dev/null
for service in node-a nat-b; do
  wait_for_service "$service"
done

"${COMPOSE[@]}" up -d node-b >/dev/null
wait_for_service node-b

NODE_B_PRIVATE_IFACE="$(private_iface_for_ip node-b 172.30.242.3/24)"
[[ -n "$NODE_B_PRIVATE_IFACE" ]]

"${COMPOSE[@]}" exec -T node-b sh -lc \
  "ip route del default >/dev/null 2>&1 || true; ip route add default via 172.30.242.2 dev $NODE_B_PRIVATE_IFACE"
block_docker_gateway_fips_shortcut

BOB_UNDERLAY_ROUTE="$("${COMPOSE[@]}" exec -T node-b sh -lc "ip route get 198.51.100.10 | tr -d '\r'")"
if ! grep -q 'via 172.30.242.2' <<<"$BOB_UNDERLAY_ROUTE"; then
  echo "fips nat safe-mtu e2e failed: bob underlay route to alice did not pass through nat-b" >&2
  echo "$BOB_UNDERLAY_ROUTE" >&2
  exit 1
fi

for node in node-a node-b; do
  "${COMPOSE[@]}" exec -T "$node" nvpn init --force >/dev/null
done

ALICE_NPUB="$(nostr_pubkey_from_config node-a)"
BOB_NPUB="$(nostr_pubkey_from_config node-b)"

if [[ -z "$ALICE_NPUB" || -z "$BOB_NPUB" ]]; then
  echo "fips nat safe-mtu e2e failed: unable to resolve node npubs from config" >&2
  exit 1
fi

"${COMPOSE[@]}" exec -T node-a nvpn set \
  --network-id "$NETWORK_ID" \
  --node-name alice \
  --participant "$ALICE_NPUB" \
  --participant "$BOB_NPUB" \
  --endpoint "198.51.100.10:51820" \
  --listen-port 51820 \
  --fips-advertise-endpoint false \
  --fips-peer-endpoint "$BOB_NPUB=198.51.100.11:51820" >/dev/null

"${COMPOSE[@]}" exec -T node-b nvpn set \
  --network-id "$NETWORK_ID" \
  --node-name bob \
  --participant "$ALICE_NPUB" \
  --participant "$BOB_NPUB" \
  --endpoint "198.51.100.11:51820" \
  --listen-port 51820 \
  --fips-advertise-endpoint false \
  --fips-peer-endpoint "$ALICE_NPUB=198.51.100.10:51820" >/dev/null

for node in node-a node-b; do
  replace_peer_aliases "$node"
  disable_nat_discovery "$node"
done

start_nvpn_daemon node-a
start_nvpn_daemon node-b

ALICE_STATUS="$(wait_for_mesh node-a)" || {
  echo "fips nat safe-mtu e2e failed: alice mesh did not reach 1/1" >&2
  echo "$ALICE_STATUS"
  exit 1
}
BOB_STATUS="$(wait_for_mesh node-b)" || {
  echo "fips nat safe-mtu e2e failed: bob mesh did not reach 1/1" >&2
  echo "$BOB_STATUS"
  exit 1
}

assert_peer_online_via_fips "$ALICE_STATUS" "$BOB_NPUB" "alice"
assert_peer_online_via_fips "$BOB_STATUS" "$ALICE_NPUB" "bob"
assert_no_docker_gateway_shortcut "$ALICE_STATUS" "alice"
assert_no_docker_gateway_shortcut "$BOB_STATUS" "bob"
assert_tunnel_mtu node-a
assert_tunnel_mtu node-b

BOB_TUNNEL_IP="$(resolve_magic_dns node-a bob.nvpn)"
ALICE_TUNNEL_IP="$(resolve_magic_dns node-b alice.nvpn)"

if [[ -z "$BOB_TUNNEL_IP" || -z "$ALICE_TUNNEL_IP" ]]; then
  echo "fips nat safe-mtu e2e failed: magic dns did not resolve alice.nvpn/bob.nvpn" >&2
  exit 1
fi

ALICE_TUNNEL_ROUTE="$("${COMPOSE[@]}" exec -T node-a sh -lc "ip route get '$BOB_TUNNEL_IP' | tr -d '\r'")"
BOB_TUNNEL_ROUTE="$("${COMPOSE[@]}" exec -T node-b sh -lc "ip route get '$ALICE_TUNNEL_IP' | tr -d '\r'")"

if ! grep -q 'dev utun100' <<<"$ALICE_TUNNEL_ROUTE"; then
  echo "fips nat safe-mtu e2e failed: alice route to bob.nvpn did not use utun100" >&2
  echo "$ALICE_TUNNEL_ROUTE" >&2
  exit 1
fi

if ! grep -q 'dev utun100' <<<"$BOB_TUNNEL_ROUTE"; then
  echo "fips nat safe-mtu e2e failed: bob route to alice.nvpn did not use utun100" >&2
  echo "$BOB_TUNNEL_ROUTE" >&2
  exit 1
fi

if ! ping_tunnel_payload node-a "$BOB_TUNNEL_IP" /tmp/nat-alice-to-bob-safe-mtu-ping.log; then
  echo "fips nat safe-mtu e2e failed: alice could not move $PING_PAYLOAD_SIZE-byte no-fragment ping payload to NATed bob over FIPS" >&2
  cat /tmp/nat-alice-to-bob-safe-mtu-ping.log >&2 2>/dev/null || true
  exit 1
fi

if ! ping_tunnel_payload node-b "$ALICE_TUNNEL_IP" /tmp/nat-bob-to-alice-safe-mtu-ping.log; then
  echo "fips nat safe-mtu e2e failed: NATed bob could not move $PING_PAYLOAD_SIZE-byte no-fragment ping payload to alice over FIPS" >&2
  cat /tmp/nat-bob-to-alice-safe-mtu-ping.log >&2 2>/dev/null || true
  exit 1
fi

assert_bidirectional_continuity \
  node-a "$BOB_TUNNEL_IP" "alice-to-nated-bob" /tmp/nat-alice-to-bob-continuity.log \
  node-b "$ALICE_TUNNEL_IP" "nated-bob-to-alice" /tmp/nat-bob-to-alice-continuity.log

ALICE_TO_BOB_PAYLOAD="$(printf 'alice-to-bob-nat-safe-mtu-%0950d' 0)"
BOB_TO_ALICE_PAYLOAD="$(printf 'bob-to-alice-nat-safe-mtu-%0950d' 0)"

start_udp_listener node-b /tmp/bob-nat-udp.out
send_udp_payload node-a "$BOB_TUNNEL_IP" "$ALICE_TO_BOB_PAYLOAD"
wait_for_payload_file node-b /tmp/bob-nat-udp.out "alice-to-bob-nat-safe-mtu" 900

start_udp_listener node-a /tmp/alice-nat-udp.out
send_udp_payload node-b "$ALICE_TUNNEL_IP" "$BOB_TO_ALICE_PAYLOAD"
wait_for_payload_file node-a /tmp/alice-nat-udp.out "bob-to-alice-nat-safe-mtu" 900

echo "--- Alice status ---"
echo "$ALICE_STATUS"
echo "--- Bob status ---"
echo "$BOB_STATUS"
echo "--- Underlay route from NATed bob to alice ---"
echo "$BOB_UNDERLAY_ROUTE"
echo "--- Tunnel routes ---"
echo "$ALICE_TUNNEL_ROUTE"
echo "$BOB_TUNNEL_ROUTE"
echo "--- Safe MTU pings ---"
cat /tmp/nat-alice-to-bob-safe-mtu-ping.log
cat /tmp/nat-bob-to-alice-safe-mtu-ping.log
echo "--- Continuity pings (${CONTINUITY_DURATION_SECS}s) ---"
tail -n 20 /tmp/nat-alice-to-bob-continuity.log
tail -n 20 /tmp/nat-bob-to-alice-continuity.log
echo "--- UDP payload sizes ---"
"${COMPOSE[@]}" exec -T node-b sh -lc 'wc -c /tmp/bob-nat-udp.out'
"${COMPOSE[@]}" exec -T node-a sh -lc 'wc -c /tmp/alice-nat-udp.out'

echo "fips nat safe-mtu docker e2e passed: NATed bob and public alice showed each other online via FIPS, continuity stayed up for ${CONTINUITY_DURATION_SECS}s, and moved safe-MTU ping plus UDP payloads both directions"
