#!/usr/bin/env bash
# Windows GUI E2E driver. Runs on the host (macOS); pushes the working tree
# to a Windows VM reachable over SSH. Set NVPN_WINDOWS_SSH_HOST for local
# machine-specific hostnames.
# builds the app, launches the GUI, and pulls back a cropped screenshot of
# the main window.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SSH_HOST="${NVPN_WINDOWS_SSH_HOST:-${1:-windows-dev}}"
GUEST_REPO="${NVPN_WINDOWS_GUEST_REPO_PATH:-C:\\src\\nostr-vpn}"
ARTIFACT_ROOT="${ARTIFACT_ROOT:-$ROOT/artifacts}"
RECT_JSON="$ARTIFACT_ROOT/windows-gui-rect.json"
FULL_SCREENSHOT="$ARTIFACT_ROOT/windows-vm-full.png"
APP_SCREENSHOT="$ARTIFACT_ROOT/windows-e2e-gui.png"

mkdir -p "$ARTIFACT_ROOT"

# Run a PowerShell script on the remote SSH host. We base64-encode so the
# script survives the SSH/cmd.exe boundary without quoting headaches.
run_ps() {
  local script="$1"
  local encoded
  encoded="$(printf '%s' "$script" | iconv -t UTF-16LE | base64)"
  ssh "$SSH_HOST" powershell.exe -NoProfile -EncodedCommand "$encoded"
}

# Sync host -> remote with a tar pipe over SSH. Matches the exclusion list
# used by scripts/local-release.mjs; both sides see the same archive format.
sync_repo() {
  run_ps "New-Item -ItemType Directory -Force -Path '$GUEST_REPO' | Out-Null"
  run_ps "Get-ChildItem -LiteralPath '$GUEST_REPO' -Recurse -Force -Filter '._*' -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue"
  local guest_repo_unix="${GUEST_REPO//\\//}"
  COPYFILE_DISABLE=1 tar \
    --exclude='._*' \
    --exclude='*/._*' \
    --exclude=./target \
    --exclude=./dist \
    --exclude=./.git \
    --exclude=./artifacts \
    --exclude=./node_modules \
    --exclude=./.env.release.local \
    --exclude=./.env.zapstore.local \
    --exclude=./macos/.build \
    --exclude=./linux/target \
    -cf - -C "$ROOT" . \
    | ssh "$SSH_HOST" tar -xf - -C "$guest_repo_unix"
}

cleanup_gui() {
  run_ps 'Get-Process -Name NostrVpn.Windows -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue' >/dev/null 2>&1 || true
  run_ps 'Get-Process -Name Consent -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue' >/dev/null 2>&1 || true
  rm -f "$FULL_SCREENSHOT"
}
trap cleanup_gui EXIT

sync_repo

run_ps "\$ErrorActionPreference = 'Stop'
Set-Location '$GUEST_REPO'
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\\scripts\\windows-build.ps1
exit \$LASTEXITCODE"

run_ps "\$ErrorActionPreference = 'Stop'
Set-Location '$GUEST_REPO'
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\\scripts\\windows-e2e.ps1 -SkipGui
exit \$LASTEXITCODE"

run_ps 'Get-Process -Name Consent -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue'

run_ps "\$ErrorActionPreference = 'Stop'
\$AppExe = '$GUEST_REPO\\windows\\NostrVpn.Windows\\bin\\Debug\\net8.0-windows\\NostrVpn.Windows.exe'
Get-Process -Name NostrVpn.Windows -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Process -FilePath \$AppExe"

# Wait for the window to appear, then capture its on-screen rect into a JSON
# file in the guest's user TEMP — we'll pull that file back below.
GUEST_RECT_PATH='$env:TEMP\\nvpn-windows-gui-rect.json'
run_ps "\$ErrorActionPreference = 'Stop'
\$RectPath = $GUEST_RECT_PATH
Add-Type @'
using System;
using System.Runtime.InteropServices;
public struct RECT { public int Left; public int Top; public int Right; public int Bottom; }
public static class NvpnWindowRect {
  [DllImport(\"user32.dll\")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
  [DllImport(\"user32.dll\")] public static extern bool SetForegroundWindow(IntPtr hWnd);
}
'@
for (\$i = 0; \$i -lt 40; \$i++) {
  Start-Sleep -Milliseconds 500
  \$proc = Get-Process -Name NostrVpn.Windows -ErrorAction SilentlyContinue | Select-Object -First 1
  \$uac = Get-Process -Name Consent -ErrorAction SilentlyContinue | Select-Object -First 1
  if (\$uac -and \$uac.MainWindowTitle -like '*User Account Control*') { throw 'UAC prompt appeared while launching Windows GUI' }
  if (\$proc -and \$proc.MainWindowHandle -ne 0) { break }
}
if (!\$proc -or \$proc.MainWindowHandle -eq 0) { throw 'GUI main window did not appear' }
[NvpnWindowRect]::SetForegroundWindow(\$proc.MainWindowHandle) | Out-Null
Start-Sleep -Seconds 2
\$rect = New-Object RECT
if (-not [NvpnWindowRect]::GetWindowRect(\$proc.MainWindowHandle, [ref]\$rect)) { throw 'GetWindowRect failed' }
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
run_ps "\$ErrorActionPreference = 'Stop'
\$uac = Get-Process -Name Consent -ErrorAction SilentlyContinue | Select-Object -First 1
if (\$uac -and \$uac.MainWindowTitle -like '*User Account Control*') { throw 'UAC prompt appeared before Windows GUI capture' }"

# Capture the full desktop on the remote, then pull it back.
run_ps "\$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms,System.Drawing
\$bounds = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
\$bmp = New-Object System.Drawing.Bitmap \$bounds.Width, \$bounds.Height
\$g = [System.Drawing.Graphics]::FromImage(\$bmp)
\$g.CopyFromScreen(\$bounds.Location, [System.Drawing.Point]::Empty, \$bounds.Size)
\$out = Join-Path \$env:TEMP 'nvpn-windows-vm-full.png'
\$bmp.Save(\$out, [System.Drawing.Imaging.ImageFormat]::Png)
\$g.Dispose(); \$bmp.Dispose()"

ssh "$SSH_HOST" tar -cf - -C "%TEMP%" 'nvpn-windows-vm-full.png' 'nvpn-windows-gui-rect.json' 2>/dev/null \
  | tar -xf - -C "$ARTIFACT_ROOT"
mv -f "$ARTIFACT_ROOT/nvpn-windows-vm-full.png" "$FULL_SCREENSHOT"
mv -f "$ARTIFACT_ROOT/nvpn-windows-gui-rect.json" "$RECT_JSON"

node - "$RECT_JSON" "$FULL_SCREENSHOT" "$APP_SCREENSHOT" <<'NODE'
const fs = require('fs');
const { spawnSync } = require('child_process');

const [rectPath, fullPath, outputPath] = process.argv.slice(2);
const rect = JSON.parse(fs.readFileSync(rectPath, 'utf8').replace(/^﻿/, ''));
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
