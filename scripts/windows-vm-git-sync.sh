#!/usr/bin/env bash
# Push the current workspace to a git remote on an SSH-reachable Windows VM and
# fast-forward the VM checkout. This intentionally avoids tar/rsync code syncs.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC_ROOT="$(cd "$ROOT/.." && pwd)"
SSH_HOST="${NVPN_WINDOWS_SSH_HOST:-${1:-win11-dev}}"
GUEST_REPO="${NVPN_WINDOWS_GUEST_REPO_PATH:-C:\\src\\nostr-vpn}"
GUEST_BARE_REPO="${NVPN_WINDOWS_GIT_BARE_PATH:-C:\\src\\nostr-vpn.git}"
REMOTE_REF="${NVPN_WINDOWS_GIT_REF:-refs/heads/codex/windows-vm-sync}"
REMOTE_URL="${NVPN_WINDOWS_GIT_REMOTE_URL:-${SSH_HOST}:${GUEST_BARE_REPO//\\//}}"

run_ps() {
  local script="$1"
  local encoded
  encoded="$(printf '%s' "$script" | iconv -t UTF-16LE | base64)"
  ssh "$SSH_HOST" powershell.exe -NoProfile -EncodedCommand "$encoded"
}

ps_quote() {
  local value="${1//\'/\'\'}"
  printf "'%s'" "$value"
}

make_sync_commit() {
  local repo_dir="$1"
  local git_dir
  local tmp_index
  local tree
  local parent
  git_dir="$(git -C "$repo_dir" rev-parse --path-format=absolute --git-dir)"
  tmp_index="$(mktemp "$git_dir/windows-vm-index.XXXXXX")"
  (
    export GIT_INDEX_FILE="$tmp_index"
    git -C "$repo_dir" read-tree HEAD
    git -C "$repo_dir" add -A
    tree="$(git -C "$repo_dir" write-tree)"
    if [[ "${NVPN_WINDOWS_GIT_SYNC_WITH_HISTORY:-0}" =~ ^(1|true|TRUE|True|yes|YES|Yes|on|ON|On)$ ]]; then
      parent="$(git -C "$repo_dir" rev-parse HEAD)"
      printf 'Temporary Windows VM sync\n' | git -C "$repo_dir" commit-tree "$tree" -p "$parent"
    else
      printf 'Temporary Windows VM sync\n' | git -C "$repo_dir" commit-tree "$tree"
    fi
  )
  rm -f "$tmp_index"
}

ensure_remote_bare_repo() {
  local bare_repo="$1"
  run_ps "\$ErrorActionPreference = 'Stop'
\$BareRepo = $(ps_quote "$bare_repo")
\$BareParent = Split-Path -Parent \$BareRepo
New-Item -ItemType Directory -Force -Path \$BareParent | Out-Null
if (!(Test-Path \$BareRepo)) {
  git init --bare \$BareRepo
} else {
  \$isBare = git -C \$BareRepo rev-parse --is-bare-repository
  if (\$LASTEXITCODE -ne 0 -or \$isBare.Trim() -ne 'true') {
    throw \"Windows git remote is not a bare repository: \$BareRepo\"
  }
}"
}

checkout_remote_ref() {
  local worktree="$1"
  local bare_repo="$2"
  local remote_ref="$3"
  local branch_name="${remote_ref#refs/heads/}"
  run_ps "\$ErrorActionPreference = 'Stop'
\$BareRepo = $(ps_quote "$bare_repo")
\$Worktree = $(ps_quote "$worktree")
\$RemoteRef = $(ps_quote "$remote_ref")
\$BranchName = $(ps_quote "$branch_name")
if (!(Test-Path (Join-Path \$Worktree '.git'))) {
  Remove-Item -Recurse -Force -Path \$Worktree -ErrorAction SilentlyContinue
  git clone \$BareRepo \$Worktree
}
Set-Location \$Worktree
git remote set-url origin \$BareRepo
git fetch origin \$RemoteRef
git checkout -B \$BranchName FETCH_HEAD
git reset --hard FETCH_HEAD
git clean -ffd -e target/ -e dist/ -e artifacts/ -e windows/NostrVpn.Windows/bin/ -e windows/NostrVpn.Windows/obj/
git status --short --branch"
}

sync_repo() {
  local label="$1"
  local local_repo="$2"
  local worktree="$3"
  local bare_repo="$4"
  local remote_ref="$5"
  local remote_url="${SSH_HOST}:${bare_repo//\\//}"
  local sync_commit
  local local_tree
  local remote_tree

  if ! git -C "$local_repo" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    echo "Skipping Windows VM git sync for $label; local checkout not found at $local_repo"
    return
  fi

  ensure_remote_bare_repo "$bare_repo"
  sync_commit="$(make_sync_commit "$local_repo")"
  local_tree="$(git -C "$local_repo" rev-parse "$sync_commit^{tree}")"
  if git -C "$local_repo" fetch --quiet "$remote_url" "$remote_ref" 2>/dev/null; then
    remote_tree="$(git -C "$local_repo" rev-parse "FETCH_HEAD^{tree}")"
    if [[ "$remote_tree" == "$local_tree" ]]; then
      echo "WINDOWS_VM_GIT_SYNC_UNCHANGED $label"
      checkout_remote_ref "$worktree" "$bare_repo" "$remote_ref"
      return
    fi
  fi

  git -C "$local_repo" push --force "$remote_url" "$sync_commit:$remote_ref"
  checkout_remote_ref "$worktree" "$bare_repo" "$remote_ref"
  echo "WINDOWS_VM_GIT_SYNC_OK $label"
}

sync_repo "nostr-vpn" "$ROOT" "$GUEST_REPO" "$GUEST_BARE_REPO" "$REMOTE_REF"

case "${NVPN_WINDOWS_SYNC_PATH_DEPS:-1}" in
  0|false|FALSE|False|no|NO|No|off|OFF|Off)
    ;;
  *)
    sync_repo "nostr-pubsub" "$SRC_ROOT/nostr-pubsub" "C:\\src\\nostr-pubsub" "C:\\src\\nostr-pubsub.git" "refs/heads/codex/windows-vm-sync-nostr-pubsub"
    sync_repo "fips" "$SRC_ROOT/fips" "C:\\src\\fips" "C:\\src\\fips.git" "refs/heads/codex/windows-vm-sync-fips"
    sync_repo "hashtree" "$SRC_ROOT/hashtree" "C:\\src\\hashtree" "C:\\src\\hashtree.git" "refs/heads/codex/windows-vm-sync-hashtree"
    ;;
esac
