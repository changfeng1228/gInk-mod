# 输入验收记录：鼠标、压感笔与触摸

**任务：** Task 7（Pointer 输入与可书写 MVP）  
**日期：** 2026-11-16  
**机器：** Windows 11 Home China 10.0.26200 x86_64；AMD Radeon 集成显卡 + NVIDIA GeForce RTX 4070 Laptop（D3D11 默认适配器为集成显卡，显存与系统内存共享）；逻辑分辨率 1707×1067、150% DPI、240Hz、单显示器。  
**状态：** 鼠标路径已实测通过；压感笔与触摸**未验证**（本机无相应硬件）。本文件不声称笔/触摸已验收。

## 1. 结论摘要

| 输入 | 状态 | 证据 |
|---|---|---|
| 鼠标（当前设备） | **已验收（自动化 + 真实输入）** | `tests/ui_ink.rs` 的桌面测试；真实 `SendInput`/`mouse_event` 书写、工具栏选笔/撤销/清空、真实 `plnk.exe` 启动与干净退出 |
| 合并输入（coalesced history） | **已验收（纯逻辑 + 桌面消息链）** | `coalesced_history_preserves_order`（headless）、`pointer_timestamp_to_submit_is_monotonic`（桌面） |
| 压感笔（压力、笔反转端） | **未验证** | 无压感笔；只有 `GetPointerPenInfoHistory` 实现与纯函数/模拟测试，不得声称通过 P-027/P-028 |
| 触摸 | **未验证** | 无触摸屏；只有 `GetPointerTouchInfoHistory` 实现与模拟测试，不得声称通过 P-026 |
| 多显示器 | **未验证** | 覆盖层只覆盖光标所在显示器（首版范围） |

## 2. 自动化与鼠标证据

运行方式（桌面测试会短暂创建置顶窗口并移动真实光标；不会注册全局热键、不会触碰正在运行的旧版 gInk）：

```text
cargo test --manifest-path plnk/Cargo.toml --test ui_ink -- --ignored --test-threads=1
cargo test --release --manifest-path plnk/Cargo.toml --test ui_ink -- --ignored --test-threads=1 ink_mvp_probe --nocapture
```

2026-11-16 实际结果：

| 测试 | 结果 | 说明 |
|---|---|---|
| `coalesced_history_preserves_order` | PASS（headless） | `GetPointer*InfoHistory` 返回**倒序（最新在前）**的合并样本；normalizer 反转为时间顺序、时间戳经 `MonotonicClock::from_performance_count` 映射到进程单调时钟、乱序样本钳制不回退 |
| `contact_cancel_does_not_commit_partial_history` | PASS（headless） | 触点取消（捕获丢失/离开/模式切换）不改变文档、undo/redo 深度或后续笔画 |
| `present_only_submits_dirty_region` | PASS（桌面） | 脏区域为 40×40 px（整面 640×360）；`painted_area == 脏区面积`；`uploaded_bytes == 0`（无 CPU 逐帧全屏上传）；屏幕像素确认活动笔画可见 |
| `pointer_timestamp_to_submit_is_monotonic` | PASS（桌面） | 12 帧 × 3 个合并样本：输入时间戳非递减、提交时间非递减、输入不晚于提交 |
| `mvp_end_to_end_mouse_writing_selects_undo_clear_and_exits` | PASS（桌面） | 真实路由鼠标输入绘制红/蓝两笔；真实点击工具栏 Pen 2 / Undo / Clear；屏幕像素逐一验证；退出后画布清空、窗口销毁 |
| `renderer_caps_dots_and_highlighter_blend_are_visible` | PASS（桌面） | 屏幕像素确认：线段两端为**圆帽**（端点半宽内着色、半宽外不着色）、单击为**圆点**、半透明荧光笔交叉**重叠变深**（缓存增量合成正确） |
| `mvp_real_app_launch_draws_and_exits_cleanly` | PASS（桌面） | 启动真实 `plnk.exe --no-hotkeys --draw`：覆盖层出现、真实鼠标书写可见、`WM_CLOSE` 干净退出（退出码 0，日志含 `mode Tray -> Drawing` 与 `shutdown complete`） |
| `ink_mvp_probe` | PASS（Release 专用） | 见第 4 节；debug 构建会跳过（spec §4.2 只接受 Release 探针） |

