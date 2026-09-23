param(
    [switch]$Dev,
    [string]$DataDir,
    [switch]$FreshData,
    [string]$BootstrapVerifiedDatabase,
    [int]$GatewayPort = 38123,
    [string]$Label,
    [string]$CodexHome,
    [string]$Identifier,
    [string]$ProductName,
    [string]$DeepLinkScheme,
    [string]$WindowTitle
)

# Rebuild the isolated AI Router Test instance from the current worktree.
#
# Why a script: the ordering here is load-bearing and was got wrong once
# already. `pnpm build` must finish *before* `cargo build --release`, because
# Tauri embeds `dist/` at compile time; a cargo build that runs first ships the
# previous frontend. And `cargo clean -p ai-toolbox --release` is what forces the
# relink, because a rebuilt `dist/` alone does not change any Rust input.
#
# TAURI_CONFIG takes the *contents* of the override file, not its path: passing a
# path fails with `expected value at line 1 column 1`.
#
# --features tauri/custom-protocol is mandatory, and its absence is invisible at
# compile time. `tauri-macros-2.5.2/src/context.rs:155` sets
# `dev: cfg!(not(feature = "custom-protocol"))`, and
# `tauri-codegen-2.5.2/src/context.rs:180-184` embeds NO assets when `dev` is set
# and a `devUrl` exists - the binary then loads `http://127.0.0.1:5173` (the Vite
# dev server). A plain `cargo build` therefore produces an exe that shows
# `ERR_CONNECTION_REFUSED`, and it looks identical on disk. The normal
# `pnpm tauri build` workflow passes this feature via the Tauri CLI, which is why
# the hand-rolled cargo builds here (release builds included) shipped a broken UI.

# -Dev builds a debug executable instead: seconds-to-minutes instead of half an
# hour, at the cost of runtime speed and ~2.5x size. Everything else (isolation,
# deploy target, TAURI_CONFIG) is identical, so the two builds are
# interchangeable for a functional smoke test. Use it for "did my fix work"
# rounds and keep the release build for acceptance.

$ErrorActionPreference = 'Stop'

$worktree = 'D:\Codex\2026-09-17\wt-integration'
$testRoot = 'D:\Codex\2026-09-19\ai-router-test'
$confPath = Join-Path $testRoot 'build\tauri.ai-router-test.conf.json'
$targetDir = 'D:\Temp\ai-toolbox-catalog-panel-target'

$safetyScript = Join-Path $PSScriptRoot 'IsolatedInstanceSafety.ps1'
if (-not (Test-Path -LiteralPath $safetyScript)) {
    throw "Safety helper not found: $safetyScript"
}
. $safetyScript

$targetDir = Assert-SafeAbsolutePath -Path $targetDir

# The label drives the on-disk names AND the compiled Tauri identity, so a canary
# build can never overwrite the acceptance build's artifacts and can never collide
# with another instance's single-instance mutex. With no label the historic
# AI Router Test names/identity are reproduced exactly (exe, launcher, config path).
$label = if ([string]::IsNullOrWhiteSpace($Label)) { '' } else { $Label.Trim().ToLowerInvariant() }
$labelPattern = '^[a-z0-9][a-z0-9-]{0,23}$'
if ($label -and $label -notmatch $labelPattern) {
    throw "Refusing isolated build: label must match $labelPattern -> got: $Label"
}
$exeStem = if ($label) { "ai-router-$label" } else { 'ai-router-test' }
$TargetExeName = "$exeStem.exe"
$launcherName = "Start-$exeStem-Detached.cmd"

if ([string]::IsNullOrWhiteSpace($Identifier)) { $Identifier = if ($label) { "com.ai-router-$label" } else { 'com.ai-router-test' } }
if ([string]::IsNullOrWhiteSpace($ProductName)) { $ProductName = if ($label) { "AI Router $($label.Substring(0,1).ToUpperInvariant() + $label.Substring(1))" } else { 'AI Router Test' } }
if ([string]::IsNullOrWhiteSpace($DeepLinkScheme)) { $DeepLinkScheme = if ($label) { "airouter$label" } else { 'airoutertest' } }
if ([string]::IsNullOrWhiteSpace($WindowTitle)) { $WindowTitle = $ProductName }

