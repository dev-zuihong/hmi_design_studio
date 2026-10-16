param([switch]$SkipWebview, [switch]$SkipPython, [switch]$SkipInstall, [switch]$Debug)
$ErrorActionPreference = 'Stop'
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Push-Location $workspace
try {
    if ($env:PROCESSOR_ARCHITECTURE -ne 'AMD64') { throw 'Windows x64 build host required.' }
    if (-not $SkipInstall) {
        & npm.cmd ci --no-audit --no-fund
        if ($LASTEXITCODE -ne 0) { throw 'npm ci failed' }
    }
    & npm.cmd run build
    if ($LASTEXITCODE -ne 0) { throw 'Frontend build failed' }
    if (-not $SkipPython) { & (Join-Path $PSScriptRoot 'prepare-python.ps1') }
    if (-not $SkipWebview) { & (Join-Path $PSScriptRoot 'prepare-webview.ps1') }
    $cargoArgs = @('build', '--workspace', '--target', 'x86_64-pc-windows-msvc', '--features', 'hmi-studio/custom-protocol,hmi-runner/custom-protocol')
    $profile = 'debug'
    if (-not $Debug) { $cargoArgs += '--release'; $profile = 'release' }
    & cargo @cargoArgs
    if ($LASTEXITCODE -ne 0) { throw 'Rust build failed' }
    $binaries = Join-Path $workspace "target/x86_64-pc-windows-msvc/$profile"
    $template = Join-Path $workspace '.cache/templates/windows-x64'
    $plugin = Join-Path $workspace '.cache/plugins/demo-temperature'
    New-Item -ItemType Directory -Force -Path $template, $plugin | Out-Null
    Copy-Item -LiteralPath (Join-Path $binaries 'hmi-runner.exe') -Destination (Join-Path $template 'HmiRunner.exe') -Force
    Copy-Item -LiteralPath (Join-Path $binaries 'demo-protocol.exe') -Destination (Join-Path $plugin 'demo-protocol.exe') -Force
    Copy-Item -LiteralPath (Join-Path $workspace 'crates/demo-protocol/package/manifest.json') -Destination (Join-Path $plugin 'manifest.json') -Force
    $distribution = Join-Path $workspace ("artifacts/HmiStudio-win-x64-" + (Get-Date -Format 'yyyyMMdd-HHmmss'))
    New-Item -ItemType Directory -Path $distribution | Out-Null
    Copy-Item -LiteralPath (Join-Path $binaries 'hmi-studio.exe') -Destination (Join-Path $distribution 'HmiStudio.exe')
    $resources = Join-Path $distribution 'resources'
    New-Item -ItemType Directory -Path $resources | Out-Null
    foreach ($resourceName in @('templates','plugins','backends','webview2')) {
        $source = Join-Path $workspace ".cache/$resourceName"
        if (Test-Path -LiteralPath $source) { Copy-Item -LiteralPath $source -Destination $resources -Recurse }
    }
    $pythonSource = Join-Path $workspace '.cache/python/3.14.8-win_amd64'
    if (Test-Path -LiteralPath $pythonSource) {
        New-Item -ItemType Directory -Path (Join-Path $resources 'python') | Out-Null
        Copy-Item -LiteralPath $pythonSource -Destination (Join-Path $resources 'python') -Recurse
    }
    Copy-Item -LiteralPath (Join-Path $workspace 'resources/example') -Destination $distribution -Recurse
    Copy-Item -LiteralPath (Join-Path $workspace 'README.md') -Destination $distribution
    Write-Host "Designer ready: $distribution"
} finally { Pop-Location }
