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

**这是最容易搞错的一处。** 笔形光标 ICO 的不透明像素（alpha > 20 且 AND 掩码位为 0）只占画布的一个角，其余全透明；而热点是位图里**与鼠标坐标对齐的那一个像素**，必须取在笔尖上。设成位图角 (0,0) 之类就落在透明区，整支笔形被画偏，且**不报错**——只是「看起来不对」。

**本文件不记录绝对坐标。** 热点数值由 `plnk\src\platform\cursor.rs` 的 `hotspot_tests` 从资产推导并钉住：该模块解码四支笔色 ICO 的像素，按 `biHeight` 符号翻成视觉顺序，取最高一行不透明像素的中位 x 作为笔尖，并断言常量 `PEN_CURSOR_HOTSPOT` 与之相等——资产或常量任改一方，测试就红。真值只在代码里，文档与代码不会再分叉。

要亲手量一遍像素、或核对 `PEN_CURSOR_HOTSPOT` 与资产是否仍一致，用第 7 节的脚本；不要凭感觉调常量。

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

自动化测不了"看起来对不对"。这几件事只能人看：

1. 托盘右键 → Options → 应看到热键列表和「开机自启动」复选框
2. 画一笔 → 笔尖应正好落在光标尖端
3. **热点的观感部分只能人眼验完**：下面那个量像素的脚本和 `hotspot_tests` 只证明常量等于解码出的笔尖像素，证明不了屏幕上的最终视觉——笔刷缩放、DPI 缩放、主题笔形都能让坐标全对却看着歪。必须真机画一笔，确认笔尖贴住墨迹起点——不是偏出半个笔宽、也不是把墨迹画在光标旁边。

人眼之前先过**数值判据**（不过就别浪费眼睛；过了也不等于验收通过）：

- `偏移 = 写入热点 − 解码尖端 = (0, 0)`，四支笔色全部成立——判定式与脚本见下条"量光标热点的办法"，不成立时脚本 `exit 1`
- `cargo test --manifest-path plnk/Cargo.toml --lib platform::cursor` 全绿：`hotspot_tests` 用同一套解码断言 `PEN_CURSOR_HOTSPOT == 笔尖`，资产与常量任改一方就红
- 四支笔色的解码 `apex` 两两差 ≤2 px（脚本输出逐字一致即满足）

### 量光标热点的办法（复现第 5.3 节）

热点是**位图里的一个像素坐标**，量它就得按 Windows 的读法解那张 DIB。三个坑每一个都能算出一个"看起来很有道理"的错值：

1. **BMP 自下而上存。** `biHeight` 数的是 XOR 位图 + 尾部 AND 掩码两份；XOR 行从**最底一行**开始往上写，`biHeight < 0` 才是自上而下。不显式翻行序，量到的"顶点"是笔尾，热点会偏出约 64 px。
2. **XOR 行按 4 字节对齐**：每行 `(width*4 + 3) & ~3` 字节，不是 `width*4`；AND 掩码每行 `((width+31)//32)*4` 字节，位序 MSB-first、**位 0 = 不透明**。本资产 `width=128` 正好整除，漏掉对齐也能跑，换个尺寸就整行错位。
3. **`ICONDIRENTRY` 的字段位置**：`dwImageOffset` 在 +12（绝对偏移 18）。从偏移 10 开始用 `'<HHII'` 解，四个字段全部错位 2 字节，读出来的"热点"是噪声。

把下面这段存成 `measure-cursor-hotspot.py`（放仓库外即可），在仓库根目录跑（Python 3）。它遍历**全部扫描线**，按 `biHeight` 符号决定 bottom-up / top-down，处理「XOR 位图 + AND 掩码」两倍高与行 4 字节对齐，输出 top-down 的 bbox 与笔尖 apex，并算 `偏移 = 写入热点 − 解码尖端`。不带参数时扫 `plnk/assets/icons/cursor*.ico`，也可传文件路径；`--hotspot X,Y` 覆盖它从源码读到的写入值（用来验证脚本自己会不会报错）。

