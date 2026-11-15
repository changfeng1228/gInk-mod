# gInk 1.1.1 + 截图钉图升级 — 项目边界说明

本目录是 gInk 的**源码工作副本**，在上游 gInk 1.1.1 基础上增加了「截图钉图」功能。
实际运行的程序不在本目录，见下方「运行实例」。

## 目录职责

| 路径 | 职责 | 是否入库 |
|---|---|---|
| `src/` | gInk 源码（C# WinForms，namespace gInk）。`PinForm.cs` 为本次升级新增 | 是 |
| `src/Properties/`、`src/resources/`、`*.resx` | 程序集信息与窗体资源 | 是 |
| `gInk.sln`、`readme.md`、`changelog.txt`、`license.txt`、`screenshot*.jpg` | 上游工程与文档 | 是 |
| `build/` | **构建工作区**：`sources.rsp`（csc 响应文件）、`res/`（预编译 .resources）、`ExtractResources.cs` + `extract.exe`（资源提取）、`mk_rsp.py`、`verify_res.py`，产物 `gInk.exe` | 否（上游 .gitignore 已忽略） |
| `build/_scratch/` | 升级过程中留下的一次性调试脚本、窗口抓取日志、界面截图、资源新旧备份。**不属于项目代码**，可随时删除 | 否 |
| `bin/` | 上游发布目录结构（依赖 dll、icon、lang、开发用 ini）。**其中的 ini 是开发时的配置，不是用户实际配置** | exe/dll/ini 已忽略 |

## 两个 gInk 副本的关系（勿混淆）

| 位置 | 身份 | 现状 |
|---|---|---|
| `D:\DSHXM\gInk-mod` | 源码工程（本目录） | 一切改动的源头 |
| `D:\Download\gInk\gInk_v1.1.1` | **运行实例**（开机自启指向这里） | `gInk.exe` 与 `build/gInk.exe` md5 相同（9868bbf7…），配置为用户实际使用中的版本 |

改动流程：改 `src/` → csc 按 `build/sources.rsp` 编译出 `build/gInk.exe` → 复制到运行实例目录覆盖 `gInk.exe` → 重启 gInk。

运行实例目录里的特殊文件：

- `gInk.exe.orig-1.1.1` — 上游原版备份（无钉图功能），仅供对照/回滚。
- `__open_options.disabled` — 原名 `__open_options`。该标记文件存在时，源码 `Root.cs` 的 TEST HOOK 会在启动 1.2 秒后弹出「选项」窗口（每次开机自启都弹）。已重命名以关闭弹窗；改回原名即恢复该行为。
- `crash.txt` — 一次「以管理员权限启动」导致的崩溃记录：Microsoft.Ink COM 激活失败 `0x8007007E`（发生在 `Root.SetDefaultPens()`）。普通权限启动不受影响，**不要用管理员身份运行 gInk**。

## 本次升级改动（相对上游 gInk 1.1.1）

- 新增 `src/PinForm.cs`：截图钉图窗口（拖动、缩放、透明度、置顶、复制、保存、关闭）。
- `src/Root.cs`：`Pins` 集合、`OnSnapPin`、`SnapToPin`、`SnapToPinFromDesktop`、`CloseAllPins`、`Hotkey_SnapPin`、`AddPin`、`RebuildTrayMenu`、`SetThreadDpiAwarenessContext`、`Log`/`DebugLog`。
- `src/Hotkey.cs`：`KeyToString` 等热键显示相关。
- `src/Local.cs`：钉图右键菜单与提示文案（`PinMenu*`、`PinWindowTitle` 等）。
- `src/FormOptions.*`：钉图热键设置项与图标开关。
- `Root.cs` 中 `TestMessageFilter`：新增第二全局热键（id=1）→ `SnapToPinFromDesktop()`。

## 动画帧率优化（2026-11-15）

高刷新率屏幕上工具栏滑入/退出动画掉帧。根因：动画逻辑与画面呈现各由 `System.Windows.Forms.Timer`（15ms）驱动，实测回调约 52–64 次/秒；即便 `timeBeginPeriod(1)` + Interval=4，WM_TIMER 与 `System.Threading.Timer` 仍被系统合并到约 15ms。

- 新增 `src/HighResolutionTimer.cs`：后台调度线程 + `BeginInvoke` 回 UI 线程，忙帧不排队；仅活动期启用 1ms 计时分辨率。
- 工具栏动画改为绝对时间缓动（入场约 300ms，快速退出约 160ms），去掉逐帧累加与 120px 钳制。
- 定时器活动期 4ms、静止恢复 15ms；截图冷却与自动滚动改为按真实时间计时，保持原行为速度。
- 回归测试：`tests\run-animation-tests.ps1`（动画回调帧率、入口位移平滑度、截图冷却、静止降频、退出后定时器释放）。
- 部署前的旧 exe 备份：运行目录下 `gInk.exe.bak-before-highfps-1115`。

## 已知遗留

- `src/Root.cs:166-180` 保留着 TEST HOOK（调试期用于启动即打开选项窗口）。目前靠运行目录的标记文件被禁用；若重新编译部署后仍出现开机弹窗，检查运行目录是否又出现了 `__open_options`。
- 本仓库 2026-11-03 之前没有任何提交，上游改动无法逐行 diff。当前提交为现状基线，此后的改动才有历史可查。
