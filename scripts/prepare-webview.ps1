param(
    [string]$CacheDirectory = '',
    [string]$ArchivePath = ''
)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$source = Get-Content -LiteralPath (Join-Path $projectRoot 'resources/webview2-source.json') -Raw -Encoding UTF8 | ConvertFrom-Json
if (!$CacheDirectory) { $CacheDirectory = Join-Path $projectRoot '.cache' }
$CacheDirectory = [IO.Path]::GetFullPath($CacheDirectory)
[IO.Directory]::CreateDirectory($CacheDirectory) | Out-Null
if (!$ArchivePath) {
    $ArchivePath = Join-Path $CacheDirectory $source.filename
    if (!(Test-Path -LiteralPath $ArchivePath)) {
        Write-Output "Downloading fixed WebView2 $($source.version) x64..."
        $previousProgressPreference = $ProgressPreference
        try {
            $ProgressPreference = 'SilentlyContinue'
            Invoke-WebRequest -UseBasicParsing -Uri $source.url -OutFile $ArchivePath
        } finally { $ProgressPreference = $previousProgressPreference }
    }
}
$ArchivePath = [IO.Path]::GetFullPath($ArchivePath)
if ((Get-Item -LiteralPath $ArchivePath).Length -ne $source.size) { throw 'WebView2 CAB size does not match the pinned source.' }
$digest = (Get-FileHash -LiteralPath $ArchivePath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($digest -ne $source.sha256) { throw 'WebView2 CAB SHA-256 does not match the pinned source.' }
$destination = Join-Path $CacheDirectory 'webview2'
if (Test-Path -LiteralPath $destination) {
    $existing = Get-Content -LiteralPath (Join-Path $destination 'webview2-manifest.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($existing.archiveSha256 -ne $digest -or $existing.version -ne $source.version -or $existing.platform -ne $source.platform) { throw 'Existing WebView2 does not match the pinned release.' }
    foreach ($entry in $existing.files.PSObject.Properties) {
        $file = [IO.Path]::GetFullPath((Join-Path $destination $entry.Name))
        if (!$file.StartsWith($destination + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Invalid existing WebView2 manifest path.' }
        if ((Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant() -ne $entry.Value) { throw "Existing WebView2 file has changed: $($entry.Name)" }
    }
    Write-Output "Reusing verified fixed WebView2: $destination"
    return
}
$staging = Join-Path $CacheDirectory ('.webview2-' + [Guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($staging) | Out-Null
try {
    $expander = Join-Path $env:SystemRoot 'System32/expand.exe'
    & $expander '-F:*' $ArchivePath $staging | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "expand.exe failed with exit code $LASTEXITCODE" }
    $executables = @(Get-ChildItem -LiteralPath $staging -Filter 'msedgewebview2.exe' -File -Recurse)
    if ($executables.Length -ne 1) { throw 'Expected exactly one msedgewebview2.exe in the fixed runtime.' }
    $executable = $executables[0]
    $fileVersion = $executable.VersionInfo.ProductVersion
    if ($fileVersion -ne $source.version) { throw "WebView2 product version mismatch: $fileVersion" }
    $stream = [IO.File]::OpenRead($executable.FullName)
    $reader = New-Object IO.BinaryReader($stream)
    try {
        if ($reader.ReadUInt16() -ne 0x5a4d) { throw 'WebView2 is not a Windows PE executable.' }
        $stream.Position = 60
        $peOffset = $reader.ReadUInt32()
        $stream.Position = $peOffset
        if ($reader.ReadUInt32() -ne 0x00004550 -or $reader.ReadUInt16() -ne 0x8664) { throw 'WebView2 must be Windows x64.' }
    } finally { $reader.Dispose() }
    $runtimeDirectory = [IO.Path]::GetFullPath($executable.DirectoryName)
    $resolvedStaging = [IO.Path]::GetFullPath($staging)
    if ($runtimeDirectory -ne $resolvedStaging -and !$runtimeDirectory.StartsWith($resolvedStaging + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Extracted runtime escaped its staging directory.'
    }
    if ([IO.Path]::GetFullPath($destination) -ne (Join-Path $CacheDirectory 'webview2')) { throw 'Unexpected destination path.' }
    $files = [ordered]@{}
    Get-ChildItem -LiteralPath $runtimeDirectory -File -Recurse | Sort-Object FullName | ForEach-Object {
        $relative = $_.FullName.Substring($runtimeDirectory.Length + 1).Replace('\', '/')
        $files[$relative] = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    $manifest = [ordered]@{ schemaVersion = 1; version = $source.version; platform = $source.platform; archiveSha256 = $digest; files = $files }
    [IO.File]::WriteAllText((Join-Path $runtimeDirectory 'webview2-manifest.json'), ($manifest | ConvertTo-Json -Depth 10) + "`n", (New-Object System.Text.UTF8Encoding($false)))
    Move-Item -LiteralPath $runtimeDirectory -Destination $destination
    Write-Output "Prepared fixed WebView2 $($source.version) x64: $destination"
} finally {
    if (Test-Path -LiteralPath $staging) {
        $resolvedStaging = (Resolve-Path -LiteralPath $staging).Path
        if (!$resolvedStaging.StartsWith($CacheDirectory + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Refusing cleanup outside the cache directory.' }
        Remove-Item -LiteralPath $resolvedStaging -Recurse -Force
    }
}
