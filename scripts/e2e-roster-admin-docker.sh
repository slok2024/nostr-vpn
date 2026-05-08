#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
PROJECT_NAME="nostr-vpn-e2e-roster-admin"
COMPOSE=(docker compose -p "$PROJECT_NAME" -f "$ROOT_DIR/docker-compose.e2e.yml")

NETWORK_ID="docker-roster-admin"

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
  echo "roster/admin docker e2e failed, collecting debug output..."
  "${COMPOSE[@]}" ps || true
  for service in node-a node-b node-c; do
    echo "--- logs: $service ---"
    "${COMPOSE[@]}" logs --no-color --tail 120 "$service" || true
  done
  for node in node-a node-b node-c; do
    echo "--- $node status ---"
    "${COMPOSE[@]}" exec -T "$node" nvpn status --json --discover-secs 0 || true
    echo "--- $node config ---"
    "${COMPOSE[@]}" exec -T "$node" sh -lc "cat /root/.config/nvpn/config.toml 2>/dev/null || true" || true
    echo "--- $node daemon.log ---"
    "${COMPOSE[@]}" exec -T "$node" sh -lc "tail -n 240 /root/.config/nvpn/daemon.log 2>/dev/null || true" || true
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

  echo "roster/admin docker e2e failed: service '$service' did not reach running state" >&2
  exit 1
}

toml_array() {
  local result="["
  local first=true
  local item
  for item in "$@"; do
    if [[ "$first" == true ]]; then
      first=false
    else
      result+=", "
    fi
    result+="\"$item\""
  done
  result+="]"
  printf '%s' "$result"
}

read_npub() {
  local service="$1"
  "${COMPOSE[@]}" exec -T "$service" sh -lc \
    "nvpn init --force >/dev/null && awk '
      /^\\[nostr\\]$/ { in_nostr = 1; next }
      /^\\[/ { in_nostr = 0 }
      in_nostr && /^public_key[[:space:]]*=/ {
        print \$3;
        exit
      }
    ' /root/.config/nvpn/config.toml" | tr -d '\r\"'
}

start_daemon() {
  local service="$1"
  if ! "${COMPOSE[@]}" exec -T "$service" sh -lc "nvpn start --daemon --connect >/tmp/nvpn-start.log 2>&1"; then
    echo "roster/admin docker e2e failed: daemon start failed on $service" >&2
    "${COMPOSE[@]}" exec -T "$service" sh -lc "cat /tmp/nvpn-start.log" >&2 || true
    exit 1
  fi
}

reload_daemon() {
  local service="$1"
  local output=""

  for _ in $(seq 1 10); do
    if output="$("${COMPOSE[@]}" exec -T "$service" nvpn reload 2>&1)"; then
      return 0
    fi

    if grep -q "did not acknowledge control request" <<<"$output"; then
      sleep 1
      continue
    fi

    echo "roster/admin docker e2e failed: daemon reload failed on $service" >&2
    echo "$output" >&2
    exit 1
  done

  echo "roster/admin docker e2e failed: daemon reload never acknowledged on $service" >&2
  echo "$output" >&2
  exit 1
}

status_connected_peer_count() {
  local service="$1"
  "${COMPOSE[@]}" exec -T "$service" sh -lc \
    "nvpn status --json | perl -0ne 'print \$1 if /\"connected_peer_count\"\\s*:\\s*(\\d+)/'"
}

wait_for_connected_peer_count() {
  local service="$1"
  local expected="$2"
  local description="$3"
  local attempts="${4:-80}"
  local current=""

  for _ in $(seq 1 "$attempts"); do
    current="$(status_connected_peer_count "$service" || true)"
    if [[ "$current" == "$expected" ]]; then
      return 0
    fi
    sleep 1
  done

  echo "roster/admin docker e2e failed: $description (expected connected_peer_count=$expected, got '$current')" >&2
  "${COMPOSE[@]}" exec -T "$service" nvpn status --json || true
  exit 1
}

config_array_block() {
  local service="$1"
  local key="$2"
  "${COMPOSE[@]}" exec -T "$service" sh -lc \
    "perl -0ne 'if (/^${key}\\s*=\\s*(\\[[^\\]]*\\])/ms) { print \$1 }' /root/.config/nvpn/config.toml"
}

shared_roster_updated_at() {
  local service="$1"
  "${COMPOSE[@]}" exec -T "$service" sh -lc \
    "perl -0ne 'print \$1 if /^shared_roster_updated_at\\s*=\\s*(\\d+)/m' /root/.config/nvpn/config.toml"
}

next_shared_roster_updated_at() {
  local service="$1"
  local current=""
  local now=""

  current="$(shared_roster_updated_at "$service" || true)"
  now="$(date +%s)"
  if [[ "$current" =~ ^[0-9]+$ ]]; then
    printf '%s' "$((current + 1))"
  else
    printf '%s' "$now"
  fi
}

next_global_shared_roster_updated_at() {
  local max=""
  local current=""
  local service=""

  max="$(($(date +%s) + 30))"
  for service in "$@"; do
    current="$(shared_roster_updated_at "$service" || true)"
    if [[ "$current" =~ ^[0-9]+$ && "$current" -gt "$max" ]]; then
      max="$current"
    fi
  done

  printf '%s' "$((max + 1))"
}

wait_for_config_array_contains() {
  local service="$1"
  local key="$2"
  local needle="$3"
  local description="$4"
  local block=""

  for _ in $(seq 1 60); do
    block="$(config_array_block "$service" "$key")"
    if grep -qF "$needle" <<<"$block"; then
      return 0
    fi
    sleep 1
  done

  echo "roster/admin docker e2e failed: $description" >&2
  echo "$block" >&2
  exit 1
}

wait_for_config_array_lacks() {
  local service="$1"
  local key="$2"
  local needle="$3"
  local description="$4"
  local block=""

  for _ in $(seq 1 60); do
    block="$(config_array_block "$service" "$key")"
    if ! grep -qF "$needle" <<<"$block"; then
      return 0
    fi
    sleep 1
  done

  echo "roster/admin docker e2e failed: $description" >&2
  echo "$block" >&2
  exit 1
}

config_peer_alias() {
  local service="$1"
  local participant="$2"
  "${COMPOSE[@]}" exec -T -e PARTICIPANT="$participant" "$service" sh -lc '
    perl -0ne '"'"'
      my $participant = $ENV{PARTICIPANT};
      if (/\[peer_aliases\]\s*\n(.*?)(?:^\[|\z)/ms) {
        my $block = $1;
        print $1 if $block =~ /^\Q$participant\E\s*=\s*"([^"]*)"/m;
      }
    '"'"' /root/.config/nvpn/config.toml
  '
}

