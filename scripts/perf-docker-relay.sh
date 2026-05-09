#!/usr/bin/env bash
# 3-node FIPS overlay perf bench: A and B can only reach each other through C.
#
# Topology (docker bridge 10.203.0.0/24):
#   A (.10)  ──┐                    ┌──  B (.11)
#              └──> C (.12) <───────┘
#
# A's only static FIPS peer is C. B's only static FIPS peer is C. C peers
# directly with both. fips-core's spanning-tree routing learns that B
# (and A) is reachable via C and forwards encrypted session datagrams
# through C — every A↔B byte transits C.
#
# iperf3 between A's mesh tunnel IP and B's mesh tunnel IP exercises that
# transit path.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
PROJECT_NAME="${PROJECT_NAME:-nvpn-perf-relay}"
COMPOSE=(docker compose -p "$PROJECT_NAME" -f "$ROOT_DIR/docker-compose.e2e.yml")

NETWORK_ID="docker-perf-relay"
DURATION="${DURATION:-10}"

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
  echo "perf-relay: service '$service' did not start" >&2
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
"${COMPOSE[@]}" up -d node-a node-b node-c >/dev/null
for service in node-a node-b node-c; do
  wait_for_service "$service"
done

# Force the test to actually exercise the transit path: drop direct UDP
# between A and B. With Nostr discovery + NAT traversal enabled, A and B
# would otherwise discover each other on the bridge (10.203.0.0/24) and
# bypass C entirely. With these iptables rules in place the only way
# A's tunnel datagrams reach B is via C's spanning-tree forwarding.
"${COMPOSE[@]}" exec -T node-a iptables -A OUTPUT -p udp -d 10.203.0.11 -j DROP
"${COMPOSE[@]}" exec -T node-a iptables -A INPUT  -p udp -s 10.203.0.11 -j DROP
"${COMPOSE[@]}" exec -T node-b iptables -A OUTPUT -p udp -d 10.203.0.10 -j DROP
"${COMPOSE[@]}" exec -T node-b iptables -A INPUT  -p udp -s 10.203.0.10 -j DROP

"${COMPOSE[@]}" exec -T node-a nvpn init --force >/dev/null
"${COMPOSE[@]}" exec -T node-b nvpn init --force >/dev/null
"${COMPOSE[@]}" exec -T node-c nvpn init --force >/dev/null
A_NPUB="$(nostr_pubkey_from_config node-a)"
B_NPUB="$(nostr_pubkey_from_config node-b)"
C_NPUB="$(nostr_pubkey_from_config node-c)"

# Each node knows ALL participants (membership) but only the direct
# physical peer endpoints below. A and B don't know each other's UDP
# endpoint, so reaching the other's tunnel IP must go via C.
"${COMPOSE[@]}" exec -T node-a nvpn set \
  --network-id "$NETWORK_ID" \
  --participant "$A_NPUB" \
  --participant "$B_NPUB" \
  --participant "$C_NPUB" \
  --endpoint "10.203.0.10:51820" \
  --listen-port 51820 \
  --fips-advertise-endpoint true \
  --fips-peer-endpoint "$C_NPUB=10.203.0.12:51820" >/dev/null

"${COMPOSE[@]}" exec -T node-b nvpn set \
  --network-id "$NETWORK_ID" \
  --participant "$A_NPUB" \
  --participant "$B_NPUB" \
  --participant "$C_NPUB" \
  --endpoint "10.203.0.11:51820" \
  --listen-port 51820 \
  --fips-advertise-endpoint true \
  --fips-peer-endpoint "$C_NPUB=10.203.0.12:51820" >/dev/null

"${COMPOSE[@]}" exec -T node-c nvpn set \
  --network-id "$NETWORK_ID" \
  --participant "$A_NPUB" \
  --participant "$B_NPUB" \
  --participant "$C_NPUB" \
  --endpoint "10.203.0.12:51820" \
  --listen-port 51820 \
  --fips-advertise-endpoint true \
  --fips-peer-endpoint "$A_NPUB=10.203.0.10:51820" \
  --fips-peer-endpoint "$B_NPUB=10.203.0.11:51820" >/dev/null

