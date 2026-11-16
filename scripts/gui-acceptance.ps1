<#
.SYNOPSIS
Runs the desktop acceptance tests that need the real pointer, keyboard and pixels.

.DESCRIPTION
Each test target runs separately so one failure cannot hide the others. The
tests themselves yield when the user is active; this script reports that as
SKIP and never as success.

Exit codes:
  0 - every target passed
  1 - at least one target failed
  2 - at least one target skipped because the user was active, and none failed
#>
[CmdletBinding()]
param(
    [switch]$WhatIf,
    [switch]$Release,
    [string]$Repo
)

# 'Continue', not 'Stop': cargo writes progress to stderr, and PowerShell 5.1
# surfaces native stderr as a NativeCommandError that 'Stop' would turn into a
# terminating error. The exit code is inspected explicitly instead.
$ErrorActionPreference = 'Continue'
# $PSScriptRoot is not bound while parameter defaults are evaluated, so the
# repository root is resolved here instead of inside the param block.
if (-not $Repo) { $Repo = Split-Path -Parent $PSScriptRoot }
$manifest = Join-Path $Repo 'plnk\Cargo.toml'
$profileArgs = if ($Release) { @('--release') } else { @() }

# Targets holding the desktop tests. Keep in step with the plan at
# docs/superpowers/plans/2026-11-16-gui-acceptance-harness.md.
$targets = @('ui_ink', 'ui_smoke', 'ui_toolbar')
$results = @()

foreach ($target in $targets) {
    # --nocapture is required: without it Cargo swallows the test stdout and the
    # PLNK_SKIP marker never reaches this script, which would then report every
    # skipped test as Passed.
    $tail = @('--', '--ignored', '--test-threads=1', '--nocapture')
    $arguments = @('test', '--manifest-path', $manifest, '--test', $target) + $tail
    $rendered = "cargo " + (($profileArgs + $arguments) -join ' ')

    if ($WhatIf) {
        Write-Host "[WhatIf] $rendered"
        $results += [pscustomobject]@{ Target = $target; State = 'WhatIf'; Detail = $rendered }
        continue
    }

    Write-Host "== $target =="
    # `2>&1` at the call would turn cargo's stderr into PowerShell errors and
    # abort the loop; merging streams through a variable keeps them as text.
    $output = (& cargo @profileArgs @arguments 2>&1 | Out-String)
    $exit = $LASTEXITCODE
    Write-Host $output

    $skipped = ([regex]::Matches($output, 'PLNK_SKIP:')).Count
    if ($exit -ne 0) {
        $state = 'Failed'
    } elseif ($skipped -gt 0) {
        $state = 'Skipped'
    } else {
        $state = 'Passed'
    }
    $results += [pscustomobject]@{
        Target = $target
        State = $state
        Detail = "exit=$exit skipped=$skipped"
    }
}

$summaryPath = Join-Path $Repo '.rivet\scratch\gui-acceptance.json'
$summaryDirectory = Split-Path -Parent $summaryPath
if (-not (Test-Path -LiteralPath $summaryDirectory)) {
    New-Item -ItemType Directory -Path $summaryDirectory -Force | Out-Null
}
$results | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath $summaryPath -Encoding utf8
Write-Host "summary: $summaryPath"

$results | Format-Table -AutoSize

if ($results.State -contains 'Failed') { exit 1 }
if ($results.State -contains 'Skipped') { exit 2 }
exit 0