```python
import glob, re, struct, sys

ALPHA = 20                      # 与 cursor.rs 的 OPAQUE_ALPHA 一致：alpha>20 才算笔身
SRC   = 'plnk/src/platform/cursor.rs'

def u16(b, o): return struct.unpack_from('<H', b, o)[0]
def u32(b, o): return struct.unpack_from('<I', b, o)[0]
def i32(b, o): return struct.unpack_from('<i', b, o)[0]

def written_hotspot():
    """读代码里写进 .cur 头的那两个 u16：PEN_CURSOR_HOTSPOT。"""
    src = open(SRC, encoding='utf-8').read()
    m = re.search(r'PEN_CURSOR_HOTSPOT\s*:\s*\(\s*u16\s*,\s*u16\s*\)\s*=\s*\(\s*(\d+)\s*,\s*(\d+)\s*\)', src)
    return (int(m.group(1)), int(m.group(2))) if m else None

def decode(path):
    """解 32bpp ICO：显式翻 bottom-up、显式按 4 字节对齐行。"""
    d = open(path, 'rb').read()
    idtype, count = u16(d, 2), u16(d, 4)                        # 1=ICO 2=CUR
    off = u32(d, 6 + 16 * (count - 1) + 12)                     # 最后一张的 dwImageOffset
    bi_size, width, bi_height = u32(d, off), i32(d, off + 4), i32(d, off + 8)
    bits = u16(d, off + 14)                                     # biBitCount
    if bits != 32:
        return None, f"{bits}bpp -- only 32bpp BGRA; skipped"
    top_down = bi_height < 0                                    # biHeight<0 = 自上而下
    height = abs(bi_height) // 2 if idtype == 1 else abs(bi_height)  # ICO 含 AND 掩码两倍高
    stride, mstride = (width * 4 + 3) & ~3, ((width + 31) // 32) * 4  # XOR 行 4 字节 / 掩码行 32 位对齐
    pix, mask = off + bi_size, off + bi_size + stride * height
    x0, y0, x1, y1, rows = width, height, -1, -1, {}
    for file_row in range(height):                              # 遍历全部扫描线
        y = file_row if top_down else height - 1 - file_row      # 翻成 top-down（左上为原点）
        for x in range(width):
            a = d[pix + file_row * stride + x * 4 + 3]          # BGRA 第 4 字节 = alpha
            m = (d[mask + file_row * mstride + x // 8] >> (7 - x % 8)) & 1
            if a > ALPHA and m == 0:                            # AND 掩码位 0 = 不透明
                x0, y0, x1, y1 = min(x0, x), min(y0, y), max(x1, x), max(y1, y)
                rows.setdefault(y, []).append(x)
    if not rows:
        return None, "no opaque pixel"
    top = min(rows)                                             # 最上面那行不透明像素
    apex_x = sorted(rows[top])[len(rows[top]) // 2]              # 中位 x，不是左边缘
    return ((x0, y0, x1, y1), (apex_x, top), (width, height)), None

def main(argv):
    paths, forced, i = [], None, 0
    while i < len(argv):
        if argv[i] == '--hotspot':
            x, y = argv[i + 1].split(','); forced = (int(x), int(y)); i += 2
        else:
            paths.append(argv[i]); i += 1
    paths = paths or sorted(glob.glob('plnk/assets/icons/cursor*.ico'))
    written = forced or written_hotspot()
    bad = 0
    for path in paths:
        name = path.replace('\\', '/').rsplit('/', 1)[-1]
        got, why = decode(path)
        if got is None:
            print(f"{name}: {why}")
            continue
        bbox, apex, (w, h) = got
        line = f"{name}: {w}x{h} bbox(left, top, right, bottom)={bbox}  apex={apex}"
        if written is None:
            print(line + f"   [written hotspot unknown: pass --hotspot x,y or check {SRC}]")
            continue
        delta = (written[0] - apex[0], written[1] - apex[1])
        ok = delta == (0, 0)
        bad += 0 if ok else 1
        print(f"{line}  written={written}  offset=written-decoded={delta}  " + ("OK" if ok else "FAIL"))
    if written is not None and bad:
        print(f"{bad} asset(s) with offset != (0, 0)")
        return 1
    return 0

sys.exit(main(sys.argv[1:]))
```

四支笔色资产在同一份资产上的实测输出（bbox 是 top-down 坐标、含端点，四支笔色逐字相同）：

```
cursorred.ico: 128x128 bbox(left, top, right, bottom)=(64, 64, 127, 127)  apex=(67, 64)  written=(67, 64)  offset=written-decoded=(0, 0)  OK
cursorsnap.ico: 8bpp -- only 32bpp BGRA; skipped
```

先确认脚本真会报错（别只信绿灯）：

```bash
python measure-cursor-hotspot.py --hotspot 0,0
#   ... offset=written-decoded=(-67, -64)  FAIL
#   4 asset(s) with offset != (0, 0)                → exit 1
```

**判定式只有一条：**

```
偏移 = 写入热点 − 解码尖端，必须为 (0, 0)
```

- **写入热点**：`plnk\src\platform\cursor.rs` 的 `PEN_CURSOR_HOTSPOT`（脚本从源码 grep 它），也等于 `ico_to_cursor_bytes` 写进 `.cur` 头的两个 `u16`——`[10..12]` = x、`[12..14]` = y。这两处不一致同样是 bug：改代码，别改判据。
- **解码尖端**：脚本的 `apex`，即最上面那行不透明像素的**中位** x（那一行是 4~5 像素宽的对称笔尖，取左边缘会偏 2 px）。`bbox` 顺带说明笔身只占画布的一个角，所以位图角点 (0,0) 必然落在透明区。
- 偏移不为 `(0,0)` 时脚本 `exit 1`。四支笔色必须全部 `(0,0)`，且 apex 两两差 ≤2 px。偏移不对时先跑 `cargo test --manifest-path plnk/Cargo.toml --lib platform::cursor`，再决定改常量还是改资产。
- `cursorsnap.ico` 是 8bpp 索引色，脚本只解 32bpp，会打印提示跳过——它不是笔形光标，别拿它当反例。

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
