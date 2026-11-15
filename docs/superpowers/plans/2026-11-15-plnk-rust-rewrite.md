# plnk Rust Rewrite Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 用 Rust 重写现有 gInk 修改版，在完整保留批注、截图与贴图工作流的前提下，实现原生、低资源占用和显示器原生刷新率动画。

**Architecture:** 使用全屏 InkOverlayWindow 与小尺寸 ToolbarWindow 分离的原生双窗口层。UI 线程拥有窗口、输入和笔迹状态；Direct2D 负责脏区域绘制，DirectComposition/DWM 负责工具栏动画，文件 IO 使用有界工作队列，不运行常驻高频动画线程。

**Tech Stack:** stable Rust（MSVC）、Microsoft `windows` crate、Win32、Direct2D、DirectWrite、DirectComposition、DXGI、WIC、Pointer Input、Rust 标准库、PowerShell 验证脚本。

**Spec:** `docs/superpowers/specs/2026-11-15-plnk-rust-rewrite-design.md`

## Global Constraints

- 首个正式版仅承诺用户当前 Windows 11、x86_64、MSVC、鼠标和当前显示器验收；保留 Windows 10 1607 API 下限但不声称 Win10 已验证，不增加旧系统 VM 发布门；不要求管理员权限。
- 不使用 WebView、Electron、Qt、WPF、Tokio 或通用 GUI 大框架；新增运行时 crate 必须记录必要性。
- 新程序与旧 gInk 的 exe、互斥体、配置目录隔离；新配置默认在 exe 同级 `data/`，不可写时回退 `%LOCALAPPDATA%\plnk`；旧配置导入只读，未经用户批准不修改旧启动项。
- 不静默删除旧功能；任何延期必须写入功能矩阵并由用户接受。压感笔、触摸和实体多显示器保留实现，但当前无设备项目标为“未验证”，不阻塞当前 Windows 11 + 鼠标发布。
- Undo 恢复仅覆盖同一进程内关闭/重开批注；不增加退出程序后的笔迹或历史持久化。
- 工具栏动画不得依赖 4ms 常驻轮询；笔迹仅提交脏区域，不做每帧全屏位图上传。
- 性能验收必须使用可见呈现/DWM 时间戳，不能用回调频率代替。
- 保留 31 个语言资源、MIT 许可证与 Weizhi Nai 版权声明。
- 每个任务先写失败测试再实现；UI/硬件测试不能伪装成自动化已通过。
- 只有用户授权时才执行各任务的 Commit 步骤，否则保留改动并报告。
- 实施前先将当前 C# 动画修复与 Rust 工作区分；不得丢失现有未提交修改。

## Review Focus

1. **混合 DPI、负坐标和屏幕热插拔：** 笔迹、工具栏、截图和贴图必须落在同一物理像素位置；由 Task 12 的 `mixed_dpi_negative_origin_roundtrip` 等自动测试覆盖，实体多屏在当前设备下标为未验证。
2. **真实笔的压感、合并输入、橡皮端和实体多屏：** 自动/模拟测试必须通过；当前没有设备的项目标为“未验证”，不得以鼠标模拟冒充通过，也不阻塞当前发布；由 Task 7 与 Task 12 记录。
3. **旧 INI 的非标准分节、缺项和损坏值：** 导入必须保留有效值并使用默认值，不得覆盖旧文件；由 Task 2 的 legacy fixtures 覆盖。
4. **剪贴板被占用、磁盘失败、截图/保存期间连续 Esc：** UI 不得死锁或留下选择遮罩；由 Task 10 的故障注入和 Task 13 的开关 soak 覆盖。
5. **动画/IO 进行中反复开关、句柄销毁重建：** 回调不得访问已销毁对象，不得积累队列；由 Task 4 的生命周期测试与 Task 13 的 100 次开关测试覆盖。
6. **语言缺失键与非拉丁文字：** 必须回退英语且不乱码；由 Task 9 的 `missing_translation_falls_back_to_english` 和中文/阿拉伯文 UI 验收覆盖。

