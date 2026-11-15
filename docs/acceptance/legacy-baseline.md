# 旧版行为与性能基线（Task 1）

**状态：** 已固化（2026-11-15）  
**旧版：** gInk 1.1.1（C# WinForms，namespace `gInk`）+ 当前工作树的截图贴图与动画修改  
**作用：** 记录默认值、状态转换、已知缺陷和旧版性能数据，供 Task 2–14 引用。矩阵中的 `P-xxx` 是功能项；本文中的 `D-xxx` 是缺陷，不是兼容要求；`B-xxx` 是测量记录。

## 1. 测量环境

| 项 | 值 | 来源 |
|---|---|---|
| 操作系统 | Windows 11 家庭中文版，版本 10.0.26200 | `Win32_OperatingSystem`（2026-11-15 查询） |
| 架构 | x86_64；旧版进程为 x86（`PlatformTarget=x86`） | `src/gInk.csproj:24,37` |
| 显示器 | 单显示器 `\\.\DISPLAY1`，主屏，逻辑 1707×1067 | `System.Windows.Forms.Screen.AllScreens`（2026-11-15 查询） |
| DPI | 150% | spec §1；与 1707×1067 逻辑分辨率一致 |
| 刷新率 | 240Hz | spec §4.2 与用户描述；旧版未做正式可见帧测量 |
| 输入设备 | 鼠标 | 当前设备范围（spec §4.1）；无压感笔/触摸设备 |
| 显示器数量 | 1 | 查询结果；实体多显示器项目标“未验证” |
| 旧版运行时 | .NET Framework 3.5、x86、Microsoft.Ink COM 6.1 | `src/gInk.csproj:12,24,51-53`; `src/App.config` |

**当前设备范围：** Windows 11、x86_64、鼠标、单显示器、150% DPI、240Hz。压感笔、触摸和实体多显示器只能做自动/模拟覆盖，不得声称已通过（spec §4.1）。

## 2. 旧版身份与构建

| 项 | 记录 |
|---|---|
| 源码 | 仓库 `src/` 的 gInk 1.1.1 工作树；含未提交的截图贴图与动画修改（HEAD `11c05ba` 加工作树改动） |
| 构建 | `csc.exe`（.NET Framework 4.x 编译器，目标 `v3.5`）；`build/build.ps1`；产物 `build/out/gInk.exe` |
| 产物大小 | 414,208 字节（工作树构建，含贴图与动画修改；BUILD.md:96-102） |
| 可复现性 | 功能可复现，非字节级可复现（csc 不支持 `-deterministic`；BUILD.md:90-102） |
| 单实例 | 命名互斥量 `Global\86280230-c3d2-4da0-8621-e2a2466fc136`；开发构建用 `...137` |
| 配置目录 | 程序从当前目录优先、其次 exe 目录读取 `config.ini`、`pens.ini`、`hotkeys.ini`；`config_default.ini` 只是安装模板，程序不读取 |
| 语言目录 | 当前目录或 exe 目录下的 `lang/`，共 31 个 `.txt` |
| 截图目录 | 默认 `%USERPROFILE%/Pictures/gInk/`（仅当配置值与默认字符串完全相同才自动创建） |
| 运行实例 | 用户本机的旧版运行目录只读参考；不在本仓库内，本文不记录其绝对路径 |
| 工作树边界 | 不改动、不提交、不 stash/reset 现有 C# 修改；本文与 fixtures 只读旧版 |

## 3. 性能基线

### 3.1 已记录数据

| ID | 指标 | 数值 | 条件 | 可信度与说明 |
|---|---|---|---|---|
| B-001 | 托盘空闲工作集 | 约 37.9 MB | 托盘空闲，单显示器，150% DPI | spec §1 的单次测量；不是稳定基准 |
| B-002 | 托盘空闲私有内存 | 约 17.6 MB | 同上 | spec §1 的单次测量；不是稳定基准 |
| B-003 | 环境 | 逻辑 1707×1067、150% DPI、240Hz、单显示器 | 同上 | spec §1/§4.2；2026-11-15 查询确认分辨率与单屏 |
| B-004 | 旧版 exe 大小 | 414,208 字节 | 工作树构建（含贴图/动画） | BUILD.md:96-102；仅作旧版上下文，不是新程序目标 |
| B-005 | 动画回调历史 | 优化前约 52–64 次/秒；`WM_TIMER` 常被合并到约 15.6ms；`HighResolutionTimer` 活动期 4ms、静止 15ms | 工具栏滑入/退出 | NOTES.md 与 `src/HighResolutionTimer.cs` 注释；**回调频率不是可见帧率**，不得拿来当 240Hz 证据 |
| B-006 | 冷启动到托盘可用 | 未测 | — | 旧版没有正式测量；不得编造 |
| B-007 | 托盘无操作 CPU | 未测 | — | 旧版没有正式测量 |
| B-008 | 书写模式私有内存 | 未测 | — | 旧版没有正式测量 |
| B-009 | 240Hz 可见帧间隔 | 未测 | — | 旧版没有 DWM/屏幕捕获时间戳 |
| B-010 | Pointer 到提交延迟 | 未测 | — | 旧版没有探针 |
| B-011 | 100 次开关/泄漏 | 未测 | — | 旧版未做正式 soak |

