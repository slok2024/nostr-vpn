param(
  [ValidateSet("Debug", "Release")]
  [string]$Configuration = "Debug",
  [switch]$Run,
  [switch]$Publish
)

$ErrorActionPreference = "Stop"
$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
$Project = Join-Path $Root "windows\NostrVpn.Windows\NostrVpn.Windows.csproj"

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

Invoke-Checked cargo @("build", "-p", "nostr-vpn-app-core", "-p", "nostr-vpn-cli")

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