wait_for_peer_alias() {
  local service="$1"
  local participant="$2"
  local expected="$3"
  local description="$4"
  local current=""

  for _ in $(seq 1 60); do
    current="$(config_peer_alias "$service" "$participant" || true)"
    if [[ "$current" == "$expected" ]]; then
      return 0
    fi
    sleep 1
  done

  echo "roster/admin docker e2e failed: $description" >&2
  echo "expected alias '$expected' for $participant on $service, got '$current'" >&2
  "${COMPOSE[@]}" exec -T "$service" sh -lc "sed -n '/^\\[peer_aliases\\]/,/^\\[/p' /root/.config/nvpn/config.toml" >&2 || true
  exit 1
}

set_membership() {
  local service="$1"
  local participants_toml="$2"
  local admins_toml="$3"
  local signer="$4"
  local inviter="$5"
  local shared_at_arg="${6:-}"
  local shared_at
  if [[ -n "$shared_at_arg" ]]; then
    shared_at="$shared_at_arg"
  else
    shared_at="$(next_shared_roster_updated_at "$service")"
  fi

  "${COMPOSE[@]}" exec -T \
    -e PARTICIPANTS_TOML="$participants_toml" \
    -e ADMINS_TOML="$admins_toml" \
    -e SHARED_BY="$signer" \
    -e SHARED_AT="$shared_at" \
    -e INVITER="$inviter" \
    "$service" sh -lc '
cfg=/root/.config/nvpn/config.toml
tmp=$(mktemp)
perl -0pe '"'"'
  s/^participants\s*=\s*\[[^\]]*\]/participants = $ENV{PARTICIPANTS_TOML}/ms;
  if (/^admins\s*=/m) {
    s/^admins\s*=\s*\[[^\]]*\]/admins = $ENV{ADMINS_TOML}/ms;
  } else {
    s/^participants\s*=\s*\[[^\]]*\]/participants = $ENV{PARTICIPANTS_TOML}\nadmins = $ENV{ADMINS_TOML}/ms;
  }
  if (/^invite_inviter\s*=/m) {
    s/^invite_inviter\s*=.*$/invite_inviter = "$ENV{INVITER}"/m;
  } else {
    s/^admins\s*=.*$/admins = $ENV{ADMINS_TOML}\ninvite_inviter = "$ENV{INVITER}"/m;
  }
  if (/^shared_roster_updated_at\s*=/m) {
    s/^shared_roster_updated_at\s*=.*$/shared_roster_updated_at = $ENV{SHARED_AT}/m;
  } else {
    s/^invite_inviter\s*=.*$/invite_inviter = "$ENV{INVITER}"\nshared_roster_updated_at = $ENV{SHARED_AT}/m;
  }
  if (/^shared_roster_signed_by\s*=/m) {
    s/^shared_roster_signed_by\s*=.*$/shared_roster_signed_by = "$ENV{SHARED_BY}"/m;
  } else {
    s/^shared_roster_updated_at\s*=.*$/shared_roster_updated_at = $ENV{SHARED_AT}\nshared_roster_signed_by = "$ENV{SHARED_BY}"/m;
  }
