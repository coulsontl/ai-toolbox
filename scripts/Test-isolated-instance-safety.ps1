$ErrorActionPreference = 'Stop'

$safetyScript = Join-Path $PSScriptRoot 'IsolatedInstanceSafety.ps1'
if (-not (Test-Path -LiteralPath $safetyScript)) {
    throw "Safety helper not found: $safetyScript"
}
. $safetyScript

$root = Join-Path ([System.IO.Path]::GetTempPath()) ("ai-router-isolation-test-" + [guid]::NewGuid().ToString('N'))
$codexHome = Join-Path $root 'codex-home'
$externalRoot = Join-Path $root 'external-codex'
$data = Join-Path $root 'data'
$manifest = Join-Path $data 'proxy-gateway\cli-proxy\codex\manifest.json'
$launcherRoot = Join-Path $root 'launcher'
$profileRoot = Join-Path $root 'profile'
$customExeName = 'canary-router.exe'
$customConfig = Join-Path $root 'build\canary-tauri.json'

function Assert-Throws([scriptblock]$Action, [string]$Name) {
    try {
        & $Action
    } catch {
        Write-Host "PASS $Name"
        return
    }
    throw "Expected rejection did not occur: $Name"
}

try {
    New-Item -ItemType Directory -Force -Path $codexHome, $externalRoot, $data, $launcherRoot | Out-Null

    Assert-Throws {
        Assert-IsolatedWriteTarget -Path 'relative-target' -RequiredRoot $root
    } 'relative target is rejected before write'

    Assert-IsolatedWriteTarget -Path $launcherRoot -RequiredRoot $root
    $launcherPath = Write-IsolatedLauncher `
        -TestRoot $root `
        -CodexHome $codexHome `
        -DataDir $data `
        -ProfileRoot $profileRoot `
        -GatewayPort 38124 `
        -TargetExeName $customExeName `
        -PreflightScript (Join-Path $root 'preflight.ps1')
    $launcherText = Get-Content -LiteralPath $launcherPath -Raw
    if ($launcherText -notmatch 'CODEX_HOME=.*codex-home' -or
        $launcherText -notmatch 'AI_TOOLBOX_DATA_DIR=.*data' -or
        $launcherText -notmatch 'preflight\.ps1') {
        throw 'Generated launcher did not pin the isolated environment and preflight.'
    }
    Write-Host 'PASS generated launcher pins test CODEX_HOME/DATA_DIR and preflight'
    if ($launcherText -notmatch '38124' -or
        $launcherText -match '38123' -or
        $launcherText -notmatch [regex]::Escape($customExeName) -or
        $launcherText -notmatch [regex]::Escape($profileRoot)) {
        throw 'Generated launcher did not pin canary port/profile/executable.'
    }
    Write-Host 'PASS generated launcher pins canary port/profile/executable'

    $preflight = Join-Path $root 'preflight.ps1'
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'Validate-ai-router-test-isolation.ps1') -Destination $preflight
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'IsolatedInstanceSafety.ps1') -Destination (Join-Path $root 'IsolatedInstanceSafety.ps1')
    $preflightOutput = & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $preflight `
        -TestRoot $root `
        -CodexHome $codexHome `
        -DataDir $data `
        -GatewayPort 38124 2>&1
    if ($LASTEXITCODE -ne 0 -or ($preflightOutput -notmatch 'Isolation preflight OK')) {
        throw "Canary preflight did not accept GatewayPort 38124: $($preflightOutput -join "`n")"
    }
    Write-Host 'PASS preflight accepts canary GatewayPort 38124'

    $missingTarget = Join-Path $root 'missing.exe'
    $backupResult = Backup-IsolatedExecutable -Target $missingTarget -BackupDirectory $root -Prefix 'test.prev'
    if ($null -ne $backupResult) {
        throw 'Missing executable unexpectedly produced a backup path'
    }
    Write-Host 'PASS first deploy skips backup when old executable is absent'

    Assert-Throws {
        Remove-IsolatedTemporaryTree -TemporaryRoot $root -TargetPath (Split-Path -Parent $root)
    } 'temporary cleanup rejects target outside temporary root'

    Assert-Throws {
        Assert-IsolatedCodexRuntimeTarget -CodexHome $codexHome -RuntimeRoot $externalRoot
    } 'external runtime root is rejected'

    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $manifest) | Out-Null
    @{
        files = @(
            @{
                path = (Join-Path $externalRoot 'config.toml')
            }
        )
    } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $manifest -Encoding UTF8
    Assert-Throws {
        Assert-IsolatedManifestRuntimePaths -ManifestPath $manifest -CodexHome $codexHome
    } 'manifest file outside test codex-home is rejected'

    Remove-Item -LiteralPath $manifest -Force
    New-Item -ItemType File -Force -Path (Join-Path $data 'ai-toolbox.db') | Out-Null
    Assert-Throws {
        Assert-IsolatedInstanceLayout -TestRoot $root -CodexHome $codexHome -DataDir $data -GatewayPort 38124
    } 'existing database without a verifiable manifest is rejected'

    $bootstrapMarker = Join-Path $data 'bootstrap-isolation.json'
    Assert-Throws {
        Assert-IsolatedInstanceLayout `
            -TestRoot $root `
            -CodexHome $codexHome `
            -DataDir $data `
            -BootstrapVerifiedDatabase $bootstrapMarker `
            -GatewayPort 38124
    } 'bootstrap marker is rejected when missing'

    $dbPath = Join-Path $data 'ai-toolbox.db'
    $dbHash = (Get-FileHash -LiteralPath $dbPath -Algorithm SHA256).Hash
    @{
        schema_version = 1
        purpose = 'ai-router-test-database-bootstrap'
        database_path = $dbPath
        database_sha256 = $dbHash
        codex_home = $codexHome
        codex_root_dir = $codexHome
        data_dir = $data
        gateway_port = 38124
    } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $bootstrapMarker -Encoding UTF8

    $tamperedMarker = Join-Path $data 'bootstrap-tampered.json'
    @{
        schema_version = 1
        purpose = 'ai-router-test-database-bootstrap'
        database_path = $dbPath
        database_sha256 = ('0' * 64)
        codex_home = $codexHome
        codex_root_dir = $codexHome
        data_dir = $data
        gateway_port = 38124
    } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $tamperedMarker -Encoding UTF8
    Assert-Throws {
        Assert-IsolatedInstanceLayout `
            -TestRoot $root `
            -CodexHome $codexHome `
            -DataDir $data `
            -BootstrapVerifiedDatabase $tamperedMarker `
            -GatewayPort 38124
    } 'tampered database hash is rejected'

    $wrongRootMarker = Join-Path $data 'bootstrap-wrong-root.json'
    @{
        schema_version = 1
        purpose = 'ai-router-test-database-bootstrap'
        database_path = $dbPath
        database_sha256 = $dbHash
        codex_home = $codexHome
        codex_root_dir = (Join-Path $root 'wrong-codex-home')
        data_dir = $data
        gateway_port = 38124
    } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $wrongRootMarker -Encoding UTF8
    Assert-Throws {
        Assert-IsolatedInstanceLayout `
            -TestRoot $root `
            -CodexHome $codexHome `
            -DataDir $data `
            -BootstrapVerifiedDatabase $wrongRootMarker `
            -GatewayPort 38124
    } 'bootstrap marker with wrong root is rejected'

    Assert-IsolatedInstanceLayout `
        -TestRoot $root `
        -CodexHome $codexHome `
        -DataDir $data `
        -BootstrapVerifiedDatabase $bootstrapMarker `
        -GatewayPort 38124
    Write-Host 'PASS correctly bound bootstrap marker is accepted'

    $generatedMarker = Join-Path $data 'bootstrap-generated.json'
    New-IsolatedBootstrapMarker `
        -MarkerPath $generatedMarker `
        -TestRoot $root `
        -CodexHome $codexHome `
        -DataDir $data `
        -DatabasePath $dbPath `
        -GatewayPort 38124
    Assert-IsolatedBootstrapMarker `
        -MarkerPath $generatedMarker `
        -TestRoot $root `
        -CodexHome $codexHome `
        -DataDir $data `
        -GatewayPort 38124
    Write-Host 'PASS bootstrap marker helper binds DB hash and paths'

    $baseConfig = Join-Path $root 'base-tauri.json'
    @{
        productName = 'Base'
        identifier = 'com.example.base'
        bundle = @{ active = $true; createUpdaterArtifacts = $true }
    } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $baseConfig -Encoding UTF8
    New-IsolatedTauriConfig `
        -BaseConfigPath $baseConfig `
        -OutputPath $customConfig `
        -Identifier 'com.example.canary' `
        -ProductName 'Canary' `
        -DeepLinkScheme 'airoutercanary'
    $config = Get-Content -LiteralPath $customConfig -Raw | ConvertFrom-Json
    if ($config.identifier -ne 'com.example.canary' -or
        $config.productName -ne 'Canary' -or
        $config.bundle.active -ne $false) {
        throw 'Canary Tauri config was not isolated.'
    }
    Write-Host 'PASS canary Tauri identifier enters generated config'

    # The deep-link plugin calls register_all() on every Windows startup, which rewrites
    # HKCU\Software\Classes\<scheme> to point at the running exe. An isolated build that
    # keeps the production `aitoolbox` scheme would steal the installed app's handler.
    if (@($config.plugins.'deep-link'.desktop.schemes) -notcontains 'airoutercanary') {
        throw "Canary deep-link scheme did not enter the generated config: $($config.plugins.'deep-link'.desktop.schemes -join ',')"
    }
    Write-Host 'PASS canary deep-link scheme replaces the production scheme'

    Assert-Throws {
        New-IsolatedTauriConfig `
            -BaseConfigPath $baseConfig `
            -OutputPath (Join-Path $root 'build\bad-scheme.json') `
            -Identifier 'com.example.canary' `
            -ProductName 'Canary' `
            -DeepLinkScheme 'Not A Scheme'
    } 'invalid deep-link scheme is rejected'

    # The single-instance mutex is derived from the compiled identifier, so shipping the
    # production identifier silently hands every launch to the production instance.
    $fakeExe = Join-Path $root 'fake-instance.exe'
    # 28591 *is* Latin-1 and exists on Windows PowerShell 5.1, unlike Encoding::Latin1.
    $latin1 = [System.Text.Encoding]::GetEncoding(28591)
    [System.IO.File]::WriteAllBytes($fakeExe, $latin1.GetBytes(
        "identifier=com.example.other product=Other"))
    Assert-Throws {
        Assert-IsolatedExecutableIdentity -Path $fakeExe -Identifier 'com.example.canary' -ProductName 'Canary'
    } 'built executable without the isolated identifier is rejected'
    Assert-Throws {
        Assert-IsolatedExecutableIdentity -Path $fakeExe -Identifier 'com.ai-toolbox' -ProductName 'Other'
    } 'production identifier is rejected as an isolated identity'
    [System.IO.File]::WriteAllBytes($fakeExe, $latin1.GetBytes(
        "identifier=com.example.canary product=Canary single-instance"))
    if (-not (Assert-IsolatedExecutableIdentity -Path $fakeExe -Identifier 'com.example.canary' -ProductName 'Canary')) {
        throw 'Matching isolated identity was not accepted.'
    }
    Write-Host 'PASS compiled instance identity is asserted before deploy'

    $fresh = New-IsolatedDataDirectory -TestRoot $root
    Assert-IsolatedInstanceLayout -TestRoot $root -CodexHome $codexHome -DataDir $fresh -GatewayPort 38124 -RequireFreshData
    if (Test-Path -LiteralPath (Join-Path $data 'ai-toolbox.db')) {
        Write-Host 'PASS old data was not deleted'
    } else {
        throw 'Existing data was unexpectedly removed'
    }

    Write-Host 'PASS fresh data directory is accepted without a database'
} finally {
    if (Test-Path -LiteralPath $root) {
        Remove-IsolatedTemporaryTree -TemporaryRoot $root -TargetPath $root
    }
}