回归：Task 3/5 的桌面验收（`ui_toolbar` 2 项、`ui_smoke` 3 项含 graphics gate probe）在本次改动后仍全部 PASS。

## 3. 鼠标行为实现说明

- **两条输入路径，不重复：** 覆盖层自行处理 `WM_POINTERDOWN/UPDATE/UP/LEAVE/CAPTURECHANGED` 并返回“已处理”，因此 `DefWindowProcW` 不会把笔/触摸接触提升为重复的鼠标消息；物理鼠标继续走 `WM_LBUTTONDOWN/MOUSEMOVE/LBUTTONUP` 回退路径，使用固定全压力宽度。
- **取消安全：** `WM_CAPTURECHANGED`、`WM_POINTERCAPTURECHANGED`、`WM_POINTERLEAVE`、第二触点、模式切换（绘图↔指针）都会 `InkController::cancel()`：活动笔画被丢弃，静态缓存不重绘该笔画，文档与历史不变。
- **活动/静态分层：** 静态缓存位图保存已提交笔画；活动笔画单独绘制在活动层；指针移动只清除/重绘该局部脏矩形。提交时新笔画**增量**画入静态缓存（不重绘其下已有笔画），活动层同区域清除。
- **无逐帧全屏上传：** `InkSurface::present` 只做 GPU 纹理合成（`DrawBitmap` 静态+活动缓存），报告 `uploaded_bytes = 0`；旧版 GDI 的每帧全屏位图上传路径不复用。
- **当前显示器：** 覆盖层覆盖光标所在显示器的物理像素矩形；笔迹坐标为物理像素，窗口原点在绘制时减除。

## 4. 性能与内存实测（Release，本机）

`ink_mvp_probe`：1000 条混合笔画（普通红 80px、蓝 30px、500px 荧光笔 alpha 80、压感 80px；每条 200 点，共 20 万点），240Hz、表面 1707×1067。

```text
[ink-probe] memory.baseline_private_bytes=1523712 device_private_bytes=56922112 surface_private_bytes=73043968
[ink-probe] strokes=1000 points=200000 write_wall_ms=1386.8 append_ms=11.7 commit_ms=1326.6 present_ms=47.3 strokes_in_dirty_avg=134.3 max=699
[ink-probe] memory.after_250_strokes.private_bytes=79605760
[ink-probe] memory.after_500_strokes.private_bytes=81641472
[ink-probe] memory.after_750_strokes.private_bytes=83910656
[ink-probe] memory.after_1000_strokes.private_bytes=85843968
[ink-probe] memory.private_bytes=85843968 working_set_bytes=52137984 raw_point_bytes=6400000 growth_since_surface_bytes=12800000
[ink-probe] latency.pointer_to_submit_ms median=0.876 p95=1.824 max=2.836 samples=2000
```

- **Pointer 消息到提交绘制（应用侧，含 Present 等待、不含数字化仪硬件）：** 中位 **0.876 ms**、p95 **1.824 ms**（目标 ≤2/≤4 ms）。测量区间：`normalize` 合并样本 → `append` → 活动层局部重绘 → `present`。
- **1000 条笔画后内存：** 测试进程私有 85.8 MB、工作集 52.1 MB；**书写期间增量**（表面创建后 → 1000 条笔画）12.8 MB，其中原始点 6.4 MB，随笔画数线性且无泄漏迹象（250/500/750/1000 条：79.6/81.6/83.9/85.8 MB）。
- **注意（spec §4.2 的 50MB 目标）：** 本机 D3D11 默认适配器为共享内存的集成显卡，图形栈本身（D3D11+D2D+DComp 设备，尚未创建任何笔画）在测试进程中已占 ~56.9 MB 私有内存；表面（两张全屏缓存位图 + 双缓冲交换链）再 +16.1 MB。50 MB“书写模式私有内存（不含显存）”目标在该机器上**未能达成/无法按私有字节直接判定**，因为共享显存被计入了私有字节。该数据与原因在此记录，供 Task 13 复核（需要按显存预算 API 分离统计或在独立进程测量正式 exe）；不得用省略测量来声称达标。
- **提交代价：** 单条笔画提交（含 500px 荧光笔的轮廓三角化）平均 ~1.3 ms；光标移动帧（median/p95 如上）不受文档规模影响。稀疏文档与密集文档的撤销/清空仍走 `redraw_static`，会按脏区重新三角化相交笔画，密集文档下是一次性较高代价（探针中脏区平均相交 134 条笔画），Task 8/13 可再优化；不影响书写延迟目标。

