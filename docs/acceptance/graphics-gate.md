# 图形技术决策门 — 原生窗口层验收

**状态：** 通过（PASS）  
**日期：** 2026-11-15  
**分支：** `feature/plnk-rust`  
**执行人：** 实现代理（自动化探针）+ 本文档记录  
**关联：** 设计 §6.1（双窗口层）、§6.3（图形与输入）、§4.2（性能目标）

本文件记录 Task 3 决策门的实测证据。所有数字来自本机 Release 探针
（`plnk/tests/ui_smoke.rs::graphics_gate_probe`）在真实桌面窗口上的运行结果，
不是目标值或估算值。

## 1. 测量环境

| 项目 | 实测值 |
|---|---|
| 操作系统 | Windows 11 Home China，10.0.26200，x86_64（未验证 Win10 1607；设计仅保留 API 下限） |
| CPU | AMD Ryzen 7 7745HX（8 核 / 16 逻辑核） |
| 活动显示适配器 | NVIDIA GeForce RTX 4070 Laptop GPU，驱动 32.0.15.8157 |
| 活动显示器 | 1 台（`WmiMonitorBasicDisplayParams` = 1；另有 Todesk/GameViewer 虚拟显示适配器，未参与测试） |
| 物理分辨率 | 2560 × 1600 |
| 刷新率 | DEVMODE 240Hz；CIM 239Hz；DWM `rateRefresh` 239.98–240.15Hz；DComp `currentCompositionRate` 239.9–240.0Hz |
| DPI | 144（150%，`GetDpiForWindow`；`DpiMap` scale = 1.500） |
| 工具链 | rustc/cargo 1.100.0 stable MSVC，`windows` crate 0.62.2（运行时唯一依赖） |
| 构建 | `cargo test --release`（LTO thin、codegen-units=1、opt-level=3） |

## 2. 执行命令

```text
# 日常（非桌面）测试
cargo test --manifest-path plnk/Cargo.toml

# 桌面 UI 验收（短暂创建真实置顶窗口，自动销毁；不注册热键、不点击、不打扰旧版 gInk）
cargo test --manifest-path plnk/Cargo.toml --test ui_smoke -- --ignored --test-threads=1

# 决策门探针（Release，真实桌面窗口，约 10s）
cargo test --release --manifest-path plnk/Cargo.toml --test ui_smoke -- --ignored --test-threads=1 graphics_gate_probe --nocapture
```

## 3. 通过条件与实测结果

### 3.1 透明顶层 + 输入穿透（关键条件：通过）

- 覆盖层为 `WS_EX_NOREDIRECTIONBITMAP` + DirectComposition（premultiplied alpha）
  窗口；点击穿透模式切换 `WS_EX_TRANSPARENT`。
- 透明内容用屏幕可见像素验证：白色背景窗口上叠加 50% 红（`Rgba8(255,0,0,128)`），
  从合成后的桌面 `BitBlt` 取中心像素：

| 场景 | 中心像素 RGB |
|---|---|
| 仅白色背景窗 | (255, 255, 255) |
| 覆盖层绘制模式 | (255, 127, 127) |
| 覆盖层点击穿透模式（仍可见） | (255, 127, 127) |

- 输入路由用真实 `WM_MOUSEMOVE` 计数验证（`SetCursorPos` 短暂移动系统光标，
  测试结束恢复原位；不注入点击）。测试窗口下方叠放接收窗：

| 模式 | 覆盖层收到 WM_MOUSEMOVE | 下方窗口收到 WM_MOUSEMOVE |
|---|---:|---:|
| 绘制模式（click-through = false） | 2 | 0 |
| 指针模式（click-through = true） | 0 | 2 |

**关键技术结论（影响后续设计）：** 在 Windows 11 上，未分层的
`WS_EX_NOREDIRECTIONBITMAP` 顶层窗口只设置 `WS_EX_TRANSPARENT` **不会**把鼠标输入
路由到其他进程的窗口。必须叠加 `WS_EX_LAYERED` 并调用一次
`SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA)`，让 DComp 帧自带的逐像素
alpha 继续生效。首轮实测（仅 `WS_EX_TRANSPARENT`）为 覆盖层=2 / 下方窗口=0，
记录后改为分层+透明组合并通过。`WindowFromPoint` 不模拟
`WS_EX_TRANSPARENT` 的输入路由（两种模式下都返回覆盖层），因此不作为穿透证据。

### 3.2 组合器驱动的动画（关键条件：通过）

- `ToolbarWindow::animate_to` 只提交起点、终点、时长（600ms）与缓动
  （Linear）；DirectComposition 动画由 DWM 在合成节拍上求值，程序无逐帧回调。
- 呈现帧时间戳来源：`IDCompositionDevice::GetFrameStatistics().lastFrameTime`
  （QPC 时间戳，紧密轮询，不与屏幕采样混合）+ `timeFrequency` 换算。
- 实测（240Hz 目标：中位 ≤4.2ms、p95 ≤8.4ms）：

| 指标 | 实测（代表性 Release 运行） |
|---|---:|
| 观测到的合成帧数 | 181 |
| 帧间隔样本数 | 179 |
| 中位 | **4.165 ms**（另一轮 4.167 ms） |
| p95 | **4.194 ms**（另一轮 4.167 ms） |
| 最小 / 最大 | 4.093 / 4.253 ms（另一轮 4.167 / 4.167 ms） |
| 应用级逐帧回调（WM_TIMER / WM_PAINT / WM_APP+1） | **0 / 0 / 0** |
| 回调计数器自检（SetTimer / RedrawWindow / PostMessage） | 全部触发（计数器确实有效） |

