#!/usr/bin/env bash
# Release-gate FIPS dataplane regression check.
#
# This is not a benchmark leaderboard. It is a conservative pass/fail guard for
# the failure modes that hurt interactive traffic: collapsed TCP throughput,
# packet loss, and ICMP/liveness packets sitting behind a saturated TCP flow for
# seconds after the direct path itself is healthy.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
PROJECT_NAME="${PROJECT_NAME:-nostr-vpn-e2e-fips-perf}"
COMPOSE=(docker compose -p "$PROJECT_NAME" -f "$ROOT_DIR/docker-compose.e2e.yml")

NETWORK_ID="docker-fips-perf"
CONFIG_PATH="/root/.config/nvpn/config.toml"
DURATION="${NVPN_PERF_DURATION_SECS:-8}"
LOAD_DURATION="${NVPN_PERF_LOAD_DURATION_SECS:-12}"
MIN_TCP_MBIT="${NVPN_PERF_MIN_TCP_MBIT:-100}"
MIN_REVERSE_TCP_MBIT="${NVPN_PERF_MIN_REVERSE_TCP_MBIT:-100}"
MAX_PING_LOSS_PERCENT="${NVPN_PERF_MAX_PING_LOSS_PERCENT:-2}"
MAX_PING_AVG_MS="${NVPN_PERF_MAX_PING_AVG_MS:-250}"
MAX_PING_MAX_MS="${NVPN_PERF_MAX_PING_MAX_MS:-1000}"
PING_COUNT="${NVPN_PERF_PING_COUNT:-30}"
PING_INTERVAL="${NVPN_PERF_PING_INTERVAL:-0.1}"
FIPS_NOSTR_DISCOVERY_POLICY="${NVPN_FIPS_NOSTR_DISCOVERY_POLICY:-configured_only}"

if [[ -z "${NVPN_FIPS_REPO_PATH:-}" && -d "$ROOT_DIR/../fips/crates/fips-core" ]]; then
  export NVPN_FIPS_REPO_PATH="$ROOT_DIR/../fips"
fi

cleanup() {
  "${COMPOSE[@]}" down -v --remove-orphans >/dev/null 2>&1 || true
  docker network rm "${PROJECT_NAME}_e2e" >/dev/null 2>&1 || true
}