---

### Task 1: 固化旧版功能与性能基线

**Files:**
- Create: `docs/parity-matrix.md`
- Create: `docs/acceptance/legacy-baseline.md`
- Create: `plnk/tests/fixtures/legacy/README.md`
- Create: `scripts/check-parity.ps1`

**Interfaces:**
- Produces: 每项旧功能的 `P-xxx` 标识、旧源码引用、验收方法、当前设备状态（必测/未验证/延期）和负责人；后续任务必须引用这些标识。

- [ ] **Step 1: 写失败的覆盖检查**

`check-parity.ps1` 要求功能矩阵至少包含：托盘/热键、鼠标/触摸/压感笔、10 笔配置、荧光笔、橡皮、撤销/重做、隐藏笔迹、指针/平移、停靠/拖动、自动滚动、截图、贴图、选项、31 语言、多 DPI、多显示器、配置导入。

- [ ] **Step 2: 运行检查并确认因功能项缺失而失败**

Run: `powershell -File scripts/check-parity.ps1`  
Expected: FAIL，逐项列出尚未记录的 P-ID。

- [ ] **Step 3: 从当前工作树源码和实际旧版记录功能与基线**

读取 `Root.cs`、`FormCollection.cs`、`FormDisplay.cs`、`FormOptions.*`、`PinForm.cs`、`Local.cs`、INI 与语言文件。记录默认值、状态转换、已知旧缺陷（不得误当兼容要求）及旧版性能数据；脱敏配置 fixtures 不包含用户私人路径或截图。

- [ ] **Step 4: 重跑覆盖检查**

Expected: PASS，所有 P-ID 都有自动或人工验收方法。

- [ ] **Step 5: 建立独立工作分支并提交文档（经用户批准）**

保留当前 C# 修改，先单独提交旧版修复，再创建 `feature/plnk-rust`；不得用 stash/reset 丢弃现有修改。

### Task 2: 建立 Rust 工程与兼容设置层

**Files:**
- Create: `plnk/Cargo.toml`, `plnk/Cargo.lock`, `plnk/rust-toolchain.toml`, `plnk/build.rs`
- Create: `plnk/src/main.rs`, `plnk/src/settings.rs`, `plnk/src/error.rs`
- Create: `plnk/tests/settings.rs`
- Create: `plnk/tests/fixtures/legacy/{default,current,malformed}/`

**Interfaces:**
- Produces: `Settings::defaults() -> Settings`、`SettingsImport { settings: Settings, warnings: Vec<SettingsWarning> }`、`Settings::from_legacy(config: &mut dyn BufRead, pens: &mut dyn BufRead, hotkeys: &mut dyn BufRead) -> Result<SettingsImport, SettingsError>`、`SettingsStore::load_or_import() -> Result<SettingsImport, SettingsError>`。
- 约束：缺项回退到与“出厂 INI”或“完全缺文件”场景分别一致的默认值；单个非法值回退该键并追加带键名的 `SettingsWarning`，有效项继续导入；只有文件不可读等整体失败返回 `SettingsError`；导入不写旧路径。
- 兼容：保留旧文件查找顺序、注释和未知键；区分出厂默认值、代码缺文件回退值与已确认的旧版 bug，不能照抄错误行为。

- [ ] **Step 1: 写设置失败测试**

测试：`legacy_defaults_match_gink`、`shipped_and_missing_file_defaults_are_distinct`、`current_pen_and_hotkey_values_survive_import`、`comments_and_unknown_lines_survive_save`、`malformed_values_fall_back_without_writing_legacy`、`all_hotkey_families_roundtrip`。使用 Task 1 fixtures 中的精确布尔、颜色、宽度、路径和热键值，并覆盖 F1–F12、数字键盘与组合键。

