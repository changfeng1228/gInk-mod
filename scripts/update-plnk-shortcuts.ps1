# Points the plnk Startup shortcut (and the Desktop shortcut when it already
# points at a packaged release) at the given plnk package. package-plnk.ps1
# calls this automatically after every package build, so the autostart entry
# always follows the newest version. Run it manually only after moving or
# deleting a package folder. It never copies packages or zips anywhere.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$PackagePath,
    [switch]$Desktop,
    [switch]$Startup
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repo = Split-Path -Parent $PSScriptRoot

if (-not (Test-Path -LiteralPath $PackagePath -PathType Container)) {
    throw "Package folder does not exist: $PackagePath"
}
$package = (Get-Item -LiteralPath $PackagePath).FullName
$exe = Join-Path $package 'plnk.exe'
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) {
    throw "plnk.exe is missing from the package: $exe"
}

# The package VERSION is authoritative; the versioned folder name is only a
# fallback so a manually assembled package can still be pointed at.
$versionFile = Join-Path $package 'VERSION'
$version = ''
if (Test-Path -LiteralPath $versionFile -PathType Leaf) {
    $versionMatch = [regex]::Match([IO.File]::ReadAllText($versionFile), '(?m)^plnk (\d+\.\d+\.\d+)\s*$')
    if ($versionMatch.Success) {
        $version = $versionMatch.Groups[1].Value
    }
}
if ([string]::IsNullOrEmpty($version)) {
    $nameMatch = [regex]::Match((Split-Path -Leaf $package), '^plnk-v(\d+\.\d+\.\d+)-win64$')
    if ($nameMatch.Success) {
        $version = $nameMatch.Groups[1].Value
    }
}
if ([string]::IsNullOrEmpty($version)) {
    throw "Could not determine the plnk version from $versionFile or the package folder name."
}

$description = "plnk $version"
$icon = "$exe,0"
$shell = New-Object -ComObject WScript.Shell

function Get-ShortcutInfo([string]$Path) {
    $shortcut = $shell.CreateShortcut($Path)
    return [pscustomobject]@{
        Path = $Path
        TargetPath = $shortcut.TargetPath
        WorkingDirectory = $shortcut.WorkingDirectory
        Description = $shortcut.Description
        IconLocation = $shortcut.IconLocation
        WindowStyle = $shortcut.WindowStyle
    }
}

function Test-ReleaseShortcut([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return $false }
    $target = ''
    try {
        $target = $shell.CreateShortcut($Path).TargetPath
    }
    catch {
        return $false
    }
    if ([string]::IsNullOrWhiteSpace($target)) { return $false }
    # Only shortcuts that already point into this repo's release\plnk-v*
    # tree may be retargeted; user shortcuts are never touched.
    $normalized = $target -replace '/', '\'
    $releaseRoot = (Join-Path $repo 'release') -replace '/', '\'
    $pattern = '^' + [regex]::Escape($releaseRoot) + '\\plnk-v[^\\]+(\\|$)'
    return [regex]::IsMatch($normalized, $pattern, [Text.RegularExpressions.RegexOptions]::IgnoreCase)
}

function Update-PlnkShortcut([string]$Path) {
    try {
        $shortcut = $shell.CreateShortcut($Path)
        $shortcut.TargetPath = $exe
        $shortcut.WorkingDirectory = $package
        $shortcut.Description = $description
        $shortcut.IconLocation = $icon
        $shortcut.WindowStyle = 7
        $shortcut.Save()
    }
    catch {
        throw "Could not write shortcut ${Path}: $($_.Exception.Message)"
    }
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Shortcut was not created: $Path"
    }
    $info = Get-ShortcutInfo $Path
    $problems = @()
    if (-not [string]::Equals($info.TargetPath, $exe, [StringComparison]::OrdinalIgnoreCase)) {
        $problems += "target '$($info.TargetPath)' (expected '$exe')"
    }
    if (-not [string]::Equals($info.WorkingDirectory, $package, [StringComparison]::OrdinalIgnoreCase)) {
        $problems += "working directory '$($info.WorkingDirectory)' (expected '$package')"
    }
    if ($info.Description -ne $description) {
        $problems += "description '$($info.Description)' (expected '$description')"
    }
    if (-not [string]::Equals($info.IconLocation, $icon, [StringComparison]::OrdinalIgnoreCase)) {
        $problems += "icon '$($info.IconLocation)' (expected '$icon')"
    }
    if ($info.WindowStyle -ne 7) {
        $problems += "window style $($info.WindowStyle) (expected 7)"
    }
    if ($problems.Count -gt 0) {
        throw ("Shortcut read-back failed for ${Path}: " + ($problems -join '; '))
    }
    Write-Host "Shortcut ${Path} -> $($info.TargetPath)"
    return $info
}

$updateDesktop = [bool]$Desktop
$updateStartup = [bool]$Startup
if (-not $updateDesktop -and -not $updateStartup) {
    $updateDesktop = $true
    $updateStartup = $true
}

if ($updateStartup) {
    $startupDirectory = [Environment]::GetFolderPath('Startup')
    if ([string]::IsNullOrWhiteSpace($startupDirectory)) {
        throw 'The Startup folder is not available for this user.'
    }
    if (-not (Test-Path -LiteralPath $startupDirectory -PathType Container)) {
        New-Item -ItemType Directory -Path $startupDirectory -Force | Out-Null
    }
    Update-PlnkShortcut (Join-Path $startupDirectory 'plnk.lnk') | Out-Null
}

if ($updateDesktop) {
    $desktopDirectory = [Environment]::GetFolderPath('Desktop')
    if ([string]::IsNullOrWhiteSpace($desktopDirectory) -or
        -not (Test-Path -LiteralPath $desktopDirectory -PathType Container)) {
        throw "The Desktop folder is not available: '$desktopDirectory'"
    }
    $existing = @()
    foreach ($candidateName in @('plnk.lnk', 'gInk.lnk')) {
        $candidate = Join-Path $desktopDirectory $candidateName
        if (Test-Path -LiteralPath $candidate -PathType Leaf) {
            $existing += $candidate
        }
    }
    if ($existing.Count -eq 0) {
        Update-PlnkShortcut (Join-Path $desktopDirectory 'plnk.lnk') | Out-Null
    }
    else {
        $retargeted = $false
        foreach ($candidate in $existing) {
            if (Test-ReleaseShortcut $candidate) {
                Update-PlnkShortcut $candidate | Out-Null
                $retargeted = $true
                break
            }
        }
        if (-not $retargeted) {
            Write-Host 'Desktop shortcut left untouched (it does not point into release\plnk-v*):'
            foreach ($candidate in $existing) {
                Write-Host "  ${candidate} -> $((Get-ShortcutInfo $candidate).TargetPath)"
            }
        }
    }
}
