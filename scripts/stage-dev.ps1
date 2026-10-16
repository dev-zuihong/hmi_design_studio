$ErrorActionPreference = 'Stop'
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$plugin = Join-Path $workspace '.cache/plugins/demo-temperature'
New-Item -ItemType Directory -Force -Path $plugin | Out-Null
$executable = Join-Path $workspace 'target/debug/demo-protocol.exe'
if (-not (Test-Path -LiteralPath $executable)) { throw 'Run cargo build -p demo-protocol first.' }
Copy-Item -LiteralPath $executable -Destination (Join-Path $plugin 'demo-protocol.exe') -Force
Copy-Item -LiteralPath (Join-Path $workspace 'crates/demo-protocol/package/manifest.json') -Destination (Join-Path $plugin 'manifest.json') -Force
Write-Host "Demo protocol prepared: $plugin"

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
