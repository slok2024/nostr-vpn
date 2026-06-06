#!/usr/bin/env bash
# Smoke-test the userspace (boringtun) WG upstream path by running
# `nvpn wg-upstream-test` against a real wg-quick server inside Docker.
#
# Two stages:
#   1. Handshake-only probe: no tun device, no route changes. Safest
#      possible test — even a broken config can never blackhole the
#      host.
#   2. Scoped-host data plane: brings up a userspace tun, installs a
#      *single* host route (203.0.113.100) through it — the default
#      route is untouched — and pings the target through the WG
#      tunnel, verifying boringtun encrypt → wg-quick decrypt →
#      forward → reply path.
#
# Topology (reuses docker-compose.wireguard-exit-e2e.yml):
#   internet (10.203.0.0/24)          public (203.0.113.0/24)
#     - wg-upstream  10.203.0.20        - wg-upstream    203.0.113.20
#     - node-a       10.203.0.10        - internet-target 203.0.113.100
#
# Pass criteria: both stages succeed and internet-target sees pings
# arriving from the WG-upstream's public IP (proving they actually went
# through the tunnel rather than leaking direct).

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
PROJECT_NAME="nostr-vpn-e2e-wireguard-exit-userspace"
COMPOSE=(docker compose -p "$PROJECT_NAME" -f "$ROOT_DIR/docker-compose.wireguard-exit-e2e.yml")

WG_UPSTREAM_IP="10.203.0.20"
WG_UPSTREAM_PUBLIC_IP="203.0.113.20"
NODE_A_IP="10.203.0.10"
TARGET_IP="203.0.113.100"
WG_LISTEN_PORT="51820"
WG_TUNNEL_NET="10.99.99.0/24"
WG_SERVER_TUNNEL_IP="10.99.99.1"
WG_CLIENT_TUNNEL_IP="10.99.99.2"

cleanup() {
  "${COMPOSE[@]}" down -v --remove-orphans >/dev/null 2>&1 || true
  docker network rm \
    "${PROJECT_NAME}_internet" \
    "${PROJECT_NAME}_public" >/dev/null 2>&1 || true
  for network in "${PROJECT_NAME}_internet" "${PROJECT_NAME}_public"; do
    for _ in $(seq 1 20); do
      docker network inspect "$network" >/dev/null 2>&1 || break
      sleep 1
    done
  done
}

dump_debug() {
  set +e
  echo "wg-upstream userspace e2e failed, collecting debug output..."
  "${COMPOSE[@]}" ps || true
  echo "--- node-a: nvpn version ---"
  "${COMPOSE[@]}" exec -T node-a nvpn version --json || true
  echo "--- node-a: cat wg-upstream.conf ---"
  "${COMPOSE[@]}" exec -T node-a sh -lc "cat /tmp/wg-upstream.conf 2>/dev/null || echo 'no config'" || true
  echo "--- wg-upstream: wg show ---"
  "${COMPOSE[@]}" exec -T wg-upstream sh -lc "wg show || true" || true
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

  echo "wg-upstream userspace e2e failed: service '$service' did not reach running state" >&2
  exit 1
}

cleanup

"${COMPOSE[@]}" build wg-upstream node-a internet-target >/dev/null
"${COMPOSE[@]}" up -d wg-upstream node-a internet-target >/dev/null
for service in wg-upstream node-a internet-target; do
  wait_for_service "$service"
done

# Generate WG keypairs on the upstream server.
# The e2e image only ships dash; do not enable -o pipefail here.
"${COMPOSE[@]}" exec -T wg-upstream sh -eu -c '
umask 077
mkdir -p /etc/wireguard
[ -s /etc/wireguard/server.key ] || wg genkey > /etc/wireguard/server.key
[ -s /etc/wireguard/client.key ] || wg genkey > /etc/wireguard/client.key
wg pubkey < /etc/wireguard/server.key > /etc/wireguard/server.pub
wg pubkey < /etc/wireguard/client.key > /etc/wireguard/client.pub
' >/dev/null

SERVER_PRIV="$("${COMPOSE[@]}" exec -T wg-upstream cat /etc/wireguard/server.key | tr -d '\r\n')"
SERVER_PUB="$("${COMPOSE[@]}" exec -T wg-upstream cat /etc/wireguard/server.pub | tr -d '\r\n')"
CLIENT_PRIV="$("${COMPOSE[@]}" exec -T wg-upstream cat /etc/wireguard/client.key | tr -d '\r\n')"
CLIENT_PUB="$("${COMPOSE[@]}" exec -T wg-upstream cat /etc/wireguard/client.pub | tr -d '\r\n')"

