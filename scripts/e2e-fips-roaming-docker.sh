#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
PROJECT_NAME="nostr-vpn-e2e-fips-roaming"
COMPOSE=(docker compose -p "$PROJECT_NAME" -f "$ROOT_DIR/docker-compose.e2e.yml")

NETWORK_ID="docker-fips-roaming"
CONFIG_PATH="/root/.config/nvpn/config.toml"
UDP_PORT=45454
SAFE_TUNNEL_MTU=1150
PING_PAYLOAD_SIZE=1000
FALLBACK_DEADLINE_SECS="${NVPN_E2E_ROAMING_FALLBACK_SECS:-35}"
DIRECT_RECOVERY_DEADLINE_SECS="${NVPN_E2E_DIRECT_RECOVERY_SECS:-25}"
FALLBACK_HOLD_SECS="${NVPN_E2E_ROAMING_FALLBACK_HOLD_SECS:-12}"
FIPS_NOSTR_DISCOVERY_POLICY="${NVPN_FIPS_NOSTR_DISCOVERY_POLICY:-configured_only}"

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
  echo "fips roaming e2e failed, collecting debug output..."
  "${COMPOSE[@]}" ps || true
  for service in node-a node-b node-c; do
    echo "--- logs: $service ---"
    "${COMPOSE[@]}" logs --no-color --tail 160 "$service" || true
  done
  for node in node-a node-b node-c; do
    echo "--- $node status ---"
    "${COMPOSE[@]}" exec -T "$node" nvpn status --json --discover-secs 0 || true
    echo "--- $node daemon.state.json ---"
    "${COMPOSE[@]}" exec -T "$node" sh -lc "cat /root/.config/nvpn/daemon.state.json 2>/dev/null || true" || true
    echo "--- $node daemon.log ---"
    "${COMPOSE[@]}" exec -T "$node" sh -lc "tail -n 320 /root/.config/nvpn/daemon.log 2>/dev/null || true" || true
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

  echo "fips roaming e2e failed: service '$service' did not reach running state" >&2
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

start_nvpn_daemon() {
  local node="$1"
  "${COMPOSE[@]}" exec -T "$node" env \
    NVPN_FIPS_NOSTR_DISCOVERY_POLICY="$FIPS_NOSTR_DISCOVERY_POLICY" \
    NVPN_MESH_MTU_PROFILE=safe \
    NVPN_MESH_UNDERLAY_UDP_MTU=1280 \
    NVPN_MESH_TUNNEL_MTU="$SAFE_TUNNEL_MTU" \
    nvpn start --daemon --connect >/dev/null
}

block_direct_alice_bob_udp() {
  "${COMPOSE[@]}" exec -T node-a sh -lc '
    iptables -C OUTPUT -p udp -d 10.203.0.11 -j DROP 2>/dev/null || iptables -I OUTPUT -p udp -d 10.203.0.11 -j DROP
    iptables -C INPUT -p udp -s 10.203.0.11 -j DROP 2>/dev/null || iptables -I INPUT -p udp -s 10.203.0.11 -j DROP
  '
  "${COMPOSE[@]}" exec -T node-b sh -lc '
    iptables -C OUTPUT -p udp -d 10.203.0.10 -j DROP 2>/dev/null || iptables -I OUTPUT -p udp -d 10.203.0.10 -j DROP
    iptables -C INPUT -p udp -s 10.203.0.10 -j DROP 2>/dev/null || iptables -I INPUT -p udp -s 10.203.0.10 -j DROP
  '
}

