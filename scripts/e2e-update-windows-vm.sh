#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VM_NAME="${VM_NAME:-${1:-Windows 11}}"
SHARED_REPO="${NVPN_WINDOWS_SHARED_REPO_PATH:-C:\\Mac\\Home\\src\\nostr-vpn}"
GUEST_REPO="${GUEST_REPO:-C:\\Users\\sirius\\src\\nostr-vpn}"
GUEST_ARTIFACT_ROOT="${GUEST_ARTIFACT_ROOT:-C:\\Mac\\Home\\src\\nostr-vpn\\artifacts}"

run_ps_user() {
  local script="$1"
  local ps_tmp_dir="$ROOT/target/windows-vm-ps"
  mkdir -p "$ps_tmp_dir"
  local host_script
  host_script="$(mktemp "$ps_tmp_dir/prlctl.XXXXXX")"
  mv "$host_script" "$host_script.ps1"
  host_script="$host_script.ps1"
  printf '%s\n' "$script" >"$host_script"

  local rel_script="${host_script#"$ROOT"/}"
  rel_script="${rel_script//\//\\}"
  local guest_script="${SHARED_REPO}\\${rel_script}"

  prlctl exec "$VM_NAME" --current-user powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$guest_script"
  local status=$?
  rm -f "$host_script"
  return "$status"
}

run_ps_user "\$ErrorActionPreference = \"Stop\"
\$sharedRepo = \"$SHARED_REPO\"
\$guestRepo = \"$GUEST_REPO\"
\$guestRoot = Split-Path \$guestRepo
New-Item -ItemType Directory -Force -Path \$guestRoot | Out-Null
robocopy \$sharedRepo \$guestRepo /MIR /XD target dist .git artifacts /XF .env.release.local | Out-Null
if (\$LASTEXITCODE -ge 8) { throw \"robocopy failed with exit code \$LASTEXITCODE\" }
exit 0"

run_ps_user "Set-Location \"$GUEST_REPO\"
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\\scripts\\e2e-update-windows.ps1 -ArtifactRoot \"$GUEST_ARTIFACT_ROOT\"
exit \$LASTEXITCODE"