# A dev build must not evict the acceptance build's artifact, so the debug
# executable is copied out under its own name first. Cargo would keep it either
# way (`ai-toolbox` is the only bin, and a dev build never deletes a release
# output), but a named copy means "the release exe in this target dir" is always
# exactly the release exe, whatever ran before it.
$keptDevExe = Join-Path $targetDir ("debug\{0}-dev.exe" -f $exeStem)

# A labelled build gets its own CODEX_HOME too: two instances sharing one codex-home
# would both rewrite the same config.toml/manifest, so the running instance's managed
# files could change underneath it. Default keeps the historic single test codex-home.
if ([string]::IsNullOrWhiteSpace($CodexHome)) {
    $CodexHome = if ($label) { Join-Path $testRoot "codex-home-$label" } else { Join-Path $testRoot 'codex-home' }
} elseif (-not [System.IO.Path]::IsPathRooted($CodexHome)) {
    $CodexHome = Join-Path $testRoot $CodexHome
}
if (-not (Test-Path -LiteralPath $CodexHome)) {
    New-Item -ItemType Directory -Force -Path $CodexHome | Out-Null
}
$testCodexHome = $CodexHome
if ([string]::IsNullOrWhiteSpace($DataDir)) {
    $DataDir = Join-Path $testRoot 'data'
} elseif (-not [System.IO.Path]::IsPathRooted($DataDir)) {
    $DataDir = Join-Path $testRoot $DataDir
}
if ($FreshData) {
    if (Test-Path -LiteralPath $DataDir) {
        throw "Refusing -FreshData because target already exists (old data is never deleted): $DataDir"
    }
    New-Item -ItemType Directory -Force -Path $DataDir | Out-Null
}
if (-not [string]::IsNullOrWhiteSpace($BootstrapVerifiedDatabase)) {
    if (-not [System.IO.Path]::IsPathRooted($BootstrapVerifiedDatabase)) {
        $BootstrapVerifiedDatabase = Join-Path $testRoot $BootstrapVerifiedDatabase
    }
    $BootstrapVerifiedDatabase = [System.IO.Path]::GetFullPath($BootstrapVerifiedDatabase)
}
Assert-IsolatedInstanceLayout `
    -TestRoot $testRoot `
    -CodexHome $testCodexHome `
    -DataDir $DataDir `
    -BootstrapVerifiedDatabase $BootstrapVerifiedDatabase `
    -GatewayPort $GatewayPort `
    -RequireFreshData:$FreshData
Write-Host ("isolation preflight OK: CODEX_HOME={0}; data={1}" -f $testCodexHome, $DataDir)

# Keep the deploy/start contract explicit. The build itself does not launch the
# app, but it owns the executable that the detached launcher will start; refresh
# that launcher before compiling so it cannot silently retain a production root
# or data directory.
$preflightScript = Join-Path $PSScriptRoot 'Validate-ai-router-test-isolation.ps1'
if (-not (Test-Path -LiteralPath $preflightScript)) {
    throw "Isolation preflight script not found: $preflightScript"
}

