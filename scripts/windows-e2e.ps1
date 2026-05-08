param(
  [string]$Configuration = "Debug",
  [string]$ArtifactRoot = "C:\Mac\Home\src\nostr-vpn\artifacts",
  [switch]$SkipCleanupOnFailure,
  [switch]$PacketDebug,
  [switch]$SkipGui
)

$ErrorActionPreference = "Stop"

$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
$Bin = Join-Path $Root "target\debug\nvpn.exe"
$AppExe = Join-Path $Root "windows\NostrVpn.Windows\bin\$Configuration\net8.0-windows\NostrVpn.Windows.exe"
$E2eRoot = Join-Path $env:TEMP "nvpn-windows-e2e"
$Screenshot = Join-Path $ArtifactRoot "windows-e2e-gui.png"
$StatusPath = Join-Path $ArtifactRoot "windows-e2e-status.json"
$GuiDir = Join-Path $env:APPDATA "Nostr VPN"
$AliceConfig = Join-Path $GuiDir "config.toml"
$BobConfig = Join-Path $E2eRoot "bob.toml"
$BackupConfig = Join-Path $E2eRoot "appdata-config.backup.toml"
$IcmpRuleName = "Nostr VPN E2E ICMPv4 $PID"

function Stop-Name {
  param([string]$Name)
  Get-Process -Name $Name -ErrorAction SilentlyContinue |
    Stop-Process -Force -ErrorAction SilentlyContinue
}

function Invoke-Nvpn {
  param([string[]]$Arguments)
  & $Bin @Arguments
  if ($LASTEXITCODE -ne 0) {
    throw "nvpn $($Arguments -join ' ') failed with exit code $LASTEXITCODE"
  }
}

function Read-NostrPublicKey {
  param([string]$Path)
  $inNostr = $false
  foreach ($line in Get-Content $Path) {
    if ($line -match '^\[nostr\]') {
      $inNostr = $true
      continue
    }
    if ($line -match '^\[') {
      $inNostr = $false
    }
    if ($inNostr -and $line -match '^public_key\s*=\s*"([^"]+)"') {
      return $Matches[1]
    }
  }
  throw "public_key not found in $Path"
}

function Read-Text {
  param([string]$Path)
  if (Test-Path $Path) {
    return Get-Content $Path -Raw
  }
  ""
}

function Join-CommandLine {
  param([string[]]$Arguments)
  ($Arguments | ForEach-Object { '"' + ($_ -replace '"', '\"') + '"' }) -join " "
}

function Wait-ForLog {
  param(
    [string]$Path,
    [string]$Pattern,
    [string]$Label,
    [System.Diagnostics.Process]$Process,
    [string]$ErrorLog
  )
  for ($i = 0; $i -lt 45; $i++) {
    $text = Read-Text $Path
    if ($text -match $Pattern) {
      return
    }
    if ($Process -and $Process.HasExited) {
      $errorText = Read-Text $ErrorLog
      throw "$Label exited before '$Pattern'. stdout:`n$text`nstderr:`n$errorText"
    }
    Start-Sleep -Seconds 1
  }
  $text = Read-Text $Path
  $errorText = Read-Text $ErrorLog
  throw "$Label did not match '$Pattern'. stdout:`n$text`nstderr:`n$errorText"
}

function Capture-Window {
  param(
    [int]$ProcessId,
    [string]$Path
  )
  Add-Type @"
using System;
using System.Runtime.InteropServices;
public struct RECT { public int Left; public int Top; public int Right; public int Bottom; }
public static class NativeWindowCapture {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
}
"@
  Add-Type -AssemblyName System.Drawing

  $proc = Get-Process -Id $ProcessId
  for ($i = 0; $i -lt 30 -and $proc.MainWindowHandle -eq 0; $i++) {
    Start-Sleep -Milliseconds 500
    $proc = Get-Process -Id $ProcessId
  }
  if ($proc.MainWindowHandle -eq 0) {
    throw "GUI main window did not appear"
  }

  [NativeWindowCapture]::SetForegroundWindow($proc.MainWindowHandle) | Out-Null
  Start-Sleep -Seconds 2

  $rect = New-Object RECT
  if (-not [NativeWindowCapture]::GetWindowRect($proc.MainWindowHandle, [ref]$rect)) {
    throw "GetWindowRect failed"
  }
  $width = $rect.Right - $rect.Left
  $height = $rect.Bottom - $rect.Top
  if ($width -lt 400 -or $height -lt 300) {
    throw "GUI window too small: ${width}x${height}"
  }

  New-Item -ItemType Directory -Force -Path (Split-Path $Path) | Out-Null
  $bitmap = New-Object System.Drawing.Bitmap($width, $height)
  $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
  try {
    $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size)
    $bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
  } finally {
    $graphics.Dispose()
    $bitmap.Dispose()
  }
}

