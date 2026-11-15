$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
& (Join-Path $root 'build\build.ps1')

$outDir = Join-Path $root 'build\_scratch\animation-tests'
New-Item -ItemType Directory -Path $outDir -Force | Out-Null
$testExe = Join-Path $outDir 'AnimationFrameTests.exe'
$csc = Join-Path $env:WINDIR 'Microsoft.NET\Framework\v4.0.30319\csc.exe'
$cscArgs = @(
    '-nologo', '-optimize+', '-platform:x86',
    ('-out:' + $testExe),
    '-r:System.dll', '-r:System.Core.dll', '-r:System.Windows.Forms.dll',
    (Join-Path $PSScriptRoot 'AnimationFrameTests.cs')
)
& $csc $cscArgs
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$gInkExe = Join-Path $root 'build\out\gInk.exe'
$testProcess = Start-Process -FilePath $testExe -ArgumentList ('"' + $gInkExe + '"') -NoNewWindow -PassThru
if (-not $testProcess.WaitForExit(30000)) {
    $testProcess.Kill()
    throw 'Animation tests timed out after 30 seconds.'
}
exit $testProcess.ExitCode
