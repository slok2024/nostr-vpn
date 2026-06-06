param(
  [Parameter(Mandatory = $true)]
  [string]$InstallerPath,
  [string]$InstallDir,
  [string]$ArtifactRoot
)

$ErrorActionPreference = "Stop"

$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
if (!$ArtifactRoot) {
  $ArtifactRoot = Join-Path $Root "artifacts"
}
if (!$InstallDir) {
  $InstallDir = Join-Path $env:TEMP "nostr-vpn-installer-smoke"
}

$InstallerPath = [System.IO.Path]::GetFullPath($InstallerPath)
$InstallDir = [System.IO.Path]::GetFullPath($InstallDir)
$ArtifactRoot = [System.IO.Path]::GetFullPath($ArtifactRoot)

if (!(Test-Path $InstallerPath)) {
  throw "Windows installer not found: $InstallerPath"
}

New-Item -ItemType Directory -Force -Path $ArtifactRoot | Out-Null
Get-Process -Name NostrVpn.Windows -ErrorAction SilentlyContinue |
  Stop-Process -Force -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force $InstallDir -ErrorAction SilentlyContinue

try {
  $installArgs = @(
    "/VERYSILENT",
    "/SUPPRESSMSGBOXES",
    "/NORESTART",
    "/DIR=$InstallDir"
  )
  $setup = Start-Process -FilePath $InstallerPath -ArgumentList $installArgs -Wait -PassThru
  if ($setup.ExitCode -ne 0) {
    throw "Installer exited with code $($setup.ExitCode)"
  }

  $appExe = Join-Path $InstallDir "NostrVpn.Windows.exe"
  if (!(Test-Path $appExe)) {
    throw "Installed Windows app not found: $appExe"
  }

  & (Join-Path $PSScriptRoot "windows-app-launch-smoke.ps1") -AppExe $appExe -ArtifactRoot $ArtifactRoot -NoWindowRequired
  Write-Host "WINDOWS_INSTALLER_SMOKE_OK"
} finally {
  $uninstaller = Join-Path $InstallDir "unins000.exe"
  if (Test-Path $uninstaller) {
    $uninstallArgs = @("/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART")
    Start-Process -FilePath $uninstaller -ArgumentList $uninstallArgs -Wait -ErrorAction SilentlyContinue | Out-Null
  }
}