- [ ] **Step 2: 运行并确认编译/断言失败**

Run: `cargo test --manifest-path plnk/Cargo.toml --test settings`  
Expected: 设置 API 尚不存在，或断言明确失败。

- [ ] **Step 3: 安装并验证构建前置环境（经用户批准）**

安装 rustup stable MSVC 与 Visual Studio C++ Build Tools；验证 `rustc --version`、`cargo --version`、`link.exe`。固定 `x86_64-pc-windows-msvc`，不在系统目录写入项目文件。

- [ ] **Step 4: 实现最小 Cargo 包和设置层**

Release 使用 `lto = "thin"`、`codegen-units = 1`、`opt-level = 3`；整体错误面向用户显示路径，键级警告显示键名，不以 panic 作为正常错误处理。

- [ ] **Step 5: 验证设置测试和工程门槛**

Run: `cargo test --manifest-path plnk/Cargo.toml`；`cargo fmt --check`；`cargo clippy --all-targets -- -D warnings`。  
Expected: 全部 PASS。

- [ ] **Step 6: Commit（经用户批准）** `feat(plnk): add build skeleton and legacy settings import`

### Task 3: 通过原生图形技术决策门

**Files:**
- Create: `plnk/src/platform/window.rs`, `plnk/src/platform/dpi.rs`
- Create: `plnk/src/gfx/device.rs`, `plnk/src/gfx/composition.rs`
- Create: `plnk/tests/ui_smoke.rs`
- Create: `docs/acceptance/graphics-gate.md`

**Interfaces:**
- Produces: `GraphicsDevice::new() -> Result<GraphicsDevice, GfxError>`、`OverlayWindow::set_click_through(bool)`、`ToolbarWindow::animate_to(PhysicalPoint, Duration, Easing)`、`MonotonicClock::now() -> Duration`。

- [ ] **Step 1: 写失败的最小 UI 验收**

`transparent_overlay_accepts_and_passes_input_by_mode`、`toolbar_animation_has_no_per_frame_app_callback`、`dpi_physical_coordinates_roundtrip`。动画验收采集 DWM/可见帧，目标 240Hz 中位 ≤4.2ms、p95 ≤8.4ms。

- [ ] **Step 2: 运行并确认测试因窗口/图形实现不存在而失败**

Run: `cargo test --manifest-path plnk/Cargo.toml --test ui_smoke -- --ignored --test-threads=1`。

- [ ] **Step 3: 实现最小透明层、D2D/DComp 设备和工具栏动画**

只画测试笔迹矩形与工具栏色块；验证 DirectComposition 透明顶层内容、输入穿透和句柄销毁。若 DComp 路径失败，记录证据并只允许一次有明确理由的 Direct2D 脏矩形备用原型，不静默退回全屏逐帧上传。

- [ ] **Step 4: 在用户机器执行图形决策门**

记录 60/240Hz、150% DPI、内存、CPU、动画可见帧、输入穿透与 100 次窗口重建结果。

- [ ] **Step 5: 决策门审查**

未满足透明输入、可见动画或笔迹坐标任一关键条件时停止后续功能开发，保留旧版并回到设计；通过后才 Commit（经用户批准）`feat(plnk): validate native graphics architecture`。

### Task 4: 应用生命周期、托盘、热键与日志

**Files:**
- Create: `plnk/src/app.rs`, `plnk/src/tray.rs`, `plnk/src/hotkeys.rs`, `plnk/src/telemetry.rs`
- Create: `plnk/tests/lifecycle.rs`

**Interfaces:**
- Produces: `App::run(Settings) -> Result<(), AppError>`、`AppMode::{Tray, Drawing, Pointer, Snapping}`、`HotkeyMap::matches(KeyboardEvent) -> Option<Command>`、`LogSink`（有界、滚动）。

- [ ] **Step 1: 写失败测试**

