param(
  [ValidateSet("Debug", "Release")]
  [string]$Configuration = "Debug",
  [switch]$Run,
  [switch]$Publish
)

$ErrorActionPreference = "Stop"
$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
$Project = Join-Path $Root "windows\NostrVpn.Windows\NostrVpn.Windows.csproj"
$CargoTargetRoot = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Root "target" }
$CargoProfile = if ($Configuration -eq "Release") { "release" } else { "debug" }

Set-Location $Root

$LlvmBin = if ($env:NVPN_WINDOWS_LLVM_BIN) { $env:NVPN_WINDOWS_LLVM_BIN } else { "C:\Program Files\LLVM\bin" }
if (Test-Path (Join-Path $LlvmBin "clang.exe")) {
  $env:PATH = "$LlvmBin;$env:PATH"
}

function Invoke-Checked {
  param(
    [string]$FilePath,
    [string[]]$Arguments
  )
  & $FilePath @Arguments
  if ($LASTEXITCODE -ne 0) {
    throw "$FilePath failed with exit code $LASTEXITCODE"
  }
}

$CargoArgs = @("build", "-p", "nostr-vpn-app-core", "-p", "nostr-vpn-cli", "-p", "nostr-vpn-relay")
if ($Configuration -eq "Release") {
  $CargoArgs += "--release"
}
Invoke-Checked cargo $CargoArgs

$CargoOutputDir = Join-Path $CargoTargetRoot $CargoProfile
$AppCargoDir = Join-Path $Root "target\$Configuration"
New-Item -ItemType Directory -Force -Path $AppCargoDir | Out-Null
foreach ($FileName in @("nostr_vpn_app_core.dll", "nvpn.exe")) {
  $Source = Join-Path $CargoOutputDir $FileName
  $Destination = Join-Path $AppCargoDir $FileName
  if (Test-Path $Source) {
    if ([System.IO.Path]::GetFullPath($Source) -ine [System.IO.Path]::GetFullPath($Destination)) {
      Copy-Item -Force $Source $Destination
    }
  }
}

if ($Publish) {
  Invoke-Checked dotnet @("publish", $Project, "-c", $Configuration, "-r", "win-x64", "--self-contained", "false")
} else {
  Invoke-Checked dotnet @("build", $Project, "-c", $Configuration)
}

if ($Run) {
  $exe = Join-Path $Root "windows\NostrVpn.Windows\bin\$Configuration\net8.0-windows\NostrVpn.Windows.exe"
  if (!(Test-Path $exe)) {
    throw "Built Windows app not found: $exe"
  }
  & $exe
}
