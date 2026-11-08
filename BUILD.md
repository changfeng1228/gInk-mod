# 构建说明

gInk 是 .NET Framework WinForms 程序，用 Windows 自带的 C# 编译器（`csc.exe`）构建，
不需要 Visual Studio、不需要 NuGet、不需要联网。

## 快速开始

```powershell
.\build\build.ps1
```

产物在 `build\out\`，是一个可直接运行的完整目录。

## 依赖

| 项 | 说明 |
|---|---|
| Windows 10/11 | 需要 GDI+ 与 Microsoft Ink COM 组件 |
| .NET Framework 4.x | 用 `C:\Windows\Microsoft.NET\Framework\v4.0.30319\csc.exe` |
| `Microsoft.Ink.dll` | **不在仓库里**（不可再分发），见下 |

### Microsoft.Ink.dll 从哪来

它随 Windows 平板/手写组件提供，也存在于任何一个已安装的 gInk 1.1.1 目录里。
构建脚本按顺序查找：

1. `bin\Microsoft.Ink.dll`
2. `D:\Download\gInk\gInk_v1.1.1\Microsoft.Ink.dll`
3. `%CommonProgramFiles%\Microsoft Shared\Ink\InkObj.dll`

都不存在时显式指定：

```powershell
.\build\build.ps1 -InkDll 'C:\path\to\Microsoft.Ink.dll'
```

### build\res\ 里的 .resources

编译时要用 `-resource:` 嵌入 8 个 `.resources`（窗体布局、图标位图集合）。
它们**已随仓库提供**，所以克隆下来就能直接编译。

只有在这些文件丢失或需要重新生成时才执行：

```powershell
.\build\extract-resources.ps1 -SourceExe 'C:\path\to\gInk.exe.orig-1.1.1'
```

脚本会用 `build\ExtractResources.cs` 编译出 `extract.exe`，再从指定的 exe 里提取。
任何 gInk 1.1.1 版本的 exe 都可以作为来源，这些资源在 1.1.1 各构建间一致。

## 产物结构

`build\out\` 就是运行所需的完整布局：

```
gInk.exe                 ← 主程序
Microsoft.Ink.dll        ← 手写笔组件
icon_red.ico             ← 托盘图标（红）
icon_white.ico           ← 托盘图标（白）
config.ini               ← 通用设置
config_default.ini       ← 默认设置（首次运行的模板）
pens.ini                 ← 笔刷设置
hotkeys.ini              ← 热键设置
lang\                    ← 界面语言文件（31 种）
```

程序从**自己所在目录**读取 ini 和 lang，所以部署就是整目录拷贝。

## 部署

```powershell
# 停掉正在运行的实例
Get-Process gInk -ErrorAction SilentlyContinue | Stop-Process -Force

# 备份后替换
Copy-Item 'D:\DSHXM\gInk-mod\build\out\gInk.exe' 'D:\Download\gInk\gInk_v1.1.1\gInk.exe' -Force
```

> 只替换 `gInk.exe`，**不要覆盖** `config.ini` / `pens.ini` / `hotkeys.ini`，
> 否则用户的笔刷、热键、语言设置会被重置。

## 关于可复现性

本项目的可复现指**功能可复现**，不是字节级可复现：

- `csc.exe`（.NET Framework 4.x 自带）**不支持 `-deterministic`** 开关。实测每次编译
  产物相差约 44 字节，位于 PE 时间戳与模块 MVID，属于编译器行为而非代码差异。
- 相同源码 + 相同源文件顺序 + 相同资源，产物**大小完全一致**（414,208 字节），
  且所有字符串常量偏移量逐一相同 —— 可以用这一点核对构建是否走的是同一份输入。

核对方法：

```powershell
(Get-Item build\out\gInk.exe).Length      # 应为 414208
```

## 已知约束

### 启动方式影响 Ink COM 激活

实测结论：

- 从 **explorer** 或快捷方式启动 → 正常
- 从某些 shell 进程树直接启动 → `Root.SetDefaultPens()` 处崩溃，
  `0x8007007E`（Ink COM 类工厂激活失败）

这是宿主进程上下文导致的，与 exe 本身无关：同一个 exe 换启动方式就能跑。
部署后请用快捷方式或双击启动，不要从脚本里直接 `& .\gInk.exe` 拉起。

### DPI

gInk 主程序以 DPI 非感知运行（见 `src\gInk.exe.manifest` 的 `<dpiAware>false`），
但**贴图窗口**在创建时临时切到 per-monitor-v2 感知上下文，以保证一图像素对一物理像素。
`PinForm.DpiScope` 负责这件事；所有窗口几何的读写都必须走它，否则在 125%/150%
缩放的显示器上会出现写入被缩放、读取不缩放的不一致。详见 `src\PinForm.cs` 顶部注释。

### 单实例

`Program.cs` 用命名互斥量 `Global\86280230-c3d2-4da0-8621-e2a2466fc136` 保证单实例。
用 `-define:GINK_DEV_INSTANCE` 编译时改用 `...137`，可以让开发版和正式版并存
（开发版同时应换用不同热键，否则两者会抢注册）。**发布构建不要带这个 define。**

## 目录职责

| 路径 | 内容 | 入库 |
|---|---|---|
| `src\` | 全部 C# 源码 | 是 |
| `bin\lang\` | 界面语言文件 | 是 |
| `bin\*.ico` | 托盘图标 | 是 |
| `bin\*.ini` | 配置模板（开发用） | **否** |
| `build\res\` | 编译所需的 .resources | 是 |
| `build\*.ps1` `build\*.py` | 构建/资源脚本 | 是 |
| `build\out\` | 构建产物 | 否 |
| `build\_scratch\` | 一次性调试脚本与日志 | 否 |
| `release\` | 本地发布副本 | 否 |

## 源码结构

| 文件 | 职责 |
|---|---|
| `Program.cs` | 入口、单实例锁、全局异常处理 |
| `Root.cs` | 主控制器：托盘图标、热键注册、绘制模式、贴图集合 |
| `FormDisplay.cs` | 全屏画布、笔画渲染、截图 |
| `FormCollection.cs` | 工具栏与按钮 |
| `FormOptions.*` | 选项对话框（通用/笔刷/热键三页） |
| `PinForm.cs` | **贴图窗口**：拖动、角部缩放、滚轮缩放、透明度、复制/保存 |
| `Hotkey.cs` | 热键解析与显示 |
| `Local.cs` | 多语言字段与加载（反射按字段名覆盖） |
| `FormButtonHitter.cs` | 悬浮按钮点击区 |
| `FormAbout.*` | 关于对话框 |

## 超出上游 1.1.1 的改动

见 `NOTES.md`。主要包括：

- 截图钉图（`PinForm.cs` + `Root` 的 `Pins` / `AddPin` / `SnapToPinFromDesktop`）
- 贴图角部拖拽缩放、滚轮缩放、右键倍率菜单
- 贴图窗口的 DPI 一致性修复（`DpiScope`）