`second_instance_does_not_replace_first`、`invalid_or_conflicting_hotkey_is_reported`、`close_during_pending_io_drops_result`、`log_rotation_is_bounded`。

- [ ] **Step 2: 运行并确认失败** `cargo test --manifest-path plnk/Cargo.toml --test lifecycle`。

- [ ] **Step 3: 实现单实例、托盘菜单、热键注册/注销、消息路由和顶层错误处理**

开发版使用不同互斥体并允许禁用全局热键，避免与旧版冲突；不得注册服务或修改启动项。

- [ ] **Step 4: 验证**

单元测试 PASS；真实桌面完成 20 次打开/退出、冲突热键与托盘退出，日志无敏感截图内容。

- [ ] **Step 5: Commit（经用户批准）** `feat(plnk): add native application shell`

### Task 5: 原生工具栏与显示器刷新率动画

**Files:**
- Create: `plnk/src/ui/toolbar.rs`, `plnk/src/ui/animation.rs`, `plnk/src/ui/icons.rs`
- Create: `plnk/tests/toolbar.rs`, `plnk/tests/ui_toolbar.rs`

**Interfaces:**
- Produces: `ToolbarModel::visible_buttons(&Settings) -> Vec<ButtonId>`、`ToolbarLayout::hit_test(DipPoint) -> Option<ButtonId>`、`ToolbarAnimator::{enter,fast_exit,dock_to}`（入场常量 300ms、快速退出常量 160ms）、`ToolbarWindow::set_model(ToolbarModel)`。

- [ ] **Step 1: 写失败测试**

`layout_matches_legacy_enabled_button_order`、`hit_test_tracks_dpi_and_docking`、`easing_has_exact_endpoints_and_zero_end_velocity`、`retargeting_starts_from_current_visual_position`。

- [ ] **Step 2: 运行并确认失败** `cargo test --manifest-path plnk/Cargo.toml --test toolbar`。

- [ ] **Step 3: 实现工具栏绘制、命中测试、图标状态、停靠和 DComp 动画**

复刻现有布局，不新增形状工具；动画重定向不得瞬移，不在 UI 线程执行逐帧位置循环。

- [ ] **Step 4: 做视觉与性能验收**

比较旧版截图；验证 240Hz 滑入/退出/停靠、动画中 Esc、快速反复开关，应用无每帧动画回调。

- [ ] **Step 5: Commit（经用户批准）** `feat(plnk): add composited native toolbar`

### Task 6: 笔迹模型、几何与编辑历史

**Files:**
- Create: `plnk/src/ink/model.rs`, `plnk/src/ink/geometry.rs`, `plnk/src/ink/history.rs`, `plnk/src/ink/dirty.rs`
- Create: `plnk/tests/ink.rs`

**Interfaces:**
- Produces: `PenPoint { position: PhysicalPointF, pressure: f32, timestamp: Duration }`、`Stroke::push(PenPoint) -> DirtyRect`、`InkDocument::apply(Edit) -> DirtyRegion`、`undo() -> Option<DirtyRegion>`、`redo() -> Option<DirtyRegion>`。

- [ ] **Step 1: 写失败测试**

`pressure_is_clamped_and_pressureless_devices_use_fixed_width`、`smoothing_has_no_nan_for_duplicate_points`、`stroke_erase_uses_stroke_bounds_and_geometry`、`undo_redo_preserves_order_and_depth`、`dirty_region_contains_old_and_new_geometry`。

- [ ] **Step 2: 运行并确认失败** `cargo test --manifest-path plnk/Cargo.toml --test ink`。

- [ ] **Step 3: 实现笔迹与历史**

支持鼠标、触摸、压感笔、荧光笔和旧版历史语义：一个当前状态加最多 7 个 Undo 状态，Redo 在新编辑后失效；关闭批注时保存可恢复历史，重新打开后 Undo 能恢复退出前笔迹。内部使用编辑命令，不每帧克隆文档。

- [ ] **Step 4: 验证与压力测试**