- 可见性：对屏幕区域用 `BitBlt` 采样 54 次，品红测试块左边缘从 20px 移到
  380px（物理像素），证明动画在实际显示内容上发生。
- DWM 交叉验证：`DwmGetCompositionTimingInfo(窗口 HWND)` 对
  `WS_EX_NOREDIRECTIONBITMAP` 窗口返回 `E_INVALIDARG (0x80070057)`，因此改用
  桌面合成目标；其 `rateRefresh` 为 239.98–240.15Hz，与 DComp 一致。桌面目标的
  `cFramesDisplayed` 未推进（区间样本 0），所以帧间隔只采用 DComp 设备时间戳，
  **未** 用任何回调频率冒充帧率。
- 动画期间应用 CPU（`GetProcessTimes`，100ns 分辨率，1.5s 动画，探针不轮询）：
  多次 Release 运行分别测得 0.000ms 与 15.625ms CPU / 约 1.5s 墙钟
  （0–1.04% 单个逻辑核；较高值包含测试自身 2ms 消息泵循环），均满足设计 §4.2
  「动画期间应用 CPU ≤ 2% 单逻辑核」。

### 3.3 物理像素坐标（关键条件：通过）

- 150% DPI 下 `DpiMap::physical_to_dip` → `dip_to_physical` 在 96/120/144/168/
  192/216/240/288 DPI、坐标 ±1,000,000 范围内逐点精确往返；DIP 往返误差不超过
  半个物理像素（物理像素是权威单位）。
- 真实窗口以物理矩形 `(120,140)-(380,260)` 创建，`GetWindowRect` 返回完全相同的
  物理像素矩形（per-monitor-v2 运行时设置生效，无虚拟化）。
- DirectComposition 视觉偏移经实测为物理像素：请求 20→380（360px）时屏幕可见
  左边缘正好移动 20→380，未按 150% 缩放；笔迹/动画坐标可直接使用物理像素。

### 3.4 100 次窗口创建/销毁与资源（通过）

默认探针连续执行 **3 × 100** 次「透明覆盖层 + 工具栏」创建/绘制/显示/销毁，
测量点均在预热 20 次并 pump 200ms 之后：

| 批次 | 句柄 | GDI 对象 | USER 对象 | 私有字节 |
|---|---:|---:|---:|---:|
| 基线（预热后） | 535 | 4 | 8 | 59,928,576 |
| +100 次 | 535 | 4 | 8 | 60,260,352 |
| +200 次 | 535 | 4 | 8 | 60,555,264 |
| +300 次 | 536 | 4 | 8 | 60,850,176 |

句柄/GDI/USER 无增长（±1 波动）；私有字节 300 次增长约 0.9–1.0MB（1.5–1.7%），
低于设计 ±5% 预算。附加的 1000 次扩展 soak（未纳入默认探针）显示句柄结束时仍为
535、GDI/USER 不变，私有字节累计 +3.6% 且后期趋平（第 8→10 批仅 +0.15MB），
表现为驱动/分配器缓存而非逐窗口泄漏。完整应用级 100 次批注开关泄漏门仍按计划由
Task 13 执行。

### 3.5 原型内存（记录，不作为本门阈值）

Release、两个原生窗口 + 动画运行中：私有字节约 59.7–60.9MB（56.9–58.1MiB）、
工作集约 42.4–42.6MB（40.4–40.6MiB），为多次运行的实测范围。其中包含
D3D11/D2D/DComp/DXGI 运行时与驱动分配；设计 §4.2 的托盘空闲 25MB/35MB 目标
适用于最终应用（Task 13），本门只记录基线，不据此声称达标。

## 4. 明确说明「测了什么 / 没测什么」

- **测了**：DComp 设备报告的合成帧时间戳（QPC）及其中位/p95；屏幕可见像素移动；
  真实路由的 `WM_MOUSEMOVE`（含穿透）；窗口过程收到的 WM_TIMER/WM_PAINT/自定义
  消息数（并自检计数器有效）；物理坐标往返与真实窗口矩形；句柄/GDI/USER/私有字节。
- **没测**：逐帧屏幕捕获（BitBlt 采样无法分辨 240Hz，本门未声称逐帧可见性）；
  DWM 每窗口呈现计数（该 API 对 NOREDIRECTIONBITMAP 窗口不可用）；60Hz 模式
  （当前显示器为 240Hz；测试仅在刷新率 ≥200Hz 时套用 240Hz 目标）；压感笔、
  触摸、实体多显示器（无设备，标为「未验证」）；Windows 10 1607（仅保留 API
  下限，未验证）；长时间（>1000 次）soak。
- **不打扰旧版**：测试不注册全局热键、不点击、不发送合成点击，仅短暂创建置顶
  窗口（每个 <2s）并短暂移动/恢复系统光标；不修改旧 gInk 配置或进程。

## 5. 决策

透明输入、组合器可见动画、物理坐标三项关键条件全部满足，资源与 CPU 记录无阻塞
问题。**决策门通过**，可以继续后续功能开发；若后续实现偏离本次验证的调用方式
（例如把覆盖层改回非分层窗口、或改为程序逐帧动画），必须重新执行本门。
