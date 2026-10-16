param([switch]$SkipBuild, [string]$OutputDirectory)
$ErrorActionPreference = 'Stop'
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $workspace ('artifacts/ing-ptb-' + (Get-Date -Format 'yyyyMMdd-HHmmss')) }
$OutputDirectory = [System.IO.Path]::GetFullPath($OutputDirectory)
Push-Location $workspace
try {
    if ($env:PROCESSOR_ARCHITECTURE -ne 'AMD64') { throw 'Windows x64 build host required.' }
    New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
    $backend = Join-Path $workspace '.cache/backends/x86_64-pc-windows-msvc/ing-ptb'
    if (-not $SkipBuild) {
        & (Join-Path $workspace 'backends/ing-ptb/build.ps1') -OutputDirectory $backend
        if ($LASTEXITCODE -ne 0) { throw 'PTB backend publish failed.' }
    }
    & node scripts/generate-ptb-demo.mjs
    if ($LASTEXITCODE -ne 0) { throw 'PTB project generation failed.' }
    if (-not $SkipBuild) {
        & (Join-Path $workspace 'scripts/prepare-webview.ps1')
        & npm.cmd run build
        if ($LASTEXITCODE -ne 0) { throw 'Frontend build failed.' }
        & cargo build --workspace --release --target x86_64-pc-windows-msvc --features hmi-studio/custom-protocol,hmi-runner/custom-protocol
        if ($LASTEXITCODE -ne 0) { throw 'Native build failed.' }
    }
    # Stage the selected release even with -SkipBuild, avoiding a stale template.
    $template = Join-Path $workspace '.cache/templates/windows-x64'
    New-Item -ItemType Directory -Force -Path $template | Out-Null
    Copy-Item -LiteralPath (Join-Path $workspace 'target/x86_64-pc-windows-msvc/release/hmi-runner.exe') -Destination (Join-Path $template 'HmiRunner.exe') -Force
    $cli = Join-Path $workspace 'target/x86_64-pc-windows-msvc/release/hmi-cli.exe'
    $project = Join-Path $workspace 'examples/ing-ptb/project.json'
    foreach ($operation in @('validate','smoke','package','export')) {
        $arguments = @($operation,'--project',$project,'--resources',(Join-Path $workspace '.cache'))
        if ($operation -eq 'smoke') { $arguments += @('--flow','flow_scan') }
        if ($operation -eq 'package') { $arguments += @('--destination',(Join-Path $OutputDirectory 'ING-PTB.hmiproj')) }
        if ($operation -eq 'export') { $arguments += @('--destination',(Join-Path $OutputDirectory 'ING-PTB-win-x64.zip')) }
        $result = & $cli @arguments
        if ($LASTEXITCODE -ne 0) { throw "PTB $operation failed: $result" }
        $parsed = $result | ConvertFrom-Json
        if ($operation -eq 'validate' -and -not $parsed.valid) { throw "PTB validation failed: $result" }
        [System.IO.File]::WriteAllText((Join-Path $OutputDirectory "$operation.json"), ($result -join "`n"), [System.Text.UTF8Encoding]::new($false))
    }
    Copy-Item -LiteralPath (Join-Path $workspace 'examples/ing-ptb/README.md') -Destination (Join-Path $OutputDirectory 'README.md')
    Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem
    $sourceRoot = Join-Path $workspace 'backends/ing-ptb'
    $sourceZip = [System.IO.Compression.ZipFile]::Open((Join-Path $OutputDirectory 'ing-ptb-backend-source.zip'), [System.IO.Compression.ZipArchiveMode]::Create)
    try {
        foreach ($sourceFile in (Get-ChildItem -LiteralPath $sourceRoot -Recurse -File -Force)) {
            $relative = $sourceFile.FullName.Substring($sourceRoot.Length + 1).Replace('\','/')
            if ($relative -match '(^|/)(bin|obj|publish)/') { continue }
            [System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile($sourceZip, $sourceFile.FullName, ('ing-ptb/' + $relative), [System.IO.Compression.CompressionLevel]::Optimal) | Out-Null
        }
    } finally { $sourceZip.Dispose() }
    Write-Host "PTB demo ready: $OutputDirectory"
} finally { Pop-Location }
