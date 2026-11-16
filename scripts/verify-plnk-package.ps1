[CmdletBinding()]
param(
    [string]$Package = '',
    [string]$EvidenceDirectory = (Join-Path $env:TEMP 'plnk-package-evidence')
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$repo = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($Package)) {
    $manifest = Join-Path $repo 'plnk\Cargo.toml'
    $manifestText = [IO.File]::ReadAllText($manifest)
    $packageSection = [regex]::Match($manifestText, '(?ms)^\[package\]\s*$.*?(?=^\[|\z)')
    $manifestVersion = [regex]::Match($packageSection.Value, '(?m)^\s*version\s*=\s*"([^"]+)"')
    if (-not $packageSection.Success -or -not $manifestVersion.Success) {
        throw "Could not read the [package] version from $manifest"
    }
    $Package = Join-Path ([Environment]::GetFolderPath('Desktop')) "plnk-v$($manifestVersion.Groups[1].Value)-win64"
}
$Package = (Resolve-Path -LiteralPath $Package).Path

$versionFile = Join-Path $Package 'VERSION'
if (-not (Test-Path -LiteralPath $versionFile -PathType Leaf)) {
    throw "Package VERSION file missing: $versionFile"
}
$versionText = [IO.File]::ReadAllText($versionFile)
$versionMatch = [regex]::Match($versionText, '(?m)^plnk (\d+\.\d+\.\d+)\s*$')
if (-not $versionMatch.Success) {
    throw "Package VERSION does not start with 'plnk x.y.z': $versionFile"
}
$Version = $versionMatch.Groups[1].Value
if ($versionText -notmatch '(?m)^Commit=\S+\s*$') {
    throw "Package VERSION has no Commit= line; it cannot be traced to a Git commit: $versionFile"
}
if ($versionText -match '(?m)^Commit=unknown\s*$') {
    Write-Warning 'Package VERSION records Commit=unknown; Git traceability is not proven.'
}

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

public static class PlnkPackageNative {
    [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr awarenessContext);

    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left, Top, Right, Bottom; }

    [StructLayout(LayoutKind.Sequential)]
    public struct NOTIFYICONIDENTIFIER {
        public int cbSize;
        public IntPtr hWnd;
        public uint uID;
        public Guid guidItem;
    }

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern IntPtr FindWindowW(string className, string windowName);
    [DllImport("user32.dll")]
    public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr hWnd);
    [DllImport("user32.dll")]
    public static extern uint GetDpiForWindow(IntPtr hWnd);
    [DllImport("user32.dll")]
    public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")]
    public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extraInfo);
    [DllImport("user32.dll")]
    public static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extraInfo);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern IntPtr SendMessageW(IntPtr hWnd, uint message, IntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern bool PostMessageW(IntPtr hWnd, uint message, IntPtr wParam, IntPtr lParam);
    [DllImport("shell32.dll")]
    public static extern int Shell_NotifyIconGetRect(ref NOTIFYICONIDENTIFIER identifier, out RECT rect);
}
'@

if (-not [PlnkPackageNative]::SetProcessDpiAwarenessContext([IntPtr]::new(-4))) {
    throw 'Could not enable per-monitor-v2 DPI awareness in the verifier'
}

function Wait-PlnkWindow([string]$Class, [string]$Title, [int]$TimeoutMs = 10000, [bool]$RequireVisible = $true) {
    $deadline = [Environment]::TickCount + $TimeoutMs
    do {
        $hwnd = [PlnkPackageNative]::FindWindowW($Class, $Title)
        if ($hwnd -ne [IntPtr]::Zero -and (-not $RequireVisible -or [PlnkPackageNative]::IsWindowVisible($hwnd))) {
            return $hwnd
        }
        Start-Sleep -Milliseconds 100
    } while ([Environment]::TickCount -lt $deadline)
    throw "Window did not become visible: $Class / $Title"
}