### 3.2 关键告诫

- B-001/B-002 是**单次测量**，不能当作稳定基准或验收阈值；正式验收必须在同一台机器上重新测量（spec §1）。
- 不得用动画回调次数、定时器间隔或 `timeBeginPeriod` 设置替代可见呈现帧率；Task 3/13 必须采集 DWM/屏幕捕获时间戳（spec §4.2）。
- 旧版的全屏 GDI 画布与逐帧 `UpdateLayeredWindow` 上传是架构限制（D-020），不是可供新程序继承的性能模型。
- 新程序目标（spec §4.2）在下表单独列出，**不是旧版基线**，也不是本文的测量结果。

### 3.3 新程序目标（spec §4.2，供 Task 13 使用）

| 指标 | 目标 | 测量方式 |
|---|---:|---|
| 托盘空闲私有内存 | ≤ 25 MB | 30 秒稳定后取进程私有字节 |
| 托盘空闲工作集 | ≤ 35 MB | 同一采样窗口 |
| 书写模式私有内存（不含显存） | ≤ 50 MB | 1000 条混合笔画后 |
| Release exe | ≤ 8 MB（语言资源外置时） | 文件大小 |
| 冷启动到托盘可用 | ≤ 150 ms | 20 次冷启动取 p95 |
| 托盘无操作 CPU | ≤ 0.5% | 30 秒平均值 |
| 动画期间应用 CPU | ≤ 2% 单个逻辑核心 | CPU 时间 ÷ 动画墙钟时间；同时记录原始百分比 |
| 240Hz 可见动画帧间隔 | 中位 ≤ 4.2ms，p95 ≤ 8.4ms | 240fps 捕获/DWM 时间戳 |
| Pointer 消息到提交绘制 | 中位 ≤ 2ms，p95 ≤ 4ms | Release 性能探针；不含数字化仪硬件延迟 |
| 资源泄漏 | 100 次开关后句柄/线程/内存回到预热基线 ±5%；另做 20 次冷启动 | 自动化 soak 与冷启动 |

### 3.4 复测旧版基线的方法

1. 用当前工作树构建或已部署的同一版本，确保只有一个实例在运行。
2. 启动到托盘，等待 30 秒稳定；期间不使用鼠标热键、不打开批注。
3. 用 `Get-Process -Name gInk` 采样 `WorkingSet64` 与 `PrivateMemorySize64`，至少 3 次，记录时间与进程 ID。
4. 同时记录 OS 版本、显示器分辨率/DPI/刷新率、是否连接第二屏、正在运行的其他应用。
5. 结果写入 Task 13 的验收记录；不得把单次读数写成稳定基准。旧版 B-001/B-002 只作为对照起点。

## 4. 默认值

### 4.1 `pens.ini` 出厂值（`bin/pens.ini`）

| 笔 | Enabled | RGB | Alpha（不透明度） | Width | 备注 |
|---|---:|---|---:|---:|---|
| Pen0 | False | 80,80,80 | 255 | 80 | 深灰；代码默认同为 False |
| Pen1 | True | 225,60,60 | 255 | 80 | 红；`CurrentPen=1`/`LastPen=1` |
| Pen2 | True | 30,110,200 | 255 | 80 | 蓝 |
| Pen3 | True | 250,180,55 | 255 | 80 | 琥珀；**代码缺文件默认是 235,180,55** |
| Pen4 | True | 120,175,70 | 255 | 80 | 绿 |
| Pen5 | True | 235,125,15 | 80 | 500 | 橙色荧光笔（内部 Transparency=175） |
| Pen6 | False | 230,230,230 | 255 | 80 | 白 |
| Pen7 | False | 240,140,200 | 255 | 80 | 粉；**代码缺文件默认是 250,140,200** |
| Pen8 | False | 25,180,175 | 255 | 80 | 青 |
| Pen9 | False | 145,70,160 | 80 | 500 | 紫色荧光笔（内部 Transparency=175） |

