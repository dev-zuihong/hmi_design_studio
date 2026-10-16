param([string]$OutputDirectory = (Join-Path $PSScriptRoot 'publish'))
$ErrorActionPreference = 'Stop'
$publishPath = [System.IO.Path]::GetFullPath($OutputDirectory)
New-Item -ItemType Directory -Force -Path $publishPath | Out-Null
dotnet publish (Join-Path $PSScriptRoot 'IngPtb.Backend.csproj') -c Release -r win-x64 --self-contained true -p:PublishAot=true -o $publishPath
if ($LASTEXITCODE -ne 0) { throw 'ING PTB NativeAOT publish failed.' }
$symbolFile = Join-Path $publishPath 'ing-ptb-backend.pdb'
if (Test-Path -LiteralPath $symbolFile) { Remove-Item -LiteralPath $symbolFile }
foreach ($name in @('manifest.json', 'default-settings.json', 'README.md', 'source-provenance.json')) {
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot $name) -Destination (Join-Path $publishPath $name) -Force
}
Write-Output "ING PTB NativeAOT provider published: $publishPath"