function Get-PlnkRect([IntPtr]$Hwnd) {
    $rect = New-Object PlnkPackageNative+RECT
    if (-not [PlnkPackageNative]::GetWindowRect($Hwnd, [ref]$rect)) {
        throw "GetWindowRect failed for $Hwnd"
    }
    return $rect
}

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class PlnkCaptureNative {
    [DllImport("user32.dll")] public static extern IntPtr GetDC(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern int ReleaseDC(IntPtr hWnd, IntPtr hDC);
    [DllImport("gdi32.dll")] public static extern bool BitBlt(IntPtr destination, int x, int y, int width, int height, IntPtr source, int sourceX, int sourceY, uint operation);
}
'@

function Capture-PlnkWindow([IntPtr]$Hwnd, [string]$Path) {
    $rect = Get-PlnkRect $Hwnd
    Capture-PlnkRegion $rect.Left $rect.Top ($rect.Right - $rect.Left) ($rect.Bottom - $rect.Top) $Path
}

function Capture-PlnkRegion([int]$Left, [int]$Top, [int]$Width, [int]$Height, [string]$Path) {
    if ($Width -le 0 -or $Height -le 0) { throw "Invalid capture bounds" }
    $bitmap = New-Object Drawing.Bitmap $Width, $Height
    $screen = [IntPtr]::Zero
    $graphics = $null
    $destination = [IntPtr]::Zero
    try {
        $screen = [PlnkCaptureNative]::GetDC([IntPtr]::Zero)
        if ($screen -eq [IntPtr]::Zero) { throw 'GetDC failed' }
        $graphics = [Drawing.Graphics]::FromImage($bitmap)
        $destination = $graphics.GetHdc()
        $operation = [uint32]0x00CC0020 -bor [uint32]0x40000000
        if (-not [PlnkCaptureNative]::BitBlt($destination, 0, 0, $Width, $Height, $screen, $Left, $Top, $operation)) {
            throw 'BitBlt(SRCCOPY|CAPTUREBLT) failed'
        }
        $graphics.ReleaseHdc($destination)
        $destination = [IntPtr]::Zero
        $bitmap.Save($Path, [Drawing.Imaging.ImageFormat]::Png)
    }
    finally {
        if ($null -ne $graphics) {
            if ($destination -ne [IntPtr]::Zero) { $graphics.ReleaseHdc($destination) }
            $graphics.Dispose()
        }
        if ($screen -ne [IntPtr]::Zero) { [PlnkCaptureNative]::ReleaseDC([IntPtr]::Zero, $screen) | Out-Null }
        $bitmap.Dispose()
    }
}

function Invoke-PlnkClick([int]$X, [int]$Y, [bool]$Right = $false) {
    [PlnkPackageNative]::SetCursorPos($X, $Y) | Out-Null
    Start-Sleep -Milliseconds 80
    if ($Right) {
        [PlnkPackageNative]::mouse_event(0x0008, 0, 0, 0, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 50
        [PlnkPackageNative]::mouse_event(0x0010, 0, 0, 0, [UIntPtr]::Zero)
    }
    else {
        [PlnkPackageNative]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 60
        [PlnkPackageNative]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
    }
    Start-Sleep -Milliseconds 250
}

function Invoke-PlnkDrag([int]$X1, [int]$Y1, [int]$X2, [int]$Y2) {
    [PlnkPackageNative]::SetCursorPos($X1, $Y1) | Out-Null
    Start-Sleep -Milliseconds 80
    [PlnkPackageNative]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
    for ($step = 1; $step -le 20; $step++) {
        $x = $X1 + [int](($X2 - $X1) * $step / 20)
        $y = $Y1 + [int](($Y2 - $Y1) * $step / 20)
        [PlnkPackageNative]::SetCursorPos($x, $y) | Out-Null
        Start-Sleep -Milliseconds 12
    }
    [PlnkPackageNative]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 350
}

function Invoke-PlnkKey([byte]$Key) {
    [PlnkPackageNative]::keybd_event($Key, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 50
    [PlnkPackageNative]::keybd_event($Key, 0, 2, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 150
}

function Assert-PlnkNoDialog {
    $dialog = [PlnkPackageNative]::FindWindowW('#32770', 'plnk')
    if ($dialog -ne [IntPtr]::Zero) { throw 'A plnk error dialog is visible' }
}

function Get-PlnkRedPixels([Drawing.Bitmap]$Bitmap) {
    $count = 0
    for ($y = 0; $y -lt $Bitmap.Height; $y++) {
        for ($x = 0; $x -lt $Bitmap.Width; $x++) {
            $pixel = $Bitmap.GetPixel($x, $y)
            if ($pixel.A -gt 100 -and $pixel.R -gt 140 -and $pixel.G -lt 130 -and $pixel.B -lt 130) { $count++ }
        }
    }
    return $count
}

$exe = Join-Path $Package 'plnk.exe'
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { throw "Packaged exe missing: $exe" }
if (Get-Process -Name plnk -ErrorAction SilentlyContinue) {
    throw 'Another plnk instance is already running; refusing to disturb it.'
}
New-Item -ItemType Directory -Path $EvidenceDirectory -Force | Out-Null
$toolbarShot = Join-Path $EvidenceDirectory 'toolbar.png'
$drawingShot = Join-Path $EvidenceDirectory 'drawing.png'
$penWidthShot = Join-Path $EvidenceDirectory 'pen-width.png'
$trayShot = Join-Path $EvidenceDirectory 'tray-icon.png'
$referenceTrayShot = Join-Path $EvidenceDirectory 'reference-icon-red.png'
$trayMenuShot = Join-Path $EvidenceDirectory 'tray-menu.png'

$log = Join-Path $Package 'data\plnk.log'
$logStart = if (Test-Path -LiteralPath $log -PathType Leaf) {
    (Get-Item -LiteralPath $log).Length
}
else { 0 }

$process = $null
$shell = [IntPtr]::Zero
$verified = $false
try {
    $process = Start-Process -FilePath $exe -WorkingDirectory $Package -PassThru -ArgumentList @(
        '--no-hotkeys', '--draw', '--pen-width', '--exit-after-ms', '120000'
    )
    $overlay = Wait-PlnkWindow 'plnk_gate_window' 'plnk overlay'
    $toolbar = Wait-PlnkWindow 'plnk_gate_window' 'plnk toolbar'
    $shell = Wait-PlnkWindow 'plnk_app_shell_window' 'plnk' 10000 $false
    Start-Sleep -Milliseconds 700
    Assert-PlnkNoDialog
    if ($process.HasExited) { throw "Packaged app exited early: $($process.ExitCode)" }

    $iconHandle = [PlnkPackageNative]::SendMessageW($shell, 0x007F, [IntPtr]::Zero, [IntPtr]::Zero)
    if ($iconHandle -eq [IntPtr]::Zero) {
        $iconHandle = [PlnkPackageNative]::SendMessageW($shell, 0x007F, [IntPtr]::new(1), [IntPtr]::Zero)
    }
    if ($iconHandle -eq [IntPtr]::Zero) { throw 'The shell has no legacy window/tray HICON' }
    $icon = [Drawing.Icon]::FromHandle($iconHandle)
    $iconBitmap = $icon.ToBitmap()
    try {
        $iconBitmap.Save($trayShot, [Drawing.Imaging.ImageFormat]::Png)
        $actualRed = Get-PlnkRedPixels $iconBitmap
    }
    finally { $iconBitmap.Dispose(); $icon.Dispose() }
    $referenceIcon = [Drawing.Icon]::new((Join-Path $Package 'assets\icons\icon_red.ico'), 32, 32)
    $referenceBitmap = $referenceIcon.ToBitmap()
    try {
        $referenceBitmap.Save($referenceTrayShot, [Drawing.Imaging.ImageFormat]::Png)
        $referenceRed = Get-PlnkRedPixels $referenceBitmap
    }
    finally { $referenceBitmap.Dispose(); $referenceIcon.Dispose() }
    if ($actualRed -le 0 -or $referenceRed -le 0) {
        throw "Legacy red icon proof failed (actual=$actualRed reference=$referenceRed)"
    }

    Capture-PlnkWindow $toolbar $toolbarShot
    Invoke-PlnkDrag 400 400 700 470
    Capture-PlnkRegion 300 300 500 260 $drawingShot
    Assert-PlnkNoDialog

    $toolbarRect = Get-PlnkRect $toolbar
    $height = $toolbarRect.Bottom - $toolbarRect.Top
    $button = 0.88 * $height
    # Defaults + --pen-width: five pens, eraser/pan/pointer, then pen width.
    $penWidthX = $toolbarRect.Left + [int](12.1 * $button)
    $penWidthY = $toolbarRect.Top + [int](0.51 * $height)
    Invoke-PlnkClick $penWidthX $penWidthY
    $panel = Wait-PlnkWindow 'plnk_gate_window' 'plnk pen width' 3000
    Capture-PlnkWindow $panel $penWidthShot
    $panelRect = Get-PlnkRect $panel
    $scale = [PlnkPackageNative]::GetDpiForWindow($panel) / 96.0
    Invoke-PlnkDrag ($panelRect.Left + [int](120 * $scale)) ($panelRect.Top + [int](40 * $scale)) ($panelRect.Left + [int](180 * $scale)) ($panelRect.Top + [int](40 * $scale))
    if ([PlnkPackageNative]::IsWindowVisible($panel)) { throw 'Pen-width panel did not close on release' }

    Invoke-PlnkKey 0x1B
    $deadline = [Environment]::TickCount + 5000
    while ([PlnkPackageNative]::IsWindowVisible($overlay) -and [Environment]::TickCount -lt $deadline) {
        Start-Sleep -Milliseconds 100
    }
    if ([PlnkPackageNative]::IsWindowVisible($overlay)) { throw 'Esc did not leave annotation mode' }
    Assert-PlnkNoDialog

    # The icon can live in the notification overflow, where its rectangle is
    # not clickable. Deliver the same tray callback as Explorer, causing the
    # application to open its real TrackPopupMenu at a visible cursor position.
    $menuX = [Math]::Max(100, $toolbarRect.Left - 300)
    $menuY = [Math]::Max(100, $toolbarRect.Top - 300)
    [PlnkPackageNative]::SetCursorPos($menuX, $menuY) | Out-Null
    if (-not [PlnkPackageNative]::PostMessageW($shell, 0x8014, [IntPtr]::Zero, [IntPtr]::new(0x007B))) {
        throw 'Could not deliver the tray context-menu callback'
    }
    Start-Sleep -Milliseconds 500
    Capture-PlnkRegion ([Math]::Max(0, $menuX - 100)) ([Math]::Max(0, $menuY - 100)) 500 400 $trayMenuShot
    Invoke-PlnkKey 0x28
    Invoke-PlnkKey 0x0D
    if (-not $process.HasExited) { Wait-Process -Id $process.Id -Timeout 10 -ErrorAction Stop }
    $process.WaitForExit()
    if ($process.ExitCode -ne 0) { throw "Tray Exit returned $($process.ExitCode)" }

    if (-not (Test-Path -LiteralPath $log -PathType Leaf)) { throw "Packaged log missing: $log" }
    $stream = [IO.File]::OpenRead($log)
    try {
        if ($stream.Length -lt $logStart) { throw 'Log rotated during verification; rerun with a fresh package' }
        $stream.Position = $logStart
        $reader = [IO.StreamReader]::new($stream)
        try { $logText = $reader.ReadToEnd() }
        finally { $reader.Dispose() }
    }
    finally { $stream.Dispose() }
    foreach ($expected in @(
        "plnk $Version starting", 'tray icon added (legacy icon_red.ico ICO)',
        'original toolbar bitmaps loaded: 19/19', 'original pen-width panel bitmap loaded: true',
        'ink committed: 1 document strokes', 'global pen width:', 'leaving annotation: Escape',
        'exit requested: tray menu Exit', 'shutdown complete', 'plnk exited normally'
    )) {
        if (-not $logText.Contains($expected)) { throw "Log evidence missing: $expected" }
    }
    if ($logText.Contains('could not be loaded') -or $logText.Contains('fatal:')) {
        throw 'Asset fallback or fatal error was logged; see plnk.log'
    }

    $result = [pscustomobject]@{
        Status = 'PASS'
        Package = $Package
        Pid = $process.Id
        ExitCode = $process.ExitCode
        ActualRedPixels = $actualRed
        ReferenceRedPixels = $referenceRed
        ToolbarScreenshot = $toolbarShot
        DrawingScreenshot = $drawingShot
        PenWidthScreenshot = $penWidthShot
        TrayScreenshot = $trayShot
        Log = $log
        VerifiedAt = [DateTime]::Now.ToString('o')
    }
    $result | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $EvidenceDirectory 'verification.json') -Encoding UTF8
    $result | Format-List
    $verified = $true
}
finally {
    if ($null -ne $process -and -not $process.HasExited) {
        # If verification failed with the tray popup open, close that nested
        # modal menu before requesting the shell's normal Exit command.
        Invoke-PlnkKey 0x1B
        if ($shell -ne [IntPtr]::Zero) {
            [PlnkPackageNative]::PostMessageW($shell, 0x0110, [IntPtr]::new(1), [IntPtr]::Zero) | Out-Null
            Start-Sleep -Milliseconds 800
        }
        if (-not $process.HasExited) {
            $process.Kill()
            $process.WaitForExit()
        }
    }
    if (-not $verified) { Write-Warning 'Package verification did not complete; see the exception and evidence directory.' }
}