补充：

- `PenN_Enabled` 在选项页的语义是“显示按钮”（`OptionsPensShow`），不是“禁用”；隐藏的笔仍可用热键选择（P-036）。
- `_Alpha` 是 0–255 的不透明度；读入后 `Transparency = 255 − Alpha`。`Transparency ≥ 100` 使用荧光笔图标（P-045）。
- 代码缺文件默认在 `Root.SetDefaultPens()`（`src/Root.cs:475-532`）中；它先于读取文件执行。因此 Pens 3/7 的颜色在“文件缺失”和“出厂 pens.ini”之间不同。

### 4.2 `config.ini` 出厂值 vs 代码缺文件默认

| 键 | 出厂 `config_default.ini` | 代码缺文件/缺键默认 | 解析规则 |
|---|---|---|---|
| `Language_File` | `en-us` | 未设置；`Local` 字段为英文 | 字符串；触发 `ChangeLanguage` |
| `White_Tray_Icon` | False | false | False/0/OFF 关闭；其他保持 |
| `Snapshot_Path` | `%USERPROFILE%/Pictures/gInk/` | 同左 | 字符串；末尾自动补 `/` 或 `\` |
| `Eraser_Icon` | True | true | 仅 False/0/OFF 关闭 |
| `Pan_Icon` | True | true | 仅 False/0/OFF 关闭 |
| `Pointer_Icon` | True | true | 仅 False/0/OFF 关闭 |
| `Pen_Width_Icon` | False | false | True/1/ON 开启；False/0/OFF 关闭 |
| `InkVisible_Icon` | True | true | 仅 False/0/OFF 关闭 |
| `Snapshot_Icon` | True | true | 仅 False/0/OFF 关闭 |
| `Undo_Icon` | True | true | 仅 False/0/OFF 关闭 |
| `Clear_Icon` | True | true | 仅 False/0/OFF 关闭 |
| `Canvas_Cursor` | 0 | 0 | 0=箭头，1=笔尖；其他值忽略 |
| `Allow_Dragging_Toolbar` | False | **true** | 仅 False/0/OFF 关闭；**与出厂值不同** |
| `Allow_Hotkey_In_Pointer_Mode` | False | **true** | 仅 False/0/OFF 关闭；**与出厂值不同** |
| `Toolbar_Left` | 0 | 0 | int；0,0 会触发默认位置重定位 |
| `Toolbar_Top` | 0 | 0 | int；同上 |
| `Close_On_Snap` | `BlankOnly` | `"blankonly"`（字段默认） | True/False/BlankOnly，大小写不敏感 |
| `Always_Hide_Toolbar` | False | false | 仅 True/1/ON 开启 |
| `Pin_On_Snap` | True | **未解析** | 旧版注释存在但 `ReadOptions` 无 case；见 D-005，不得导入为行为 |
| `Toolbar_Height` | 0.05 | **0.06**（字段默认） | float，InvariantCulture；**与出厂值不同** |
| `AutoScroll` | 不存在 | false | 无 INI/UI 入口；见 D-008 |

补充：

- 出厂模板由 `bin/config_default.ini` 提供；`build/build.ps1:148-163` 在干净安装时把它复制到 `config.ini`。程序自身只读 `config.ini`。
- 行尾：`bin/config_default.ini` 与 `bin/hotkeys.ini` 为 LF；`bin/config.ini` 与 `bin/pens.ini` 为 CRLF。解析器都接受；fixtures 需保持所选文件的原始行尾。
- 旧版 `SaveOptions` 只更新文件中**已经存在**的键；不会创建缺失文件，也不会补写缺失键。当前用户配置里没有 `Pin_On_Snap`，程序也不会自动补上。
- `SaveOptions` 保留注释和未知行，但会用 `Name= value` 重写已知键，并用 `StreamWriter.WriteLine` 统一行尾；见 D-015。
- 工具栏默认位置：当 `Toolbar_Left/Top` 为 0,0 或四角不在可见屏幕内时，使用主屏工作区右下角，并留 15px 底边距（`src/FormCollection.cs:233-260`）。

### 4.3 `hotkeys.ini` 出厂值 vs 代码缺文件默认

| 键 | 出厂 `hotkeys.ini` | 代码缺文件/缺键默认 |
|---|---|---|
| `Hotkey_Global` | `Ctrl + Alt + G` | `Ctrl + Alt + G` |
| `Hotkey_Eraser` | `E` | None |
| `Hotkey_InkVisible` | `V` | None |
| `Hotkey_Pointer` | `None` | None |
| `Hotkey_Pan` | `None` | None |
| `Hotkey_Undo` | `Ctrl + Z` | None |
| `Hotkey_Redo` | `Ctrl + Y` | None |
| `Hotkey_Snapshot` | `Ctrl + Alt + S` | None |
| `Hotkey_SnapPin` | `Ctrl + Alt + Shift + P` | `Ctrl + Alt + Shift + P` |
| `Hotkey_Clear` | `None` | None |
| `Pen0_Hotkey` … `Pen9_Hotkey` | `0` … `9` | None |

补充：

- 只有 `Hotkey_Global`（注册 id 0）和 `Hotkey_SnapPin`（id 1）用 `RegisterHotKey` 注册；其余工具热键在批注期间由 `GetKeyState` 轮询（P-013）。
- 缺少 `hotkeys.ini` 时，只有全局热键和贴图热键可用；这不是出厂配置的等价行为（P-166）。
- 当前用户的实际热键与出厂值不同；本文不记录个人配置值。fixtures 可保留结构合法的自定义热键字符串，但不得包含个人路径或截图。
- 热键解析的旧缺陷：F10–F12 无法从 INI 解析（D-001）、数字键盘显示成控制字符（D-002）、非法值会清掉修饰键但保留旧主键（D-003）。新程序必须修正，不能照抄。

### 4.4 其他派生默认值

| 项 | 默认 | 说明 |
|---|---|---|
| `CurrentPen` / `LastPen` | 1 / 1 | 启动时若 Pen1 未显示，选第一个已显示笔；全无则 `-2` 指针模式 |
| `GlobalPenWidth` | 80 | 全局笔宽面板的值；不写入 INI，重启回到 80 |
| `InkVisible` | true | `StartInk` 时强制显示 |
| `EraserMode` / `PointerMode` / `PanMode` | false / false / false | 每次启动 |
| `Docked` | false（初始） | `StartInk` 不重置；同进程内退出再进入会保留上次停靠状态 |
| `PinTopMost` | true | 所有贴图共享；不持久化 |
| `SnapToPin` | false | 仅“截图并贴图”入口置 true，捕获后复位 |
| `CloseOnSnap` | `blankonly` | 字段默认；出厂键 `BlankOnly` |
| `AlwaysHideToolbar` | false | 出厂键 False |
| `ToolbarHeight` | 0.06（代码）/ 0.05（出厂） | 见 4.2 |
| `DebugLog` | true | 旧版默认写 `gInk_debug.log`；见 D-013 |

### 4.5 语言资源（`bin/lang/`）

- 文件数：31 个 `.txt`：`ar, as, bg, bn-bd, cz, de, en-us, es-latam, fa, fr, hd, he, hr, hu, id-in, it, ja-jp, kn, ko-kr, lt, pl, pt-br, pt, ro, ru-ru, sk, th, tr, vi-vn, zh-cn, zh-tw`。
- 编码：全部为合法 UTF-8；`cz.txt` 与 `pl.txt` 带 UTF-8 BOM，其余无 BOM。
- 加载方式：`Local` 的公开字段默认英文；`LoadLocalFile` 按字段名反射覆盖；`ButtonNamePen0`…`ButtonNamePen9` 单独解析；首个 `LanguageName` 行用于下拉列表显示名。
- 缺键：文件没有的键保留英文默认值，不报错。贴图新增键目前只存在于 `en-us` 与 `zh-cn`，其余 29 种语言回退英文（P-116）。
- 已知键名不一致：语言文件使用 `MenuEntryPinOnSnap`，`Local` 字段是 `MenuEntrySnapPin`，因此托盘菜单该项在所有语言下都可能显示英文（D-006）。
- 列表顺序：`DirectoryInfo.GetFiles("*.txt")` 未排序，语言下拉顺序不稳定（D-016）。

## 5. 状态转换

### 5.1 状态定义

| 状态 | 判定 | 说明 |
|---|---|---|
| `TrayIdle` | `FormDisplay == null && FormCollection == null` | 只有托盘图标 |
| `InkDrawing` | 两个窗体非空 | 全屏覆盖层 + 工具栏；子状态为笔/橡皮/指针/平移 |
| `SnappingWaiting` | `Root.Snapping == 1` | 等待第一次按下 |
| `SnappingDragging` | `Root.Snapping == 2` | 拖拽选区 |
| `SnappingCooldown` | `Root.Snapping` 在 -60..-1 | 退出截图后的冷却倒计时 |
| `OptionsOpen` | `FormOptions != null` | 仅 `TrayIdle` 可进入 |
| `Docked` | `Root.Docked == true` | 与模式正交；只影响工具栏位置 |
| `Pins` | `Root.Pins.Count > 0` | 与批注模式正交的贴图窗口集合 |

### 5.2 主转换表

| # | 起始 | 触发 | 结果 | 源码 |
|---|---|---|---|---|
| T-01 | `TrayIdle` | 托盘左键 | 重读 `pens.ini`/`config.ini`/`hotkeys.ini`，`StartInk()` | `src/Root.cs:190-204` |
| T-02 | `TrayIdle` | 全局热键 | `StartInk()` | `src/Root.cs:23-48` |
| T-03 | `InkDrawing` + 指针模式 | 全局热键 | `SelectPen(LastPen)` 回到上次笔 | `src/Root.cs:38-39` |
| T-04 | `InkDrawing` + 非指针 | 全局热键 | `SelectPen(-2)` 进入指针模式 | `src/Root.cs:43` |
| T-05 | `InkDrawing` | 笔按钮/数字热键 | `SelectPen(k)`：应用笔属性、退出橡皮/指针/平移、恢复全窗口输入 | `src/FormCollection.cs:633-749` |
| T-06 | `InkDrawing` | 橡皮按钮/E | `SelectPen(-1)`：`EditingMode.Delete`、橡皮图标 | `src/FormCollection.cs:603-631,668-699` |
| T-07 | `InkDrawing` | 指针按钮/热键 | `Root.Pointer()`：`WS_EX_TRANSPARENT`，显示 `FormButtonHitter` | `src/Root.cs:447-468`；`src/FormCollection.cs:580-601` |
| T-08 | `InkDrawing` | 平移按钮/热键 | `SelectPen(-3)`：输入矩形 1×1，`PanMode=true` | `src/FormCollection.cs:633-656` |
| T-09 | `InkDrawing` | 退出按钮/ESC（无更高优先级） | `RetreatAndExit()`：穿透、清笔迹、存 Undo、保存 `config.ini`、160ms 滑出、`StopInk()` | `src/FormCollection.cs:751-762,875-884`；`src/Root.cs:242-258` |
| T-10 | `InkDrawing` | 截图按钮/热键 | `SnappingWaiting`；十字光标；输入矩形 1×1；工具栏收起；退出指针模式 | `src/FormCollection.cs:818-849` |
| T-11 | `SnappingWaiting` | 鼠标按下 | 记录起点，`SnappingDragging` | `src/FormCollection.cs:451-476` |
| T-12 | `SnappingDragging` | 鼠标移动 | 更新选区；仅鼠标移动时重绘遮罩 | `src/FormCollection.cs:479-507`；`src/FormDisplay.cs:504-515` |
| T-13 | `SnappingDragging` | 鼠标松开 | 任一边 <5px → 全屏；否则矩形（加窗体原点）；`UponTakingSnap=true`，`ExitSnapping()` | `src/FormCollection.cs:509-527` |
| T-14 | `SnappingCooldown` | 定时 tick | 按 60fps 步进把 `Snapping` 从 -60 推到 0 | `src/FormCollection.cs:1225-1234` |
| T-15 | `SnappingCooldown` + `UponTakingSnap` | 定时 tick | 全屏时 `Sleep(200)`；清画布、画笔迹、更新、捕获、按需贴图、复位 `SnapToPin`、执行 CloseOnSnap、清 `UponTakingSnap` | `src/FormDisplay.cs:477-502` |
| T-16 | 捕获成功 | — | 尝试复制剪贴板；保存 PNG；记录最新路径；置 `UponBalloonSnap` | `src/FormDisplay.cs:338-360` |
| T-17 | `InkDrawing` | 停靠按钮 | `Docked=true`；工具栏滑出只留 dock 按钮；关闭笔宽面板 | `src/Root.cs:424-434`；`src/FormCollection.cs:768-786` |
| T-18 | `Docked` + 托盘左键 | — | `UnDock()`；工具栏滑入 | `src/Root.cs:201-203,436-445` |
| T-19 | `InkDrawing` + 隐藏笔迹 | 落笔/光标按下/Undo | 自动 `SetInkVisible(true)` | `src/FormCollection.cs:441-444,468-471,1432-1444` |
| T-20 | `InkDrawing` | 新笔画/清空/平移结束/退出 | `SaveUndoStrokes()`；新笔画清 `RedoDepth` | `src/FormCollection.cs:417-437,529-531,1420-1430` |
| T-21 | `InkDrawing` | Undo/Redo | 从环形快照恢复整文档；重绘全部；隐藏时先显示 | `src/Root.cs:357-422` |
| T-22 | `TrayIdle` | 托盘 Options | 注销两个全局热键；打开三页选项 | `src/Root.cs:1019-1031`；`src/FormOptions.cs:33-35` |
| T-23 | `OptionsOpen` | 关闭 | 重注册热键；保存 `pens.ini`/`config.ini`/`hotkeys.ini` | `src/FormOptions.cs:290-299` |
| T-24 | `TrayIdle`/`InkDrawing` | 贴图热键/托盘项 | 必要时先 `StartInk()`；置 `SnapToPin=true`；进入截图；捕获后 `AddPin()` | `src/Root.cs:338-349,1068-1085`；`src/FormDisplay.cs:486-489` |
| T-25 | 任意 | 托盘 Exit | 注销热键、关闭所有贴图、释放托盘、退出进程 | `src/Root.cs:1094-1101` |
| T-26 | `StopInk` | — | 关闭窗体、`GC.Collect()`、清空窗体引用；若 `UponBalloonSnap` 则显示气泡；Undo 历史保留 | `src/Root.cs:242-258` |

### 5.3 截图状态机的关键约束

- `Snapping > 0` 时普通工具热键被禁用；ESC 单独处理。`Snapping == 0` 时 ESC 才执行退出（P-018）。
- 全屏判定基于 `SnappingRect.Width == this.Width && Height == this.Height`，不是单独的标志；用户正好拖出全屏尺寸也会触发 200ms 等待。
- 选区坐标加的是窗体左上角（虚拟屏原点）；随后按主屏 `GetDeviceCaps` 的物理/逻辑比例缩放（P-092）。
- 捕获时只画笔迹、不画工具栏；隐藏笔迹时笔迹不进入截图（P-097）。
- `CloseOnSnap` 在捕获后执行：`true` 总是退出；`blankonly` 只在 `Ink.Strokes.Count == 0` 时退出；`false` 保持批注。
- 保存目录不存在时 `SnapShot` 返回 null；`CloseOnSnap` 仍可能执行，且不会显示气泡（D-019）。
- 连续 ESC 只取消当前截图/关闭笔宽面板，不会在冷却期再次退出（P-098）。

### 5.4 撤销/重做状态

- 历史是 8 槽环形缓冲；初始化时槽 0 保存空文档，`UndoDepth=0`、`UndoP=0`。
- 每次新笔画把 `RedoDepth` 清零；`UndoDepth` 最大 7；`UndoP` 递增并回绕。
- `Clear` 先删除全部笔画，再 `SaveUndoStrokes()`，因此清空本身可撤销。
- 平移结束、退出批注也保存快照；退出时保存的是清空后的空文档，所以重开后第一次 Undo 恢复退出前笔迹。
- 退出/重开只保留同进程历史；不写磁盘（P-057）。
- 旧版擦除不新增快照（D-004）；新程序按 Task 6 的编辑命令实现，不复制折叠行为。

### 5.5 贴图状态

- `PinForm` 是独立无边框顶层窗口；共享 `Root.PinTopMost`，默认 true。
- 左键拖动移动；四角 20px 热区按对角锚定等比缩放；滚轮 ×/÷1.1 以左上角为锚；Alt+滚轮改透明度（0.2–1.0，步进 0.05）；Alt 按住点击穿透。
- 右键菜单：复制、另存为、放大、缩小、原始大小、半透明、置顶、关闭、关闭所有。
- 双击（显示后 >400ms 且不在角部）或 Esc 关闭；Ctrl+C 复制；关闭时从 `Root.Pins` 移除。
- 缩放范围 0.05–8×，且最短边不小于 24px；透明度下限 0.2。
- 贴图窗口的坐标读写全部走 `DpiScope`（per-monitor-v2），保证 1 图像像素 = 1 物理像素。

## 6. 已知旧缺陷（不得复制为兼容要求）

| ID | 症状 | 旧版源码 | 新程序期望 |
|---|---|---|---|
| D-001 | F10–F12 无法从 INI 解析：`Hotkey.Parse` 只看倒数第二个字符是否为 `F`；UI 可录入但重启后丢失 | `src/Hotkey.cs:82-87`；`src/Hotkey.cs:17-28`；`changelog.txt` v1.2.0 | 支持 F1–F12 的解析/显示/保存往返 |
| D-002 | NumPad 热键显示成控制字符/反引号，配置文件不可读 | `src/Hotkey.cs:30-44` | 使用稳定、可读、可往返的名称 |
| D-003 | 非法热键值清空修饰键但保留旧主键，可能产生意外裸键热键 | `src/Hotkey.cs:75-105` | 整个热键回退到默认/无，并带键名警告 |
| D-004 | 擦除不新增撤销快照；多次擦除折叠为一次 Undo | `src/FormCollection.cs:417-437,603-631` | 按编辑命令实现擦除；不复制折叠行为 |
| D-005 | `Pin_On_Snap` 在 `config.ini` 注释中存在，但 `ReadOptions` 无 case；普通截图从不贴图 | `bin/config.ini:65-68`；`src/Root.cs:658-777` | 不导入该键为行为；普通截图与“截图并贴图”入口分离（P-173） |
| D-006 | 语言键 `MenuEntryPinOnSnap` 与 `Local.MenuEntrySnapPin` 不一致，托盘菜单回退英文 | `src/Local.cs:62`；`bin/lang/en-us.txt:58` | 统一键名；缺键回退英文并记录 |
| D-007 | `INKVISIBLE_ICON` 保存时误用 `PanEnabled` | `src/Root.cs:961-966` | 用 `InkVisibleEnabled` 正确保存 |
| D-008 | `AutoScroll` 只有代码字段，无 INI/UI 入口，发布版不可达 | `src/Root.cs:67,547`；`src/FormDisplay.cs:575` | 默认关闭、无新增配置/UI，仅在指针模式实现并由自动测试覆盖（P-085） |
| D-009 | 截图 BitBlt 失败路径生成蓝图并继续保存；红色初始填充可能泄漏；文件保存无异常处理 | `src/FormDisplay.cs:321-336,352-363` | 明确错误、不保存伪图、不阻塞 UI |
| D-010 | 全屏截图 `Sleep(200)` 阻塞 UI 线程 | `src/FormDisplay.cs:479-480` | 不复制；用有界异步/等待帧方案 |
| D-011 | 混合 DPI/多显示器已知问题：主屏 `GetDeviceCaps` 统一缩放、工具栏命中偏移；无 `WM_DISPLAYCHANGE` 处理 | `src/FormDisplay.cs:306-318`；`readme.md:33` | 统一物理坐标/显示器拓扑；自动测试覆盖（Task 12） |
| D-012 | 全屏层高度 -2、工具栏用主屏几何和 15px 底边距等启发式 | `src/FormDisplay.cs:64-65`；`src/FormCollection.cs:57-58,216-255` | 按物理坐标与显示器拓扑实现；不复制 hack |
| D-013 | `DebugLog` 默认 true，写 exe 同级 `gInk_debug.log`，无大小上限 | `src/Root.cs:113,273-283` | 写新程序自有数据目录，有界滚动（spec §6.4） |
| D-014 | TEST HOOK `__open_options` 在标记文件存在时启动 1.2s 后弹选项 | `src/Root.cs:166-180` | 不复制开发钩子 |
| D-015 | `SaveOptions` 不创建缺失文件、不补写缺失键；只重写已有键 | `src/Root.cs:783-1006` | 新配置原子保存；旧文件只读导入（Task 2/9） |
| D-016 | 语言文件枚举顺序由 `DirectoryInfo.GetFiles` 决定，未排序；无效 `Language_File` 静默回退 | `src/Local.cs:100-118,151-158` | 稳定排序；缺键/无效文件回退英文并记录 |
| D-017 | 全局热键注册失败不提示（只有贴图热键写日志） | `src/Root.cs:1045-1055` | 冲突/失败必须报告（Task 4） |
| D-018 | 配置查找先当前目录、后 exe 目录；从不同 cwd 启动可能读到不同配置 | `src/Root.cs:576-580` | 新程序独立数据目录；旧配置显式只读导入 |
| D-019 | 自定义截图目录不存在时返回 null；`CloseOnSnap` 仍可能退出且无气泡；保存异常未处理 | `src/FormDisplay.cs:298-304,360-363,493-501` | 明确错误；不因失败误关闭；不死锁 |
| D-020 | 全屏 `UpdateLayeredWindow` 每帧上传整屏位图，工具栏动画靠 4ms 定时器回调 | `src/FormDisplay.cs:434-457`；`src/FormCollection.cs:1010-1094` | 新架构用 D2D 脏矩形 + DComp；不复制全屏上传 |
| D-021 | 托盘“Close all pinned images”启用状态只在构造/关闭所有/切语言时刷新；新增贴图后仍可能为灰色 | `src/Root.cs:325-336` 对比 `285-312,244-248` | 贴图增删后刷新菜单状态 |
| D-022 | 笔尖光标每次 `bitmaptip.GetHicon()` 创建图标句柄，未 `DestroyIcon`；频繁切换/改宽可能累积 GDI 句柄 | `src/FormCollection.cs:969-1007` | 缓存/释放光标资源；Task 13 泄漏测试覆盖 |
| D-023 | 形如 `PEN10_RED` 的键会因 `Substring(3,1)` 被当成 Pen1 | `src/Root.cs:610-613` | 只接受 Pen0–Pen9；非法笔号回退并警告 |
| D-024 | `GlobalPenWidth` 不写入 INI，重启回到 80 | `src/Root.cs:130-131` | 作为运行期状态；若需持久化另行设计，不默认继承 |

## 7. 未验证与延期

| 项目 | 状态 | 原因与边界 |
|---|---|---|
| 压感笔压力、合并输入、橡皮端（P-027/P-028） | 未验证 | 当前无笔设备；自动/模拟测试必须通过；不得用鼠标模拟冒充“笔已验收” |
| 触摸（P-026） | 未验证 | 当前无触摸屏；自动/模拟测试必须通过 |
| 实体混合 DPI 显示器（P-149） | 未验证 | 只有一块显示器；混合 DPI 坐标自动测试通过，但实体屏幕未验收 |
| 实体多显示器（P-157） | 未验证 | 只有一块显示器；拓扑/负坐标自动测试通过，但截图、工具栏、指针、贴图的实体多屏未验收 |
| `Pin_On_Snap` 自动贴图（P-173） | 延期 | 旧版键未接线；Task 10 明确不采用；用户已接受 |
| Windows 10 | 未验证/非发布门 | 保留 1607 API 下限设计，但不增加 Win10 VM 或旧系统发布门（spec §3） |
| 31 种语言视觉布局 | 部分人工 | 31 个文件解析自动测试；当前只人工检查中文/英文/阿拉伯文布局；其余语言依赖缺键回退与自动解析 |
| 旧版 240Hz 可见帧/延迟/冷启动 | 未测 | B-006–B-011 无可靠旧版数据；新程序由 Task 3/13 测量 |

## 8. 脱敏、证据与边界

- 旧版运行目录只读参考；不写入、不复制旧配置、不改启动项。
- fixtures 和本文不包含：用户名、绝对用户路径、机器名、邮箱、截图、剪贴板图像、崩溃转储、调试日志内容。
- 路径统一写成 `%USERPROFILE%/...` 或 `<redacted>/...`；仓库内路径写成 `src/...`、`bin/...`、`docs/...`。
- 当前用户配置的实际热键值不记录在文档中；fixtures 如需自定义热键，保留结构合法的非身份值。
- 证据来源：`src/` 工作树、`bin/` 出厂模板、`docs/superpowers/` 计划与设计、`BUILD.md`/`NOTES.md`/`readme.md`/`changelog.txt`。未使用截图作为证据。
- 现有 C# 工作树修改保持不动；本任务不执行任何会改变 git 状态的命令。

## 9. 变更记录

| 日期 | 变更 | 说明 |
|---|---|---|
| 2026-11-15 | 创建 | Task 1 固化默认值、状态转换、缺陷 D-001–D-024、性能 B-001–B-011 |
