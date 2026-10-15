param(
    [string]$CacheDirectory = '',
    [string]$ArchivePath = ''
)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$sourceRoot = Join-Path $projectRoot 'resources/python'
$source = Get-Content -LiteralPath (Join-Path $sourceRoot 'runtime-source.json') -Raw -Encoding UTF8 | ConvertFrom-Json
if (!$CacheDirectory) { $CacheDirectory = Join-Path $projectRoot '.cache/python' }
$CacheDirectory = [IO.Path]::GetFullPath($CacheDirectory)
[IO.Directory]::CreateDirectory($CacheDirectory) | Out-Null
if (!$ArchivePath) {
    $ArchivePath = Join-Path $CacheDirectory 'python-3.14.8-embed-amd64.zip'
    if (!(Test-Path -LiteralPath $ArchivePath)) {
        Invoke-WebRequest -UseBasicParsing -Uri $source.url -OutFile $ArchivePath
    }
}
$digest = (Get-FileHash -LiteralPath $ArchivePath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($digest -ne $source.sha256) { throw 'Python archive SHA-256 does not match the pinned official release.' }
$destination = Join-Path $CacheDirectory '3.14.8-win_amd64'
if (Test-Path -LiteralPath $destination) {
    $existing = Get-Content -LiteralPath (Join-Path $destination 'runtime-manifest.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($existing.archiveSha256 -ne $digest -or $existing.version -ne $source.version -or $existing.platform -ne $source.platform) { throw 'Existing Python runtime does not match the pinned release.' }
    foreach ($entry in $existing.files.PSObject.Properties) {
        $file = [IO.Path]::GetFullPath((Join-Path $destination $entry.Name))
        if (!$file.StartsWith($destination + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Invalid existing runtime manifest path.' }
        if ((Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant() -ne $entry.Value) { throw "Existing Python file has changed: $($entry.Name)" }
    }
    if ((Get-FileHash -LiteralPath (Join-Path $sourceRoot 'worker.py') -Algorithm SHA256).Hash.ToLowerInvariant() -ne $existing.files.'executor/worker.py' -or
        (Get-FileHash -LiteralPath (Join-Path $sourceRoot 'python314._pth') -Algorithm SHA256).Hash.ToLowerInvariant() -ne $existing.files.'python314._pth') {
        throw 'Python executor sources changed. Prepare a new -CacheDirectory and import that runtime.'
    }
    Write-Output "Reusing verified Python runtime: $destination"
    return
}
Expand-Archive -LiteralPath $ArchivePath -DestinationPath $destination
[IO.Directory]::CreateDirectory((Join-Path $destination 'executor')) | Out-Null
[IO.Directory]::CreateDirectory((Join-Path $destination 'dependencies')) | Out-Null
Copy-Item -LiteralPath (Join-Path $sourceRoot 'worker.py') -Destination (Join-Path $destination 'executor/worker.py')
Copy-Item -LiteralPath (Join-Path $sourceRoot 'python314._pth') -Destination $destination
$utf8 = New-Object System.Text.UTF8Encoding($false)
$lock = [ordered]@{ schemaVersion = 1; pythonVersion = $source.version; platform = $source.platform; abi = $source.abi; packages = @(); imports = @() }
[IO.File]::WriteAllText((Join-Path $destination 'dependencies.lock.json'), ($lock | ConvertTo-Json -Depth 10) + "`n", $utf8)
$files = [ordered]@{}
Get-ChildItem -LiteralPath $destination -File -Recurse | Sort-Object FullName | ForEach-Object {
    $relative = $_.FullName.Substring($destination.Length + 1).Replace('\', '/')
    $files[$relative] = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
}
$manifest = [ordered]@{ schemaVersion = 1; version = $source.version; platform = $source.platform; abi = $source.abi; archiveSha256 = $digest; files = $files }
[IO.File]::WriteAllText((Join-Path $destination 'runtime-manifest.json'), ($manifest | ConvertTo-Json -Depth 10) + "`n", $utf8)
Write-Output "Prepared Python embeddable runtime: $destination"
Write-Output 'Run cargo test -p hmi-python to verify execution, limits and process cleanup.'