dump_debug() {
  set +e
  echo "fips perf regression e2e failed, collecting debug output..."
  "${COMPOSE[@]}" ps || true
  for service in node-a node-b; do
    echo "--- logs: $service ---"
    "${COMPOSE[@]}" logs --no-color --tail 120 "$service" || true
    echo "--- $service status ---"
    "${COMPOSE[@]}" exec -T "$service" nvpn status --json --discover-secs 0 || true
    echo "--- $service daemon.log ---"
    "${COMPOSE[@]}" exec -T "$service" sh -lc "tail -n 240 /root/.config/nvpn/daemon.log 2>/dev/null || true" || true
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

  echo "fips perf regression e2e failed: service '$service' did not reach running state" >&2
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

wait_for_mesh() {
  for _ in $(seq 1 45); do
    local a b
    a="$("${COMPOSE[@]}" exec -T node-a sh -lc 'cat /tmp/connect.log 2>/dev/null || true')"
    b="$("${COMPOSE[@]}" exec -T node-b sh -lc 'cat /tmp/connect.log 2>/dev/null || true')"
    if grep -q "mesh: 1/1 peers connected" <<<"$a" \
      && grep -q "mesh: 1/1 peers connected" <<<"$b"; then
      return 0
    fi
    sleep 1
  done

  echo "fips perf regression e2e failed: mesh did not converge to 1/1" >&2
  return 1
}

assert_float_at_least() {
  local actual="$1"
  local min="$2"
  local label="$3"
  awk -v actual="$actual" -v min="$min" -v label="$label" '
    BEGIN {
      if ((actual + 0) < (min + 0)) {
        printf "fips perf regression e2e failed: %s %.1f below minimum %.1f\n", label, actual, min > "/dev/stderr"
        exit 1
      }
    }
  '
}

assert_float_at_most() {
  local actual="$1"
  local max="$2"
  local label="$3"
  awk -v actual="$actual" -v max="$max" -v label="$label" '
    BEGIN {
      if ((actual + 0) > (max + 0)) {
        printf "fips perf regression e2e failed: %s %.1f above maximum %.1f\n", label, actual, max > "/dev/stderr"
        exit 1
      }
    }
  '
}

iperf_mbps() {
  jq -r '(.end.sum_received.bits_per_second // .end.sum.bits_per_second) / 1000000'
}

iperf_retransmits() {
  jq -r '(.end.sum_sent.retransmits // .end.sum.retransmits // 0)'
}

run_iperf_json() {
  local label="$1"
  shift
  local output
  if ! output="$("${COMPOSE[@]}" exec -T node-a iperf3 \
      -J -c "$BOB_TUNNEL_IP" -t "$DURATION" -O 1 --connect-timeout 3000 "$@" 2>&1)"; then
    echo "fips perf regression e2e failed: iperf $label failed" >&2
    printf '%s\n' "$output" >&2
    exit 1
  fi
  printf '%s\n' "$output"
}

parse_ping_stats() {
  awk '
    /packets transmitted/ {
      loss = $0
      sub(/^.*received, /, "", loss)
      sub(/% packet loss.*$/, "", loss)
    }
    /^rtt / || /^round-trip / {
      split($0, parts, "=")
      split(parts[2], values, "/")
      avg = values[2]
      max = values[3]
      sub(/^ /, "", avg)
      sub(/^ /, "", max)
    }
    END {
      if (loss == "" || avg == "" || max == "") {
        exit 1
      }
      printf "%s %s %s\n", loss, avg, max
    }
  '
}

assert_ping_ok() {
  local label="$1"
  local output="$2"
  local stats loss avg max
  if ! stats="$(printf '%s\n' "$output" | parse_ping_stats)"; then
    echo "fips perf regression e2e failed: could not parse ping stats for $label" >&2
    printf '%s\n' "$output" >&2
    exit 1
  fi
  read -r loss avg max <<<"$stats"
  printf '%s ping: loss=%s%% avg=%sms max=%sms\n' "$label" "$loss" "$avg" "$max"
  assert_float_at_most "$loss" "$MAX_PING_LOSS_PERCENT" "$label ping loss %"
  assert_float_at_most "$avg" "$MAX_PING_AVG_MS" "$label ping avg ms"
  assert_float_at_most "$max" "$MAX_PING_MAX_MS" "$label ping max ms"
}

cleanup
BUILDKIT_PROGRESS=plain "${COMPOSE[@]}" build node-a node-b
"${COMPOSE[@]}" up -d node-a node-b >/dev/null
for service in node-a node-b; do
  wait_for_service "$service"
done

"${COMPOSE[@]}" exec -T node-a nvpn init --force >/dev/null
"${COMPOSE[@]}" exec -T node-b nvpn init --force >/dev/null
ALICE_NPUB="$(nostr_pubkey_from_config node-a)"
BOB_NPUB="$(nostr_pubkey_from_config node-b)"
if [[ -z "$ALICE_NPUB" || -z "$BOB_NPUB" ]]; then
  echo "fips perf regression e2e failed: unable to resolve node npubs from config" >&2
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
  --participant "$ALICE_NPUB" \
  --participant "$BOB_NPUB" \
  --endpoint "10.203.0.10:51820" \
  --listen-port 51820 \
  --fips-advertise-endpoint true \
  --fips-nostr-discovery-enabled false \
  --fips-bootstrap-enabled false \
  --fips-peer-endpoint "$BOB_NPUB=10.203.0.11:51820" >/dev/null

"${COMPOSE[@]}" exec -T node-b nvpn set \
  --network-id "$NETWORK_ID" \
  --participant "$ALICE_NPUB" \
  --participant "$BOB_NPUB" \
  --endpoint "10.203.0.11:51820" \
  --listen-port 51820 \
  --fips-advertise-endpoint true \
  --fips-nostr-discovery-enabled false \
  --fips-bootstrap-enabled false \
  --fips-peer-endpoint "$ALICE_NPUB=10.203.0.10:51820" >/dev/null

ALICE_TUNNEL_IP="$("${COMPOSE[@]}" exec -T node-a nvpn ip | tr -d '\r')"
BOB_TUNNEL_IP="$("${COMPOSE[@]}" exec -T node-b nvpn ip | tr -d '\r')"

"${COMPOSE[@]}" exec -d node-a sh -lc \
  "NVPN_FIPS_NOSTR_DISCOVERY_POLICY='$FIPS_NOSTR_DISCOVERY_POLICY' nvpn connect > /tmp/connect.log 2>&1"
"${COMPOSE[@]}" exec -d node-b sh -lc \
  "NVPN_FIPS_NOSTR_DISCOVERY_POLICY='$FIPS_NOSTR_DISCOVERY_POLICY' nvpn connect > /tmp/connect.log 2>&1"

wait_for_mesh

if ! "${COMPOSE[@]}" exec -T node-a ping -c 3 -W 2 "$BOB_TUNNEL_IP" >/dev/null; then
  echo "fips perf regression e2e failed: baseline tunnel ping failed" >&2
  exit 1
fi

echo "alice tunnel ip: $ALICE_TUNNEL_IP"
echo "bob   tunnel ip: $BOB_TUNNEL_IP"
echo "thresholds: tcp>=${MIN_TCP_MBIT}M reverse>=${MIN_REVERSE_TCP_MBIT}M ping_loss<=${MAX_PING_LOSS_PERCENT}% ping_avg<=${MAX_PING_AVG_MS}ms ping_max<=${MAX_PING_MAX_MS}ms"

"${COMPOSE[@]}" exec -d node-b sh -lc "iperf3 -s -D --logfile /tmp/iperf3-server.log"
sleep 1

forward_json="$(run_iperf_json "forward TCP")"
forward_mbps="$(printf '%s\n' "$forward_json" | iperf_mbps)"
forward_retrans="$(printf '%s\n' "$forward_json" | iperf_retransmits)"
printf 'forward TCP: %.1f Mbps retrans=%s\n' "$forward_mbps" "$forward_retrans"
assert_float_at_least "$forward_mbps" "$MIN_TCP_MBIT" "forward TCP throughput Mbps"

reverse_json="$(run_iperf_json "reverse TCP" -R)"
reverse_mbps="$(printf '%s\n' "$reverse_json" | iperf_mbps)"
reverse_retrans="$(printf '%s\n' "$reverse_json" | iperf_retransmits)"
printf 'reverse TCP: %.1f Mbps retrans=%s\n' "$reverse_mbps" "$reverse_retrans"
assert_float_at_least "$reverse_mbps" "$MIN_REVERSE_TCP_MBIT" "reverse TCP throughput Mbps"

concurrent_json_path="$(mktemp)"
concurrent_err_path="$(mktemp)"
"${COMPOSE[@]}" exec -T node-a iperf3 \
  -J -c "$BOB_TUNNEL_IP" -t "$LOAD_DURATION" -O 1 --connect-timeout 3000 \
  >"$concurrent_json_path" 2>"$concurrent_err_path" &
iperf_pid=$!
sleep 1
concurrent_ping="$("${COMPOSE[@]}" exec -T node-a ping \
  -c "$PING_COUNT" -i "$PING_INTERVAL" -W 2 "$BOB_TUNNEL_IP" 2>&1)"
if ! wait "$iperf_pid"; then
  echo "fips perf regression e2e failed: concurrent iperf failed" >&2
  cat "$concurrent_err_path" >&2
  exit 1
fi
concurrent_mbps="$(iperf_mbps <"$concurrent_json_path")"
concurrent_retrans="$(iperf_retransmits <"$concurrent_json_path")"
printf 'concurrent TCP load: %.1f Mbps retrans=%s\n' "$concurrent_mbps" "$concurrent_retrans"
assert_float_at_least "$concurrent_mbps" "$MIN_TCP_MBIT" "concurrent TCP throughput Mbps"
assert_ping_ok "during TCP load" "$concurrent_ping"

post_ping="$("${COMPOSE[@]}" exec -T node-a ping \
  -c "$PING_COUNT" -i "$PING_INTERVAL" -W 2 "$BOB_TUNNEL_IP" 2>&1)"
assert_ping_ok "after TCP load" "$post_ping"

echo "fips perf regression docker e2e passed: throughput stayed above floor and pings did not wedge under or after TCP load"