# Bring up the WG server interface on wg-upstream and install
# MASQUERADE on the public-side eth so decrypted client traffic can
# reach the internet-target. (Same setup the kernel-WG e2e uses.)
"${COMPOSE[@]}" exec -T wg-upstream sh -eu -c "
public_iface=\"\$(ip -o -4 addr show | awk '\$4 == \"${WG_UPSTREAM_PUBLIC_IP}/24\" { print \$2; exit }')\"
[ -n \"\$public_iface\" ]

ip link del wg0 2>/dev/null || true
ip link add dev wg0 type wireguard
ip address add ${WG_SERVER_TUNNEL_IP}/24 dev wg0
wg set wg0 listen-port ${WG_LISTEN_PORT} private-key /etc/wireguard/server.key
wg set wg0 peer ${CLIENT_PUB} allowed-ips ${WG_CLIENT_TUNNEL_IP}/32
ip link set wg0 up

iptables -P FORWARD ACCEPT
iptables -A FORWARD -i wg0 -j ACCEPT
iptables -A FORWARD -o wg0 -j ACCEPT
iptables -t nat -C POSTROUTING -o \"\$public_iface\" -s ${WG_TUNNEL_NET} -j MASQUERADE 2>/dev/null \
  || iptables -t nat -A POSTROUTING -o \"\$public_iface\" -s ${WG_TUNNEL_NET} -j MASQUERADE
" >/dev/null

# Compose the WG config text and drop it on node-a as a file. Reading
# from a file (rather than from --wireguard-exit-config inline) is what
# the production GUI flow does too, so this exercises the same parser
# path.
WG_CONFIG="[Interface]
PrivateKey = ${CLIENT_PRIV}
Address = ${WG_CLIENT_TUNNEL_IP}/32
MTU = 1420

[Peer]
PublicKey = ${SERVER_PUB}
Endpoint = ${WG_UPSTREAM_IP}:${WG_LISTEN_PORT}
AllowedIPs = 0.0.0.0/0, ::/0
PersistentKeepalive = 25
"

"${COMPOSE[@]}" exec -T node-a sh -lc 'cat > /tmp/wg-upstream.conf' <<<"$WG_CONFIG"

# Stage 1: handshake-only probe. Safe even on a host with live
# internet — no tun, no route changes.
echo "--- stage 1: handshake-only probe ---"
"${COMPOSE[@]}" exec -T node-a nvpn wg-upstream-test \
  --config-file /tmp/wg-upstream.conf \
  --timeout-secs 15

# Stage 2: scoped-host data plane. node-a creates a userspace tun,
# installs a *single* host route (203.0.113.100 via the tun), and
# sends ICMP through it. The default route stays via eth0, so the
# only thing this can possibly break is the route to 203.0.113.100
# itself.
echo "--- stage 2: scoped-host data plane ---"

# Counters on internet-target so we can verify pings actually traversed
# the WG tunnel (source = MASQUERADEd public IP, not node-a's eth IP).
"${COMPOSE[@]}" exec -T internet-target sh -lc "
iptables -F nvpn-wg-userspace-counts 2>/dev/null || iptables -N nvpn-wg-userspace-counts
iptables -A nvpn-wg-userspace-counts -s ${WG_UPSTREAM_PUBLIC_IP} -p icmp -j RETURN
iptables -A nvpn-wg-userspace-counts -s ${NODE_A_IP} -p icmp -j RETURN
iptables -C INPUT -j nvpn-wg-userspace-counts 2>/dev/null \
  || iptables -I INPUT -j nvpn-wg-userspace-counts
iptables -Z nvpn-wg-userspace-counts
" >/dev/null

"${COMPOSE[@]}" exec -T node-a nvpn wg-upstream-test \
  --config-file /tmp/wg-upstream.conf \
  --timeout-secs 15 \
  --scoped-host "${TARGET_IP}" \
  --ping-count 5

# Read counters back from the target.
COUNT_VIA_WG="$("${COMPOSE[@]}" exec -T internet-target sh -lc "iptables -L nvpn-wg-userspace-counts -v -n -x | awk -v ip='${WG_UPSTREAM_PUBLIC_IP}' '\$0 ~ ip { print \$1 }'" | tr -d '\r')"
COUNT_DIRECT="$("${COMPOSE[@]}" exec -T internet-target sh -lc "iptables -L nvpn-wg-userspace-counts -v -n -x | awk -v ip='${NODE_A_IP}' '\$0 ~ ip { print \$1 }'" | tr -d '\r')"

echo "--- ICMP packet counts at internet-target ---"
"${COMPOSE[@]}" exec -T internet-target iptables -L nvpn-wg-userspace-counts -v -n -x | tr -d '\r'

if [[ -z "$COUNT_VIA_WG" || "$COUNT_VIA_WG" == "0" ]]; then
  echo "wg-upstream userspace e2e failed: target saw 0 ICMP packets from the WG upstream's public IP" >&2
  exit 1
fi
if [[ -n "$COUNT_DIRECT" && "$COUNT_DIRECT" != "0" ]]; then
  echo "wg-upstream userspace e2e failed: target saw ${COUNT_DIRECT} ICMP packets directly from node-a's bridge IP — traffic leaked outside the WG tunnel" >&2
  exit 1
fi

# Stage 3: --replace-default. Same boringtun runtime, but this time we
# capture the original default route, install a /32 bypass for the WG
# upstream's UDP endpoint, and swap the default route to dev <wg-tun>.
# All outbound traffic (ICMP to a target on a separate subnet) should
# round-trip through the WG tunnel; the watchdog should fire and
# auto-revert if the handshake doesn't complete in time.
echo "--- stage 3: --replace-default ---"

# Reset counters.
"${COMPOSE[@]}" exec -T internet-target sh -lc "
iptables -Z nvpn-wg-userspace-counts
" >/dev/null

"${COMPOSE[@]}" exec -T node-a nvpn wg-upstream-test \
  --config-file /tmp/wg-upstream.conf \
  --timeout-secs 15 \
  --replace-default \
  --probe-target "${TARGET_IP}" \
  --ping-count 5

# Confirm the pings traversed the tunnel just like in stage 2.
COUNT_VIA_WG_DEFAULT="$("${COMPOSE[@]}" exec -T internet-target sh -lc "iptables -L nvpn-wg-userspace-counts -v -n -x | awk -v ip='${WG_UPSTREAM_PUBLIC_IP}' '\$0 ~ ip { print \$1 }'" | tr -d '\r')"
COUNT_DIRECT_DEFAULT="$("${COMPOSE[@]}" exec -T internet-target sh -lc "iptables -L nvpn-wg-userspace-counts -v -n -x | awk -v ip='${NODE_A_IP}' '\$0 ~ ip { print \$1 }'" | tr -d '\r')"

echo "--- ICMP packet counts at internet-target (--replace-default) ---"
"${COMPOSE[@]}" exec -T internet-target iptables -L nvpn-wg-userspace-counts -v -n -x | tr -d '\r'

if [[ -z "$COUNT_VIA_WG_DEFAULT" || "$COUNT_VIA_WG_DEFAULT" == "0" ]]; then
  echo "wg-upstream userspace e2e failed (replace-default): target saw 0 ICMP packets from the WG upstream's public IP" >&2
  exit 1
fi
if [[ -n "$COUNT_DIRECT_DEFAULT" && "$COUNT_DIRECT_DEFAULT" != "0" ]]; then
  echo "wg-upstream userspace e2e failed (replace-default): target saw ${COUNT_DIRECT_DEFAULT} ICMP packets directly from node-a's bridge IP" >&2
  exit 1
fi

# Confirm node-a's default route is back to the docker bridge after the
# command exited (proves the FullDefaultRoute Drop guard fired).
DEFAULT_AFTER="$("${COMPOSE[@]}" exec -T node-a sh -lc 'ip -4 route show default' | tr -d '\r')"
if grep -qE 'dev (utun|nvpn-wg|wg-)' <<<"$DEFAULT_AFTER"; then
  echo "wg-upstream userspace e2e failed (replace-default): default route was NOT restored on cleanup; still: ${DEFAULT_AFTER}" >&2
  exit 1
fi
echo "default route restored cleanly: ${DEFAULT_AFTER}"

echo "wg-upstream userspace e2e passed: handshake + scoped-host (5 pkts) + replace-default (${COUNT_VIA_WG_DEFAULT} pkts) all worked, default route restored"