# Generate the isolated Tauri config from the base config *here*, so the identity
# that the single-instance mutex is derived from cannot silently stay at the
# production value. Historically the checked-in conf file was passed straight to
# TAURI_CONFIG, and a hand-rolled cargo build shipped an exe carrying
# identifier=com.ai-toolbox - which the running production instance silently
# swallowed (mutex already exists -> process::exit(0), no window, no crash event).
$baseConfPath = Join-Path $worktree 'tauri\tauri.conf.json'
if (-not (Test-Path -LiteralPath $baseConfPath)) {
    throw "Base Tauri config not found: $baseConfPath"
}
New-IsolatedTauriConfig `
    -BaseConfigPath $baseConfPath `
    -OutputPath $confPath `
    -Identifier $Identifier `
    -ProductName $ProductName `
    -DeepLinkScheme $DeepLinkScheme | Out-Null
Write-Host ("isolated Tauri config generated: identifier={0}; product={1}; scheme={2}" -f $Identifier, $ProductName, $DeepLinkScheme)

$launcherPath = Join-Path $testRoot $launcherName
Write-IsolatedLauncher `
    -TestRoot $testRoot `
    -CodexHome $testCodexHome `
    -DataDir $DataDir `
    -ProfileRoot (Join-Path $testRoot ("profile-" + $exeStem)) `
    -TargetExeName $TargetExeName `
    -WindowTitle $WindowTitle `
    -PreflightScript ([System.IO.Path]::GetFullPath($preflightScript)) `
    -BootstrapVerifiedDatabase $BootstrapVerifiedDatabase `
    -GatewayPort $GatewayPort `
    -LauncherPath $launcherPath | Out-Null
Write-Host ("launcher refreshed with isolated CODEX_HOME/DATA_DIR: {0}" -f $launcherPath)
$env:CODEX_HOME = $testCodexHome
$env:AI_TOOLBOX_DATA_DIR = $DataDir

Set-Location -LiteralPath $worktree
$env:CARGO_TARGET_DIR = $targetDir
$env:TAURI_CONFIG = (Get-Content -LiteralPath $confPath -Raw)

$distEntry = Get-Item -LiteralPath (Join-Path $worktree 'dist\index.html')

# Everything the frontend bundle is built from. `-Dev` skips `pnpm build` when
# none of it is newer than `dist/`, which is what makes an iteration that only
# touched Rust take about a minute instead of ten. The release path always
# rebuilds: its output is the thing being accepted, so it never reuses a
# frontend bundle it did not watch being produced.
$frontendInputs = @(
    (Join-Path $worktree 'web'),
    (Join-Path $worktree 'vite.config.ts'),
    (Join-Path $worktree 'package.json')
)
$newestFrontend = $null
foreach ($input in $frontendInputs) {
    if (-not (Test-Path -LiteralPath $input)) { continue }
    if ((Get-Item -LiteralPath $input).PSIsContainer) {
        $candidate = Get-ChildItem -LiteralPath $input -Recurse -File -ErrorAction SilentlyContinue |
            Sort-Object LastWriteTime -Descending | Select-Object -First 1
    } else {
        $candidate = Get-Item -LiteralPath $input
    }
    if ($candidate -and ((-not $newestFrontend) -or ($candidate.LastWriteTime -gt $newestFrontend.LastWriteTime))) {
        $newestFrontend = $candidate
    }
}

$frontendStale = $true
if ($newestFrontend -and ($newestFrontend.LastWriteTime -lt $distEntry.LastWriteTime)) {
    $frontendStale = $false
}

if ($Dev -and (-not $frontendStale)) {
    Write-Host '--- 1/4 frontend build (skipped: dist/ is newer than every frontend input) ---'
    Write-Host ("  dist {0} >= newest input {1} ({2})" -f $distEntry.LastWriteTime, $newestFrontend.LastWriteTime, $newestFrontend.FullName)
} else {
    Write-Host '--- 1/4 frontend build ---'
    pnpm build
    if ($LASTEXITCODE -ne 0) { throw "pnpm build failed ($LASTEXITCODE)" }
    $distEntry = Get-Item -LiteralPath (Join-Path $worktree 'dist\index.html')
    Write-Host ("dist/index.html written {0}" -f $distEntry.LastWriteTime)
}

if ($Dev) {
    # Forcing the rebuild is not optional, and `dist/` alone cannot do it:
    # tauri-codegen does NOT emit `rerun-if-changed` for the frontend directory,
    # so cargo declares the crate fresh and the new frontend is never embedded
    # (`tauri-codegen-2.5.2` has no dist watch; `tauri-build-2.5.3` watches only
    # the config and capabilities). Touching a source file is the cheapest
    # correct trigger: the app crate recompiles, its ~29 GB of dependencies do
    # not, and the release artifacts in the same target dir stay untouched.
    # Ceiling on correctness: the same reasoning means a frontend change in a
    # *release* build is picked up only because the release path always runs
    # `cargo clean -p ai-toolbox` below.
    Write-Host '--- 2/4 force app-crate rebuild (touch src/lib.rs) ---'
    $touch = Join-Path $worktree 'tauri\src\lib.rs'
    (Get-Item -LiteralPath $touch).LastWriteTime = Get-Date
    Write-Host '--- 3/4 debug build ---'
    cargo build --manifest-path tauri\Cargo.toml --features tauri/custom-protocol
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed ($LASTEXITCODE)" }
    Copy-Item -LiteralPath (Join-Path $targetDir 'debug\ai-toolbox.exe') -Destination $keptDevExe -Force
    $built = $keptDevExe
} else {
    Write-Host '--- 2/4 force relink ---'
    cargo clean --manifest-path tauri\Cargo.toml -p ai-toolbox --release

    Write-Host '--- 3/4 release build ---'
    cargo build --release --manifest-path tauri\Cargo.toml --features tauri/custom-protocol
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed ($LASTEXITCODE)" }
    $built = Join-Path $targetDir 'release\ai-toolbox.exe'
}

if (-not (Test-Path -LiteralPath $built)) { throw "missing build output: $built" }

# The only reliable proof that a build is shippable, because a "dev" binary is a
# perfectly valid executable: the frontend used to be embedded as static assets
# under `assets/index-*`, and an empty list means this exe will try to reach the
# Vite dev server instead of showing the UI.
Write-Host '--- verify embedded frontend assets ---'
# Windows PowerShell 5.1 has no `Encoding::Latin1` (that is .NET Core+), and the
# accessor returning $null made the check below a silent no-op on 5.1. Code page
# 28591 *is* Latin-1 and exists on both, so the check now runs everywhere.
$exeText = [System.Text.Encoding]::GetEncoding(28591).GetString([System.IO.File]::ReadAllBytes($built))
$embedded = [regex]::Matches($exeText, 'assets/index-[A-Za-z0-9_-]{4,}') |
    ForEach-Object { $_.Value } | Select-Object -Unique
if (-not $embedded) {
    throw "build has NO embedded frontend assets -> it would try http://127.0.0.1:5173 and fail to load. Missing --features tauri/custom-protocol?"
}
# A stale `dist/` is the other way to ship a broken build: the embedded names
# have to be the ones on disk right now, not the ones from the last frontend
# build. Frontend and binary are compared by name, not by timestamp.
$expected = Get-ChildItem -LiteralPath (Join-Path $worktree 'dist\assets') -File -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -like 'index-*' } |
    ForEach-Object { "assets/index-{0}" -f ($_.Name -replace '^index-','' -replace '\.(js|css)$','') } |
    Select-Object -Unique
$missing = $expected | Where-Object { $_ -notin $embedded }
if ($missing) {
    throw ("embedded frontend is stale: dist/ has {0} but the binary does not. Rebuild the frontend, then relink." -f ($missing -join ', '))
}
Write-Host ("embedded: {0}" -f ($embedded -join ', '))

# Compile-time identity gate: the single-instance mutex and window class are derived
# from the compiled Tauri identifier. If the isolated config did not reach the build,
# this stops the deploy instead of shipping an exe that silently hands its launch to
# the production instance (mutex exists -> process::exit(0), no window, no log).
Write-Host '--- verify compiled instance identity ---'
Assert-IsolatedExecutableIdentity -Path $built -Identifier $Identifier -ProductName $ProductName | Out-Null
Write-Host ("identity OK: identifier={0}; product={1}" -f $Identifier, $ProductName)

Write-Host '--- 4/4 deploy ---'
$target = Join-Path $testRoot $TargetExeName
# Dev snapshots get their own namespace: `prev<N>` is reserved for release
# builds, so "the release to fall back to" is never a debug exe someone left
# behind. `<prefix>1` is the oldest snapshot, `0` is unused on purpose so a
# missing `prev0` never looks like a deleted file.
$prefix = if ($Dev) { "$exeStem.devprev" } else { "$exeStem.prev" }
$backup = Backup-IsolatedExecutable -TargetPath $target -BackupDirectory $testRoot -Prefix $prefix
Assert-IsolatedWriteTarget -Path $target -RequiredRoot $testRoot
Copy-Item -LiteralPath $built -Destination $target -Force

$info = Get-Item -LiteralPath $target
$hash = (Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash
$mode = if ($Dev) { 'DEV' } else { 'RELEASE' }
Write-Host ("DEPLOYED [{0}] {1} bytes {2}" -f $mode, $info.Length, $info.LastWriteTime)
Write-Host ("SOURCE {0}" -f $built)
Write-Host ("SHA256 {0}" -f $hash)
Write-Host ("BUILT_AT {0}" -f (Get-Item -LiteralPath $built).LastWriteTime)
