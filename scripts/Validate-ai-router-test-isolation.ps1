param(
    [Parameter(Mandatory)][string]$TestRoot,
    [string]$BootstrapVerifiedDatabase,
    [string]$CodexHome,
    [string]$DataDir,
    [int]$GatewayPort = 38123
)

$ErrorActionPreference = 'Stop'
$helper = Join-Path $PSScriptRoot 'IsolatedInstanceSafety.ps1'
if (-not (Test-Path -LiteralPath $helper)) {
    throw "Isolation safety helper not found: $helper"
}
. $helper

$root = [System.IO.Path]::GetFullPath($TestRoot)
$codexHome = if ([string]::IsNullOrWhiteSpace($CodexHome)) { Join-Path $root 'codex-home' } else { [System.IO.Path]::GetFullPath($CodexHome) }
$dataDir = if ([string]::IsNullOrWhiteSpace($DataDir)) { Join-Path $root 'data' } else { [System.IO.Path]::GetFullPath($DataDir) }
Assert-IsolatedInstanceLayout `
    -TestRoot $root `
    -CodexHome $codexHome `
    -DataDir $dataDir `
    -BootstrapVerifiedDatabase $BootstrapVerifiedDatabase `
    -GatewayPort $GatewayPort
Assert-IsolatedGatewayPortAvailable -GatewayPort $GatewayPort
Write-Output ("Isolation preflight OK: CODEX_HOME={0}; DATA_DIR={1}" -f $codexHome, $dataDir)