'"'"' "$cfg" > "$tmp" && mv "$tmp" "$cfg"
'
}

set_peer_alias() {
  local service="$1"
  local participant="$2"
  local alias="$3"
  local signer="$4"
  local shared_at_arg="${5:-}"
  local shared_at
  if [[ -n "$shared_at_arg" ]]; then
    shared_at="$shared_at_arg"
  else
    shared_at="$(next_shared_roster_updated_at "$service")"
  fi

  "${COMPOSE[@]}" exec -T \
    -e PARTICIPANT="$participant" \
    -e ALIAS_VALUE="$alias" \
    -e SHARED_BY="$signer" \
    -e SHARED_AT="$shared_at" \
    "$service" sh -lc '
cfg=/root/.config/nvpn/config.toml
tmp=$(mktemp)
	perl -0pe '"'"'
	  my $participant = $ENV{PARTICIPANT};
	  my $alias = $ENV{ALIAS_VALUE};
	  my $entry = "$participant = \"$alias\"";

	  if (/^\[peer_aliases\]\s*$/m) {
	    s{
	      (^\[peer_aliases\]\s*\n)
	      (.*?)
	      (?=^\[|\z)
	    }{
	      my ($header, $body) = ($1, $2);
	      if ($body =~ s/^\Q$participant\E\s*=.*$/$entry/m) {
	        $header . $body;
	      } else {
	        $header . $entry . "\n" . $body;
	      }
	    }xmse;
	  } else {
	    $_ .= "\n[peer_aliases]\n$entry\n";
	  }

  s/^shared_roster_updated_at\s*=.*$/shared_roster_updated_at = $ENV{SHARED_AT}/m;
  s/^shared_roster_signed_by\s*=.*$/shared_roster_signed_by = "$ENV{SHARED_BY}"/m;
'"'"' "$cfg" > "$tmp" && mv "$tmp" "$cfg"
'
}

ping_until_success() {
  local service="$1"
  local target="$2"
  local log_file="$3"
  for _ in $(seq 1 30); do
    if "${COMPOSE[@]}" exec -T "$service" ping -c 1 -W 2 "$target" >"$log_file" 2>&1; then
      return 0
    fi
    sleep 1
  done
  return 1
}

ping_until_failure() {
  local service="$1"
  local target="$2"
  local log_file="$3"
  for _ in $(seq 1 30); do
    if ! "${COMPOSE[@]}" exec -T "$service" ping -c 1 -W 2 "$target" >"$log_file" 2>&1; then
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

ALICE_NPUB="$(read_npub node-a)"
BOB_NPUB="$(read_npub node-b)"
CAROL_NPUB="$(read_npub node-c)"

if [[ -z "$ALICE_NPUB" || -z "$BOB_NPUB" || -z "$CAROL_NPUB" ]]; then
  echo "roster/admin docker e2e failed: unable to resolve participant npubs" >&2
  exit 1
fi

AB_PARTICIPANTS="$(toml_array "$ALICE_NPUB" "$BOB_NPUB")"
ABC_PARTICIPANTS="$(toml_array "$ALICE_NPUB" "$BOB_NPUB" "$CAROL_NPUB")"
A_ADMIN="$(toml_array "$ALICE_NPUB")"
AB_ADMINS="$(toml_array "$ALICE_NPUB" "$BOB_NPUB")"
B_ADMIN="$(toml_array "$BOB_NPUB")"

"${COMPOSE[@]}" exec -T node-a nvpn set \
  --network-id "$NETWORK_ID" \
  --participant "$ALICE_NPUB" \
  --participant "$BOB_NPUB" \
  --endpoint "10.203.0.10:51820" \
  --listen-port 51820 \
  --fips-advertise-endpoint true \
  --fips-peer-endpoint "$BOB_NPUB=10.203.0.11:51820" \
  --fips-peer-endpoint "$CAROL_NPUB=10.203.0.12:51820" >/dev/null

"${COMPOSE[@]}" exec -T node-b nvpn set \
  --network-id "$NETWORK_ID" \
  --participant "$ALICE_NPUB" \
  --participant "$BOB_NPUB" \
  --endpoint "10.203.0.11:51820" \
  --listen-port 51820 \
  --fips-advertise-endpoint true \
  --fips-peer-endpoint "$ALICE_NPUB=10.203.0.10:51820" \
  --fips-peer-endpoint "$CAROL_NPUB=10.203.0.12:51820" >/dev/null

ALICE_TUNNEL_IP="$("${COMPOSE[@]}" exec -T node-a nvpn ip | tr -d '\r')"
BOB_TUNNEL_IP="$("${COMPOSE[@]}" exec -T node-b nvpn ip | tr -d '\r')"

if [[ -z "$ALICE_TUNNEL_IP" || -z "$BOB_TUNNEL_IP" ]]; then
  echo "roster/admin docker e2e failed: unable to resolve initial FIPS tunnel IPs" >&2
  exit 1
fi

INITIAL_SHARED_AT="$(next_global_shared_roster_updated_at node-a node-b)"
set_membership node-a "$AB_PARTICIPANTS" "$A_ADMIN" "$ALICE_NPUB" "$ALICE_NPUB" "$INITIAL_SHARED_AT"
set_membership node-b "$AB_PARTICIPANTS" "$A_ADMIN" "$ALICE_NPUB" "$ALICE_NPUB" "$INITIAL_SHARED_AT"

start_daemon node-a
start_daemon node-b

wait_for_connected_peer_count node-a 1 "alice never reached the initial 1/1 mesh"
wait_for_connected_peer_count node-b 1 "bob never reached the initial 1/1 mesh"

if ! ping_until_success node-a "$BOB_TUNNEL_IP" /tmp/nvpn-roster-admin-a-to-b.log; then
  echo "roster/admin docker e2e failed: initial alice -> bob ping failed" >&2
  cat /tmp/nvpn-roster-admin-a-to-b.log >&2 || true
  exit 1
fi

ADD_CAROL_SHARED_AT="$(next_global_shared_roster_updated_at node-a node-b)"
set_membership node-a "$ABC_PARTICIPANTS" "$AB_ADMINS" "$ALICE_NPUB" "$ALICE_NPUB" "$ADD_CAROL_SHARED_AT"
reload_daemon node-a

wait_for_config_array_contains node-b participants "$CAROL_NPUB" \
  "bob never applied alice's signed participant add for carol"
wait_for_config_array_contains node-b admins "$BOB_NPUB" \
  "bob never applied alice's signed admin promotion"

"${COMPOSE[@]}" exec -T node-c nvpn set \
  --network-id "$NETWORK_ID" \
  --participant "$ALICE_NPUB" \
  --participant "$BOB_NPUB" \
  --participant "$CAROL_NPUB" \
  --endpoint "10.203.0.12:51820" \
  --listen-port 51820 \
  --fips-advertise-endpoint true \
  --fips-peer-endpoint "$ALICE_NPUB=10.203.0.10:51820" \
  --fips-peer-endpoint "$BOB_NPUB=10.203.0.11:51820" >/dev/null

CAROL_TUNNEL_IP="$("${COMPOSE[@]}" exec -T node-c nvpn ip | tr -d '\r')"

if [[ -z "$CAROL_TUNNEL_IP" ]]; then
  echo "roster/admin docker e2e failed: unable to resolve carol FIPS tunnel IP" >&2
  exit 1
fi

set_membership node-c "$ABC_PARTICIPANTS" "$AB_ADMINS" "$ALICE_NPUB" "$ALICE_NPUB" "$ADD_CAROL_SHARED_AT"
start_daemon node-c

wait_for_connected_peer_count node-a 2 "alice never reached the 2/2 mesh after carol joined"
wait_for_connected_peer_count node-b 2 "bob never reached the 2/2 mesh after carol joined"
wait_for_connected_peer_count node-c 2 "carol never reached the 2/2 mesh after being added"

if ! ping_until_success node-c "$ALICE_TUNNEL_IP" /tmp/nvpn-roster-admin-c-to-a.log; then
  echo "roster/admin docker e2e failed: carol could not reach alice after being added" >&2
  cat /tmp/nvpn-roster-admin-c-to-a.log >&2 || true
  exit 1
fi

ALIAS_SHARED_AT="$(next_global_shared_roster_updated_at node-a node-b node-c)"
set_peer_alias node-b "$ALICE_NPUB" "founder" "$BOB_NPUB" "$ALIAS_SHARED_AT"
set_membership node-b "$ABC_PARTICIPANTS" "$AB_ADMINS" "$BOB_NPUB" "$ALICE_NPUB" "$ALIAS_SHARED_AT"
reload_daemon node-b

wait_for_peer_alias node-c "$ALICE_NPUB" "founder" \
  "carol never applied bob's signed alias update for alice"

REMOVE_CAROL_SHARED_AT="$(next_global_shared_roster_updated_at node-a node-b node-c)"
set_membership node-b "$AB_PARTICIPANTS" "$AB_ADMINS" "$BOB_NPUB" "$ALICE_NPUB" "$REMOVE_CAROL_SHARED_AT"
reload_daemon node-b

wait_for_config_array_lacks node-a participants "$CAROL_NPUB" \
  "alice never applied bob's signed participant removal for carol"
wait_for_connected_peer_count node-a 1 "alice never returned to 1/1 after bob removed carol"
wait_for_connected_peer_count node-b 1 "bob never returned to 1/1 after removing carol"

if ! ping_until_failure node-c "$ALICE_TUNNEL_IP" /tmp/nvpn-roster-admin-c-to-a-removed.log; then
  echo "roster/admin docker e2e failed: carol still reached alice after bob removed her" >&2
  cat /tmp/nvpn-roster-admin-c-to-a-removed.log >&2 || true
  exit 1
fi

READD_CAROL_SHARED_AT="$(next_global_shared_roster_updated_at node-a node-b node-c)"
set_membership node-b "$ABC_PARTICIPANTS" "$AB_ADMINS" "$BOB_NPUB" "$ALICE_NPUB" "$READD_CAROL_SHARED_AT"
reload_daemon node-b

wait_for_config_array_contains node-a participants "$CAROL_NPUB" \
  "alice never applied bob's signed participant re-add for carol"
wait_for_connected_peer_count node-a 2 "alice never returned to 2/2 after bob re-added carol"
wait_for_connected_peer_count node-b 2 "bob never returned to 2/2 after re-adding carol"
wait_for_connected_peer_count node-c 2 "carol never rejoined after bob re-added her"

if ! ping_until_success node-c "$ALICE_TUNNEL_IP" /tmp/nvpn-roster-admin-c-to-a-rejoined.log; then
  echo "roster/admin docker e2e failed: carol did not rejoin after bob re-added her" >&2
  cat /tmp/nvpn-roster-admin-c-to-a-rejoined.log >&2 || true
  exit 1
fi

SOLE_ADMIN_SHARED_AT="$(next_global_shared_roster_updated_at node-a node-b node-c)"
set_membership node-b "$ABC_PARTICIPANTS" "$B_ADMIN" "$BOB_NPUB" "$BOB_NPUB" "$SOLE_ADMIN_SHARED_AT"
reload_daemon node-b

wait_for_config_array_contains node-a admins "$BOB_NPUB" \
  "alice never applied bob's signed admin removal"
wait_for_config_array_lacks node-a admins "$ALICE_NPUB" \
  "alice still appeared as an admin after bob removed her"
wait_for_config_array_contains node-c admins "$BOB_NPUB" \
  "carol never applied bob's signed sole-admin roster"
wait_for_config_array_lacks node-c admins "$ALICE_NPUB" \
  "carol still trusted alice as admin after bob removed her"
wait_for_connected_peer_count node-a 2 "alice lost connectivity after bob became sole admin"
wait_for_connected_peer_count node-b 2 "bob lost connectivity after becoming sole admin"
wait_for_connected_peer_count node-c 2 "carol lost connectivity after bob became sole admin"

echo "--- Alice participants line ---"
config_array_block node-a participants
echo "--- Alice admins line ---"
config_array_block node-a admins
echo "--- Bob participants line ---"
config_array_block node-b participants
echo "--- Bob admins line ---"
config_array_block node-b admins
echo "--- Carol participants line ---"
config_array_block node-c participants
echo "--- Carol admins line ---"
config_array_block node-c admins
echo "--- Carol peer aliases ---"
"${COMPOSE[@]}" exec -T node-c sh -lc "sed -n '/^\\[peer_aliases\\]/,/^\\[/p' /root/.config/nvpn/config.toml"
echo "--- Carol ping after first add ---"
cat /tmp/nvpn-roster-admin-c-to-a.log
echo "--- Carol ping after removal ---"
cat /tmp/nvpn-roster-admin-c-to-a-removed.log
echo "--- Carol ping after rejoin ---"
cat /tmp/nvpn-roster-admin-c-to-a-rejoined.log

echo "roster/admin docker e2e passed: admin promotion propagated, alias changes propagated, a promoted admin removed and re-added a participant, the participant rejoined, and the new admin removed the old admin across peers"
