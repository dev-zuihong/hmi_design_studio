$ErrorActionPreference = 'Stop'
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$plugin = Join-Path $workspace '.cache/plugins/demo-temperature'
New-Item -ItemType Directory -Force -Path $plugin | Out-Null
$executable = Join-Path $workspace 'target/debug/demo-protocol.exe'
if (-not (Test-Path -LiteralPath $executable)) { throw 'Run cargo build -p demo-protocol first.' }
Copy-Item -LiteralPath $executable -Destination (Join-Path $plugin 'demo-protocol.exe') -Force
Copy-Item -LiteralPath (Join-Path $workspace 'crates/demo-protocol/package/manifest.json') -Destination (Join-Path $plugin 'manifest.json') -Force
Write-Host "Demo protocol prepared: $plugin"
