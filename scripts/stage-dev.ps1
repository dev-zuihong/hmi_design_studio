$ErrorActionPreference = 'Stop'
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
# Compile the bundled Runner used by the independent live preview.
Push-Location $workspace
try {
    & (Join-Path $workspace 'node_modules/.bin/vite.cmd') build --config vite.runner.config.ts
    if ($LASTEXITCODE -ne 0) { throw 'Runner frontend build failed.' }
    cargo build -p hmi-runner --features custom-protocol
    if ($LASTEXITCODE -ne 0) { throw 'Runner build failed.' }
    $template = Join-Path $workspace '.cache/templates/windows-x64'
    New-Item -ItemType Directory -Force -Path $template | Out-Null
    Copy-Item -LiteralPath (Join-Path $workspace 'target/debug/hmi-runner.exe') -Destination (Join-Path $template 'HmiRunner.exe') -Force
} finally { Pop-Location }