function Write-NetworkDiagnostics {
  param(
    [string]$AliceConfig,
    [string]$BobConfig
  )

  Write-Host "--- adapters ---"
  Get-NetAdapter |
    Select-Object Name, InterfaceDescription, ifIndex, Status, MacAddress |
    Format-Table -AutoSize
  Write-Host "--- tunnel addresses ---"
  Get-NetIPAddress -AddressFamily IPv4 |
    Where-Object { $_.InterfaceAlias -like "nvpn*" -or $_.IPAddress -like "10.44.*" } |
    Format-List
  Write-Host "--- interface routes ---"
  netsh interface ipv4 show route
  Write-Host "--- 10.44 routes ---"
  route print 10.44.*
  Write-Host "--- Alice status ---"
  & $Bin status --json --discover-secs 0 --config $AliceConfig
  Write-Host "--- Bob status ---"
  & $Bin status --json --discover-secs 0 --config $BobConfig
}

if (!(Test-Path $Bin)) {
  throw "Missing nvpn.exe: $Bin"
}
if (!$SkipGui -and !(Test-Path $AppExe)) {
  throw "Missing Windows app: $AppExe"
}

Stop-Name "NostrVpn.Windows"
Stop-Name "nvpn"
Remove-Item -Recurse -Force $E2eRoot -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $E2eRoot | Out-Null
New-Item -ItemType Directory -Force -Path $GuiDir | Out-Null
New-Item -ItemType Directory -Force -Path $ArtifactRoot | Out-Null

$HadConfig = $false
if (Test-Path $AliceConfig) {
  Copy-Item $AliceConfig $BackupConfig -Force
  $HadConfig = $true
}

$AliceProc = $null
$BobProc = $null
$AppProc = $null
$Succeeded = $false

