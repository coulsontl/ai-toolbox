function Convert-ToNormalizedPath {
    param([Parameter(Mandatory)][string]$Path)
    $full = [System.IO.Path]::GetFullPath($Path)
    $full.TrimEnd('\', '/')
}

function Get-Sha256Hash {
    param([Parameter(Mandatory)][string]$Path)
    # Windows PowerShell 5.1 loses module auto-loading for Microsoft.PowerShell.Utility
    # when the process inherits PowerShell 7 module paths from the Codex runtime, and
    # `Get-FileHash` then disappears ("The term 'Get-FileHash' is not recognized").
    # The launcher preflight runs in exactly that environment, so hash through .NET.
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Cannot hash missing file: $Path"
    }
    $sha = [System.Security.Cryptography.SHA256]::Create()
    try {
        $stream = [System.IO.File]::OpenRead($Path)
        try {
            ($sha.ComputeHash($stream) | ForEach-Object { $_.ToString('X2') }) -join ''
        } finally {
            $stream.Dispose()
        }
    } finally {
        $sha.Dispose()
    }
}

function Assert-IsolatedWriteTarget {
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)][string]$RequiredRoot
    )
    if (-not [System.IO.Path]::IsPathRooted($Path)) {
        throw "Refusing isolated write: target must be an absolute path: $Path"
    }
    if (-not [System.IO.Path]::IsPathRooted($RequiredRoot)) {
        throw "Refusing isolated write: required root must be an absolute path: $RequiredRoot"
    }
    Assert-NoReparsePointOnExistingPath -Path $Path
    Assert-NoReparsePointOnExistingPath -Path $RequiredRoot
    if (-not (Test-PathWithinRoot -Path $Path -Root $RequiredRoot)) {
        throw "Refusing isolated write: '$Path' is outside required root '$RequiredRoot'."
    }
}

function Assert-SafeAbsolutePath {
    param([Parameter(Mandatory)][string]$Path)
    if (-not [System.IO.Path]::IsPathRooted($Path)) {
        throw "Refusing write: path must be absolute: $Path"
    }
    Assert-NoReparsePointOnExistingPath -Path $Path
    Convert-ToNormalizedPath $Path
}

function Assert-NoReparsePointOnExistingPath {
    param([Parameter(Mandatory)][string]$Path)
    $current = Convert-ToNormalizedPath $Path
    while ($current) {
        if (Test-Path -LiteralPath $current) {
            $item = Get-Item -LiteralPath $current -Force
            if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "Refusing isolated instance: reparse point in runtime path '$current'."
            }
        }
        $parent = Split-Path -Parent $current
        if (-not $parent -or $parent -eq $current) {
            break
        }
        $current = $parent
    }
}

function Test-PathWithinRoot {
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)][string]$Root
    )
    Assert-NoReparsePointOnExistingPath -Path $Path
    Assert-NoReparsePointOnExistingPath -Path $Root
    $pathFull = Convert-ToNormalizedPath $Path
    $rootFull = Convert-ToNormalizedPath $Root
    $comparison = [System.StringComparison]::OrdinalIgnoreCase
    $pathFull.Equals($rootFull, $comparison) -or
        $pathFull.StartsWith($rootFull + '\', $comparison)
}

function Assert-IsolatedCodexRuntimeTarget {
    param(
        [Parameter(Mandatory)][string]$CodexHome,
        [Parameter(Mandatory)][string]$RuntimeRoot
    )
    if (-not (Test-PathWithinRoot -Path $RuntimeRoot -Root $CodexHome)) {
        throw "Refusing isolated instance: Codex runtime root '$RuntimeRoot' is outside test CODEX_HOME '$CodexHome'."
    }
}

function Assert-IsolatedManifestRuntimePaths {
    param(
        [Parameter(Mandatory)][string]$ManifestPath,
        [Parameter(Mandatory)][string]$CodexHome
    )
    if (-not (Test-Path -LiteralPath $ManifestPath)) {
        throw "Manifest does not exist: $ManifestPath"
    }
    try {
        $manifest = Get-Content -LiteralPath $ManifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
    } catch {
        throw "Refusing isolated instance: manifest is not valid JSON: $ManifestPath"
    }
    foreach ($file in @($manifest.files)) {
        if ($null -eq $file.path -or [string]::IsNullOrWhiteSpace([string]$file.path)) {
            throw "Refusing isolated instance: manifest file entry has no path."
        }
        $filePath = [System.IO.Path]::GetFullPath([string]$file.path)
        $runtimeRoot = Split-Path -Parent $filePath
        Assert-IsolatedCodexRuntimeTarget -CodexHome $CodexHome -RuntimeRoot $runtimeRoot
    }
    if (@($manifest.files).Count -eq 0) {
        throw "Refusing isolated instance: manifest has no managed runtime files: $ManifestPath"
    }
}

