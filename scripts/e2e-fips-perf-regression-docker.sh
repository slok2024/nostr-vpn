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
PING_COUNT="${NVPN_PERF_PING_COUNT:-60}"
PING_INTERVAL="${NVPN_PERF_PING_INTERVAL:-0.1}"
FIPS_NOSTR_DISCOVERY_POLICY="${NVPN_FIPS_NOSTR_DISCOVERY_POLICY:-configured_only}"
CONSTRAINED_RATE_MBIT="${NVPN_PERF_CONSTRAINED_RATE_MBIT:-250}"
CONSTRAINED_BURST_KB="${NVPN_PERF_CONSTRAINED_BURST_KB:-32}"
CONSTRAINED_LATENCY_MS="${NVPN_PERF_CONSTRAINED_LATENCY_MS:-100}"
CONSTRAINED_MIN_TCP_MBIT="${NVPN_PERF_CONSTRAINED_MIN_TCP_MBIT:-100}"
CONSTRAINED_MIN_REVERSE_TCP_MBIT="${NVPN_PERF_CONSTRAINED_MIN_REVERSE_TCP_MBIT:-100}"
CONSTRAINED_MAX_PING_LOSS_PERCENT="${NVPN_PERF_CONSTRAINED_MAX_PING_LOSS_PERCENT:-2}"
CONSTRAINED_MAX_PING_AVG_MS="${NVPN_PERF_CONSTRAINED_MAX_PING_AVG_MS:-250}"
CONSTRAINED_MAX_PING_MAX_MS="${NVPN_PERF_CONSTRAINED_MAX_PING_MAX_MS:-1000}"
RX_MAINT_FAULT_MS="${NVPN_PERF_RX_MAINT_FAULT_MS:-250}"
RX_MAINT_MIN_TCP_MBIT="${NVPN_PERF_RX_MAINT_MIN_TCP_MBIT:-100}"
RX_MAINT_MIN_REVERSE_TCP_MBIT="${NVPN_PERF_RX_MAINT_MIN_REVERSE_TCP_MBIT:-100}"
RX_MAINT_MAX_PING_LOSS_PERCENT="${NVPN_PERF_RX_MAINT_MAX_PING_LOSS_PERCENT:-0}"
RX_MAINT_MAX_PING_AVG_MS="${NVPN_PERF_RX_MAINT_MAX_PING_AVG_MS:-50}"
RX_MAINT_MAX_PING_MAX_MS="${NVPN_PERF_RX_MAINT_MAX_PING_MAX_MS:-80}"
RX_MAINT_POST_MAX_PING_AVG_MS="${NVPN_PERF_RX_MAINT_POST_MAX_PING_AVG_MS:-100}"
RX_MAINT_POST_MAX_PING_MAX_MS="${NVPN_PERF_RX_MAINT_POST_MAX_PING_MAX_MS:-150}"
WORKER_QUEUE_PRESSURE_CAP="${NVPN_PERF_WORKER_QUEUE_PRESSURE_CAP:-8}"
WORKER_QUEUE_PRESSURE_MIN_TCP_MBIT="${NVPN_PERF_WORKER_QUEUE_PRESSURE_MIN_TCP_MBIT:-20}"
WORKER_QUEUE_PRESSURE_MIN_REVERSE_TCP_MBIT="${NVPN_PERF_WORKER_QUEUE_PRESSURE_MIN_REVERSE_TCP_MBIT:-20}"
WORKER_QUEUE_PRESSURE_MAX_PING_LOSS_PERCENT="${NVPN_PERF_WORKER_QUEUE_PRESSURE_MAX_PING_LOSS_PERCENT:-10}"
WORKER_QUEUE_PRESSURE_MAX_PING_AVG_MS="${NVPN_PERF_WORKER_QUEUE_PRESSURE_MAX_PING_AVG_MS:-100}"
WORKER_QUEUE_PRESSURE_MAX_PING_MAX_MS="${NVPN_PERF_WORKER_QUEUE_PRESSURE_MAX_PING_MAX_MS:-200}"
WORKER_QUEUE_PRESSURE_POST_MAX_PING_LOSS_PERCENT="${NVPN_PERF_WORKER_QUEUE_PRESSURE_POST_MAX_PING_LOSS_PERCENT:-5}"
WORKER_QUEUE_PRESSURE_POST_MAX_PING_AVG_MS="${NVPN_PERF_WORKER_QUEUE_PRESSURE_POST_MAX_PING_AVG_MS:-100}"
WORKER_QUEUE_PRESSURE_POST_MAX_PING_MAX_MS="${NVPN_PERF_WORKER_QUEUE_PRESSURE_POST_MAX_PING_MAX_MS:-150}"

