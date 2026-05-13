#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

VM_HOST="${VM_HOST:-}"
VM_USER="${VM_USER:-ubuntu}"
VM_PORT="${VM_PORT:-}"
VM_DIR="${VM_DIR:-~/nostr-vpn}"
BUILD_PROFILE="${BUILD_PROFILE:-debug}"
NETWORK_ID="${NETWORK_ID:-utm-host-vm}"
HOST_CONFIG="${HOST_CONFIG:-}"
HOST_CONFIG_WAS_SET="${HOST_CONFIG_WAS_SET:-}"
VM_CONFIG="${VM_CONFIG:-/home/${VM_USER}/.config/nvpn/config.toml}"
REMOTE_VM_DIR=""

SSH_TARGET="${VM_USER}@${VM_HOST}"
SSH_OPTS=(-o BatchMode=yes -o StrictHostKeyChecking=accept-new)

if [[ -n "$VM_PORT" ]]; then
  SSH_OPTS=(-p "$VM_PORT" "${SSH_OPTS[@]}")
  RSYNC_RSH="ssh -p ${VM_PORT} -o BatchMode=yes -o StrictHostKeyChecking=accept-new"
else
  RSYNC_RSH="ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new"
fi

log() {
  printf '[up] %s\n' "$*" >&2
}

die() {
  printf '[up] error: %s\n' "$*" >&2
  exit 1
}

find_repo_root() {
  local dir="$SCRIPT_DIR"

  while [[ "$dir" != "/" ]]; do
    if [[ -f "$dir/Cargo.toml" ]]; then
      printf '%s\n' "$dir"
      return
    fi
    dir="$(dirname "$dir")"
  done

  die "could not find repo root from $SCRIPT_DIR"
}

ROOT_DIR="$(find_repo_root)"

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

profile_dir() {
  if [[ "$BUILD_PROFILE" == "release" ]]; then
    printf '%s' 'release'
  else
    printf '%s' 'debug'
  fi
}