所有单元测试 PASS；1000 条混合笔画、每条 1000 点的几何处理无异常，脏区域不会无限扩大。

- [ ] **Step 5: Commit（经用户批准）** `feat(plnk): add scalable ink document`

### Task 7: Pointer 输入与可书写 MVP

**Files:**
- Create: `plnk/src/input/pointer.rs`, `plnk/src/input/mouse.rs`
- Create: `plnk/src/gfx/ink_surface.rs`, `plnk/src/ink/renderer.rs`
- Create: `plnk/tests/ui_ink.rs`
- Create: `docs/acceptance/stylus.md`

**Interfaces:**
- Consumes: Task 3 窗口/设备、Task 6 笔迹模型。
- Produces: `PointerNormalizer::next(POINTER_INFO) -> Vec<PenPoint>`、`InkSurface::present(DirtyRegion)`、`InkController::{begin,append,end,cancel}`。

- [ ] **Step 1: 写失败测试**

`coalesced_history_preserves_order`、`contact_cancel_does_not_commit_partial_history`、`present_only_submits_dirty_region`、`pointer_timestamp_to_submit_is_monotonic`。

- [ ] **Step 2: 运行并确认失败** `cargo test --manifest-path plnk/Cargo.toml --test ui_ink -- --ignored --test-threads=1`。

- [ ] **Step 3: 实现输入规范化、活动笔画和静态缓存**

使用 Pointer 历史降低丢点；活动笔画与静态缓存分层，取消/离开窗口不产生错误笔画。

- [ ] **Step 4: MVP 验收**

可实际用鼠标书写、选笔、撤销、清空和退出；1000 条笔画后检查内存。用户用固定鼠标笔迹集（慢直线、快曲线、锯齿、圆、短点、荧光笔重叠）与旧版并排比较，自然、无明显额外抖动且响应不劣化即通过，不要求像素一致。压感笔/触摸当前标为“未验证”，不阻塞 MVP。

- [ ] **Step 5: 决策门与 Commit（经用户批准）**

笔迹质量和延迟不达标则暂停扩展；通过后 `feat(plnk): deliver native drawing MVP`。

### Task 8: 完整批注模式与工具行为

**Files:**
- Create: `plnk/src/ink/tools.rs`, `plnk/src/ui/cursor.rs`
- Modify: `plnk/src/app.rs`, `plnk/src/input/pointer.rs`
- Create: `plnk/tests/modes.rs`

**Interfaces:**
- Produces: `ToolController::dispatch(Command) -> Vec<Effect>`，覆盖笔、橡皮、指针、平移、隐藏笔迹、清空、撤销/重做和笔宽调整。

- [ ] **Step 1: 写状态机失败测试**

逐条覆盖 `P-xxx`：指针模式桌面点击穿透但工具栏仍可点击、橡皮端、平移坐标、隐藏后再写或 Undo 自动显示、隐藏笔按钮仍可用热键选择、Esc 优先级、截图模式禁止普通热键、退出后 Undo 恢复退出前笔迹、Redo 被新编辑清空。

- [ ] **Step 2: 运行并确认失败** `cargo test --manifest-path plnk/Cargo.toml --test modes`。

- [ ] **Step 3: 实现模式转换和工具行为**

明确 Esc：关闭笔宽面板/取消截图/退出批注的优先级；不保留旧版负帧计数作为时间协议。

- [ ] **Step 4: 验证**

状态机与 UI 测试 PASS；每种工具执行一次真实桌面冒烟测试。

- [ ] **Step 5: Commit（经用户批准）** `feat(plnk): complete annotation tools`

### Task 9: 原生选项、语言与配置连续性

**Files:**
- Create: `plnk/src/ui/options.rs`, `plnk/src/localization.rs`, `plnk/src/settings_store.rs`
- Create: `plnk/assets/lang/`（从旧资源迁移并保留许可）
- Create: `plnk/tests/localization.rs`, `plnk/tests/settings_roundtrip.rs`