A_TUNNEL_IP="$("${COMPOSE[@]}" exec -T node-a nvpn ip | tr -d '\r')"
B_TUNNEL_IP="$("${COMPOSE[@]}" exec -T node-b nvpn ip | tr -d '\r')"
C_TUNNEL_IP="$("${COMPOSE[@]}" exec -T node-c nvpn ip | tr -d '\r')"

"${COMPOSE[@]}" exec -d node-a sh -lc "nvpn connect > /tmp/connect.log 2>&1"
"${COMPOSE[@]}" exec -d node-b sh -lc "nvpn connect > /tmp/connect.log 2>&1"
"${COMPOSE[@]}" exec -d node-c sh -lc "nvpn connect > /tmp/connect.log 2>&1"

# Wait until all three peers report mesh: 2/2 peers connected. NAT
# traversal between A and B routes through public Nostr relays; round-
# trip latency on the offer/answer DM exchange is the long pole, so
# allow up to 45s before declaring failure.
mesh_up=0
for _ in $(seq 1 45); do
  a="$("${COMPOSE[@]}" exec -T node-a sh -lc 'cat /tmp/connect.log 2>/dev/null || true')"
  b="$("${COMPOSE[@]}" exec -T node-b sh -lc 'cat /tmp/connect.log 2>/dev/null || true')"
  c="$("${COMPOSE[@]}" exec -T node-c sh -lc 'cat /tmp/connect.log 2>/dev/null || true')"
  if grep -q "mesh: 2/2 peers connected" <<<"$a" \
    && grep -q "mesh: 2/2 peers connected" <<<"$b" \
    && grep -q "mesh: 2/2 peers connected" <<<"$c"; then
    mesh_up=1
    break
  fi
  sleep 1
done

dump_logs() {
  echo "--- node-a connect log (last 15) ---"
  "${COMPOSE[@]}" exec -T node-a sh -lc 'tail -15 /tmp/connect.log 2>/dev/null' || true
  echo "--- node-b connect log (last 15) ---"
  "${COMPOSE[@]}" exec -T node-b sh -lc 'tail -15 /tmp/connect.log 2>/dev/null' || true
  echo "--- node-c connect log (last 15) ---"
  "${COMPOSE[@]}" exec -T node-c sh -lc 'tail -15 /tmp/connect.log 2>/dev/null' || true
}

if [[ $mesh_up -ne 1 ]]; then
  echo "perf-relay: mesh did not converge to 2/2 within 15s; aborting" >&2
  dump_logs
  exit 1
fi

if ! "${COMPOSE[@]}" exec -T node-a ping -c 3 -W 2 "$B_TUNNEL_IP" >/dev/null 2>&1; then
  echo "perf-relay: ping a->b over mesh failed; transit not established" >&2
  dump_logs
  exit 1
fi

echo "alice tunnel ip: $A_TUNNEL_IP"
echo "bob   tunnel ip: $B_TUNNEL_IP"
echo "carol tunnel ip: $C_TUNNEL_IP   (transit relay)"
echo

"${COMPOSE[@]}" exec -d node-b sh -lc "iperf3 -s -D --logfile /tmp/iperf3-server.log"
sleep 1

run_test() {
  local label="$1"; shift
  printf '## %s\n' "$label"
  # --connect-timeout caps the initial 3WHS so a black-holed transit path
  # bails out in 3s instead of hanging tcp_synack_retries (~120s).
  "${COMPOSE[@]}" exec -T node-a iperf3 -c "$B_TUNNEL_IP" -t "$DURATION" -i 0 -f m \
    --connect-timeout 3000 "$@" 2>&1 | tail -6
  echo
}

run_test "TCP single stream (A -> C -> B)"
run_test "TCP 4 streams" -P 4
run_test "TCP 8 streams" -P 8
run_test "UDP 200 Mbit target" -u -b 200M
run_test "UDP 1000 Mbit target" -u -b 1G

printf '## ping (300 packets, 10ms apart) over mesh transit (A -> C -> B)\n'
"${COMPOSE[@]}" exec -T node-a ping -c 300 -i 0.01 -q "$B_TUNNEL_IP" 2>&1 | tail -3
