param(
  [Parameter(Mandatory = $true)]
  [string]$AppExe,
  [string]$ArtifactRoot,
  [int]$StartupTimeoutSeconds = 30,
  [int]$AliveSeconds = 5,
  [switch]$NoWindowRequired,
  [switch]$SkipCleanup
)

$ErrorActionPreference = "Stop"

$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
if (!$ArtifactRoot) {
  $ArtifactRoot = Join-Path $Root "artifacts"
}
$ArtifactRoot = [System.IO.Path]::GetFullPath($ArtifactRoot)
$ResultPath = Join-Path $ArtifactRoot "windows-app-launch-smoke.json"
$EventsPath = Join-Path $ArtifactRoot "windows-app-launch-events.json"

function Stop-NostrVpnWindows {
  Get-Process -Name NostrVpn.Windows -ErrorAction SilentlyContinue |
    Stop-Process -Force -ErrorAction SilentlyContinue
}

function Get-NostrVpnEvents {
  param([datetime]$Since)

  try {
    Get-WinEvent -FilterHashtable @{ LogName = "Application"; StartTime = $Since } -ErrorAction SilentlyContinue |
      Where-Object {
        $_.ProviderName -in @(".NET Runtime", "Application Error", "Windows Error Reporting") -or
        $_.Message -match "NostrVpn\.Windows"
      } |
      Select-Object TimeCreated, ProviderName, Id, LevelDisplayName, Message
  } catch {
    @([pscustomobject]@{
        TimeCreated      = Get-Date
        ProviderName     = "windows-app-launch-smoke"
        Id               = 0
        LevelDisplayName = "Warning"
        Message          = "Could not read Windows Application event log: $($_.Exception.Message)"
      })
  }
}

function Write-SmokeResult {
  param(
    [bool]$Ok,
    [string]$ErrorMessage = "",
    [int]$ProcessId = 0,
    [int]$ExitCode = 0,
    [bool]$WindowSeen = $false
  )

  New-Item -ItemType Directory -Force -Path $ArtifactRoot | Out-Null
  [pscustomobject]@{
    ok          = $Ok
    appExe      = $AppExe
    processId   = $ProcessId
    exitCode    = $ExitCode
    windowSeen  = $WindowSeen
    error       = $ErrorMessage
    generatedAt = (Get-Date).ToUniversalTime().ToString("o")
  } | ConvertTo-Json -Depth 4 | Out-File -Encoding utf8 $ResultPath
}

if (!(Test-Path $AppExe)) {
  throw "Windows app executable not found: $AppExe"
}

New-Item -ItemType Directory -Force -Path $ArtifactRoot | Out-Null
Stop-NostrVpnWindows

$startTime = (Get-Date).AddSeconds(-3)
$proc = $null
$windowSeen = $false

try {
  $proc = Start-Process -FilePath $AppExe -WorkingDirectory (Split-Path -Parent $AppExe) -PassThru
  $deadline = (Get-Date).AddSeconds($StartupTimeoutSeconds)

  while ((Get-Date) -lt $deadline) {
    Start-Sleep -Milliseconds 500
    $proc.Refresh()
    if ($proc.HasExited) {
      $events = @(Get-NostrVpnEvents -Since $startTime)
      $events | ConvertTo-Json -Depth 6 | Out-File -Encoding utf8 $EventsPath
      Write-SmokeResult -Ok $false -ErrorMessage "NostrVpn.Windows exited during startup" -ProcessId $proc.Id -ExitCode $proc.ExitCode -WindowSeen $windowSeen
      $eventText = if ($events.Count -gt 0) { ($events | Select-Object -First 3 | ForEach-Object { $_.Message }) -join "`n---`n" } else { "No matching Application event-log entries were found." }
      throw "NostrVpn.Windows exited during startup with code $($proc.ExitCode). Recent event log:`n$eventText"
    }

    if ($NoWindowRequired -or $proc.MainWindowHandle -ne 0) {
      $windowSeen = $proc.MainWindowHandle -ne 0
      break
    }
  }

  if (!$NoWindowRequired -and !$windowSeen) {
    $events = @(Get-NostrVpnEvents -Since $startTime)
    $events | ConvertTo-Json -Depth 6 | Out-File -Encoding utf8 $EventsPath
    Write-SmokeResult -Ok $false -ErrorMessage "NostrVpn.Windows stayed alive but did not create a main window" -ProcessId $proc.Id -ExitCode 0 -WindowSeen $false
    throw "NostrVpn.Windows stayed alive but did not create a main window within $StartupTimeoutSeconds seconds."
  }

  $aliveUntil = (Get-Date).AddSeconds($AliveSeconds)
  while ((Get-Date) -lt $aliveUntil) {
    Start-Sleep -Milliseconds 500
    $proc.Refresh()
    if ($proc.HasExited) {
      $events = @(Get-NostrVpnEvents -Since $startTime)
      $events | ConvertTo-Json -Depth 6 | Out-File -Encoding utf8 $EventsPath
      Write-SmokeResult -Ok $false -ErrorMessage "NostrVpn.Windows exited after launch" -ProcessId $proc.Id -ExitCode $proc.ExitCode -WindowSeen $windowSeen
      $eventText = if ($events.Count -gt 0) { ($events | Select-Object -First 3 | ForEach-Object { $_.Message }) -join "`n---`n" } else { "No matching Application event-log entries were found." }
      throw "NostrVpn.Windows exited after launch with code $($proc.ExitCode). Recent event log:`n$eventText"
    }
  }

  Write-SmokeResult -Ok $true -ProcessId $proc.Id -WindowSeen $windowSeen
  Write-Host "WINDOWS_APP_LAUNCH_SMOKE_OK"
  Write-Host "Result: $ResultPath"
} finally {
  if (!$SkipCleanup -and $proc -and !$proc.HasExited) {
    Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
  }
}