**Interfaces:**
- Produces: `OptionsController::apply(SettingsPatch) -> Result<Vec<Effect>, SettingsError>`、`Language::get(MessageKey) -> &str`、`SettingsStore::save_atomic(&Settings)`。

- [ ] **Step 1: 写失败测试**

`settings_roundtrip_preserves_pen_hotkey_and_pin_values`、`save_is_atomic_and_does_not_touch_legacy`、`missing_translation_falls_back_to_english`、`all_31_language_files_parse_as_utf8`。

- [ ] **Step 2: 运行并确认失败** 对应两个 test target。

- [ ] **Step 3: 实现现有三页选项、热键编辑、语言加载和原子保存**

使用 Win32 控件/自绘与 DirectWrite 复刻现有三页选项、热键编辑、语言加载和原子保存；打开选项时注销两个全局热键，关闭时重新注册并保存。导入旧配置后由用户确认再保存到新目录，文件权限保持当前用户。隐藏按钮与禁用热键的区别、全局宽度覆盖和冲突提示必须与旧版矩阵一致。

- [ ] **Step 4: 验证**

检查中文、英文、阿拉伯文布局与缺键回退；使用当前用户配置的脱敏副本做导入对照。

- [ ] **Step 5: Commit（经用户批准）** `feat(plnk): add native options and localization`

### Task 10: 截图选择、PNG 与剪贴板

**Files:**
- Create: `plnk/src/snapshot/capture.rs`, `plnk/src/snapshot/selection.rs`, `plnk/src/snapshot/encoder.rs`
- Create: `plnk/src/platform/clipboard.rs`
- Create: `plnk/tests/snapshot.rs`, `plnk/tests/ui_snapshot.rs`

**Interfaces:**
- Produces: `SnapshotService::capture(PhysicalRect) -> Result<Bitmap, SnapshotError>`、`PngEncoder::encode(&Bitmap) -> Result<Vec<u8>, SnapshotError>`、`Clipboard::set_bitmap(Bitmap) -> Result<(), ClipboardError>`、`CloseOnSnap` 三种旧行为。

- [ ] **Step 1: 写失败测试**

`png_dimensions_and_pixels_match_capture`、`reverse_drag_and_subfive_pixel_selection_follow_legacy_rules`、`clipboard_busy_uses_bounded_retry_without_blocking_ui`、`cancel_restores_ink_and_clears_shading_after_long_frame`、`blank_only_close_matches_document_state`、`balloon_selects_latest_saved_file`。

- [ ] **Step 2: 运行并确认失败** `cargo test --manifest-path plnk/Cargo.toml --test snapshot`。

- [ ] **Step 3: 实现截图路径**

通过原型数据选择 DXGI 或 GDI 兼容路径；WIC 编码与文件 IO 离开 UI 线程，结果有界返回。剪贴板占用、零面积、取消和保存失败均有明确行为。

- [ ] **Step 4: 验证**

比较物理像素选区、PNG、剪贴板与保存目录；覆盖正/反向拖拽、任一边小于 5px 时全画布、截图包含可见笔迹、默认/自定义路径、托盘通知定位文件及 `CloseOnSnap=True/False/BlankOnly`，不得因长帧跳过清理重绘。普通截图与明确的“截图并贴图”入口保持独立，不误用旧文件中未生效的 `Pin_On_Snap` 键。

- [ ] **Step 5: Commit（经用户批准）** `feat(plnk): add native snapshot workflow`

### Task 11: 完整贴图窗口

**Files:**
- Create: `plnk/src/ui/pin.rs`, `plnk/src/ui/pin_menu.rs`, `plnk/src/snapshot/pin_store.rs`
- Create: `plnk/tests/pin.rs`, `plnk/tests/ui_pin.rs`

