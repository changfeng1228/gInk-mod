# plnk 项目交接文档

> 写于 2026-11-16，对应 `feature/plnk-rust` 分支 `7c9b769`（0.2.1）。
> 本文只写**核实过的事实**。凡未经实测的，明确标注「未验证」。

---

## 1. 这个项目是什么

`gInk` 是一款 Windows 屏幕批注工具（上课/演示时在屏幕上画线、截图、贴图）。原版是 C# WinForms + Microsoft.Ink。

`plnk` 是它的 **Rust 原生重写**：同一套工作流，但用 Win32 + Direct2D + DirectComposition 直接绘制，目标是更低的资源占用和显示器原生刷新率（240Hz）的动画。

**仓库里同时存在两套实现**，不要混淆：

| 实现 | 路径 | 语言 | 状态 |
|---|---|---|---|
| **plnk**（现行） | `plnk/` | Rust 1.100 MSVC | 开发中，**功能未对齐** |
| gInk（旧版） | `src/`、`gInk.sln`、`build/` | C# WinForms | 功能完整的参照实现 |

根目录 `readme.md` / `BUILD.md` / `NOTES.md` 描述的是**旧版 C#**，开头已加定位表说明这一点。

---

## 2. 当前状态（一句话）

**能日常书写批注并截图外部，但没有截图和贴图功能。**

从 0.1.5 到 0.2.1 的实际进展：

- ✅ 画笔光标对齐修复
- ✅ 设置窗口（热键绑定 + 开机自启）
- ✅ 工具热键接通（E 橡皮 / V 隐藏 / Ctrl+Z / Ctrl+Y / 数字选笔）
- ❌ **截图功能没有**（Task 10）
- ❌ **贴图功能没有**（Task 11）
- ❌ 选项窗口只有最小功能（无笔刷颜色/宽度编辑、无多语言界面）

### 版本与产物

| 项 | 值 |
|---|---|
| 版本 | `0.2.1`（唯一来源：`plnk/Cargo.toml` 的 `[package] version`） |
| 分支 | `feature/plnk-rust` |
| 最新提交 | `7c9b769 chore(plnk): release 0.2.1` |
| 远程 | **未配置**（`git remote -v` 为空） |
| 桌面包 | `release\plnk-v0.2.1-win64\` + 同名 zip |
| 桌面快捷方式 | `D:\Desktop\gInk.lnk` → 0.2.1 |
| 开机自启 | `%APPDATA%\...\Startup\plnk.lnk` → 0.2.1 |

代码规模：`plnk/src` 36 个文件 / 13,798 行；`plnk/tests` 12 个测试文件。

---

## 3. 怎么构建、测试、发布

### 环境要求

- rustc stable MSVC（当前 1.100.0），`rust-toolchain.toml` 已锁定
- **Windows SDK 的 `rc.exe`**（`plnk/build.rs` 用来生成版本资源；找不到可设 `RC` 环境变量）
- 无需 .NET、无需管理员权限

### 测试

```bash
./scripts/check-tests.sh
```

**务必用这个脚本，不要手写 `cargo test | grep`。** 原因见第 6 节——这个仓库历史上被管道退出码坑过两次，一个是假红灯（全绿被报成失败），一个是假绿灯（真失败被吞掉）。

### 发布（完整流程见 `docs/release.md`）

```powershell
powershell -File scripts/bump-version.ps1 -Version 0.2.2   # 改版本号
cargo build --release --locked --manifest-path plnk/Cargo.toml
powershell -File scripts/package-plnk.ps1                  # 打包 + 自动更新快捷方式
powershell -File scripts/verify-plnk-package.ps1           # 功能验证
```

打包脚本会**自动**把桌面快捷方式和开机自启指向新包，不需要手工改。

### 桌面自动化验收（会抢占鼠标）

```powershell
powershell -File scripts/gui-acceptance.ps1 -WhatIf   # 先看要跑什么
powershell -File scripts/gui-acceptance.ps1           # 真跑（需离开键盘）
```

exit code：`0` 全通过 / `1` 有失败 / `2` 因用户占用而跳过。

**`exit 2` 不是成功。** 它表示「没验成」。详见 `docs/acceptance/gui-harness.md`。

---

## 4. 代码结构（改动前必读）

```
plnk/src/
  main.rs              入口：单实例锁 → 设置导入 → 跑 shell
  app.rs               主控制器（2400+ 行）：模式状态机、窗口过程、
                       工具命令分发、笔迹会话、文件 IO 队列
  settings.rs          强类型设置 + 旧版 INI 导入（含默认值与校验）
  settings_store.rs    设置读写
  options.rs           设置窗口的模型层（哪些热键可改、冲突判定）
  startup.rs           开机自启（写 Startup 文件夹快捷方式）
  hotkeys.rs           热键解析/格式化/注册
  telemetry.rs         有界滚动日志
  tray.rs              托盘图标与菜单
  ink/                 笔迹模型、几何、历史、脏区域、控制器、渲染器
  gfx/                 D2D 设备、DComp 合成、笔迹表面
  input/               指针规范化（含合并历史）、鼠标
  platform/            窗口、DPI、光标
  ui/                  工具栏、笔宽面板、图标、动画、设置窗口