# Keep the release gate aligned with CI by default: Docker builds use the
# published FIPS crates pinned by Cargo.lock unless local patching is explicit.
# Set NVPN_PATCH_LOCAL_FIPS=1 and NVPN_FIPS_REPO_PATH=../fips while developing
# cross-repo FIPS changes.

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
    echo "--- $service connect.log ---"
    "${COMPOSE[@]}" exec -T "$service" sh -lc "tail -n 240 /tmp/connect.log 2>/dev/null || true" || true
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
  jq -er '((.end.sum_received.bits_per_second // .end.sum.bits_per_second // .end.sum_sent.bits_per_second) | select(type == "number")) / 1000000'
}

iperf_retransmits() {
  jq -r '(.end.sum_sent.retransmits // .end.sum.retransmits // 0)'
}

run_iperf_json() {
  local label="$1"
  shift
  local output code
  for attempt in 1 2 3; do
    if output="$("${COMPOSE[@]}" exec -T node-a iperf3 \
        -J -c "$BOB_TUNNEL_IP" -t "$DURATION" -O 1 --connect-timeout 3000 "$@" 2>&1)"; then
      code=0
      if printf '%s\n' "$output" | iperf_mbps >/dev/null 2>&1; then
        printf '%s\n' "$output"
        return 0
      fi
    else
      code=$?
    fi

    if [[ "$attempt" -lt 3 ]]; then
      echo "fips perf regression e2e: retrying iperf $label after attempt $attempt produced no throughput result" >&2
      start_iperf_server
      continue
    fi

    if [[ "$code" -ne 0 ]]; then
      echo "fips perf regression e2e failed: iperf $label failed with exit $code" >&2
    else
      echo "fips perf regression e2e failed: iperf $label returned no throughput result" >&2
    fi
    printf '%s\n' "$output" >&2
    exit 1
  done
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
  local max_loss="${3:-$MAX_PING_LOSS_PERCENT}"
  local max_avg="${4:-$MAX_PING_AVG_MS}"
  local max_max="${5:-$MAX_PING_MAX_MS}"
  local stats loss avg max
  if ! stats="$(printf '%s\n' "$output" | parse_ping_stats)"; then
    echo "fips perf regression e2e failed: could not parse ping stats for $label" >&2
    printf '%s\n' "$output" >&2
    exit 1
  fi
  read -r loss avg max <<<"$stats"
  printf '%s ping: loss=%s%% avg=%sms max=%sms\n' "$label" "$loss" "$avg" "$max"
  assert_float_at_most "$loss" "$max_loss" "$label ping loss %"
  assert_float_at_most "$avg" "$max_avg" "$label ping avg ms"
  assert_float_at_most "$max" "$max_max" "$label ping max ms"
}

start_iperf_server() {
  "${COMPOSE[@]}" exec -T node-b sh -c "pkill -9 iperf3 2>/dev/null; true" >/dev/null
  "${COMPOSE[@]}" exec -d node-b sh -lc "iperf3 -s -D --logfile /tmp/iperf3-server.log"
  sleep 1
}