**Interfaces:**
- Produces: `PinWindow::{move_by,zoom_wheel,resize_corner,set_opacity,set_topmost,copy,save,close}`、`PinStore::{add(Bitmap) -> PinId,remove(PinId),close_all()}`；只有用户关闭贴图时才移除记录。

- [ ] **Step 1: 写失败测试**

`wheel_zoom_uses_top_left_anchor_and_legacy_bounds`、`corner_resize_uses_opposite_anchor_and_preserves_aspect`、`alt_wheel_changes_only_opacity`、`alt_hold_enables_click_through`、`copy_and_save_use_original_image`、`close_all_during_pending_save_is_safe`。

- [ ] **Step 2: 运行并确认失败** `cargo test --manifest-path plnk/Cargo.toml --test pin`。

- [ ] **Step 3: 实现贴图交互与菜单**

保留移动、20px 角热区（按 DPI 换算）、以左上角为锚的滚轮 ×/÷1.1 缩放、菜单 ×/÷1.25、角部等比缩放、Alt+滚轮透明度（0.2–1.0，步进 0.05）、Alt 按住点击穿透、共享置顶、复制/保存原图、双击/Esc 关闭和右键菜单；缩放下限同时受 0.05–8 倍与最短边 24 物理像素约束，双击关闭保留 400ms 防误触。

- [ ] **Step 4: 验证**

在 100%/150% DPI、多张贴图、不同来源截图下比较一图像素与物理像素；连续开关无窗口/位图泄漏。

- [ ] **Step 5: Commit（经用户批准）** `feat(plnk): add pinned snapshot windows`

### Task 12: 自动滚动与多显示器/DPI 硬化

**Files:**
- Create: `plnk/src/platform/monitor.rs`, `plnk/src/ink/autoscroll.rs`
- Modify: `plnk/src/platform/dpi.rs`, `plnk/src/app.rs`
- Create: `plnk/tests/monitors.rs`, `plnk/tests/autoscroll.rs`

**Interfaces:**
- Produces: `MonitorTopology::current() -> Result<MonitorTopology, PlatformError>`、`CoordinateSpace::{dip_to_physical,physical_to_virtual}`、`AutoScrollController::sample_at(Duration) -> Option<PhysicalVector>`。

- [ ] **Step 1: 写失败测试**

`mixed_dpi_negative_origin_roundtrip`、`dpi_change_retargets_windows_without_moving_ink`、`autoscroll_samples_at_legacy_150ms_cadence`、`mode_exit_discards_pending_scroll_delta`、`monitor_removal_moves_windows_to_visible_area`。

- [ ] **Step 2: 运行并确认失败** 对应 test targets。

- [ ] **Step 3: 实现拓扑、DPI 变化和独立自动滚动节奏**

屏幕采样与呈现解耦；不得因高刷新率提高采样字节带宽。保持自动滚动默认关闭且无新增配置/UI，只在指针模式工作；处理热插拔、负坐标和混合刷新率。

- [ ] **Step 4: 人工多屏矩阵**

当前只执行自动坐标/拓扑测试和当前显示器 150% DPI、240Hz 验收；实体 60/240Hz 双屏、100/125/150/200% 混合 DPI、负坐标热插拔因无第二台显示器标为“未验证”，不阻塞发布。以后提供设备时按同一清单补测。

- [ ] **Step 5: Commit（经用户批准）** `feat(plnk): harden multimonitor behavior`

### Task 13: 全量性能、泄漏与故障验收

**Files:**
- Create: `plnk/tests/perf.rs`, `plnk/tests/soak.rs`, `scripts/measure.ps1`
- Create: `docs/acceptance/performance.md`, `docs/acceptance/hardware.md`

**Interfaces:**
- Produces: 可重复运行的 Release 测量报告；指标定义与 Spec §4.2 完全一致。

- [ ] **Step 1: 写性能门槛和故障注入**