plnk/tests/
  support/gui.rs       桌面测试的安全闸门（空闲判定 + 指针恢复）
  gui_support.rs       闸门自身的测试
  ui_ink.rs            14 个桌面测试（默认 #[ignore]）
  ui_smoke.rs          3 个 + ui_toolbar.rs 2 个，同理
  modes.rs             橡皮/重做/隐藏/清空 的 headless 测试
  options.rs           设置窗口模型测试
  ink.rs settings.rs toolbar.rs lifecycle.rs
```

### 关键约定

- **UI 线程独占**：所有窗口、笔迹状态、D2D 对象都在一个线程上。不要引入全局 `Arc<Mutex<App>>`。
- **物理像素**：笔迹和截图坐标一律物理像素；DIP 只用于工具栏/控件布局。
- **文件 IO 走有界队列**（`IoWorker`），结果用 `PostMessageW` 回到 UI 线程。
- **不要用 `RegisterHotKey` 注册工具热键**。默认值是裸键（`E`、`V`、数字），系统级注册会把字母从所有其它程序里抢走。现在用的是仅批注期间运行的 `GetKeyState` 轮询（`app.rs` 的 `poll_tool_hotkeys`）。

---

## 5. 已经踩过的坑（重要）

这些都是真实发生过、有提交记录的，重犯代价很高。

### 5.1 普通窗口收不到 `WM_INITDIALOG`

设置窗口曾经完全空白。控件建在 `WM_INITDIALOG` 里，但窗口用 `CreateWindowExW` 创建——那是普通窗口，这条消息只有 `DialogBox*` 系列会发。改成 `WM_CREATE`（提交 `ee70b5f`）。

这类 bug **完全静默**：不报错、不崩溃，只是没内容。所以有回归测试钉住消息值。

### 5.2 ICO 不是 CUR，热点会读错字段

`.ico` 是图标（`idType=1`），`.cur` 才是光标（`idType=2`）。`LoadImageW(IMAGE_CURSOR)` 会接受 ico，但把 `wPlanes`/`wBitCount` 当成热点坐标读——那批资产的值是 `(1, 32)`。

修复方式：加载前在内存里把 `idType` 改成 2、写入真实热点，经临时 `.cur` 文件交给 `LoadImageW`（`LoadImageW` 只接受路径不接受 buffer）。原始资产不动。

### 5.3 光标热点必须落在笔尖像素上

**这是最容易搞错的一处。** 那批 128×128 的笔形光标**不是满画布**：

```
不透明像素范围: x[64,127]  y[0,63]
即笔尖只在「右上角」四分之一，左下四分之一全透明
笔尖顶点: y=0 行，x=109~112，中心 110
```

所以热点是 **(110, 0)**，不是 (0,0)。设成 (0,0) 等于把热点放在透明角落，笔形会被画在落笔点右边约 110 像素。

改这个值之前，先用第 8 节的方法重新量一遍像素，不要凭感觉调。

### 5.4 `CreateIconFromResourceEx` 对这批资产不可用

实测：返回非空但无效的句柄，`windows` crate 把它包装成 `Err` 携带 `HRESULT(0x00000000)`——看起来像"操作成功"，实际不能用。只能用 `LoadImageW` + 临时文件。

### 5.5 管道的退出码会骗人（两个方向）

```bash
cargo test ... | grep -c FAILED
# grep -c 无匹配时输出 0 但退出码 1 → 经 && 串联，全绿被判失败（假红灯）

