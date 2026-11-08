<#
.SYNOPSIS
    Builds gInk.exe from source into build\out\.

.DESCRIPTION
    A plain .NET Framework WinForms application compiled with the C# compiler that
    ships with Windows. No Visual Studio required.

    build\out\ ends up with exactly what the program needs at run time: the exe
    plus the resources, language files and icons it loads from its own directory.

.PARAMETER InkDll
    Path to Microsoft.Ink.dll. Defaults to the copy in bin\. Not redistributable,
    so it is not in the repository; it ships with the Windows Tablet PC components
    and sits next to any installed gInk 1.1.1.

.PARAMETER Configuration
    Release (default) or Debug. Debug also emits a .pdb.

.EXAMPLE
    .\build.ps1
#>
[CmdletBinding()]
param(
    [string] $InkDll = '',
    [ValidateSet('Release', 'Debug')]
    [string] $Configuration = 'Release'
)

$ErrorActionPreference = 'Stop'

$buildDir = $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($buildDir)) {
    $buildDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
}
if ([string]::IsNullOrWhiteSpace($buildDir)) {
    Write-Host 'ERROR: cannot locate script directory' -ForegroundColor Red
    exit 1
}
$root = Split-Path -Parent $buildDir

function Fail($msg) {
    Write-Host "ERROR: $msg" -ForegroundColor Red
    exit 1
}

$csc = Join-Path $env:WINDIR 'Microsoft.NET\Framework\v4.0.30319\csc.exe'
if (-not (Test-Path $csc)) {
    Fail "C# compiler not found at $csc (needs .NET Framework 4.x)"
}

if ([string]::IsNullOrWhiteSpace($InkDll)) {
    $progFiles = $env:CommonProgramFiles
    if ([string]::IsNullOrWhiteSpace($progFiles)) {
        $progFiles = Join-Path $env:SystemDrive 'Program Files\Common Files'
    }
    $candidates = @(
        (Join-Path $root 'bin\Microsoft.Ink.dll'),
        'D:\Download\gInk\gInk_v1.1.1\Microsoft.Ink.dll',
        (Join-Path $progFiles 'Microsoft Shared\Ink\InkObj.dll')
    )
    $InkDll = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
}
if (-not $InkDll -or -not (Test-Path $InkDll)) {
    Fail 'Microsoft.Ink.dll not found. Pass it explicitly: .\build.ps1 -InkDll "C:\path\to\Microsoft.Ink.dll"'
}
if ((Split-Path -Leaf $InkDll) -ne 'Microsoft.Ink.dll') {
    Fail "-InkDll must point at Microsoft.Ink.dll, got: $InkDll"
}

$resDir = Join-Path $root 'build\res'
$resources = @(Get-ChildItem $resDir -Filter *.resources -ErrorAction SilentlyContinue |
                Sort-Object Name)
if ($resources.Count -eq 0) {
    Fail "No .resources in $resDir. Run .\build\extract-resources.ps1 -SourceExe <upstream gInk.exe> once."
}

$outDir = Join-Path $root 'build\out'
if (Test-Path $outDir) { Remove-Item $outDir -Recurse -Force }
New-Item -ItemType Directory -Path $outDir | Out-Null

# csc emits types and methods in the order it meets them, so file order changes the
# binary. The shipped build was produced from a plain ordinal sort of the file names
# in src\ (which is why Hotkey.cs precedes HotkeyInputBox.Designer.cs), followed by
# src\Properties\*.cs. Reproduce exactly that.
$srcDir = Join-Path $root 'src'
$ordered = New-Object 'System.Collections.Generic.List[System.IO.FileInfo]'

foreach ($f in @(Get-ChildItem $srcDir -Filter *.cs | Sort-Object -Property Name)) {
    $ordered.Add($f)
}

$propDir = Join-Path $srcDir 'Properties'
if (Test-Path $propDir) {
    foreach ($f in @(Get-ChildItem $propDir -Filter *.cs | Sort-Object -Property Name)) {
        $ordered.Add($f)
    }
}