unblock_direct_alice_bob_udp() {
  "${COMPOSE[@]}" exec -T node-a sh -lc '
    while iptables -D OUTPUT -p udp -d 10.203.0.11 -j DROP 2>/dev/null; do :; done
    while iptables -D INPUT -p udp -s 10.203.0.11 -j DROP 2>/dev/null; do :; done
  '
  "${COMPOSE[@]}" exec -T node-b sh -lc '
    while iptables -D OUTPUT -p udp -d 10.203.0.10 -j DROP 2>/dev/null; do :; done
    while iptables -D INPUT -p udp -s 10.203.0.10 -j DROP 2>/dev/null; do :; done
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

status_json() {
  local node="$1"
  "${COMPOSE[@]}" exec -T "$node" nvpn status --json --discover-secs 0 | tr -d '\r'
}

peer_matches_direct_addr() {
  local status="$1"
  local peer_key="$2"
  local direct_addr="$3"
  jq -e --arg peer_key "$peer_key" --arg direct_addr "$direct_addr" '
    .daemon.state.peers
    | any(
      (.participant_pubkey == $peer_key or .fips_endpoint_npub == $peer_key)
      and .reachable == true
      and ((.runtime_endpoint? // "") != "fips")
      and (
        ((.runtime_endpoint? // "") | contains($direct_addr))
        or ((.fips_transport_addr? // "") | contains($direct_addr))
      )
    )
  ' >/dev/null <<<"$status"
}

peer_matches_fallback_with_probe() {
  local status="$1"
  local peer_key="$2"
  jq -e --arg peer_key "$peer_key" '
    .daemon.state.peers
    | any(
      (.participant_pubkey == $peer_key or .fips_endpoint_npub == $peer_key)
      and .reachable == true
      and (.runtime_endpoint == "fips" or .endpoint == "fips")
      and (.direct_probe_pending == true or (.direct_probe_after_ms? != null))
    )
  ' >/dev/null <<<"$status"
}

wait_for_direct_peer() {
  local node="$1"
  local peer_key="$2"
  local direct_addr="$3"
  local label="$4"
  local deadline="$5"
  local status=""
  local end=$(( $(date +%s) + deadline ))
  while [[ "$(date +%s)" -le "$end" ]]; do
    status="$(status_json "$node")"
    if peer_matches_direct_addr "$status" "$peer_key" "$direct_addr"; then
      printf '%s\n' "$status"
      return 0
    fi
    sleep 1
  done

  echo "fips roaming e2e failed: $label did not use direct UDP $direct_addr within ${deadline}s" >&2
  printf '%s\n' "$status" >&2
  exit 1
}

wait_for_fallback_probe_peer() {
  local node="$1"
  local peer_key="$2"
  local label="$3"
  local deadline="$4"
  local status=""
  local end=$(( $(date +%s) + deadline ))
  while [[ "$(date +%s)" -le "$end" ]]; do
    status="$(status_json "$node")"
    if peer_matches_fallback_with_probe "$status" "$peer_key"; then
      printf '%s\n' "$status"
      return 0
    fi
    sleep 1
  done

  echo "fips roaming e2e failed: $label did not show fallback traffic with direct probing within ${deadline}s" >&2
  printf '%s\n' "$status" >&2
  exit 1
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
    echo "fips roaming e2e failed: $node utun100 MTU was '$actual', expected '$SAFE_TUNNEL_MTU'" >&2
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

assert_ping_tunnel() {
  local node="$1"
  local target_ip="$2"
  local label="$3"
  local log_path="$4"
  if ! ping_tunnel_payload "$node" "$target_ip" "$log_path"; then
    echo "fips roaming e2e failed: $label could not move $PING_PAYLOAD_SIZE-byte no-fragment ping payload" >&2
    cat "$log_path" >&2 2>/dev/null || true
    exit 1
  fi
}

run_udp_roundtrip() {
  local source_node="$1"
  local target_node="$2"
  local target_ip="$3"
  local payload="$4"
  local output="$5"
  "${COMPOSE[@]}" exec -T "$target_node" sh -lc \
    "rm -f '$output'; timeout 20 nc -u -l -p '$UDP_PORT' > '$output' & echo \$! > '${output}.pid'"
  "${COMPOSE[@]}" exec -T "$source_node" sh -lc \
    "printf '%s' '$payload' | nc -u -w 2 '$target_ip' '$UDP_PORT'"
  for _ in $(seq 1 20); do
    if "${COMPOSE[@]}" exec -T "$target_node" sh -lc "grep -q '$payload' '$output' 2>/dev/null"; then
      return 0
    fi
    sleep 1
  done
  echo "fips roaming e2e failed: UDP payload '$payload' did not arrive at $target_node" >&2
  exit 1
}

assert_no_transit_roster_peer() {
  local status="$1"
  local peer_key="$2"
  local label="$3"
  if jq -e --arg peer_key "$peer_key" '
    .daemon.state.peers
    | any(.participant_pubkey == $peer_key or .fips_endpoint_npub == $peer_key)
  ' >/dev/null <<<"$status"; then
    echo "fips roaming e2e failed: $label exposed transit hop as a data-plane peer" >&2
    printf '%s\n' "$status" >&2
    exit 1
  fi
}

run_roam_flap() {
  local flap_name="$1"
  echo "--- $flap_name: drop direct Alice<->Bob UDP, expect FIPS fallback plus direct probe ---"
  block_direct_alice_bob_udp

  local alice_fallback bob_fallback
  alice_fallback="$(wait_for_fallback_probe_peer node-a "$BOB_NPUB" "alice during $flap_name" "$FALLBACK_DEADLINE_SECS")"
  bob_fallback="$(wait_for_fallback_probe_peer node-b "$ALICE_NPUB" "bob during $flap_name" "$FALLBACK_DEADLINE_SECS")"

  assert_ping_tunnel node-a "$BOB_TUNNEL_IP" "alice-to-bob during $flap_name fallback" "/tmp/${flap_name}-alice-to-bob-fallback-ping.log"
  assert_ping_tunnel node-b "$ALICE_TUNNEL_IP" "bob-to-alice during $flap_name fallback" "/tmp/${flap_name}-bob-to-alice-fallback-ping.log"
  sleep "$FALLBACK_HOLD_SECS"

  alice_fallback="$(wait_for_fallback_probe_peer node-a "$BOB_NPUB" "alice after $flap_name hold" "$FALLBACK_DEADLINE_SECS")"
  bob_fallback="$(wait_for_fallback_probe_peer node-b "$ALICE_NPUB" "bob after $flap_name hold" "$FALLBACK_DEADLINE_SECS")"

  echo "--- $flap_name: restore LAN/direct path, expect quick upgrade away from fallback ---"
  unblock_direct_alice_bob_udp

  local alice_direct bob_direct
  alice_direct="$(wait_for_direct_peer node-a "$BOB_NPUB" "10.203.0.11:51820" "alice after $flap_name restore" "$DIRECT_RECOVERY_DEADLINE_SECS")"
  bob_direct="$(wait_for_direct_peer node-b "$ALICE_NPUB" "10.203.0.10:51820" "bob after $flap_name restore" "$DIRECT_RECOVERY_DEADLINE_SECS")"

  assert_ping_tunnel node-a "$BOB_TUNNEL_IP" "alice-to-bob after $flap_name direct restore" "/tmp/${flap_name}-alice-to-bob-direct-ping.log"
  assert_ping_tunnel node-b "$ALICE_TUNNEL_IP" "bob-to-alice after $flap_name direct restore" "/tmp/${flap_name}-bob-to-alice-direct-ping.log"

  echo "--- $flap_name fallback status: alice ---"
  echo "$alice_fallback"
  echo "--- $flap_name fallback status: bob ---"
  echo "$bob_fallback"
  echo "--- $flap_name restored direct status: alice ---"
  echo "$alice_direct"
  echo "--- $flap_name restored direct status: bob ---"
  echo "$bob_direct"
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
  echo "fips roaming e2e failed: unable to resolve node npubs from config" >&2
  exit 1
fi

"${COMPOSE[@]}" exec -T node-a nvpn set --participant "$ALICE_NPUB" >/dev/null
"${COMPOSE[@]}" exec -T node-b nvpn set --participant "$BOB_NPUB" >/dev/null
"${COMPOSE[@]}" exec -T node-c nvpn set --participant "$CHARLIE_NPUB" >/dev/null

"${COMPOSE[@]}" exec -T node-a nvpn set \
  --network-id "$NETWORK_ID" \
  --node-name alice \
  --participant "$ALICE_NPUB" \
  --participant "$BOB_NPUB" \
  --fips-peer-endpoint "$BOB_NPUB=10.203.0.11:51820" \
  --fips-peer-endpoint "$CHARLIE_NPUB=10.203.0.12:51820" \
  --endpoint "10.203.0.10:51820" \
  --listen-port 51820 \
  --fips-advertise-endpoint true \
  --fips-nostr-discovery-enabled false \
  --fips-bootstrap-enabled false >/dev/null

"${COMPOSE[@]}" exec -T node-b nvpn set \
  --network-id "$NETWORK_ID" \
  --node-name bob \
  --participant "$ALICE_NPUB" \
  --participant "$BOB_NPUB" \
  --fips-peer-endpoint "$ALICE_NPUB=10.203.0.10:51820" \
  --fips-peer-endpoint "$CHARLIE_NPUB=10.203.0.12:51820" \
  --endpoint "10.203.0.11:51820" \
  --listen-port 51820 \
  --fips-advertise-endpoint true \
  --fips-nostr-discovery-enabled false \
  --fips-bootstrap-enabled false >/dev/null

"${COMPOSE[@]}" exec -T node-c nvpn set \
  --network-id "$NETWORK_ID" \
  --node-name charlie \
  --fips-peer-endpoint "$ALICE_NPUB=10.203.0.10:51820" \
  --fips-peer-endpoint "$BOB_NPUB=10.203.0.11:51820" \
  --endpoint "10.203.0.12:51820" \
  --listen-port 51820 \
  --fips-advertise-endpoint true \
  --fips-nostr-discovery-enabled false \
  --fips-bootstrap-enabled false >/dev/null

for node in node-a node-b node-c; do
  replace_peer_aliases "$node"
  disable_nat_discovery "$node"
  start_nvpn_daemon "$node"
done

ALICE_STATUS="$(wait_for_mesh node-a 1)" || {
  echo "fips roaming e2e failed: alice mesh did not reach 1/1" >&2
  echo "$ALICE_STATUS"
  exit 1
}
BOB_STATUS="$(wait_for_mesh node-b 1)" || {
  echo "fips roaming e2e failed: bob mesh did not reach 1/1" >&2
  echo "$BOB_STATUS"
  exit 1
}

assert_no_transit_roster_peer "$ALICE_STATUS" "$CHARLIE_NPUB" "alice"
assert_no_transit_roster_peer "$BOB_STATUS" "$CHARLIE_NPUB" "bob"

ALICE_DIRECT="$(wait_for_direct_peer node-a "$BOB_NPUB" "10.203.0.11:51820" "alice initial LAN" "$DIRECT_RECOVERY_DEADLINE_SECS")"
BOB_DIRECT="$(wait_for_direct_peer node-b "$ALICE_NPUB" "10.203.0.10:51820" "bob initial LAN" "$DIRECT_RECOVERY_DEADLINE_SECS")"

BOB_TUNNEL_IP="$(resolve_magic_dns node-a bob.nvpn)"
ALICE_TUNNEL_IP="$(resolve_magic_dns node-b alice.nvpn)"

if [[ -z "$BOB_TUNNEL_IP" || -z "$ALICE_TUNNEL_IP" ]]; then
  echo "fips roaming e2e failed: magic dns did not resolve alice.nvpn/bob.nvpn" >&2
  exit 1
fi

assert_tunnel_mtu node-a
assert_tunnel_mtu node-b
assert_ping_tunnel node-a "$BOB_TUNNEL_IP" "initial alice-to-bob direct LAN" /tmp/initial-alice-to-bob-ping.log
assert_ping_tunnel node-b "$ALICE_TUNNEL_IP" "initial bob-to-alice direct LAN" /tmp/initial-bob-to-alice-ping.log
run_udp_roundtrip node-a node-b "$BOB_TUNNEL_IP" "alice-to-bob-roaming-initial" /tmp/bob-roaming-initial-udp.out

run_roam_flap "mobile-flap-1"
run_roam_flap "mobile-flap-2"

echo "--- Initial direct status: alice ---"
echo "$ALICE_DIRECT"
echo "--- Initial direct status: bob ---"
echo "$BOB_DIRECT"
echo "--- Magic DNS ---"
echo "node-a bob.nvpn -> $BOB_TUNNEL_IP"
echo "node-b alice.nvpn -> $ALICE_TUNNEL_IP"
echo "--- Initial pings ---"
cat /tmp/initial-alice-to-bob-ping.log
cat /tmp/initial-bob-to-alice-ping.log
echo "--- Initial UDP payload ---"
"${COMPOSE[@]}" exec -T node-b sh -lc 'cat /tmp/bob-roaming-initial-udp.out'

echo "fips roaming docker e2e passed: direct LAN path established, two mobile/WiFi-style direct drops used FIPS fallback while direct probing stayed pending, and each restore upgraded back to direct within ${DIRECT_RECOVERY_DEADLINE_SECS}s"