resolve_target_base() {
  if [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
    case "$CARGO_TARGET_DIR" in
      /*)
        printf '%s\n' "$CARGO_TARGET_DIR"
        ;;
      *)
        printf '%s\n' "$ROOT_DIR/$CARGO_TARGET_DIR"
        ;;
    esac
  else
    printf '%s\n' "$ROOT_DIR/target"
  fi
}

local_nvpn_path() {
  printf '%s/%s/nvpn' "$(resolve_target_base)" "$(profile_dir)"
}

ssh_run() {
  ssh "${SSH_OPTS[@]}" "$SSH_TARGET" "$@"
}

remote_login_shell() {
  local cmd="$1"
  ssh "${SSH_OPTS[@]}" "$SSH_TARGET" "bash -lc $(printf '%q' "$cmd")"
}

default_host_config() {
  case "$(uname -s)" in
    Darwin)
      printf '%s/Library/Application Support/nvpn/config.toml' "$HOME"
      ;;
    *)
      printf '%s/.config/nvpn/config.toml' "$HOME"
      ;;
  esac
}

require_inputs() {
  [[ -n "$VM_HOST" ]] || die "set VM_HOST"

  if [[ -z "$HOST_CONFIG_WAS_SET" ]]; then
    if [[ -n "$HOST_CONFIG" ]]; then
      HOST_CONFIG_WAS_SET=1
    else
      HOST_CONFIG_WAS_SET=0
    fi
  fi

  if [[ -z "$HOST_CONFIG" ]]; then
    HOST_CONFIG="$(default_host_config)"
  fi
}

ensure_local_prereqs() {
  need_cmd cargo
  need_cmd ssh
  need_cmd rsync
}

ensure_remote_prereqs() {
  log "stage: verify remote tooling"
  remote_login_shell "command -v cargo >/dev/null 2>&1 || { echo 'cargo is not installed on the remote host' >&2; exit 1; }"
}

build_local() {
  local cmd
  cmd=(cargo build -p nvpn)
  if [[ "$BUILD_PROFILE" == "release" ]]; then
    cmd+=(--release)
  fi

  log "stage: build local"
  (
    cd "$ROOT_DIR"
    "${cmd[@]}"
  )
}

resolve_remote_vm_dir() {
  case "$VM_DIR" in
    "~")
      remote_login_shell "printf '%s\n' \$HOME"
      ;;
    "~/"*)
      local suffix="${VM_DIR#~/}"
      remote_login_shell "printf '%s\n' \$HOME/$(printf '%q' "$suffix")"
      ;;
    /*)
      printf '%s\n' "$VM_DIR"
      ;;
    *)
      remote_login_shell "printf '%s\n' \$HOME/$(printf '%q' "$VM_DIR")"
      ;;
  esac
}

sync_repo() {
  log "stage: sync repo"
  ssh_run mkdir -p "$REMOTE_VM_DIR"
  rsync -az --delete \
    --exclude '.git' \
    --exclude 'target' \
    --exclude 'target-linux' \
    --exclude 'node_modules' \
    --exclude 'dist' \
    -e "$RSYNC_RSH" \
    "$ROOT_DIR/" "${SSH_TARGET}:${REMOTE_VM_DIR}/"
}

build_remote() {
  local cmd
  cmd="cd $(printf '%q' "$REMOTE_VM_DIR") && cargo build -p nvpn"
  if [[ "$BUILD_PROFILE" == "release" ]]; then
    cmd+=" --release"
  fi

  log "stage: build remote"
  remote_login_shell "$cmd"
}

resolve_remote_target_base() {
  remote_login_shell "cd $(printf '%q' "$REMOTE_VM_DIR") && if [[ -n \${CARGO_TARGET_DIR:-} ]]; then case \$CARGO_TARGET_DIR in /*) printf '%s\n' \"\$CARGO_TARGET_DIR\" ;; *) printf '%s\n' $(printf '%q' "$REMOTE_VM_DIR")/\"\$CARGO_TARGET_DIR\" ;; esac; else printf '%s\n' $(printf '%q' "$REMOTE_VM_DIR/target"); fi"
}

resolve_remote_nvpn_path() {
  local target_base
  target_base="$(resolve_remote_target_base)"
  printf '%s/%s/nvpn' "$target_base" "$(profile_dir)"
}

verify_local_nvpn() {
  local local_nvpn="$1"
  [[ -x "$local_nvpn" ]] || die "local nvpn binary was not built: $local_nvpn"
}

verify_remote_nvpn() {
  local remote_nvpn="$1"
  remote_login_shell "test -x $(printf '%q' "$remote_nvpn") || { echo 'nvpn binary is not executable on the remote host: $remote_nvpn' >&2; exit 1; }"
}

ensure_host_config_parent() {
  mkdir -p "$(dirname "$HOST_CONFIG")"
  if [[ -e "$HOST_CONFIG" && ! -f "$HOST_CONFIG" ]]; then
    die "local config path exists but is not a file: $HOST_CONFIG"
  fi
}

init_local_config() {
  local local_nvpn="$1"

  ensure_host_config_parent

  if [[ "$HOST_CONFIG_WAS_SET" -eq 0 && -f "$HOST_CONFIG" ]]; then
    return
  fi

  if [[ "$HOST_CONFIG_WAS_SET" -eq 0 ]]; then
    "$local_nvpn" init --config "$HOST_CONFIG" >/dev/null
  else
    "$local_nvpn" init --config "$HOST_CONFIG" --force >/dev/null
  fi
}

init_remote_config() {
  local remote_nvpn="$1"
  remote_login_shell "mkdir -p $(printf '%q' "$(dirname "$VM_CONFIG")") && $(printf '%q' "$remote_nvpn") init --config $(printf '%q' "$VM_CONFIG") --force >/dev/null"
}

extract_local_npub() {
  awk -F'"' '/^public_key/ {print $2; exit}' "$HOST_CONFIG"
}

extract_remote_npub() {
  remote_login_shell "awk -F'\"' '/^public_key/ {print \$2; exit}' $(printf '%q' "$VM_CONFIG")"
}

configure_local() {
  local local_nvpn="$1"
  local local_npub="$2"
  local remote_npub="$3"

  "$local_nvpn" set \
    --config "$HOST_CONFIG" \
    --network-id "$NETWORK_ID" \
    --participant "$local_npub" \
    --participant "$remote_npub" >/dev/null
}

configure_remote() {
  local remote_nvpn="$1"
  local local_npub="$2"
  local remote_npub="$3"

  remote_login_shell "$(printf '%q' "$remote_nvpn") set --config $(printf '%q' "$VM_CONFIG") --network-id $(printf '%q' "$NETWORK_ID") --participant $(printf '%q' "$local_npub") --participant $(printf '%q' "$remote_npub") >/dev/null"
}

summarize() {
  local local_nvpn="$1"
  local remote_nvpn="$2"

  cat <<EOF

UTM deploy and network setup complete.

Remote IP: $VM_HOST
Remote user: $VM_USER
Remote port: ${VM_PORT:-22}
Local nvpn: $local_nvpn
Remote nvpn: $remote_nvpn
Local config: $HOST_CONFIG
Remote config: $VM_CONFIG
Network ID: $NETWORK_ID
EOF
}

main() {
  require_inputs
  ensure_local_prereqs
  ensure_remote_prereqs
  build_local

  local local_nvpn remote_nvpn
  local local_npub remote_npub
  local_nvpn="$(local_nvpn_path)"
  verify_local_nvpn "$local_nvpn"

  REMOTE_VM_DIR="$(resolve_remote_vm_dir)"
  sync_repo
  build_remote

  remote_nvpn="$(resolve_remote_nvpn_path)"
  verify_remote_nvpn "$remote_nvpn"

  log "stage: initialize configs"
  init_local_config "$local_nvpn"
  init_remote_config "$remote_nvpn"

  local_npub="$(extract_local_npub)"
  remote_npub="$(extract_remote_npub)"
  [[ -n "$local_npub" ]] || die "failed to extract local public_key from $HOST_CONFIG"
  [[ -n "$remote_npub" ]] || die "failed to extract remote public_key from $VM_CONFIG"

  log "stage: write network membership"
  configure_local "$local_nvpn" "$local_npub" "$remote_npub"
  configure_remote "$remote_nvpn" "$local_npub" "$remote_npub"
  summarize "$local_nvpn" "$remote_nvpn"
}

main "$@"
