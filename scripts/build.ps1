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
    $pythonTarget = 'x86_64-pc-windows-msvc'
    $pythonCache = Join-Path $workspace ".cache/python/$pythonTarget"
    $pythonArchive = Join-Path $workspace '.cache/python/python-3.14.8-embed-amd64.zip'
    if (-not $SkipPython) {
        $preparePython = Join-Path $PSScriptRoot 'prepare-python.ps1'
        if (Test-Path -LiteralPath $pythonArchive) {
            & $preparePython -CacheDirectory $pythonCache -ArchivePath $pythonArchive
        } else {
            & $preparePython -CacheDirectory $pythonCache
        }
    }
    if (-not $SkipWebview) { & (Join-Path $PSScriptRoot 'prepare-webview.ps1') }
    $cargoArgs = @('build', '--workspace', '--target', 'x86_64-pc-windows-msvc', '--features', 'hmi-studio/custom-protocol,hmi-runner/custom-protocol')
    $profile = 'debug'
    if (-not $Debug) { $cargoArgs += '--release'; $profile = 'release' }
    & cargo @cargoArgs
    if ($LASTEXITCODE -ne 0) { throw 'Rust build failed' }
    $binaries = Join-Path $workspace "target/x86_64-pc-windows-msvc/$profile"
    $template = Join-Path $workspace '.cache/templates/windows-x64'
    New-Item -ItemType Directory -Force -Path $template | Out-Null
    Copy-Item -LiteralPath (Join-Path $binaries 'hmi-runner.exe') -Destination (Join-Path $template 'HmiRunner.exe') -Force
    $distribution = Join-Path $workspace ("artifacts/HmiStudio-win-x64-" + (Get-Date -Format 'yyyyMMdd-HHmmss'))
    New-Item -ItemType Directory -Path $distribution | Out-Null
    Copy-Item -LiteralPath (Join-Path $binaries 'hmi-studio.exe') -Destination (Join-Path $distribution 'HmiStudio.exe')
    $resources = Join-Path $distribution 'resources'
    New-Item -ItemType Directory -Path $resources | Out-Null
    foreach ($resourceName in @('templates','backends','webview2')) {
        $source = Join-Path $workspace ".cache/$resourceName"
        if (Test-Path -LiteralPath $source) { Copy-Item -LiteralPath $source -Destination $resources -Recurse }
    }
    $pluginSource = Join-Path $workspace '.cache/plugins'
    if (Test-Path -LiteralPath $pluginSource) {
        $pluginDestination = Join-Path $resources 'plugins'
        New-Item -ItemType Directory -Force -Path $pluginDestination | Out-Null
        foreach ($pluginEntry in Get-ChildItem -LiteralPath $pluginSource) {
            if ($pluginEntry.Name -ne 'demo-temperature') {
                Copy-Item -LiteralPath $pluginEntry.FullName -Destination $pluginDestination -Recurse
            }
        }
    }
    $pythonSource = Join-Path $pythonCache '3.14.8-win_amd64'
    if (Test-Path -LiteralPath $pythonSource) {
        $pythonDestination = Join-Path $resources "python/$pythonTarget"
        New-Item -ItemType Directory -Force -Path $pythonDestination | Out-Null
        Copy-Item -LiteralPath $pythonSource -Destination $pythonDestination -Recurse
    }
    Copy-Item -LiteralPath (Join-Path $workspace 'README.md') -Destination $distribution
    Write-Host "Designer ready: $distribution"
} finally { Pop-Location }
