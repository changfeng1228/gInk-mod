<#
.SYNOPSIS
    Extracts the embedded .resources from an upstream gInk 1.1.1 executable.

.DESCRIPTION
    gInk.exe embeds eight .resources streams (form layouts and the icon/bitmap
    resource set). The compiler needs them as separate files via -resource:, so
    they are extracted once from a shipped executable and committed under
    build\res\. They are identical across every 1.1.1 build, so this only has to
    run when starting a new tree from scratch.

    Pure source: build\ExtractResources.cs. It is compiled here the first time.

.PARAMETER SourceExe
    A gInk 1.1.1 executable to read the resources from. Either the original
    upstream release or any existing install works.

.PARAMETER OutputDir
    Defaults to build\res\ in the repository.

.EXAMPLE
    .\extract-resources.ps1 -SourceExe 'D:\Download\gInk\gInk_v1.1.1\gInk.exe.orig-1.1.1'
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string] $SourceExe,
    [string] $OutputDir = ''
)

$ErrorActionPreference = 'Stop'
$buildDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$root = Split-Path -Parent $buildDir

if (-not (Test-Path $SourceExe)) { Write-Host "ERROR: $SourceExe not found" -ForegroundColor Red; exit 1 }
if ([string]::IsNullOrWhiteSpace($OutputDir)) { $OutputDir = Join-Path $buildDir 'res' }

$csc = Join-Path $env:WINDIR 'Microsoft.NET\Framework\v4.0.30319\csc.exe'
if (-not (Test-Path $csc)) { Write-Host "ERROR: csc.exe not found" -ForegroundColor Red; exit 1 }

$extractor = Join-Path $buildDir 'extract.exe'
$source = Join-Path $buildDir 'ExtractResources.cs'

if (-not (Test-Path $extractor)) {
    Write-Host 'compiling resource extractor...'
    & $csc -nologo -target:exe -platform:x86 -out:$extractor $source
    if ($LASTEXITCODE -ne 0) { Write-Host 'ERROR: could not compile extractor' -ForegroundColor Red; exit 1 }
}

if (-not (Test-Path $OutputDir)) { New-Item -ItemType Directory -Path $OutputDir | Out-Null }

Write-Host "extracting from $SourceExe"
& $extractor $SourceExe $OutputDir
if ($LASTEXITCODE -ne 0) { Write-Host "ERROR: extraction failed (exit $LASTEXITCODE)" -ForegroundColor Red; exit 1 }

$files = @(Get-ChildItem $OutputDir -Filter *.resources)
Write-Host ''
Write-Host ("extracted {0} resource files -> {1}" -f $files.Count, $OutputDir) -ForegroundColor Green
$files | Sort-Object Name | ForEach-Object { Write-Host ("  {0,-45} {1,8:N0}" -f $_.Name, $_.Length) }

if ($files.Count -ne 8) {
    Write-Host ''
    Write-Host "WARNING: expected 8 resource files, got $($files.Count)" -ForegroundColor Yellow
}