包含 20 次完整进程冷启动、240Hz 可见动画、1000 笔画、同一常驻进程内预热后 100 次打开/退出批注、10 张贴图、剪贴板占用、磁盘失败、句柄重建和 IO 中退出。进程退出不得冒充资源释放测试。

- [ ] **Step 2: 在未经优化的实现上确认门槛能失败**

至少验证探针能识别全屏更新、100ms 人为卡顿、未释放窗口和过高内存，避免测试永远变绿。

- [ ] **Step 3: 修复测量出的瓶颈**

每次只改一个根因并重测；不得通过删除功能、隐藏笔画或提高无界队列来达标。

- [ ] **Step 4: 执行完整验收**

保存 Release 性能结果、当前 Windows 11 + 鼠标人工结果、所有未通过项和无设备未验证项；当前设备性能预算未达标不得标记正式版完成，未验证硬件不得写成通过。

- [ ] **Step 5: Commit（经用户批准）** `test(plnk): add release acceptance gates`

### Task 14: 便携发布、并存试用与切换/回滚

**Files:**
- Create: `plnk/assets/icons/`, `plnk/LICENSE`（含原 MIT 声明）
- Create: `scripts/package.ps1`, `scripts/verify-package.ps1`
- Create: `docs/install.md`, `docs/rollback.md`, `docs/acceptance/release.md`

**Interfaces:**
- Produces: 便携 ZIP、SHA-256、版本信息、离线依赖检查、并存试用和一键恢复旧启动项的说明/脚本。

- [ ] **Step 1: 写包内容失败测试**

`package_contains_exe_assets_license_and_31_languages`、`package_runs_without_dotnet_or_admin`、`fresh_and_legacy_import_configs_stay_separate`、`rollback_restores_exact_legacy_target`。

- [ ] **Step 2: 运行并确认失败** `powershell -File scripts/verify-package.ps1`。

- [ ] **Step 3: 构建并验证便携包**

`cargo build --release --locked`；检查 exe ≤8MB（语言外置）、无 .NET/Microsoft.Ink 依赖、许可证完整、无用户配置/日志/截图泄露。

- [ ] **Step 4: 并存试用与用户验收**

默认禁用冲突热键，旧启动项不变；由用户明确批准功能矩阵和性能报告后才切换。切换脚本先备份精确旧目标，回滚脚本不得删除新数据。

- [ ] **Step 5: 最终 Commit（经用户批准）** `release(plnk): native rewrite complete`

---

## 里程碑与预计量级

| 里程碑 | 任务 | 可交付结果 | 预计有效开发日 |
|---|---|---|---:|
| M0 可行性与基线 | 1–3 | 功能矩阵、设置导入、原生图形决策门 | 3–5 |
| M1 可书写 MVP | 4–7 | 托盘、工具栏动画、真实书写、历史 | 6–9 |
| M2 工作流对齐 | 8–9 | 全批注模式、选项、31 语言 | 4–5 |
| M3 截图贴图与多屏 | 10–12 | 完整旧版工作流 | 5–7 |
| M4 正式发布 | 13–14 | 性能验收、便携包、切换与回滚 | 3–4 |

总计约 **21–30 个有效开发日**。每个里程碑结束重新估算；M0 或 M1 决策门失败时停止，不在错误底座上继续堆功能。

## 完成定义（Definition of Done）

- 当前 Windows 11 + 鼠标 + 当前显示器的功能矩阵和 Spec §4.2 性能指标均有实测证据；没有通过回调频率冒充满屏帧率。
- 压感笔、触摸和实体多显示器的自动/模拟测试通过，未提供设备的项目明确标为“未验证”，不写成通过，也不阻塞当前配置发布。
- 旧程序、旧配置和用户截图仍可回滚使用；新程序不依赖 .NET 或 Microsoft.Ink。
- 便携包在干净用户目录启动，31 语言和许可证完整，不包含私人配置、日志或测试截图。
- 用户实际试用并批准后才切换默认启动程序。
