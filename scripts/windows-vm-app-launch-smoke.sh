#!/usr/bin/env bash
# Build the Windows installer on an SSH-reachable Windows VM, install it, and
# verify NostrVpn.Windows starts and stays alive instead of exiting at startup.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SSH_HOST="${NVPN_WINDOWS_SSH_HOST:-${1:-win11-dev}}"
GUEST_REPO="${NVPN_WINDOWS_GUEST_REPO_PATH:-C:\\src\\nostr-vpn}"
GUEST_ARTIFACT_ROOT="${GUEST_ARTIFACT_ROOT:-C:\\src\\nostr-vpn\\artifacts}"
ARTIFACT_ROOT="${ARTIFACT_ROOT:-$ROOT/artifacts}"
SMOKE_TAG="${NVPN_WINDOWS_APP_SMOKE_TAG:-v0.0.0}"

mkdir -p "$ARTIFACT_ROOT"

run_ps() {
  local script="$1"
  local encoded
  encoded="$(printf '%s' "$script" | iconv -t UTF-16LE | base64)"
  ssh "$SSH_HOST" powershell.exe -NoProfile -EncodedCommand "$encoded"
}

"$ROOT/scripts/windows-vm-git-sync.sh" "$SSH_HOST"

run_ps "\$ErrorActionPreference = 'Stop'
Set-Location '$GUEST_REPO'
New-Item -ItemType Directory -Force -Path '$GUEST_ARTIFACT_ROOT' | Out-Null
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\\scripts\\windows-build.ps1 -Configuration Release -Installer -Tag '$SMOKE_TAG' -OutputDir '$GUEST_ARTIFACT_ROOT'
\$installer = Join-Path '$GUEST_ARTIFACT_ROOT' 'nostr-vpn-$SMOKE_TAG-windows-x64-setup.exe'
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\\scripts\\windows-installer-smoke.ps1 -InstallerPath \$installer -ArtifactRoot '$GUEST_ARTIFACT_ROOT'
exit \$LASTEXITCODE"

echo "WINDOWS_VM_APP_LAUNCH_SMOKE_OK"
