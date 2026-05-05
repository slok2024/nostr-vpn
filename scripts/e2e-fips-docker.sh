#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
PROJECT_NAME="nostr-vpn-e2e-fips"
COMPOSE=(docker compose -p "$PROJECT_NAME" -f "$ROOT_DIR/docker-compose.fips-e2e.yml")

ALICE_FIPS_ADDR="10.204.0.10:2121"
BOB_FIPS_ADDR="10.204.0.11:2121"
ALICE_PRIVATE_IP="10.44.81.10"
BOB_PRIVATE_IP="10.44.81.11"

cleanup() {
  "${COMPOSE[@]}" down -v --remove-orphans >/dev/null 2>&1 || true
  docker network rm "${PROJECT_NAME}_fips_e2e" >/dev/null 2>&1 || true
  for _ in $(seq 1 20); do
    docker network inspect "${PROJECT_NAME}_fips_e2e" >/dev/null 2>&1 || break
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

  echo "fips docker e2e failed: service '$service' did not reach running state" >&2
  exit 1
}

nostr_config_field() {
  local service="$1"
  local field="$2"
  "${COMPOSE[@]}" exec -T "$service" sh -lc "
    awk -v field='$field' '
      /^\\[nostr\\]$/ { in_nostr = 1; next }
      /^\\[/ { in_nostr = 0 }
      in_nostr && \$1 == field {
        print \$3;
        exit
      }
    ' /root/.config/nvpn/config.toml
  " | tr -d '\r\"'
}

cleanup

"${COMPOSE[@]}" build >/dev/null
"${COMPOSE[@]}" up -d node-a node-b >/dev/null
for service in node-a node-b; do
  wait_for_service "$service"
done

"${COMPOSE[@]}" exec -T node-a nvpn init --force >/dev/null
"${COMPOSE[@]}" exec -T node-b nvpn init --force >/dev/null

ALICE_NSEC="$(nostr_config_field node-a secret_key)"
ALICE_NPUB="$(nostr_config_field node-a public_key)"
BOB_NSEC="$(nostr_config_field node-b secret_key)"
BOB_NPUB="$(nostr_config_field node-b public_key)"

if [[ -z "$ALICE_NSEC" || -z "$ALICE_NPUB" || -z "$BOB_NSEC" || -z "$BOB_NPUB" ]]; then
  echo "fips docker e2e failed: unable to resolve node Nostr keys" >&2
  exit 1
fi

"${COMPOSE[@]}" exec -d node-b sh -lc "
  nvpn-fips-probe serve \
    --identity-nsec '$BOB_NSEC' \
    --bind-addr '0.0.0.0:2121' \
    --peer-npub '$ALICE_NPUB' \
    --peer-addr '$ALICE_FIPS_ADDR' \
    --local-ip '$BOB_PRIVATE_IP' \
    --peer-ip '$ALICE_PRIVATE_IP' \
    --timeout-secs 20 > /tmp/fips-probe.log 2>&1
"

sleep 1

if ! "${COMPOSE[@]}" exec -T node-a sh -lc "
  nvpn-fips-probe send \
    --identity-nsec '$ALICE_NSEC' \
    --bind-addr '0.0.0.0:2121' \
    --peer-npub '$BOB_NPUB' \
    --peer-addr '$BOB_FIPS_ADDR' \
    --local-ip '$ALICE_PRIVATE_IP' \
    --peer-ip '$BOB_PRIVATE_IP' \
    --timeout-secs 20 > /tmp/fips-probe.log 2>&1
"; then
  echo "fips docker e2e failed: Alice probe failed" >&2
  echo "--- Alice probe log ---"
  "${COMPOSE[@]}" exec -T node-a sh -lc 'cat /tmp/fips-probe.log 2>/dev/null || true'
  echo "--- Bob probe log ---"
  "${COMPOSE[@]}" exec -T node-b sh -lc 'cat /tmp/fips-probe.log 2>/dev/null || true'
  exit 1
fi

for _ in $(seq 1 20); do
  BOB_LOG="$("${COMPOSE[@]}" exec -T node-b sh -lc 'cat /tmp/fips-probe.log 2>/dev/null || true')"
  if grep -q "probe serve passed" <<<"$BOB_LOG"; then
    break
  fi
  sleep 1
done

ALICE_LOG="$("${COMPOSE[@]}" exec -T node-a sh -lc 'cat /tmp/fips-probe.log 2>/dev/null || true')"
BOB_LOG="$("${COMPOSE[@]}" exec -T node-b sh -lc 'cat /tmp/fips-probe.log 2>/dev/null || true')"

if ! grep -q "probe send passed" <<<"$ALICE_LOG"; then
  echo "fips docker e2e failed: Alice did not receive a valid reply" >&2
  echo "$ALICE_LOG"
  exit 1
fi

if ! grep -q "probe serve passed" <<<"$BOB_LOG"; then
  echo "fips docker e2e failed: Bob did not receive and reply" >&2
  echo "$BOB_LOG"
  exit 1
fi

echo "--- Alice FIPS probe log ---"
echo "$ALICE_LOG"
echo "--- Bob FIPS probe log ---"
echo "$BOB_LOG"
echo "fips docker e2e passed: two containers exchanged raw private packets over embedded FIPS EndpointData"