function Assert-IsolatedBootstrapMarker {
    param(
        [Parameter(Mandatory)][string]$MarkerPath,
        [Parameter(Mandatory)][string]$TestRoot,
        [Parameter(Mandatory)][string]$CodexHome,
        [Parameter(Mandatory)][string]$DataDir,
        [int]$GatewayPort = 38123
    )
    Assert-IsolatedWriteTarget -Path $MarkerPath -RequiredRoot $DataDir
    if (-not (Test-Path -LiteralPath $MarkerPath)) {
        throw "Refusing isolated bootstrap: marker does not exist: $MarkerPath"
    }
    try {
        $marker = Get-Content -LiteralPath $MarkerPath -Raw -Encoding UTF8 | ConvertFrom-Json
    } catch {
        throw "Refusing isolated bootstrap: marker is not valid JSON: $MarkerPath"
    }
    if ($marker.schema_version -ne 1 -or
        $marker.purpose -ne 'ai-router-test-database-bootstrap') {
        throw "Refusing isolated bootstrap: unsupported marker schema or purpose: $MarkerPath"
    }
    if ($marker.gateway_port -ne $GatewayPort) {
        throw "Refusing isolated bootstrap: marker gateway port does not match requested port $GatewayPort."
    }

    $dbPath = [System.IO.Path]::GetFullPath([string]$marker.database_path)
    $expectedDb = [System.IO.Path]::GetFullPath((Join-Path $DataDir 'ai-toolbox.db'))
    if (-not $dbPath.Equals($expectedDb, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing isolated bootstrap: marker database path is not the selected test database."
    }
    Assert-IsolatedWriteTarget -Path $dbPath -RequiredRoot $TestRoot
    if (-not (Test-Path -LiteralPath $dbPath)) {
        throw "Refusing isolated bootstrap: database does not exist: $dbPath"
    }
    $actualHash = Get-Sha256Hash -Path $dbPath
    if (-not $actualHash.Equals([string]$marker.database_sha256, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing isolated bootstrap: database SHA256 does not match marker."
    }

    $markerCodexHome = [System.IO.Path]::GetFullPath([string]$marker.codex_home)
    $markerRootDir = [System.IO.Path]::GetFullPath([string]$marker.codex_root_dir)
    $expectedCodexHome = [System.IO.Path]::GetFullPath($CodexHome)
    if (-not $markerCodexHome.Equals($expectedCodexHome, [System.StringComparison]::OrdinalIgnoreCase) -or
        -not $markerRootDir.Equals($expectedCodexHome, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing isolated bootstrap: marker Codex root is not exactly test CODEX_HOME."
    }
    if (-not ([System.IO.Path]::GetFullPath([string]$marker.data_dir)).Equals(
            [System.IO.Path]::GetFullPath($DataDir), [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing isolated bootstrap: marker data directory is not the selected test data directory."
    }
    Assert-IsolatedCodexRuntimeTarget -CodexHome $CodexHome -RuntimeRoot $markerRootDir
}

function Assert-IsolatedGatewayPortAvailable {
    param(
        [int]$GatewayPort = 38123
    )
    if ($GatewayPort -lt 1 -or $GatewayPort -gt 65535) {
        throw "Refusing isolated instance: invalid gateway port: $GatewayPort"
    }
    $occupied = Get-NetTCPConnection -State Listen -LocalPort $GatewayPort -ErrorAction SilentlyContinue
    if ($occupied) {
        $owner = Get-Process -Id $occupied[0].OwningProcess -ErrorAction SilentlyContinue
        $ownerName = if ($owner) { $owner.ProcessName } else { "PID $($occupied[0].OwningProcess)" }
        throw "Refusing isolated instance: gateway port $GatewayPort is already used by $ownerName."
    }
}

function New-IsolatedBootstrapMarker {
    param(
        [Parameter(Mandatory)][string]$MarkerPath,
        [Parameter(Mandatory)][string]$TestRoot,
        [Parameter(Mandatory)][string]$CodexHome,
        [Parameter(Mandatory)][string]$DataDir,
        [Parameter(Mandatory)][string]$DatabasePath,
        [int]$GatewayPort = 38123
    )
    Assert-IsolatedWriteTarget -Path $MarkerPath -RequiredRoot $DataDir
    Assert-IsolatedWriteTarget -Path $DatabasePath -RequiredRoot $TestRoot
    if (-not (Test-Path -LiteralPath $DatabasePath)) {
        throw "Cannot create bootstrap marker: database does not exist: $DatabasePath"
    }
    Assert-IsolatedCodexRuntimeTarget -CodexHome $CodexHome -RuntimeRoot $CodexHome
    $marker = [ordered]@{
        schema_version = 1
        purpose = 'ai-router-test-database-bootstrap'
        database_path = [System.IO.Path]::GetFullPath($DatabasePath)
        database_sha256 = Get-Sha256Hash -Path $DatabasePath
        codex_home = [System.IO.Path]::GetFullPath($CodexHome)
        codex_root_dir = [System.IO.Path]::GetFullPath($CodexHome)
        data_dir = [System.IO.Path]::GetFullPath($DataDir)
        gateway_port = $GatewayPort
    }
    $marker | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $MarkerPath -Encoding UTF8
    $MarkerPath
}

function New-IsolatedTauriConfig {
    param(
        [Parameter(Mandatory)][string]$BaseConfigPath,
        [Parameter(Mandatory)][string]$OutputPath,
        [Parameter(Mandatory)][string]$Identifier,
        [Parameter(Mandatory)][string]$ProductName,
        [string]$DeepLinkScheme
    )
    if (-not [System.IO.Path]::IsPathRooted($BaseConfigPath) -or
        -not [System.IO.Path]::IsPathRooted($OutputPath)) {
        throw 'Refusing Tauri config generation: paths must be absolute.'
    }
    $base = Get-Content -LiteralPath $BaseConfigPath -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($Identifier -notmatch '^com\.[A-Za-z0-9][A-Za-z0-9.-]*$') {
        throw "Refusing Tauri config generation: invalid identifier: $Identifier"
    }
    $base.identifier = $Identifier
    $base.productName = $ProductName
    # The deep-link plugin calls `register_all()` on Windows at every startup, which
    # rewrites HKCU\Software\Classes\<scheme> to point at the running executable. An
    # isolated instance that keeps the production `aitoolbox` scheme would hijack the
    # installed app's handler, so isolated instances get their own scheme.
    if (-not [string]::IsNullOrWhiteSpace($DeepLinkScheme)) {
        if ($DeepLinkScheme -notmatch '^[a-z][a-z0-9-]{2,30}$') {
            throw "Refusing Tauri config generation: invalid deep-link scheme: $DeepLinkScheme"
        }
        if ($null -eq $base.plugins) {
            $base | Add-Member -NotePropertyName plugins -NotePropertyValue ([pscustomobject]@{})
        }
        $deepLink = $base.plugins.'deep-link'
        if ($null -eq $deepLink) {
            $deepLink = [pscustomobject]@{}
            $base.plugins | Add-Member -NotePropertyName 'deep-link' -NotePropertyValue $deepLink
        }
        if ($null -eq $deepLink.desktop) {
            $deepLink | Add-Member -NotePropertyName desktop -NotePropertyValue ([pscustomobject]@{})
        }
        $deepLink.desktop | Add-Member -NotePropertyName schemes -NotePropertyValue @($DeepLinkScheme) -Force
    }
    if ($null -eq $base.bundle) {
        $base | Add-Member -NotePropertyName bundle -NotePropertyValue ([pscustomobject]@{})
    }
    $base.bundle.active = $false
    $base.bundle.createUpdaterArtifacts = $false
    $parent = Split-Path -Parent $OutputPath
    New-Item -ItemType Directory -Force -Path $parent | Out-Null
    $base | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath $OutputPath -Encoding UTF8
    $OutputPath
}

function New-IsolatedDataDirectory {
    param([Parameter(Mandatory)][string]$TestRoot)
    $dataRoot = Join-Path $TestRoot 'data-fresh'
    if (Test-Path -LiteralPath $dataRoot) {
        throw "Fresh data directory already exists: $dataRoot"
    }
    New-Item -ItemType Directory -Force -Path $dataRoot | Out-Null
    $dataRoot
}

function Write-IsolatedLauncher {
    param(
        [Parameter(Mandatory)][string]$TestRoot,
        [Parameter(Mandatory)][string]$CodexHome,
        [Parameter(Mandatory)][string]$DataDir,
        [string]$ProfileRoot,
        [int]$GatewayPort = 38123,
        [string]$TargetExeName = 'ai-router-test.exe',
        [string]$WindowTitle = 'AI Router Test',
        [Parameter(Mandatory)][string]$PreflightScript,
        [string]$LauncherPath,
        [string]$BootstrapVerifiedDatabase
    )
    if ([string]::IsNullOrWhiteSpace($LauncherPath)) {
        $LauncherPath = Join-Path $TestRoot 'Start-AI-Router-Test-Detached.cmd'
    }
    Assert-IsolatedWriteTarget -Path $LauncherPath -RequiredRoot $TestRoot
    Assert-IsolatedWriteTarget -Path $CodexHome -RequiredRoot $TestRoot
    Assert-IsolatedWriteTarget -Path $DataDir -RequiredRoot $TestRoot
    if ([string]::IsNullOrWhiteSpace($ProfileRoot)) {
        $ProfileRoot = Join-Path $TestRoot 'profile'
    }
    Assert-IsolatedWriteTarget -Path $ProfileRoot -RequiredRoot $TestRoot
    # The launcher preflight is a PowerShell 5.1 child process whose APPDATA/LOCALAPPDATA
    # point into this profile; both roots must exist before it starts, or 5.1 cannot
    # initialise and module commands fail (seen as "Get-FileHash is not recognized").
    New-Item -ItemType Directory -Force -Path (Join-Path $ProfileRoot 'Roaming'), (Join-Path $ProfileRoot 'Local') | Out-Null
    Assert-IsolatedGatewayPortAvailable -GatewayPort $GatewayPort
    if ([string]::IsNullOrWhiteSpace($TargetExeName) -or $TargetExeName -notmatch '^[A-Za-z0-9_.-]+\.exe$') {
        throw "Refusing isolated launcher: invalid executable name: $TargetExeName"
    }
    if ([string]::IsNullOrWhiteSpace($WindowTitle) -or $WindowTitle.Contains('"')) {
        throw "Refusing isolated launcher: invalid window title: $WindowTitle"
    }
    if (-not [System.IO.Path]::IsPathRooted($PreflightScript)) {
        throw "Refusing isolated launcher: preflight script must be absolute: $PreflightScript"
    }
    $bootstrapArg = ''
    if (-not [string]::IsNullOrWhiteSpace($BootstrapVerifiedDatabase)) {
        Assert-IsolatedBootstrapMarker `
            -MarkerPath $BootstrapVerifiedDatabase `
            -TestRoot $TestRoot `
            -CodexHome $CodexHome `
            -DataDir $DataDir `
            -GatewayPort $GatewayPort
        $bootstrapArg = (' -BootstrapVerifiedDatabase "{0}"' -f $BootstrapVerifiedDatabase)
    }
    $launcherDir = Split-Path -Parent $LauncherPath
    New-Item -ItemType Directory -Force -Path $launcherDir | Out-Null
    $content = @(
        '@echo off',
        'rem Generated isolated launcher. Do not remove the preflight.',
        # Pin module discovery to the in-box 5.1 paths. Without this the child 5.1
        # powershell.exe inherits PSModulePath entries pointing at the PowerShell 7
        # runtime (Codex's bundled pwsh), which breaks module auto-loading for
        # in-box modules such as Microsoft.PowerShell.Utility.
        'set "PSModulePath=%SystemRoot%\system32\WindowsPowerShell\v1.0\Modules"',
        ('set "APPDATA={0}\Roaming"' -f $ProfileRoot.TrimEnd('\')),
        ('set "LOCALAPPDATA={0}\Local"' -f $ProfileRoot.TrimEnd('\')),
        ('set "CODEX_HOME={0}"' -f $CodexHome),
        ('set "AI_TOOLBOX_DATA_DIR={0}"' -f $DataDir),
        ('set "AI_TOOLBOX_WINDOW_TITLE={0}"' -f $WindowTitle),
        ('set "AI_TOOLBOX_GATEWAY_PORT={0}"' -f $GatewayPort),
        ('cd /d "{0}"' -f $TestRoot),
        ('powershell.exe -NoProfile -ExecutionPolicy Bypass -File "{0}" -TestRoot "{1}" -CodexHome "{2}" -DataDir "{3}" -GatewayPort {4}{5}' -f $PreflightScript, $TestRoot, $CodexHome, $DataDir, $GatewayPort, $bootstrapArg),
        'if errorlevel 1 exit /b 1',
        ('start "" "{0}\{1}"' -f $TestRoot, $TargetExeName)
    )
    Set-Content -LiteralPath $LauncherPath -Value $content -Encoding ASCII
    $LauncherPath
}

function Assert-IsolatedExecutableIdentity {
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)][string]$Identifier,
        [Parameter(Mandatory)][string]$ProductName
    )
    # `tauri-plugin-single-instance` derives its mutex/window names from the compiled
    # Tauri identifier (`{identifier}-sim` / `-sic` / `-siw`), so an isolated build that
    # is compiled with the production identifier silently forwards its launch to the
    # production instance and exits 0 - it never listens, never writes its own profile,
    # and leaves no crash event. That failure is invisible without this check.
    if (-not (Test-Path -LiteralPath $Path)) {
        throw "Refusing identity check: executable not found: $Path"
    }
    if ($Identifier -eq 'com.ai-toolbox') {
        throw "Refusing identity check: '$Identifier' is the production Tauri identifier (single-instance mutex collision)."
    }
    # Latin-1 (code page 28591) exists on both Windows PowerShell 5.1 and .NET Core, and
    # keeps every byte addressable; `Encoding::Latin1` does not exist on 5.1.
    $text = [System.Text.Encoding]::GetEncoding(28591).GetString([System.IO.File]::ReadAllBytes($Path))
    if ($text.IndexOf($Identifier, [System.StringComparison]::Ordinal) -lt 0) {
        throw "Built executable does not embed Tauri identifier '$Identifier' -> it would collide with another instance's single-instance mutex. Rebuild with the generated isolated config."
    }
    if ($text.IndexOf($ProductName, [System.StringComparison]::Ordinal) -lt 0) {
        throw "Built executable does not embed product name '$ProductName' -> the isolated config was not applied. Rebuild with the generated isolated config."
    }
    $true
}

function Backup-IsolatedExecutable {
    param(
        [Parameter(Mandatory)][string]$TargetPath,
        [Parameter(Mandatory)][string]$BackupDirectory,
        [Parameter(Mandatory)][string]$Prefix
    )
    Assert-IsolatedWriteTarget -Path $BackupDirectory -RequiredRoot $BackupDirectory
    Assert-IsolatedWriteTarget -Path $TargetPath -RequiredRoot $BackupDirectory
    if (-not (Test-Path -LiteralPath $TargetPath)) {
        Write-Host ("backup skipped: target executable does not exist ({0})" -f $TargetPath)
        return $null
    }
    $index = 1
    do {
        $backup = Join-Path $BackupDirectory ("{0}{1}.exe" -f $Prefix, $index)
        $index += 1
    } while (Test-Path -LiteralPath $backup)
    Copy-Item -LiteralPath $TargetPath -Destination $backup
    Write-Host ("backup {0} <- {1}" -f (Split-Path -Leaf $backup), (Split-Path -Leaf $TargetPath))
    $backup
}

function Remove-IsolatedTemporaryTree {
    param(
        [Parameter(Mandatory)][string]$TemporaryRoot,
        [Parameter(Mandatory)][string]$TargetPath
    )
    # Inspect the lexical paths before resolving them: resolving a junction/symlink
    # first would hide the reparse point and could move cleanup outside the temp
    # root.
    Assert-NoReparsePointOnExistingPath -Path $TemporaryRoot
    Assert-NoReparsePointOnExistingPath -Path $TargetPath
    $resolvedRootItem = Resolve-Path -LiteralPath $TemporaryRoot -ErrorAction Stop
    $resolvedTargetItem = Resolve-Path -LiteralPath $TargetPath -ErrorAction Stop
    $resolvedRoot = Convert-ToNormalizedPath $resolvedRootItem.Path
    $resolvedTarget = Convert-ToNormalizedPath $resolvedTargetItem.Path
    Assert-NoReparsePointOnExistingPath -Path $resolvedRoot
    Assert-NoReparsePointOnExistingPath -Path $resolvedTarget
    if (-not (Test-PathWithinRoot -Path $resolvedTarget -Root $resolvedRoot)) {
        throw "Refusing temporary cleanup outside root: '$resolvedTarget' not under '$resolvedRoot'."
    }
    if (Test-Path -LiteralPath $resolvedTarget) {
        Remove-Item -LiteralPath $resolvedTarget -Recurse -Force
    }
}

function Assert-IsolatedInstance {
    param(
        [Parameter(Mandatory)][string]$TestRoot,
        [Parameter(Mandatory)][string]$CodexHome,
        [Parameter(Mandatory)][string]$DataDir,
        [string]$BootstrapVerifiedDatabase,
        [switch]$RequireFreshData,
        [int]$GatewayPort = 38123
    )
    Assert-IsolatedGatewayPortAvailable -GatewayPort $GatewayPort
    Assert-IsolatedCodexRuntimeTarget -CodexHome $CodexHome -RuntimeRoot $CodexHome
    if (-not (Test-PathWithinRoot -Path $CodexHome -Root $TestRoot)) {
        throw "Refusing isolated instance: test CODEX_HOME '$CodexHome' is outside test root '$TestRoot'."
    }
    if (-not (Test-PathWithinRoot -Path $DataDir -Root $TestRoot)) {
        throw "Refusing isolated instance: data directory '$DataDir' is outside test root '$TestRoot'."
    }
    if ($RequireFreshData -and (Test-Path -LiteralPath (Join-Path $DataDir 'ai-toolbox.db'))) {
        throw "Refusing isolated instance: fresh data directory already contains ai-toolbox.db: $DataDir"
    }
    $manifest = Join-Path $DataDir 'proxy-gateway\cli-proxy\codex\manifest.json'
    if (Test-Path -LiteralPath $manifest) {
        if (-not [string]::IsNullOrWhiteSpace($BootstrapVerifiedDatabase)) {
            throw "Refusing isolated bootstrap: manifest already exists; bootstrap is only allowed before first startup."
        }
        Assert-IsolatedManifestRuntimePaths -ManifestPath $manifest -CodexHome $CodexHome
    } elseif (Test-Path -LiteralPath (Join-Path $DataDir 'ai-toolbox.db')) {
        if ($RequireFreshData) {
            throw "Refusing isolated instance: fresh data directory already contains a database: $DataDir"
        }
        if ([string]::IsNullOrWhiteSpace($BootstrapVerifiedDatabase)) {
            throw "Refusing isolated instance: existing database has no manifest or explicit bootstrap marker: $DataDir"
        }
        Assert-IsolatedBootstrapMarker `
            -MarkerPath $BootstrapVerifiedDatabase `
            -TestRoot $TestRoot `
            -CodexHome $CodexHome `
            -DataDir $DataDir `
            -GatewayPort $GatewayPort
    } elseif (-not [string]::IsNullOrWhiteSpace($BootstrapVerifiedDatabase)) {
        throw "Refusing isolated bootstrap: marker requires an existing database."
    }
}

function Assert-IsolatedInstanceLayout {
    param(
        [Parameter(Mandatory)][string]$TestRoot,
        [Parameter(Mandatory)][string]$CodexHome,
        [Parameter(Mandatory)][string]$DataDir,
        [string]$BootstrapVerifiedDatabase,
        [switch]$RequireFreshData,
        [int]$GatewayPort = 38123
    )
    if (-not (Test-PathWithinRoot -Path $CodexHome -Root $TestRoot)) {
        throw "Refusing isolated instance: test CODEX_HOME '$CodexHome' is outside test root '$TestRoot'."
    }
    if (-not (Test-PathWithinRoot -Path $DataDir -Root $TestRoot)) {
        throw "Refusing isolated instance: data directory '$DataDir' is outside test root '$TestRoot'."
    }
    Assert-IsolatedInstance `
        -TestRoot $TestRoot `
        -CodexHome $CodexHome `
        -DataDir $DataDir `
        -BootstrapVerifiedDatabase $BootstrapVerifiedDatabase `
        -GatewayPort $GatewayPort `
        -RequireFreshData:$RequireFreshData
}
