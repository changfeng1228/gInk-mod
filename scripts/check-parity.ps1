#requires -Version 5.1
<#
check-parity.ps1 - Task 1 gate for the plnk Rust rewrite.

Fails when docs/parity-matrix.md is missing, when any required feature family
has no P-xxx row, or when a P-xxx row is malformed (missing source, acceptance
method, status or owner, or a duplicate ID).

The family keys below are stable and greppable. Later tasks must use them in
the family column of docs/parity-matrix.md.

Usage (from the repository root, or any directory):
    powershell -File scripts/check-parity.ps1

Exit codes:
    0 = PASS: every required family has at least one valid P-xxx row.
    1 = FAIL: missing family, missing matrix file, or malformed row.
#>
[CmdletBinding()]
param(
    [string]$MatrixPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($MatrixPath)) {
    $MatrixPath = Join-Path $repoRoot 'docs\parity-matrix.md'
}

# Required by plan Task 1 Step 1. Keep the keys stable; docs and tests grep them.
# Labels are ASCII so the gate output is readable in any console code page.
$requiredFamilies = [ordered]@{
    'tray_hotkeys'  = 'tray / hotkeys'
    'input_types'   = 'mouse / touch / pen'
    'pens_10'       = '10 pen configs'
    'highlighter'   = 'highlighter'
    'eraser'        = 'eraser'
    'undo_redo'     = 'undo / redo'
    'ink_visible'   = 'hide ink'
    'pointer_pan'   = 'pointer / pan'
    'dock_drag'     = 'dock / drag'
    'autoscroll'    = 'autoscroll'
    'snapshot'      = 'snapshot'
    'pin'           = 'pin'
    'options'       = 'options'
    'languages_31'  = '31 languages'
    'multi_dpi'     = 'multi-DPI'
    'multi_monitor' = 'multi-monitor'
    'config_import' = 'config import'
}

# Status and acceptance words are built from code points so this file stays
# ASCII and keeps working even if an editor drops the UTF-8 BOM.
$statusMustTest = [string]([char]0x5FC5) + [char]0x6D4B              # must test
$statusUnverified = [string]([char]0x672A) + [char]0x9A8C + [char]0x8BC1  # unverified
$statusDeferred = [string]([char]0x5EF6) + [char]0x671F              # deferred
$wordAutomatic = [string]([char]0x81EA) + [char]0x52A8               # automatic
$wordManual = [string]([char]0x4EBA) + [char]0x5DE5                  # manual

$validStatuses = @($statusMustTest, $statusUnverified, $statusDeferred)
$rowPattern = '^\|\s*(P-\d{3})\s*\|'

$errors = New-Object System.Collections.Generic.List[string]
$familyRows = @{}
$ids = @{}
$rows = 0

function Write-FailureHeader {
    Write-Host 'FAIL: parity matrix does not satisfy Task 1.' -ForegroundColor Red
}

function Write-MissingFamilies {
    param([System.Collections.Generic.List[string]]$Missing)
    Write-Host 'Missing required feature families (no P-ID row):'
    foreach ($key in $Missing) {
        Write-Host ('  - {0} ({1})' -f $key, $requiredFamilies[$key])
    }
}

if (-not (Test-Path -LiteralPath $MatrixPath -PathType Leaf)) {
    Write-FailureHeader
    Write-Host ('  matrix not found: {0}' -f $MatrixPath)
    Write-Host ''
    $allMissing = New-Object System.Collections.Generic.List[string]
    foreach ($key in $requiredFamilies.Keys) { $allMissing.Add($key) }
    Write-MissingFamilies -Missing $allMissing
    exit 1
}

$lines = Get-Content -LiteralPath $MatrixPath -Encoding UTF8

foreach ($line in $lines) {
    if ($line -notmatch $rowPattern) { continue }
    $rows++

    # 7 cells: ID | family | feature | legacy source | acceptance | status | owner
    $cells = @($line.Trim().Trim('|') -split '\|' | ForEach-Object { $_.Trim() })
    if ($cells.Count -ne 7) {
        $errors.Add(('row has {0} columns, expected 7: {1}' -f $cells.Count, $line.Trim()))
        continue
    }

    $id = $cells[0]
    $family = $cells[1]
    $feature = $cells[2]
    $source = $cells[3]
    $acceptance = $cells[4]
    $status = $cells[5]
    $owner = $cells[6]

    if ($ids.ContainsKey($id)) {
        $errors.Add(('duplicate P-ID: {0}' -f $id))
    } else {
        $ids[$id] = $true
    }

    if (-not $familyRows.ContainsKey($family)) {
        $familyRows[$family] = New-Object System.Collections.Generic.List[string]
    }
    $familyRows[$family].Add($id)

    if ([string]::IsNullOrWhiteSpace($feature)) {
        $errors.Add(('{0}: missing feature description' -f $id))
    }
    if ([string]::IsNullOrWhiteSpace($source)) {
        $errors.Add(('{0}: missing legacy source reference' -f $id))
    }
    if ([string]::IsNullOrWhiteSpace($acceptance)) {
        $errors.Add(('{0}: missing acceptance method' -f $id))
    }
    elseif ((-not $acceptance.Contains($wordAutomatic)) -and (-not $acceptance.Contains($wordManual))) {
        $errors.Add(('{0}: acceptance method must state whether it is automatic or manual' -f $id))
    }
    if ($validStatuses -notcontains $status) {
        $errors.Add(('{0}: invalid status "{1}" (expected one of the three allowed statuses)' -f $id, $status))
    }
    if ([string]::IsNullOrWhiteSpace($owner)) {
        $errors.Add(('{0}: missing owner' -f $id))
    }
}

$missing = New-Object System.Collections.Generic.List[string]
foreach ($key in $requiredFamilies.Keys) {
    if (-not $familyRows.ContainsKey($key)) {
        $missing.Add($key)
    }
}

if ($missing.Count -gt 0 -or $errors.Count -gt 0) {
    Write-FailureHeader
    if ($missing.Count -gt 0) {
        Write-MissingFamilies -Missing $missing
    }
    if ($errors.Count -gt 0) {
        Write-Host ''
        Write-Host ('Malformed P-ID rows ({0}):' -f $errors.Count)
        foreach ($e in $errors) {
            Write-Host ('  - {0}' -f $e)
        }
    }
    exit 1
}

$statusCounts = @{}
foreach ($status in $validStatuses) { $statusCounts[$status] = 0 }
foreach ($line in $lines) {
    if ($line -notmatch $rowPattern) { continue }
    $cells = @($line.Trim().Trim('|') -split '\|' | ForEach-Object { $_.Trim() })
    if ($cells.Count -eq 7 -and $statusCounts.ContainsKey($cells[5])) {
        $statusCounts[$cells[5]]++
    }
}

Write-Host ('PASS: {0} P-ID rows cover all {1} required feature families.' -f $rows, $requiredFamilies.Count) -ForegroundColor Green
Write-Host ('  status counts: must_test={0}, unverified={1}, deferred={2}' -f $statusCounts[$statusMustTest], $statusCounts[$statusUnverified], $statusCounts[$statusDeferred])
Write-Host ('  matrix: {0}' -f $MatrixPath)
exit 0