cargo test ... | tee log > /dev/null
# 管道退出码取自最后一个命令 → 真失败被吞掉（假绿灯）
```

**假绿灯比假红灯危险得多。** 用 `scripts/check-tests.sh`，它从 `PIPESTATUS[0]` 读真实退出码。

### 5.6 托盘菜单加了项，验证脚本要跟着改

`verify-plnk-package.ps1` 靠按 `↑`+`Enter` 选 Exit 项。加了「Options...」之后，按一次 `↑` 会停在 Options 上，脚本点开设置窗口然后挂起超时。

现在按两次 `↑`——这个写法不依赖菜单顺序。

### 5.7 验证脚本有时序抖动

首次验证 0.2.1 时出现 `ink committed: 2 document strokes`（期望 1），重跑得到 1 并通过。

判断：`Invoke-PlnkDrag` 在移动光标与按下之间存在竞争。**这是脚本自身的问题，不是产品缺陷**，但会偶发假失败。**没有为了让验证通过而放宽断言**——那样会掩盖真问题。以后要修的话，方向是给拖动加更稳的时序同步。

---

## 6. 没做完的事（按优先级）

计划文档：`docs/superpowers/plans/2026-11-15-plnk-rust-rewrite.md`（14 个 Task）。功能对齐基线：`docs/parity-matrix.md`（108 条 `P-xxx`，脚本 `check-parity.ps1` 校验）。

| Task | 内容 | 状态 |
|---|---|---|
| 1–7 | 基线矩阵、设置导入、图形决策门、应用外壳、工具栏动画、笔迹模型、可书写 MVP | ✅ 已完成 |
| 8 | 完整批注模式与工具行为 | ✅ 本轮完成（橡皮/重做/隐藏/工具热键） |
| 9 | 选项窗口 + 31 语言 | ⚠️ **只有最小版**（热键、自启）。缺笔刷编辑、多语言界面 |
| **10** | **截图选区、PNG、剪贴板** | ❌ **未开始** |
| **11** | **贴图窗口** | ❌ **未开始** |
| 12 | 自动滚动、多显示器/DPI 硬化 | ❌ 未开始 |
| 13 | 性能、泄漏、故障验收 | ❌ 未开始 |
| 14 | 便携发布、切换/回滚 | ⚠️ 打包脚本已有，缺切换/回滚文档 |

**用户最在意但还没有的是截图和贴图。** 贴图在旧版里是 `src/PinForm.cs`，可以照着行为重写。

### 其他已知缺口

- **`run_tests` 工具登记错误**：项目登记的是 `dotnet test`，但本机无 .NET SDK（实测 exit=2147516571）。正确命令是 `cargo test --manifest-path plnk/Cargo.toml`。这是项目元数据，不在仓库文件里。
- **20 个桌面测试从未在真机全绿跑过**：都是 `#[ignore]`，会抢占鼠标。用 `gui-acceptance.ps1` 在离开键盘时跑。
- **压感笔 / 触摸 / 多显示器**：无设备，标为"未验证"，不得声称通过。

---

## 7. 怎么验证改动

### 每改一处都跑

```bash
./scripts/check-tests.sh        # 应输出 RESULT: PASS, exit 0
cargo clippy --manifest-path plnk/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path plnk/Cargo.toml --check
```

当前基线：**109 passed / 13 targets / 0 failed**，21 个 ignored（桌面测试）。

### 改 UI 或光标后必须人工确认

自动化测不了"看起来对不对"。这两件事只能人看：

1. 托盘右键 → Options → 应看到热键列表和「开机自启动」复选框
2. 画一笔 → 笔尖应正好落在光标尖端

### 量光标热点的办法（复现第 5.3 节）

```python
import struct
d = open('plnk/assets/icons/cursorred.ico','rb').read()
planes,bits,size,off = struct.unpack('<HHII', d[10:22])
bmp = d[off:off+size]
hs  = struct.unpack('<I', bmp[:4])[0]
bw,bh = struct.unpack('<ii', bmp[4:12])
pix = bmp[hs:]
row = [x for x in range(bw)
       if pix[x*4+3] > 20]          # y=0 行，alpha>20
print(min(row), max(row), sum(row)//len(row))
```

---

## 8. 接手第一天建议做什么

1. **跑一遍** `./scripts/check-tests.sh`，确认基线 109 passed。
2. **打开桌面快捷方式**，托盘右键 → Options，画一笔。确认第 7 节那两件事。
3. **决定做哪一件**：
   - 想做用户最在意的 → **Task 10 截图**（能截、能存盘、能复制），然后 Task 11 贴图
   - 想把设置做完整 → Task 9 的笔刷编辑页和 31 语言
   - 想提升可维护性 → `app.rs` 已 2400+ 行，工具分发和轮询逻辑可以拆出去

**不要**在没有真机验证的情况下声称截图/贴图已完成。

---

## 9. 一句话提醒

这个项目的坑集中在两类：**Windows 消息的语义**（哪条消息谁会发）和**二进制资源的格式**（ICO vs CUR、热点在哪）。

两者都不会报错，只会"看起来不对"。所以：**改完必须人眼看一次**，别只信测试。