run_concurrent_probe() {
  local phase="$1"
  local label="$2"
  local min_tcp="$3"
  local max_loss="$4"
  local max_avg="$5"
  local max_max="$6"
  shift 6

  local json_path err_path iperf_pid ping_output mbps retrans
  json_path="$(mktemp)"
  err_path="$(mktemp)"
  "${COMPOSE[@]}" exec -T node-a iperf3 \
    -J -c "$BOB_TUNNEL_IP" -t "$LOAD_DURATION" -O 1 --connect-timeout 3000 "$@" \
    >"$json_path" 2>"$err_path" &
  iperf_pid=$!
  sleep 1
  ping_output="$("${COMPOSE[@]}" exec -T node-a ping \
    -c "$PING_COUNT" -i "$PING_INTERVAL" -W 2 "$BOB_TUNNEL_IP" 2>&1)"
  if ! wait "$iperf_pid"; then
    echo "fips perf regression e2e failed: $phase $label iperf failed" >&2
    cat "$err_path" >&2
    exit 1
  fi
  if ! mbps="$(iperf_mbps <"$json_path")"; then
    echo "fips perf regression e2e failed: $phase $label iperf returned no throughput result" >&2
    cat "$err_path" >&2
    cat "$json_path" >&2
    exit 1
  fi
  retrans="$(iperf_retransmits <"$json_path")"
  rm -f "$json_path" "$err_path"
  printf '%s %s TCP load: %.1f Mbps retrans=%s\n' "$phase" "$label" "$mbps" "$retrans"
  assert_float_at_least "$mbps" "$min_tcp" "$phase $label TCP throughput Mbps"
  assert_ping_ok "$phase during $label TCP load" "$ping_output" "$max_loss" "$max_avg" "$max_max"
}

run_perf_phase() {
  local phase="$1"
  local min_tcp="$2"
  local min_reverse_tcp="$3"
  local max_loss="$4"
  local max_avg="$5"
  local max_max="$6"
  local post_max_loss="${7:-$max_loss}"
  local post_max_avg="${8:-$max_avg}"
  local post_max_max="${9:-$max_max}"

  echo "--- phase: $phase ---"
  echo "thresholds: tcp>=${min_tcp}M reverse>=${min_reverse_tcp}M ping_loss<=${max_loss}% ping_avg<=${max_avg}ms ping_max<=${max_max}ms"

  start_iperf_server

  local forward_json forward_mbps forward_retrans
  forward_json="$(run_iperf_json "$phase forward TCP")"
  forward_mbps="$(printf '%s\n' "$forward_json" | iperf_mbps)"
  forward_retrans="$(printf '%s\n' "$forward_json" | iperf_retransmits)"
  printf '%s forward TCP: %.1f Mbps retrans=%s\n' "$phase" "$forward_mbps" "$forward_retrans"
  assert_float_at_least "$forward_mbps" "$min_tcp" "$phase forward TCP throughput Mbps"

  local reverse_json reverse_mbps reverse_retrans
  reverse_json="$(run_iperf_json "$phase reverse TCP" -R)"
  reverse_mbps="$(printf '%s\n' "$reverse_json" | iperf_mbps)"
  reverse_retrans="$(printf '%s\n' "$reverse_json" | iperf_retransmits)"
  printf '%s reverse TCP: %.1f Mbps retrans=%s\n' "$phase" "$reverse_mbps" "$reverse_retrans"
  assert_float_at_least "$reverse_mbps" "$min_reverse_tcp" "$phase reverse TCP throughput Mbps"

  local post_ping
  run_concurrent_probe "$phase" "forward" "$min_tcp" "$max_loss" "$max_avg" "$max_max"
  run_concurrent_probe "$phase" "reverse" "$min_reverse_tcp" "$max_loss" "$max_avg" "$max_max" -R

  post_ping="$("${COMPOSE[@]}" exec -T node-a ping \
    -c "$PING_COUNT" -i "$PING_INTERVAL" -W 2 "$BOB_TUNNEL_IP" 2>&1)"
  assert_ping_ok "$phase after TCP load" "$post_ping" "$post_max_loss" "$post_max_avg" "$post_max_max"
}

stop_connects() {
  "${COMPOSE[@]}" exec -T node-a sh -c "pkill -9 nvpn 2>/dev/null; true" >/dev/null || true
  "${COMPOSE[@]}" exec -T node-b sh -c "pkill -9 nvpn 2>/dev/null; true" >/dev/null || true
  sleep 1
}

