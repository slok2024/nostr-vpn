#!/usr/bin/env bash
# Run the Windows update-check E2E inside a Windows VM reachable over SSH.
# Set NVPN_WINDOWS_SSH_HOST for local machine-specific hostnames.
# Replaces the previous Parallels
# `prlctl exec` flow.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SSH_HOST="${NVPN_WINDOWS_SSH_HOST:-${1:-win11-dev}}"
GUEST_REPO="${NVPN_WINDOWS_GUEST_REPO_PATH:-C:\\src\\nostr-vpn}"
GUEST_ARTIFACT_ROOT="${GUEST_ARTIFACT_ROOT:-C:\\src\\nostr-vpn\\artifacts}"
ARTIFACT_ROOT="${ARTIFACT_ROOT:-$ROOT/artifacts}"

mkdir -p "$ARTIFACT_ROOT"

run_ps() {
  local script="$1"
  local encoded
  encoded="$(printf '%s' "$script" | iconv -t UTF-16LE | base64)"
  ssh "$SSH_HOST" powershell.exe -NoProfile -EncodedCommand "$encoded"
}

"$ROOT/scripts/windows-vm-git-sync.sh" "$SSH_HOST"

run_ps "Set-Location '$GUEST_REPO'
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\\scripts\\e2e-update-windows.ps1 -ArtifactRoot '$GUEST_ARTIFACT_ROOT'
exit \$LASTEXITCODE"
