[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Version
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if ($Version -notmatch '^\d+\.\d+\.\d+$') {
    throw "Invalid version '$Version'; expected x.y.z (for example 0.1.1)."
}

$repo = Split-Path -Parent $PSScriptRoot
$manifest = Join-Path $repo 'plnk\Cargo.toml'
$lock = Join-Path $repo 'plnk\Cargo.lock'

if (-not (Test-Path -LiteralPath $manifest -PathType Leaf)) {
    throw "Cargo manifest is missing: $manifest"
}
if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
    throw 'git is required: every release version must match a committed tree.'
}

# Releases must stay Git-clean. Re-running after a partial bump is allowed, so
# an uncommitted plnk/Cargo.toml is tolerated; everything else must be committed.
$statusLines = @(& git -C $repo status --porcelain)
if ($LASTEXITCODE -ne 0) {
    throw 'git status failed; refusing to bump the version.'
}
$dirty = @()
foreach ($line in $statusLines) {
    if ([string]::IsNullOrWhiteSpace($line)) { continue }
    $path = ($line.Substring(3).Trim('"') -replace '\\', '/')
    if ($path -ne 'plnk/Cargo.toml') { $dirty += $line }
}
if ($dirty.Count -gt 0) {
    throw ("Working tree has uncommitted changes outside plnk/Cargo.toml. " +
        "Commit or stash them first so every release maps to a Git commit:`n  " +
        ($dirty -join "`n  "))
}

# Replace only the version line inside the [package] section, preserving the
# file's encoding (BOM or not) and line endings.
$bytes = [IO.File]::ReadAllBytes($manifest)
$hasBom = $bytes.Length -ge 3 -and $bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB -and $bytes[2] -eq 0xBF
$offset = 0
if ($hasBom) { $offset = 3 }
$text = [Text.Encoding]::UTF8.GetString($bytes, $offset, $bytes.Length - $offset)

$packageMatch = [regex]::Match($text, '(?m)^\[package\]\s*$')
if (-not $packageMatch.Success) {
    throw "Could not find a [package] section in $manifest"
}
$start = $packageMatch.Index + $packageMatch.Length
$nextSection = [regex]::Match($text.Substring($start), '(?m)^\[')
$length = if ($nextSection.Success) { $nextSection.Index } else { $text.Length - $start }
$section = $text.Substring($start, $length)
$versionMatch = [regex]::Match($section, '(?m)^(\s*version\s*=\s*")[^"]*(")')
if (-not $versionMatch.Success) {
    throw "Could not find the [package] version in $manifest"
}
$replacement = $versionMatch.Groups[1].Value + $Version + $versionMatch.Groups[2].Value
$newSection = $section.Substring(0, $versionMatch.Index) + $replacement +
    $section.Substring($versionMatch.Index + $versionMatch.Length)
$newText = $text.Substring(0, $start) + $newSection + $text.Substring($start + $length)
[IO.File]::WriteAllText($manifest, $newText, (New-Object System.Text.UTF8Encoding($hasBom)))
Write-Host "Updated plnk/Cargo.toml [package] version to $Version."

# Refresh Cargo.lock so the new version is recorded (and the manifest stays
# buildable). cargo update -p plnk is enough; cargo check is the fallback.
Push-Location $repo
try {
    & cargo update -p plnk --manifest-path $manifest
    if ($LASTEXITCODE -ne 0) {
        Write-Host 'cargo update -p plnk failed; falling back to cargo check to refresh the lock file.'
        & cargo check --manifest-path $manifest
        if ($LASTEXITCODE -ne 0) {
            throw 'Could not refresh plnk/Cargo.lock after the version bump.'
        }
    }
}
finally {
    Pop-Location
}

if (-not (Test-Path -LiteralPath $lock -PathType Leaf)) {
    throw "Cargo lock file is missing after the bump: $lock"
}
$lockText = [IO.File]::ReadAllText($lock)
if ($lockText -notmatch ('(?m)^name = "plnk"\r?\nversion = "' + [regex]::Escape($Version) + '"')) {
    throw "plnk/Cargo.lock does not record version $Version; check Cargo."
}
Write-Host "plnk/Cargo.lock now records plnk $Version."

Write-Host ''
Write-Host 'Next steps:'
Write-Host "  1. Commit the bump: git add plnk/Cargo.toml plnk/Cargo.lock; git commit"
Write-Host '  2. cargo test --manifest-path plnk/Cargo.toml'
Write-Host '  3. cargo build --release --locked --manifest-path plnk/Cargo.toml'
Write-Host '  4. powershell -ExecutionPolicy Bypass -File scripts/package-plnk.ps1'
Write-Host '  5. powershell -ExecutionPolicy Bypass -File scripts/verify-plnk-package.ps1'