start_connects() {
  local rx_maint_fault_ms="$1"
  local extra_env="${2:-}"
  local fault_env=""
  if [[ "$rx_maint_fault_ms" != "0" ]]; then
    fault_env="FIPS_FAULT_INJECT_RX_LOOP_SLOW_MAINTENANCE_MS='$rx_maint_fault_ms'"
  fi

  "${COMPOSE[@]}" exec -d node-a sh -lc \
    "$fault_env $extra_env NVPN_FIPS_NOSTR_DISCOVERY_POLICY='$FIPS_NOSTR_DISCOVERY_POLICY' nvpn connect > /tmp/connect.log 2>&1"
  "${COMPOSE[@]}" exec -d node-b sh -lc \
    "$fault_env $extra_env NVPN_FIPS_NOSTR_DISCOVERY_POLICY='$FIPS_NOSTR_DISCOVERY_POLICY' nvpn connect > /tmp/connect.log 2>&1"
}

underlay_dev_for_peer_cmd='peer="$1"; ip route get "$peer" | sed -n "s/.* dev \([^ ]*\).*/\1/p" | head -n1'

apply_underlay_rate_limit() {
  local service="$1"
  local peer_ip="$2"
  local rate_mbit="$3"
  local burst_kb="$4"
  local latency_ms="$5"
  "${COMPOSE[@]}" exec -T "$service" sh -lc "
    set -eu
    dev=\$(sh -c '$underlay_dev_for_peer_cmd' _ '$peer_ip')
    if [ -z \"\$dev\" ]; then
      echo \"fips perf regression e2e failed: could not resolve underlay device for $peer_ip\" >&2
      exit 1
    fi
    tc qdisc replace dev \"\$dev\" root tbf rate '${rate_mbit}'mbit burst '${burst_kb}'kb latency '${latency_ms}'ms
  "
}

clear_underlay_rate_limit() {
  local service="$1"
  local peer_ip="$2"
  "${COMPOSE[@]}" exec -T "$service" sh -lc "
    dev=\$(sh -c '$underlay_dev_for_peer_cmd' _ '$peer_ip' 2>/dev/null || true)
    if [ -n \"\$dev\" ]; then
      tc qdisc del dev \"\$dev\" root 2>/dev/null || true
    fi
  " >/dev/null 2>&1 || true
}

run_constrained_underlay_phase() {
  if [[ "$CONSTRAINED_RATE_MBIT" == "0" ]]; then
    echo "Skipping constrained-underlay phase because NVPN_PERF_CONSTRAINED_RATE_MBIT=0"
    return
  fi

  # Clean Docker veth can hide sender queues that only appear once the underlay
  # is slower than the app. Cap egress so TCP collapse and ping starvation fail
  # in CI instead of waiting for a real LAN/macOS repro.
  echo "--- applying constrained underlay: ${CONSTRAINED_RATE_MBIT}Mbit burst=${CONSTRAINED_BURST_KB}KB latency=${CONSTRAINED_LATENCY_MS}ms ---"
  apply_underlay_rate_limit node-a 10.203.0.11 "$CONSTRAINED_RATE_MBIT" "$CONSTRAINED_BURST_KB" "$CONSTRAINED_LATENCY_MS"
  apply_underlay_rate_limit node-b 10.203.0.10 "$CONSTRAINED_RATE_MBIT" "$CONSTRAINED_BURST_KB" "$CONSTRAINED_LATENCY_MS"
  run_perf_phase \
    "constrained-underlay" \
    "$CONSTRAINED_MIN_TCP_MBIT" \
    "$CONSTRAINED_MIN_REVERSE_TCP_MBIT" \
    "$CONSTRAINED_MAX_PING_LOSS_PERCENT" \
    "$CONSTRAINED_MAX_PING_AVG_MS" \
    "$CONSTRAINED_MAX_PING_MAX_MS"
  clear_underlay_rate_limit node-a 10.203.0.11
  clear_underlay_rate_limit node-b 10.203.0.10
}

run_rx_maintenance_fault_phase() {
  if [[ "$RX_MAINT_FAULT_MS" == "0" ]]; then
    echo "Skipping rx-maintenance fault phase because NVPN_PERF_RX_MAINT_FAULT_MS=0"
    return
  fi

  echo "--- restarting mesh with rx-loop maintenance fault: ${RX_MAINT_FAULT_MS}ms ---"
  stop_connects
  start_connects "$RX_MAINT_FAULT_MS"
  wait_for_mesh
  run_perf_phase \
    "rx-maintenance-fault" \
    "$RX_MAINT_MIN_TCP_MBIT" \
    "$RX_MAINT_MIN_REVERSE_TCP_MBIT" \
    "$RX_MAINT_MAX_PING_LOSS_PERCENT" \
    "$RX_MAINT_MAX_PING_AVG_MS" \
    "$RX_MAINT_MAX_PING_MAX_MS" \
    "$RX_MAINT_MAX_PING_LOSS_PERCENT" \
    "$RX_MAINT_POST_MAX_PING_AVG_MS" \
    "$RX_MAINT_POST_MAX_PING_MAX_MS"

  for service in node-a node-b; do
    if ! "${COMPOSE[@]}" exec -T "$service" sh -lc \
      "grep -q 'RX loop slow maintenance timed out' /tmp/connect.log"; then
      echo "fips perf regression e2e failed: $service did not observe forced rx-loop maintenance timeout" >&2
      exit 1
    fi
  done
}

run_worker_queue_pressure_phase() {
  if [[ "$WORKER_QUEUE_PRESSURE_CAP" == "0" ]]; then
    echo "Skipping worker-queue pressure phase because NVPN_PERF_WORKER_QUEUE_PRESSURE_CAP=0"
    return
  fi

  echo "--- restarting mesh with worker queue pressure: FIPS_WORKER_CHANNEL_CAP=${WORKER_QUEUE_PRESSURE_CAP} ---"
  stop_connects
  start_connects 0 "FIPS_WORKER_CHANNEL_CAP='$WORKER_QUEUE_PRESSURE_CAP'"
  wait_for_mesh
  run_perf_phase \
    "worker-queue-pressure" \
    "$WORKER_QUEUE_PRESSURE_MIN_TCP_MBIT" \
    "$WORKER_QUEUE_PRESSURE_MIN_REVERSE_TCP_MBIT" \
    "$WORKER_QUEUE_PRESSURE_MAX_PING_LOSS_PERCENT" \
    "$WORKER_QUEUE_PRESSURE_MAX_PING_AVG_MS" \
    "$WORKER_QUEUE_PRESSURE_MAX_PING_MAX_MS" \
    "$WORKER_QUEUE_PRESSURE_POST_MAX_PING_LOSS_PERCENT" \
    "$WORKER_QUEUE_PRESSURE_POST_MAX_PING_AVG_MS" \
    "$WORKER_QUEUE_PRESSURE_POST_MAX_PING_MAX_MS"
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

start_connects 0

wait_for_mesh

if ! "${COMPOSE[@]}" exec -T node-a ping -c 3 -W 2 "$BOB_TUNNEL_IP" >/dev/null; then
  echo "fips perf regression e2e failed: baseline tunnel ping failed" >&2
  exit 1
fi

echo "alice tunnel ip: $ALICE_TUNNEL_IP"
echo "bob   tunnel ip: $BOB_TUNNEL_IP"
run_perf_phase \
  "clean-underlay" \
  "$MIN_TCP_MBIT" \
  "$MIN_REVERSE_TCP_MBIT" \
  "$MAX_PING_LOSS_PERCENT" \
  "$MAX_PING_AVG_MS" \
  "$MAX_PING_MAX_MS"
run_constrained_underlay_phase
run_worker_queue_pressure_phase
run_rx_maintenance_fault_phase

echo "fips perf regression docker e2e passed: throughput stayed above floor and pings did not wedge under or after TCP load"