try {
  New-NetFirewallRule `
    -DisplayName $IcmpRuleName `
    -Direction Inbound `
    -Action Allow `
    -Protocol ICMPv4 `
    -IcmpType 8 `
    -Profile Any | Out-Null

  Invoke-Nvpn @("init", "--force", "--config", $AliceConfig)
  Invoke-Nvpn @("init", "--force", "--config", $BobConfig)
  $AliceNpub = Read-NostrPublicKey $AliceConfig
  $BobNpub = Read-NostrPublicKey $BobConfig

  Invoke-Nvpn @(
    "set", "--config", $AliceConfig,
    "--node-name", "Windows GUI",
    "--listen-port", "55181",
    "--endpoint", "127.0.0.1:55181",
    "--fips-advertise-endpoint", "true",
    "--participant", $AliceNpub,
    "--participant", $BobNpub,
    "--fips-peer-endpoint", "$BobNpub=127.0.0.1:55182"
  )
  Invoke-Nvpn @(
    "set", "--config", $BobConfig,
    "--node-name", "Windows peer",
    "--listen-port", "55182",
    "--endpoint", "127.0.0.1:55182",
    "--fips-advertise-endpoint", "true",
    "--participant", $AliceNpub,
    "--participant", $BobNpub,
    "--fips-peer-endpoint", "$AliceNpub=127.0.0.1:55181"
  )

  if ($PacketDebug) {
    $env:NVPN_FIPS_PACKET_DEBUG = "1"
  } else {
    Remove-Item Env:\NVPN_FIPS_PACKET_DEBUG -ErrorAction SilentlyContinue
  }
  $AliceProc = Start-Process -FilePath $Bin `
    -ArgumentList (Join-CommandLine @("connect", "--config", $AliceConfig, "--iface", "nvpn-gui", "--mesh-refresh-interval-secs", "5")) `
    -RedirectStandardOutput (Join-Path $E2eRoot "alice.out.log") `
    -RedirectStandardError (Join-Path $E2eRoot "alice.err.log") `
    -WindowStyle Hidden `
    -PassThru
  $BobProc = Start-Process -FilePath $Bin `
    -ArgumentList (Join-CommandLine @("connect", "--config", $BobConfig, "--iface", "nvpn-peer", "--mesh-refresh-interval-secs", "5")) `
    -RedirectStandardOutput (Join-Path $E2eRoot "bob.out.log") `
    -RedirectStandardError (Join-Path $E2eRoot "bob.err.log") `
    -WindowStyle Hidden `
    -PassThru
  Remove-Item Env:\NVPN_FIPS_PACKET_DEBUG -ErrorAction SilentlyContinue

  Wait-ForLog (Join-Path $E2eRoot "alice.out.log") "mesh: 1/1 peers connected" "Alice" $AliceProc (Join-Path $E2eRoot "alice.err.log")
  Wait-ForLog (Join-Path $E2eRoot "bob.out.log") "mesh: 1/1 peers connected" "Bob" $BobProc (Join-Path $E2eRoot "bob.err.log")
  Start-Sleep -Seconds 3

  $BobIp = (& $Bin ip --config $AliceConfig --peer --discover-secs 0 | Select-Object -First 1).Trim()
  if (!$BobIp) {
    throw "Alice could not resolve Bob peer tunnel IP"
  }
  $AliceIp = (& $Bin ip --config $BobConfig --peer --discover-secs 0 | Select-Object -First 1).Trim()
  if (!$AliceIp) {
    throw "Bob could not resolve Alice peer tunnel IP"
  }

  ping -n 3 $BobIp | Tee-Object -FilePath (Join-Path $E2eRoot "ping-alice-to-bob.log")
  if ($LASTEXITCODE -ne 0) {
    Write-NetworkDiagnostics $AliceConfig $BobConfig
    throw "ping Alice -> Bob ($BobIp) failed"
  }
  ping -n 3 $AliceIp | Tee-Object -FilePath (Join-Path $E2eRoot "ping-bob-to-alice.log")
  if ($LASTEXITCODE -ne 0) {
    Write-NetworkDiagnostics $AliceConfig $BobConfig
    throw "ping Bob -> Alice ($AliceIp) failed"
  }

  if (!$SkipGui) {
    $AppProc = Start-Process -FilePath $AppExe -PassThru
    Start-Sleep -Seconds 5
    Capture-Window $AppProc.Id $Screenshot

    & $Bin status --json --discover-secs 0 --config $AliceConfig |
      Out-File -Encoding utf8 $StatusPath
  }

  Write-Host "WINDOWS_E2E_OK"
  Write-Host "Alice npub: $AliceNpub"
  Write-Host "Bob npub: $BobNpub"
  Write-Host "Bob tunnel IP: $BobIp"
  Write-Host "Alice tunnel IP: $AliceIp"
  if (!$SkipGui) {
    Write-Host "Screenshot: $Screenshot"
    Write-Host "Status JSON: $StatusPath"
  }
  $Succeeded = $true
} finally {
  if ($SkipCleanupOnFailure -and !$Succeeded) {
    Write-Host "Skipping cleanup after failure. Processes: alice=$($AliceProc.Id) bob=$($BobProc.Id) app=$($AppProc.Id)"
  } else {
    if ($AppProc -and !$AppProc.HasExited) {
      Stop-Process -Id $AppProc.Id -Force -ErrorAction SilentlyContinue
    }
    if ($AliceProc -and !$AliceProc.HasExited) {
      Stop-Process -Id $AliceProc.Id -Force -ErrorAction SilentlyContinue
    }
    if ($BobProc -and !$BobProc.HasExited) {
      Stop-Process -Id $BobProc.Id -Force -ErrorAction SilentlyContinue
    }
    if ($HadConfig -and (Test-Path $BackupConfig)) {
      Copy-Item $BackupConfig $AliceConfig -Force
    } elseif (!$HadConfig -and (Test-Path $AliceConfig)) {
      Remove-Item $AliceConfig -Force -ErrorAction SilentlyContinue
    }
    Remove-NetFirewallRule -DisplayName $IcmpRuleName -ErrorAction SilentlyContinue
  }
}
