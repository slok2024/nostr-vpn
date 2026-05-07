#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VM_NAME="${VM_NAME:-${1:-Windows 11}}"
SHARED_REPO="${NVPN_WINDOWS_SHARED_REPO_PATH:-C:\\Mac\\Home\\src\\nostr-vpn}"
GUEST_REPO="${GUEST_REPO:-C:\\Users\\sirius\\src\\nostr-vpn}"
GUEST_ARTIFACT_ROOT="${GUEST_ARTIFACT_ROOT:-C:\\Mac\\Home\\src\\nostr-vpn\\artifacts}"
ARTIFACT_ROOT="${ARTIFACT_ROOT:-$ROOT/artifacts}"
RECT_JSON="$ARTIFACT_ROOT/windows-gui-rect.json"
FULL_SCREENSHOT="$ARTIFACT_ROOT/windows-vm-full.png"
APP_SCREENSHOT="$ARTIFACT_ROOT/windows-e2e-gui.png"

mkdir -p "$ARTIFACT_ROOT"

run_ps() {
  local mode="$1"
  local script="$2"
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
  local status=0
  if [[ "$mode" == "system" ]]; then
    prlctl exec "$VM_NAME" powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$guest_script" || status=$?
  else
    prlctl exec "$VM_NAME" --current-user powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$guest_script" || status=$?
  fi
  rm -f "$host_script"
  return "$status"
}

run_ps_system() {
  run_ps system "$1"
}

run_ps_user() {
  run_ps user "$1"
}

cleanup_gui() {
  run_ps_user 'Get-Process -Name NostrVpn.Windows -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue' >/dev/null 2>&1 || true
  rm -f "$FULL_SCREENSHOT"
}
trap cleanup_gui EXIT

run_ps_user "\$ErrorActionPreference = \"Stop\"
\$sharedRepo = \"$SHARED_REPO\"
\$guestRepo = \"$GUEST_REPO\"
\$guestRoot = Split-Path \$guestRepo
New-Item -ItemType Directory -Force -Path \$guestRoot | Out-Null
robocopy \$sharedRepo \$guestRepo /MIR /XD target dist .git artifacts /XF .env.release.local | Out-Null
if (\$LASTEXITCODE -ge 8) { throw \"robocopy failed with exit code \$LASTEXITCODE\" }
exit 0"

run_ps_user "Set-Location \"$GUEST_REPO\"
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\\scripts\\windows-build.ps1
exit \$LASTEXITCODE"

run_ps_system "Set-Location \"$GUEST_REPO\"
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\\scripts\\windows-e2e.ps1 -SkipGui
exit \$LASTEXITCODE"

run_ps_user "\$ErrorActionPreference = \"Stop\"
\$AppExe = \"$GUEST_REPO\\windows\\NostrVpn.Windows\\bin\\Debug\\net8.0-windows\\NostrVpn.Windows.exe\"
Get-Process -Name NostrVpn.Windows -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Process -FilePath \$AppExe"

run_ps_user "\$ErrorActionPreference = \"Stop\"
\$RectPath = \"$GUEST_ARTIFACT_ROOT\\windows-gui-rect.json\"
Add-Type @\"
using System;
using System.Runtime.InteropServices;
public struct RECT { public int Left; public int Top; public int Right; public int Bottom; }
public static class NvpnWindowRect {
  [DllImport(\"user32.dll\")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
  [DllImport(\"user32.dll\")] public static extern bool SetForegroundWindow(IntPtr hWnd);
}
\"@
for (\$i = 0; \$i -lt 40; \$i++) {
  Start-Sleep -Milliseconds 500
  \$proc = Get-Process -Name NostrVpn.Windows -ErrorAction SilentlyContinue | Select-Object -First 1
  if (\$proc -and \$proc.MainWindowHandle -ne 0) { break }
}
if (!\$proc -or \$proc.MainWindowHandle -eq 0) { throw \"GUI main window did not appear\" }
[NvpnWindowRect]::SetForegroundWindow(\$proc.MainWindowHandle) | Out-Null
Start-Sleep -Seconds 2
\$rect = New-Object RECT
if (-not [NvpnWindowRect]::GetWindowRect(\$proc.MainWindowHandle, [ref]\$rect)) { throw \"GetWindowRect failed\" }
\$window = [pscustomobject]@{
  Left = \$rect.Left
  Top = \$rect.Top
  Right = \$rect.Right
  Bottom = \$rect.Bottom
  Width = \$rect.Right - \$rect.Left
  Height = \$rect.Bottom - \$rect.Top
}
\$window | ConvertTo-Json -Compress | Out-File -Encoding utf8 \$RectPath"

sleep 3
prlctl capture "$VM_NAME" --file "$FULL_SCREENSHOT"

node - "$RECT_JSON" "$FULL_SCREENSHOT" "$APP_SCREENSHOT" <<'NODE'
const fs = require('fs');
const { spawnSync } = require('child_process');

const [rectPath, fullPath, outputPath] = process.argv.slice(2);
const rect = JSON.parse(fs.readFileSync(rectPath, 'utf8').replace(/^\uFEFF/, ''));
if (rect.Width < 400 || rect.Height < 300) {
  throw new Error(`GUI window too small: ${rect.Width}x${rect.Height}`);
}

const result = spawnSync('sips', [
  '--cropToHeightWidth',
  String(rect.Height),
  String(rect.Width),
  '--cropOffset',
  String(Math.max(0, rect.Top)),
  String(Math.max(0, rect.Left)),
  fullPath,
  '--out',
  outputPath,
], { stdio: 'inherit' });

if (result.status !== 0) {
  process.exit(result.status ?? 1);
}
NODE

rm -f "$FULL_SCREENSHOT"
echo "WINDOWS_VM_E2E_OK"
echo "Screenshot: $APP_SCREENSHOT"