# ASCII without BOM: a UTF-8 BOM makes csc treat the first quoted path as junk.
$rsp = Join-Path $outDir 'sources.rsp'
$lines = New-Object 'System.Collections.Generic.List[string]'
foreach ($f in $ordered)   { $lines.Add('"' + $f.FullName + '"') }
foreach ($r in $resources) { $lines.Add('-resource:"' + $r.FullName + '"') }
$icon = Join-Path $root 'bin\icon_red.ico'
if (Test-Path $icon) { $lines.Add('-win32icon:"' + $icon + '"') }
[System.IO.File]::WriteAllLines($rsp, $lines, (New-Object System.Text.ASCIIEncoding))

# The csc in .NET Framework 4.x has no -deterministic switch, so rebuilds differ in
# the PE timestamp and module MVID (about 44 bytes) while staying functionally
# identical. Reproducibility is checked by size, source order and behaviour, not by
# whole-file byte equality.
$exe = Join-Path $outDir 'gInk.exe'
$cscArgs = @(
    '-nologo', '-target:winexe', '-platform:x86',
    ('-out:' + $exe),
    ('-r:' + $InkDll),
    '-r:System.dll', '-r:System.Drawing.dll', '-r:System.Windows.Forms.dll',
    '-r:System.Core.dll', '-r:System.Net.dll'
)
if ($Configuration -eq 'Release') { $cscArgs += '-optimize+' }
else { $cscArgs += '-debug+', '-define:DEBUG' }
$cscArgs += ('@' + $rsp)

Write-Host "compiling $($ordered.Count) source files -> $exe"
$output = & $csc $cscArgs 2>&1
$output | Where-Object { $_ -notmatch 'warning CS' } | ForEach-Object { Write-Host $_ }

if ($LASTEXITCODE -ne 0) { Fail "compilation failed (exit $LASTEXITCODE)" }
if (-not (Test-Path $exe)) { Fail "compiler reported success but $exe is missing" }

# Run-time layout: the program reads these from its own directory. The .ini files are
# not in the repository (they are per-install user data written by the program), so
# when bin\ has no copy the factory template is used instead - the program would run
# without them anyway because SetDefaultConfig() supplies every default in code, but
# shipping them means a fresh install has an editable copy from day one.
Copy-Item $InkDll $outDir -Force
foreach ($n in @('icon_red.ico', 'icon_white.ico')) {
    $p = Join-Path $root ('bin\' + $n)
    if (Test-Path $p) { Copy-Item $p $outDir -Force }
}
$langDir = Join-Path $root 'bin\lang'
if (Test-Path $langDir) { Copy-Item $langDir $outDir -Recurse -Force }

$appConfig = Join-Path $root 'bin\gInk.exe.config'
if (Test-Path $appConfig) { Copy-Item $appConfig $outDir -Force }

$defaultIni = Join-Path $root 'bin\config_default.ini'
if (Test-Path $defaultIni) { Copy-Item $defaultIni $outDir -Force }

foreach ($ini in @('config.ini', 'pens.ini', 'hotkeys.ini')) {
    $src = Join-Path $root ('bin\' + $ini)
    $dst = Join-Path $outDir $ini
    if (Test-Path $src) {
        Copy-Item $src $dst -Force
    } elseif (-not (Test-Path $dst)) {
        # No local copy: start the install from the factory defaults. pens.ini and
        # hotkeys.ini have no separate template, config_default.ini documents the
        # same defaults the code applies when a key is missing.
        Write-Host "  note: bin\$ini absent, the program will create it on first save"
    }
}

$size = (Get-Item $exe).Length
$hash = (Get-FileHash $exe -Algorithm MD5).Hash
Write-Host ''
Write-Host ("built  {0}" -f $exe) -ForegroundColor Green
Write-Host ("size   {0:N0} bytes" -f $size)
Write-Host ("md5    {0}" -f $hash)
Write-Host ''
Write-Host 'run-time layout:'
Get-ChildItem $outDir | Sort-Object Name | ForEach-Object {
    if ($_.PSIsContainer) {
        Write-Host ("  {0,-24} <dir>" -f $_.Name)
    } else {
        Write-Host ("  {0,-24} {1:N0}" -f $_.Name, $_.Length)
    }
}
