[CmdletBinding()]
param(
    [string]$Version = '0.1.0',
    [string]$Destination = [Environment]::GetFolderPath('Desktop')
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repo = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $repo "plnk\target\release\plnk.exe"
$assets = Join-Path $repo "plnk\assets\icons"
$license = Join-Path $repo 'license.txt'
foreach ($required in @($exe, $assets, $license)) {
    if (-not (Test-Path -LiteralPath $required)) {
        throw "Required build input is missing: $required. Run cargo build --release --locked --manifest-path plnk/Cargo.toml first."
    }
}

$versionInfo = (Get-Item -LiteralPath $exe).VersionInfo
$expectedFileVersion = "$Version.0"
if ($versionInfo.FileVersion -ne $expectedFileVersion -or
    $versionInfo.ProductVersion -ne $expectedFileVersion -or
    $versionInfo.ProductName -ne 'plnk' -or
    $versionInfo.OriginalFilename -ne 'plnk.exe') {
    throw "Release VERSIONINFO does not match $expectedFileVersion / plnk.exe; rebuild before packaging."
}

if (-not (Test-Path -LiteralPath $Destination -PathType Container)) {
    throw "Desktop destination does not exist: $Destination"
}
$name = "plnk-v$Version-win64"
$package = Join-Path $Destination $name
$zip = "$package.zip"
$hadPreviousPackage = Test-Path -LiteralPath $package -PathType Container
if ((Test-Path -LiteralPath $package) -and -not $hadPreviousPackage) {
    throw "Refusing to replace an unrelated file: $package"
}
if ($hadPreviousPackage) {
    $recognisedPackage = (Test-Path -LiteralPath (Join-Path $package 'plnk.exe') -PathType Leaf) -or
        (Test-Path -LiteralPath (Join-Path $package 'VERSION') -PathType Leaf)
    if (-not $recognisedPackage) {
        throw "Refusing to replace an unrelated directory: $package"
    }
    Remove-Item -LiteralPath $package -Recurse -Force
}
if (Test-Path -LiteralPath $zip) {
    if (-not $hadPreviousPackage) {
        throw "Refusing to replace a ZIP without its earlier package folder: $zip"
    }
    if (-not (Test-Path -LiteralPath $zip -PathType Leaf)) {
        throw "Refusing to replace an unrelated directory: $zip"
    }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [IO.Compression.ZipFile]::OpenRead($zip)
    try {
        $hasExe = @($archive.Entries | Where-Object { $_.Name -eq 'plnk.exe' }).Count -gt 0
        $hasVersion = @($archive.Entries | Where-Object { $_.Name -eq 'VERSION' }).Count -gt 0
    }
    finally { $archive.Dispose() }
    if (-not $hasExe -or -not $hasVersion) {
        throw "Refusing to replace an unrelated ZIP: $zip"
    }
    Remove-Item -LiteralPath $zip -Force
}

New-Item -ItemType Directory -Path (Join-Path $package 'assets\icons') -Force | Out-Null
Copy-Item -LiteralPath $exe -Destination (Join-Path $package 'plnk.exe')
Copy-Item -Path (Join-Path $assets '*') -Destination (Join-Path $package 'assets\icons') -Recurse
Copy-Item -LiteralPath $license -Destination (Join-Path $package 'license.txt')

$utf8 = New-Object System.Text.UTF8Encoding($true)
$versionText = @"
plnk $Version
Windows 64-bit portable
FileVersion=$expectedFileVersion
Built from the gInk MIT-licensed asset set (Weizhi Nai).
"@
[IO.File]::WriteAllText((Join-Path $package 'VERSION'), $versionText + [Environment]::NewLine, $utf8)

$readme = @"
plnk $Version（Windows 64 位便携版）
====================================

使用
1. 双击 plnk.exe。通知区域会显示原版红色 plnk 图标；左键单击它进入标注模式。
2. 按住鼠标左键绘制。工具栏可选择画笔颜色、撤销、清空；工具栏最右侧 X 或 Esc 退出标注并返回托盘。
3. 右键单击托盘图标，选择 Exit 退出程序。Esc 只退出标注，不会退出整个程序。
4. 笔宽按钮为三条斜线。点击后在上方滑块拖动，松开关闭；第一次 Esc 关闭滑块，第二次 Esc 退出标注。
   若配置未显示该按钮，可用 plnk.exe --pen-width 临时启用。
5. 为避免与正在运行的旧版 gInk 抢全局热键，可使用：
   plnk.exe --no-hotkeys
   也可组合：plnk.exe --no-hotkeys --pen-width

当前版本
- 版本号 0.1.0；exe 已嵌入 FileVersion/ProductVersion 0.1.0.0 和原版红色图标。
- 工具栏使用原版 gInk PNG 位图，托盘使用 icon_red.ico；White_Tray_Icon=True 时使用 icon_white.ico。
- 笔宽面板使用原版 penwidthpanel.png；无需安装，不修改启动项。
- 资源位于 exe 旁的 assets\icons。缺少资源时记录警告并回退，不会因此崩溃。
- 可写时配置和 plnk.log 位于 exe 旁 data；否则使用 %LOCALAPPDATA%\plnk。

尚未实现（后续任务）
选项窗口、31 种语言界面、截图/固定、橡皮擦/指针/平移工具、自定义热键。
本版本是可绘制、可调笔宽的早期便携包，不应视为完整 gInk 功能替代品。

许可
原版 gInk 图像和许可来自 Weizhi Nai 的 MIT 项目；完整 MIT 许可及版权见 license.txt。
"@
[IO.File]::WriteAllText((Join-Path $package 'README-zh.txt'), $readme + [Environment]::NewLine, $utf8)

$requiredFiles = @(
    'plnk.exe', 'license.txt', 'README-zh.txt', 'VERSION',
    'assets\icons\icon_red.ico', 'assets\icons\icon_white.ico',
    'assets\icons\pencil1.png', 'assets\icons\pencil_act.png',
    'assets\icons\penwidth.png', 'assets\icons\penwidthpanel.png'
)
foreach ($relative in $requiredFiles) {
    if (-not (Test-Path -LiteralPath (Join-Path $package $relative) -PathType Leaf)) {
        throw "Package verification failed; missing $relative"
    }
}

Compress-Archive -LiteralPath $package -DestinationPath $zip -CompressionLevel Optimal
$assetCount = (Get-ChildItem -LiteralPath (Join-Path $package 'assets\icons') -File).Count
if ($assetCount -ne 29) {
    throw "Unexpected packaged asset count: $assetCount (expected 29)"
}
[pscustomobject]@{
    Package = $package
    Zip = $zip
    Version = (Get-Item -LiteralPath (Join-Path $package 'plnk.exe')).VersionInfo.FileVersion
    Assets = $assetCount
    ExeSHA256 = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $package 'plnk.exe')).Hash
    ZipSHA256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $zip).Hash
} | Format-List