## 5. 压感笔（未验证）

已实现但**没有硬件验证**：

- 每个 `WM_POINTER` 消息用 `GetPointerPenInfoHistory` 展开合并历史（Win32 顺序为最新在前，已反转）；`penMask & PEN_MASK_PRESSURE` 时压力 `0..1024` 归一化到 `[0,1]`，`PenStyle::pressure_enabled` 仅在笔设备报告压力通道时启用（宽度映射为配置宽度的 25%–100%）。
- 笔反转/橡皮端：`penFlags & (PEN_FLAG_ERASER | PEN_FLAG_INVERTED)`，`PointerNormalizer::is_eraser()` 暴露给工具层。当前 MVP 对橡皮端接触**不绘制墨迹**（用橡皮端画出笔迹是错误的），橡皮工具行为属 Task 8。
- 单一活动指针：同一时间只接受一个 pointer id 的接触；第二触点被忽略但不影响第一笔，也不产生鼠标提升。

验证所需（有设备时）：

1. 用支持压力的笔（Surface Pen/Wacom 等）在 `--draw` 模式下画慢/快笔画，确认线宽随压力变化、鼠标仍为固定宽度；
2. 反转笔端靠近屏幕，确认不产生墨迹且 `is_eraser()` 为真（可加临时诊断日志或 Task 8 的橡皮模式测试）；
3. 快速划线，确认合并历史没有丢点/折返（对照 `coalesced_history_preserves_order` 的语义）；
4. 记录设备名、驱动版本、`GetPointerType`/`penMask` 实际值。

**风险：** 覆盖层使用 `WS_EX_LAYERED + WS_EX_NOREDIRECTIONBITMAP`。鼠标消息已验证；`WM_POINTER`（笔/触摸）在分层窗口上的投递在无硬件时无法验证。若实机发现覆盖层收不到 `WM_POINTER`，需要改为非分层命中区或独立的透明命中窗口（Task 8 一并处理）。

## 6. 触摸（未验证）

- 已实现：`GetPointerTouchInfoHistory` 合并历史、`TOUCH_MASK_PRESSURE` 压力（如设备报告）、单指绘制、第二触点忽略、离开/捕获丢失取消。
- 未实现（Task 8 及以后）：触摸手势（缩放/平移）、双指行为、触摸笔迹的专用样式。
- 验证所需：触摸屏设备；单指慢/快画、双指同时接触（确认只产生一笔或按 Task 8 的规则处理）、掌托误触观察；记录 `GetPointerType = PT_TOUCH` 与 `touchMask`。

## 7. 人工鼠标笔迹集（与旧版并排）

固定笔迹集与“通过”定义见 SDD 工作区的 `task-7-user-acceptance.md`（不随仓库提交）：慢直线、快曲线、锯齿、圆、短点、荧光笔重叠。通过标准为“自然、无明显额外抖动、响应不劣化”，**不要求像素一致**；该人工验收由用户完成，本记录不声称用户已批准。

## 8. 未验证 / 已知限制汇总

- 压感笔、触摸、多显示器：未验证（见上）。
- 橡皮端不在 MVP 绘制工具内（Task 8）；当前忽略橡皮端接触。
- 指针模式的跨进程点击穿透验证归 Task 8（本任务只验证绘图模式接受输入）。
- 密集文档的撤销/清空重绘代价、共享显存计入私有内存的问题记录在第 4 节，交给 Task 13 复核。
