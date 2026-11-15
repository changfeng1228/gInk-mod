# 旧版功能对齐矩阵（Task 1 基线）

**状态：** 已固化（2026-11-15）  
**旧版：** gInk 1.1.1（C# WinForms，namespace `gInk`）+ 当前工作树的截图贴图与动画修改  
**作用：** 后续任务的唯一功能清单。任何“旧版有、重写版遗漏”的功能都必须在这里显式列为“延期”或“未验证”，不得静默消失。

## 使用规则

- `P-xxx` 是稳定标识；后续测试、提交说明和验收记录必须引用。不得重排、复用或改写已发布的 ID。
- `功能族` 列使用 `scripts/check-parity.ps1` 的稳定键；17 个必需族见脚本头部。`app_lifecycle` 是额外族，用于托盘/单实例之外的进程生命周期项。
- 状态只允许三种：
  - `必测`：当前 Windows 11 + 鼠标 + 当前显示器（150% DPI / 240Hz）必须通过；自动测试或人工验收至少一种可执行。
  - `未验证`：当前设备没有对应硬件（压感笔、触摸、实体多显示器、混合 DPI），只能做自动/模拟覆盖；不得写成“已通过”。
  - `延期`：首个正式版不做，已明确记录并由用户接受；不得静默删除。
- `旧版源码引用` 指向 `src/`、`bin/` 或工程文档；行号为 2026-11-15 工作树状态。
- `验收方法` 必须包含 `自动` 或 `人工`；自动项尽量引用计划中已有的测试名，没有测试名的引用后续任务创建。
- 旧版缺陷以 `D-xxx` 记录在 `docs/acceptance/legacy-baseline.md`。缺陷不是兼容要求；矩阵行里写“修正 D-xxx”时，表示新程序按缺陷记录中的目标行为实现。
- `负责人` 是实施计划中的任务编号，最终由控制器指派；`TBD` 只用于尚未分配的辅助项。
- Undo 恢复只覆盖同一进程内关闭/重开批注；不保存笔迹或历史到磁盘（spec §4.1）。

## ID 范围

| 范围 | 功能族 | 说明 |
|---|---|---|
| P-001–P-005 | app_lifecycle | 单实例、托盘常驻、退出清理、异常日志、关于 |
| P-010–P-019 | tray_hotkeys | 托盘菜单、全局/工具热键、解析、冲突、Esc |
| P-025–P-030 | input_types | 鼠标、触摸、压感笔、笔反转端、笔尖光标、输入区域 |
| P-035–P-041 | pens_10 | 10 个笔槽、显示/隐藏、颜色、透明度、宽度、全局笔宽 |
| P-045–P-047 | highlighter | 荧光笔判定、半透明观感、选项标签映射 |
| P-050–P-052 | eraser | 笔画擦除、按钮/热键、擦除的撤销语义 |
| P-055–P-059 | undo_redo | 撤销、重做、退出恢复、隐藏自动显示、清空 |
| P-062–P-063 | ink_visible | 隐藏/显示笔迹、按钮与热键独立 |
| P-068–P-071 | pointer_pan | 指针穿透、模式切换、平移、平移撤销 |
| P-075–P-080 | dock_drag | 停靠、拖动、默认位置、动画、按钮顺序、尺寸 |
| P-085–P-086 | autoscroll | 自动滚动节奏、采样成本 |
| P-090–P-098 | snapshot | 截图进入、选区、DPI、PNG、剪贴板、气泡、关闭策略、内容、取消 |
| P-105–P-116 | pin | 贴图入口、窗口、移动、缩放、透明度、穿透、菜单、复制保存、关闭、置顶、本地化 |
| P-120–P-128 | options | 三页选项、图标开关、光标、路径、托盘图标、拖动/热键开关、生命周期、隐藏配置 |
| P-135–P-138 | languages_31 | 31 语言加载、切换、查找、顺序 |
| P-145–P-150 | multi_dpi | 主程序 DPI、截图缩放、贴图 DpiScope、混合 DPI、实体混合、DPI 变化 |
| P-155–P-159 | multi_monitor | 虚拟屏、工具栏约束、实体多屏、负坐标、热插拔 |
| P-165–P-174 | config_import | INI 解析、默认值差异、数值/布尔、热键、查找、保存、编码、非法值、Pin_On_Snap、出厂模板 |

## 功能矩阵

| ID | 功能族 | 功能 | 旧版源码引用 | 验收方法 | 状态 | 负责人 |
|---|---|---|---|---|---|---|
| P-001 | app_lifecycle | 单实例：命名互斥量 `Global\86280230-c3d2-4da0-8621-e2a2466fc136`；第二实例立即退出；开发版用独立 GUID 以便与正式版并存 | `src/Program.cs:18-23`; `BUILD.md:125-129` | 自动：Task 4 `second_instance_does_not_replace_first`；人工：连续启动两次只留一个托盘图标 | 必测 | Task 4 |
| P-002 | app_lifecycle | 托盘常驻：左键在空闲时启动批注；批注中停靠时取消停靠；非停靠时左键无动作 | `src/Root.cs:148-156,190-204` | 人工：托盘左键三种状态各一次；自动：Task 4 生命周期测试 | 必测 | Task 4 |
| P-003 | app_lifecycle | 退出清理：注销全局热键、关闭所有贴图、释放托盘图标后退出 | `src/Root.cs:1094-1101` | 自动：Task 4 退出/重启循环；人工：退出后无残留进程与托盘图标 | 必测 | Task 4 |
| P-004 | app_lifecycle | 顶层异常与日志：UI/非 UI 异常写 `crash.txt`；调试日志 `gInk_debug.log`（旧版缺陷见 D-013） | `src/Program.cs:46-137`; `src/Root.cs:273-283` | 自动：Task 4 `log_rotation_is_bounded` 加故障注入；人工：无未处理异常弹窗 | 必测 | Task 4 |
| P-005 | app_lifecycle | 关于对话框：版本与许可信息入口 | `src/Root.cs:328,1008-1012`; `src/FormAbout.cs` | 人工：托盘 About 打开并关闭 | 必测 | Task 4 |
| P-010 | tray_hotkeys | 托盘菜单项与启用状态：About、Options、Snapshot to pin、Close all pins、Exit；有贴图时 Close all 可用 | `src/Root.cs:325-336,351-355` | 人工：逐项点击；自动：Task 4 菜单模型测试（含 D-021 状态刷新） | 必测 | Task 4 |
| P-011 | tray_hotkeys | 全局热键注册/注销：默认 `Ctrl + Alt + G`；空闲时启动批注；批注中在指针与上次笔之间切换 | `src/Root.cs:23-48,1043-1056` | 自动：Task 4 热键注册测试；人工：真实桌面按热键 | 必测 | Task 4 |
| P-012 | tray_hotkeys | 第二全局热键“截图并贴图”：默认 `Ctrl + Alt + Shift + P`；空闲时先启动批注，120ms 后进入截图 | `src/Root.cs:1049-1055,1068-1085` | 自动：Task 4/11；人工：桌面按热键后完成贴图 | 必测 | Task 4/11 |
| P-013 | tray_hotkeys | 工具热键轮询：10 笔、橡皮、隐藏笔迹、指针、平移、撤销、重做、截图、贴图截图、清空；精确 modifier 匹配 | `src/FormCollection.cs:1140-1223`; `src/Hotkey.cs:109-112` | 自动：Task 4 `HotkeyMap::matches` 加 Task 8 状态机；人工：逐项按键 | 必测 | Task 4/8 |
| P-014 | tray_hotkeys | 热键字符串解析/显示：`None`、Ctrl/Alt/Shift/Win、字母/数字；F1–F12 与数字键盘按文档意图支持，修正 D-001/D-002/D-003 | `src/Hotkey.cs:17-107`; `src/Root.cs:648-692` | 自动：Task 2 `all_hotkey_families_roundtrip`；人工：选项页录入后重启仍保留 | 必测 | Task 2/9 |
| P-015 | tray_hotkeys | 热键冲突检测：选项页两两标红；主/贴图全局热键冲突时不注册 | `src/FormOptions.cs:410-434`; `src/Root.cs:1051` | 自动：Task 4 `invalid_or_conflicting_hotkey_is_reported`；人工：设重复键看提示 | 必测 | Task 4/9 |
| P-016 | tray_hotkeys | 打开选项时注销两个全局热键，关闭时重新注册 | `src/FormOptions.cs:33-35,290-299` | 人工：选项打开期间全局热键不触发、关闭后恢复；自动：Task 9 生命周期测试 | 必测 | Task 9 |
| P-017 | tray_hotkeys | 指针模式热键开关与输入互斥：`Allow_Hotkey_In_Pointer_Mode`；按住鼠标或截图期间暂停普通热键 | `src/FormCollection.cs:1140`; `src/Root.cs:71-72,755-758` | 自动：Task 8 状态机；人工：指针模式按下/松开后按工具键 | 必测 | Task 8 |
| P-018 | tray_hotkeys | Esc 优先级：取消截图 > 关闭笔宽面板 > 退出批注；Alt+F4 走同一分支 | `src/FormCollection.cs:1117-1137,1485-1508` | 自动：Task 8 Esc 优先级测试；人工：三种状态各按一次 | 必测 | Task 8 |
| P-019 | tray_hotkeys | 按钮 tooltip 显示当前热键，语言切换后更新 | `src/FormCollection.cs:405-414` | 人工：中文/英文对比按钮 tooltip | 必测 | Task 9 |
| P-025 | input_types | 鼠标输入（当前设备）：`CollectionMode.InkOnly` 收集、绘制与平滑观感 | `src/FormCollection.cs:266-280`; `readme.md:24` | 人工：固定鼠标笔迹集与旧版并排（慢直线、快曲线、锯齿、圆、短点、荧光笔重叠）；自动：Task 7 `ui_ink` | 必测 | Task 7 |
| P-026 | input_types | 触摸输入：单指绘制与多指行为 | `readme.md:24`; `src/FormCollection.cs:266-280` | 自动/模拟：Task 7 pointer 测试；无触摸设备，不得声称通过 | 未验证 | Task 7 |
| P-027 | input_types | 压感笔输入与压感宽度；无压感设备使用固定宽度 | `readme.md:27`; `src/FormCollection.cs:266-280`; `spec §6.3` | 自动：Task 6 `pressure_is_clamped_and_pressureless_devices_use_fixed_width`；无压感设备，不得声称通过 | 未验证 | Task 7 |
| P-028 | input_types | 笔反转端自动橡皮：`Cursor.Inverted` 时进入橡皮、恢复时退出 | `src/FormCollection.cs:539-566` | 自动/模拟：Task 8 橡皮端测试；无笔设备，不得声称通过 | 未验证 | Task 8 |
| P-029 | input_types | 笔尖光标：`Canvas_Cursor=1` 时按颜色/宽度/DPI 绘制光标圈 | `src/FormCollection.cs:969-1007` | 人工：鼠标下切箭头/笔尖各画一笔；自动：Task 8/9 设置往返 | 必测 | Task 8/9 |
| P-030 | input_types | 输入区域管理：平移/截图时用 1×1 输入矩形，退出后恢复全窗口 | `src/FormCollection.cs:649-654,836-841,851-873` | 自动：Task 8/10 状态机；人工：模式切换后仍能正常书写 | 必测 | Task 8/10 |
| P-035 | pens_10 | 10 个笔槽与出厂颜色/宽度/透明度；当前笔默认 Pen1 | `src/Root.cs:54,128-130,475-532`; `bin/pens.ini` | 自动：Task 2 `legacy_defaults_match_gink`；人工：首次启动看工具栏颜色 | 必测 | Task 2 |
| P-036 | pens_10 | 笔按钮显示/隐藏：`PenN_Enabled` 是“显示”开关而非禁用；隐藏的笔仍可用热键选择 | `src/FormCollection.cs:118-128`; `src/FormOptions.cs:100-137,283-288` | 自动：Task 2/8；人工：隐藏 Pen0 后按 0 仍选中 | 必测 | Task 8 |
| P-037 | pens_10 | 选笔与当前/上次笔：`CurrentPen`/`LastPen`；无可见笔时回退指针模式 | `src/Root.cs:128-130,215-225`; `src/FormCollection.cs:633-749` | 自动：Task 8 模式测试；人工：笔、指针、全局热键连续切换 | 必测 | Task 8 |
| P-038 | pens_10 | 每笔颜色：RGB 0–255；选项色板即时生效 | `src/Root.cs:610-646`; `src/FormOptions.cs:269-281` | 自动：Task 2 导入；人工：改色后画一笔 | 必测 | Task 2/9 |
| P-039 | pens_10 | 每笔透明度：INI `_Alpha` 为不透明度 0–255；内部 `Transparency = 255 − Alpha` | `src/Root.cs:638-641`; `src/FormOptions.cs:233-249` | 自动：Task 2 导入/往返；人工：Alpha 255/80 对比 | 必测 | Task 2/9 |
| P-040 | pens_10 | 每笔宽度：30–3000；选项 Thin/Normal/Thick 映射 30/80/500 | `src/Root.cs:642-645`; `src/FormOptions.cs:251-267,367-393` | 自动：Task 2/9；人工：三种宽度各画一笔 | 必测 | Task 2/9 |
| P-041 | pens_10 | 全局笔宽面板：`Pen_Width_Icon` 显示；宽度 `x²/30`；覆盖各笔宽度；不持久化 | `src/FormCollection.cs:800-816,903-967`; `src/Root.cs:130-131,706-709` | 自动：Task 8/9；人工：拖动面板改宽度后画一笔 | 必测 | Task 8/9 |
| P-045 | highlighter | 荧光笔判定与图标：`Transparency ≥ 100` 即用荧光笔图标；默认 Pen5/Pen9 为 Alpha 80、宽 500 | `src/Root.cs:507-531`; `src/FormCollection.cs:383-397` | 自动：Task 2 默认值；人工：选 Pen5 看图标与笔迹 | 必测 | Task 2/8 |
| P-046 | highlighter | 荧光笔重叠与半透明观感：重叠区域颜色、边缘、无额外抖动 | `src/FormCollection.cs:266-280`; `spec §8` | 人工：荧光笔画交叉重叠与旧版并排 | 必测 | Task 7 |
| P-047 | highlighter | 选项“Pencil/Highlighter”“Thin/Normal/Thick”标签到数值映射：Highlighter→80，Thin/Normal/Thick→30/80/500 | `src/FormOptions.cs:367-393` | 自动：Task 9 选项模型测试；人工：切换标签看数值 | 必测 | Task 9 |
| P-050 | eraser | 笔画擦除：`InkOverlayEraserMode.StrokeErase`，整笔删除；`EditingMode.Delete` | `src/FormCollection.cs:266-271,603-631` | 自动：Task 6 `stroke_erase_uses_stroke_bounds_and_geometry`；人工：擦一笔 | 必测 | Task 6/8 |
| P-051 | eraser | 橡皮按钮/热键与激活图标 | `src/FormCollection.cs:1461-1470,1157-1162` | 自动：Task 8 模式测试；人工：按钮和 E 键各一次 | 必测 | Task 8 |
| P-052 | eraser | 擦除的撤销语义：旧版不新增快照（D-004）；新程序按编辑命令实现并显式验证 | `src/FormCollection.cs:417-437,603-631`; `docs/acceptance/legacy-baseline.md` D-004 | 自动：Task 6/8 编辑历史测试；人工：擦除后 Undo/Redo | 必测 | Task 6/8 |
| P-055 | undo_redo | 撤销：环形历史，最多 7 个 Undo 状态；恢复整文档快照 | `src/Root.cs:116-119,232-241,357-375`; `src/FormCollection.cs:422-437` | 自动：Task 6 `undo_redo_preserves_order_and_depth`；人工：多笔逐步撤销 | 必测 | Task 6 |
| P-056 | undo_redo | 重做与新编辑清空重做：新笔画/清空/平移/退出都会清 `RedoDepth` | `src/Root.cs:404-422`; `src/FormCollection.cs:424` | 自动：Task 6；人工：撤销后重做，再画后重做失效 | 必测 | Task 6 |
| P-057 | undo_redo | 关闭/重开批注后 Undo 恢复退出前笔迹：仅同进程，不落盘 | `src/Root.cs:206-241,242-258` | 自动：Task 6 恢复历史测试；人工：画、退出、重开、Undo | 必测 | Task 6 |
| P-058 | undo_redo | Undo 自动显示隐藏笔迹 | `src/FormCollection.cs:1432-1444,1171-1179` | 自动：Task 8 状态机；人工：隐藏后 Undo | 必测 | Task 8 |
| P-059 | undo_redo | 清空按钮与清空可撤销 | `src/Root.cs:260-266`; `src/FormCollection.cs:1420-1430` | 自动：Task 6/8；人工：清空后 Undo | 必测 | Task 8 |
| P-062 | ink_visible | 隐藏/显示笔迹：按钮/热键；不删除数据；再次落笔自动显示 | `src/Root.cs:390-402`; `src/FormCollection.cs:441-444,468-471,1409-1418` | 自动：Task 8；人工：隐藏后画一笔 | 必测 | Task 8 |
| P-063 | ink_visible | 隐藏笔迹按钮与热键独立：`InkVisible_Icon=false` 隐藏按钮，热键仍生效 | `src/Root.cs:747-750`; `src/FormCollection.cs:1164-1168` | 自动：Task 8/9；人工：隐藏按钮后按 V | 必测 | Task 8/9 |
| P-068 | pointer_pan | 指针模式：覆盖层点击穿透，工具栏点击区仍可点（独立 `FormButtonHitter`） | `src/Root.cs:447-468`; `src/FormCollection.cs:580-601`; `src/FormButtonHitter.cs:30-53` | 自动：Task 8 加 Task 3 输入穿透测试；人工：指针模式下点桌面与工具栏 | 必测 | Task 3/8 |
| P-069 | pointer_pan | 指针/绘图模式切换：全局热键、`LastPen`、指针按钮 | `src/Root.cs:34-45`; `src/FormCollection.cs:788-797,1188-1192` | 自动：Task 8；人工：连续按全局热键往返 | 必测 | Task 8 |
| P-070 | pointer_pan | 平移模式：拖动移动全部笔迹；像素→墨迹单位换算 | `src/Root.cs:377-388`; `src/FormCollection.cs:501-504,529-531`; `src/FormDisplay.cs:241-252` | 自动：Task 8 平移坐标测试；人工：画后平移 | 必测 | Task 8 |
| P-071 | pointer_pan | 平移结束生成撤销快照 | `src/FormCollection.cs:529-531` | 自动：Task 8；人工：平移后 Undo | 必测 | Task 8 |
| P-075 | dock_drag | 停靠/取消停靠：`Docked`；工具栏滑出只留 dock 按钮；托盘点击收回 | `src/Root.cs:424-445`; `src/FormCollection.cs:768-786,1023-1041` | 自动：Task 5 `hit_test_tracks_dpi_and_docking`；人工：按钮与托盘各一次 | 必测 | Task 5 |
| P-076 | dock_drag | 工具栏拖动：`Allow_Dragging_Toolbar`；20px 阈值；四角保持在可见屏幕；位置持久化 | `src/FormCollection.cs:1256-1389`; `src/Root.cs:751-766,979-984` | 自动：Task 5；人工：拖到各边与多屏边界（当前单屏） | 必测 | Task 5 |
| P-077 | dock_drag | 工具栏默认位置：主屏工作区右下 −15px；多屏偏移；0,0 重定位 | `src/FormCollection.cs:233-260` | 自动：Task 5；人工：首次启动位置 | 必测 | Task 5 |
| P-078 | dock_drag | 工具栏动画：入场 300ms、快速退出 160ms、smoothstep；旧版由 4ms 定时器驱动（D-020），新程序不得复用；重定向从当前视觉位置 | `src/FormCollection.cs:1010-1094`; `src/HighResolutionTimer.cs`; `spec §6.1` | 自动：Task 5 `easing_has_exact_endpoints_and_zero_end_velocity`、`retargeting_starts_from_current_visual_position`、Task 3 `toolbar_animation_has_no_per_frame_app_callback`；人工：240Hz 观察 | 必测 | Task 3/5 |
| P-079 | dock_drag | 按钮顺序与图标：笔、橡皮、平移、指针、笔宽、隐藏、截图、撤销、清空、退出；激活态图标 | `src/FormCollection.cs:60-214` | 自动：Task 5 `layout_matches_legacy_enabled_button_order`；人工：与旧版截图对比 | 必测 | Task 5 |
| P-080 | dock_drag | 工具栏尺寸比例：高度 = 主屏高度 × `ToolbarHeight`；按钮 0.88/0.07 比例 | `src/FormCollection.cs:60-88`; `src/Root.cs:78` | 自动：Task 5；人工：150% DPI 对比 | 必测 | Task 5 |
| P-085 | autoscroll | 自动滚动：默认关闭；仅指针模式；按计时器采样屏幕位移，每 150ms 应用一次移动；退出丢弃待处理位移 | `src/Root.cs:67,547`; `src/FormDisplay.cs:83-90,366-432,575-591` | 自动：Task 12 `autoscroll_samples_at_legacy_150ms_cadence`、`mode_exit_discards_pending_scroll_delta`；人工不可达（旧版无入口，见 D-008） | 必测 | Task 12 |
| P-086 | autoscroll | 自动滚动采样成本：不得因高刷新率增加采样字节或呈现负担；采样与呈现解耦 | `src/FormDisplay.cs:366-432`; `spec §5` | 自动：Task 12/13 采样带宽与 CPU 断言 | 必测 | Task 12/13 |
| P-090 | snapshot | 截图模式进入：按钮/热键；十字光标；1×1 输入；工具栏收起；取消恢复 | `src/FormCollection.cs:818-849`; `src/FormDisplay.cs:126-144` | 自动：Task 10 `cancel_restores_ink_and_clears_shading_after_long_frame`；人工：进入后取消 | 必测 | Task 10 |
| P-091 | snapshot | 拖拽选区：正向/反向；任一边 <5px 变全屏；全屏等待 200ms | `src/FormCollection.cs:490-527`; `src/FormDisplay.cs:477-502` | 自动：Task 10 `reverse_drag_and_subfive_pixel_selection_follow_legacy_rules`；人工：正向/反向拖拽 | 必测 | Task 10 |
| P-092 | snapshot | 截图像素与 DPI 缩放：物理像素；`GetDeviceCaps` 比例；BitBlt CaptureBlt，失败回退 `CopyFromScreen` | `src/FormDisplay.cs:295-336` | 自动：Task 10 `png_dimensions_and_pixels_match_capture`；人工：150% 下比对选区与 PNG 尺寸 | 必测 | Task 10 |
| P-093 | snapshot | PNG 保存：默认目录自动创建；文件名 `yyyy-MM-dd HH-mm-ss.png`；自定义目录缺失时返回 null | `src/FormDisplay.cs:298-353` | 自动：Task 10；人工：默认/自定义路径各存一次 | 必测 | Task 10 |
| P-094 | snapshot | 剪贴板复制：10×100ms 有界重试；占用时不阻塞 UI | `src/FormDisplay.cs:338-346` | 自动：Task 10 `clipboard_busy_uses_bounded_retry_without_blocking_ui` | 必测 | Task 10 |
| P-095 | snapshot | 托盘气泡通知与点击定位最新截图文件：3s；`explorer /select` | `src/Root.cs:154-155,181-188,253-257,268-271` | 自动：Task 10 `balloon_selects_latest_saved_file`；人工：点气泡 | 必测 | Task 10 |
| P-096 | snapshot | `Close_On_Snap` 三态：True/False/BlankOnly；BlankOnly 按笔画数 | `src/Root.cs:76,723-730`; `src/FormDisplay.cs:493-501` | 自动：Task 10 `blank_only_close_matches_document_state`；人工：三态各截一次 | 必测 | Task 10 |
| P-097 | snapshot | 截图内容：包含可见笔迹；不含工具栏、遮罩和隐藏笔迹 | `src/FormDisplay.cs:230-239,477-501` | 自动：Task 10 像素断言；人工：画一笔后截图比对 | 必测 | Task 10 |
| P-098 | snapshot | 截图取消/连续 Esc：清理遮罩并恢复输入区域与工具栏 | `src/FormCollection.cs:1117-1137,851-873` | 自动：Task 10 取消测试；人工：截图选择时连按 Esc | 必测 | Task 10 |
| P-105 | pin | 截图并贴图入口：托盘立即进入、第二全局热键在空闲启动批注后延迟 120ms 进入 | `src/Root.cs:338-349,1068-1085`; `src/FormDisplay.cs:486-489` | 自动：Task 11；人工：托盘与热键各一次 | 必测 | Task 11 |
| P-106 | pin | 贴图窗口创建：无边框、工具窗口、不抢焦点、不进 Alt-Tab、共享 TopMost、DPI 感知，1 图像像素 = 1 物理像素 | `src/PinForm.cs:87-132`; `src/Root.cs:285-312` | 自动：Task 11 `ui_pin`；人工：150% 下贴图清晰且不缩放 | 必测 | Task 11 |
| P-107 | pin | 拖动移动贴图 | `src/PinForm.cs:309-372` | 自动：Task 11；人工：拖动多张贴图 | 必测 | Task 11 |
| P-108 | pin | 角部等比缩放：20px 热区；最短边 ≥24px；0.05–8×；对角锚定 | `src/PinForm.cs:73-84,166-180,279-307,385-418` | 自动：Task 11 `corner_resize_uses_opposite_anchor_and_preserves_aspect`；人工：四角各拖一次 | 必测 | Task 11 |
| P-109 | pin | 滚轮缩放：×1.1 / ÷1.1，左上角锚定 | `src/PinForm.cs:432-439` | 自动：Task 11 `wheel_zoom_uses_top_left_anchor_and_legacy_bounds`；人工：滚轮缩放 | 必测 | Task 11 |
| P-110 | pin | Alt+滚轮透明度：0.2–1.0，步进 0.05 | `src/PinForm.cs:207-210,432-439` | 自动：Task 11 `alt_wheel_changes_only_opacity`；人工：Alt+滚轮 | 必测 | Task 11 |
| P-111 | pin | Alt 按住点击穿透：`WM_NCHITTEST → HTTRANSPARENT` | `src/PinForm.cs:456-466` | 自动：Task 11 `alt_hold_enables_click_through`；人工：贴图覆盖窗口时 Alt 点击 | 必测 | Task 11 |
| P-112 | pin | 右键菜单：复制、另存为、放大、缩小、原始大小、半透明、置顶、关闭、关闭所有 | `src/PinForm.cs:182-198` | 自动：Task 11 菜单模型测试；人工：逐项点击 | 必测 | Task 11 |
| P-113 | pin | 复制原图与另存为 PNG：默认名 `pin yyyy-MM-dd HH-mm-ss.png` | `src/PinForm.cs:212-242` | 自动：Task 11 `copy_and_save_use_original_image`；人工：复制/保存后核对 | 必测 | Task 11 |
| P-114 | pin | 关闭交互：双击 400ms 防误触、Esc、Ctrl+C、菜单关闭、关闭所有 | `src/PinForm.cs:244-248,420-454`; `src/Root.cs:317-323` | 自动：Task 11 `close_all_during_pending_save_is_safe`；人工：双击/Esc/菜单各一次 | 必测 | Task 11 |
| P-115 | pin | 共享置顶切换：菜单“Always on top”影响所有贴图 | `src/PinForm.cs:200-205` | 自动：Task 11；人工：两张贴图切换 | 必测 | Task 11 |
| P-116 | pin | 贴图本地化菜单/标题：29 种语言缺新键时回退英文；修正 D-006 | `src/Local.cs:64-74`; `bin/lang/en-us.txt:58-68`; `bin/lang/zh-cn.txt:59-69` | 自动：Task 9 缺键回退测试；人工：中文/英文菜单 | 必测 | Task 9/11 |
| P-120 | options | 选项窗口三页：General/Pens/Hotkeys 与现有布局 | `src/FormOptions.cs:33-231`; `src/FormOptions.Designer.cs` | 人工：逐页操作；自动：Task 9 控件模型测试 | 必测 | Task 9 |
| P-121 | options | 工具图标显隐开关：橡皮、平移、指针、笔宽、隐藏、截图、撤销、清空 | `src/Root.cs:705-750`; `src/FormOptions.cs:37-52,301-340`; `src/FormCollection.cs:129-211` | 自动：Task 9；人工：逐项开关后看工具栏 | 必测 | Task 9 |
| P-122 | options | 画布光标箭头/笔尖 | `src/FormOptions.cs:61,395-398`; `src/FormCollection.cs:969-1007` | 人工：两种光标各画一笔；自动：Task 9 设置往返 | 必测 | Task 9 |
| P-123 | options | 截图保存路径编辑/选择 | `src/FormOptions.cs:63,348-365`; `src/FormDisplay.cs:298-304` | 自动：Task 9/10；人工：改路径后截图 | 必测 | Task 9 |
| P-124 | options | 白色托盘图标即时切换 | `src/Root.cs:552-570`; `src/FormOptions.cs:342-346` | 人工：切换后看托盘；自动：Task 9 设置往返 | 必测 | Task 9 |
| P-125 | options | 允许拖动工具栏/允许指针模式热键 | `src/Root.cs:71-72`; `src/FormOptions.cs:400-408` | 自动：Task 9；人工：开关后行为变化 | 必测 | Task 9 |
| P-126 | options | 选项生命周期：仅空闲可开；打开注销热键；关闭重注册并保存三份 INI | `src/Root.cs:1019-1031`; `src/FormOptions.cs:33-35,290-299` | 自动：Task 9 保存/重注册测试；人工：批注中打开选项被拒 | 必测 | Task 9 |
| P-127 | options | 隐藏配置项：`Close_On_Snap`、`Always_Hide_Toolbar`、`Toolbar_Height`、`Toolbar_Left/Top` | `src/Root.cs:723-770` | 自动：Task 2 导入加 Task 5/10 行为测试 | 必测 | Task 2/5/10 |
| P-128 | options | 语言切换后选项文本即时刷新 | `src/FormOptions.cs:175-231,436-443` | 人工：切换中/英/阿；自动：Task 9 | 必测 | Task 9 |
| P-135 | languages_31 | 31 个语言文件加载：UTF-8；`LanguageName`；反射字段覆盖；缺键保留英文默认；2 个文件带 BOM | `src/Local.cs:9-205`; `bin/lang/*.txt`（31 个） | 自动：Task 9 `all_31_language_files_parse_as_utf8`、`missing_translation_falls_back_to_english` | 必测 | Task 9 |
| P-136 | languages_31 | 语言切换即时生效：托盘菜单重建；下一次批注使用新语言 | `src/Root.cs:1087-1092`; `src/FormOptions.cs:436-443` | 人工：切到中文/英文/阿拉伯文；自动：Task 9 | 必测 | Task 9 |
| P-137 | languages_31 | 语言文件查找顺序：`./lang/` 优先，其次 exe 目录；缺失静默回退 | `src/Local.cs:92-97,151-158` | 自动：Task 9 查找顺序测试；人工：从不同 cwd 启动 | 必测 | Task 9 |
| P-138 | languages_31 | 语言列表顺序确定：旧版 `GetFiles` 未排序（D-016）；新程序稳定排序 | `src/Local.cs:100-118` | 自动：Task 9 列表顺序断言；人工：多次打开选项 | 必测 | Task 9 |
| P-145 | multi_dpi | 主程序 DPI 非感知与全屏层坐标（manifest `dpiAware=false`） | `src/gInk.exe.manifest:3-6`; `src/FormDisplay.cs:48-65` | 自动：Task 3/12 `dpi_physical_coordinates_roundtrip`；人工：150% 下笔迹/选区落点 | 必测 | Task 3/12 |
| P-146 | multi_dpi | 截图坐标按屏幕缩放因子转换（逻辑→物理） | `src/FormDisplay.cs:306-318` | 自动：Task 10；人工：150% 下选区与 PNG 尺寸比对 | 必测 | Task 10 |
| P-147 | multi_dpi | 贴图窗口 per-monitor-v2 感知与 `DpiScope`：所有几何读写 | `src/Root.cs:285-315`; `src/PinForm.cs:21-55,103-132` | 自动：Task 11；人工：100%/150% 下贴图尺寸 | 必测 | Task 11 |
| P-148 | multi_dpi | 混合 DPI 自动坐标往返：负坐标/非 100%（模拟） | `docs/superpowers/plans/2026-11-15-plnk-rust-rewrite.md` Task 12 | 自动：Task 12 `mixed_dpi_negative_origin_roundtrip` | 必测 | Task 12 |
| P-149 | multi_dpi | 实体混合 DPI 显示器：100/125/150/200% | `readme.md:33`; `spec §4.1` | 人工：无第二块显示器，无法执行；不得声称通过 | 未验证 | Task 12 |
| P-150 | multi_dpi | DPI 变化重定向：运行时改缩放不移动笔迹 | `docs/superpowers/plans/2026-11-15-plnk-rust-rewrite.md` Task 12 | 自动：Task 12 `dpi_change_retargets_windows_without_moving_ink` | 必测 | Task 12 |
| P-155 | multi_monitor | 虚拟屏覆盖：VirtualScreen 左/上/宽/高；旧版 -2 高度 hack（D-012）不复制 | `src/FormDisplay.cs:53-65`; `src/FormCollection.cs:216-228` | 自动：Task 12 拓扑测试；人工：当前单屏覆盖 | 必测 | Task 12 |
| P-156 | multi_monitor | 工具栏可见屏幕约束与主屏偏移：`IsInsideVisibleScreen`、`PrimaryLeft/Top` | `src/FormCollection.cs:57-58,233-260,1239-1251` | 自动：Task 12 `monitor_removal_moves_windows_to_visible_area`；人工：拖到边缘 | 必测 | Task 12 |
| P-157 | multi_monitor | 实体多显示器：截图跨屏、工具栏、指针、贴图 | `readme.md:26`; `changelog.txt` v0.5.2/v0.5.4 | 人工：无第二块显示器，无法执行；不得声称通过 | 未验证 | Task 12 |
| P-158 | multi_monitor | 负坐标显示器布局（模拟） | `spec §Review 1`; `docs/superpowers/plans/2026-11-15-plnk-rust-rewrite.md` Task 12 | 自动：Task 12 `mixed_dpi_negative_origin_roundtrip` | 必测 | Task 12 |
| P-159 | multi_monitor | 显示器热插拔/分辨率变化：旧版无处理（D-011） | `src/FormDisplay.cs`、`src/FormCollection.cs` 无 WM_DISPLAYCHANGE 处理 | 自动：Task 12 `monitor_removal_moves_windows_to_visible_area` | 必测 | Task 12 |
| P-165 | config_import | 非标准 INI 解析：跳过分节/注释；键 trim 加大写；值含第二个 `=` 的行跳过；未知键保留 | `src/Root.cs:572-781` | 自动：Task 2 `legacy_defaults_match_gink` 加 fixtures | 必测 | Task 2 |
| P-166 | config_import | 出厂默认 vs 代码缺文件默认：Pen3/Pen7 颜色、ToolbarHeight、AllowDragging、AllowHotkey、工具热键差异 | `src/Root.cs:71-78,475-550`; `bin/config_default.ini`; `bin/pens.ini`; `bin/hotkeys.ini` | 自动：Task 2 `shipped_and_missing_file_defaults_are_distinct` | 必测 | Task 2 |
| P-167 | config_import | 布尔/数值解析与范围：True/1/ON、False/0/OFF；RGB 0–255；宽度 30–3000；Toolbar int/float；Canvas_Cursor 0/1 | `src/Root.cs:610-776` | 自动：Task 2 `malformed_values_fall_back_without_writing_legacy` | 必测 | Task 2 |
| P-168 | config_import | 热键值往返：10 笔加工具加全局；F1–F12、数字键盘、组合键；修正 D-001/D-002/D-003 | `src/Hotkey.cs:17-107`; `src/Root.cs:648-692` | 自动：Task 2 `all_hotkey_families_roundtrip` | 必测 | Task 2 |
| P-169 | config_import | INI 查找顺序：当前目录优先，其次 exe 目录；缺失则默认 | `src/Root.cs:572-580,783-788` | 自动：Task 2 fixtures（cwd/exe 两种布局） | 必测 | Task 2 |
| P-170 | config_import | 保存保真：注释/未知键/行序保留；只更新已有键；不写旧路径 | `src/Root.cs:783-1006` | 自动：Task 2 `comments_and_unknown_lines_survive_save` | 必测 | Task 2 |
| P-171 | config_import | 配置编码/大小写/空白：StreamReader 默认 UTF-8；InvariantCulture；值 trim | `src/Root.cs:574,606-608` | 自动：Task 2 fixtures（BOM/CRLF/大小写） | 必测 | Task 2 |
| P-172 | config_import | 单个非法值回退该键并告警（旧版静默；新要求） | `src/Root.cs:610-776`; `spec §6.4` | 自动：Task 2 `malformed_values_fall_back_without_writing_legacy` | 必测 | Task 2 |
| P-173 | config_import | `Pin_On_Snap` 键未接线（D-005）：不得导入为行为；普通截图与“截图并贴图”分离 | `bin/config.ini:65-68`; `src/Root.cs:658-777` | 自动：Task 2/10 断言该键被保留但不改变行为；人工：确认普通截图不贴图 | 延期 | Task 10 |
| P-174 | config_import | 出厂模板 `config_default.ini`：首次安装模板；程序自身不读取；缺失键按代码默认 | `build/build.ps1:148-163`; `bin/config_default.ini` | 自动：Task 2 默认值测试；人工：干净目录首启 | 必测 | Task 2 |

## 未验证与延期汇总

| 项目 | 原因 | 不阻塞条件 | 后续动作 |
|---|---|---|---|
| P-026 触摸 | 当前设备无触摸屏 | 自动/模拟测试通过 | 有设备时按 P-026 人工补测 |
| P-027 压感笔 | 当前设备无压感笔 | 自动/模拟压力测试通过 | 有设备时按 P-027/P-028 补测 |
| P-028 笔反转端 | 当前设备无笔 | 自动/模拟橡皮端测试通过 | 与 P-027 一起补测 |
| P-149 实体混合 DPI | 只有一块显示器 | 混合 DPI 自动坐标测试通过 | 有第二块显示器时按 100/125/150/200% 补测 |
| P-157 实体多显示器 | 只有一块显示器 | 拓扑/负坐标自动测试通过 | 有第二块显示器时补测截图、工具栏、指针、贴图 |
| P-173 `Pin_On_Snap` | 旧版键未生效，Task 10 明确不采用 | 用户已接受延期 | 如未来需要自动贴图，另立 P-ID |

## 变更记录

| 日期 | 变更 | 说明 |
|---|---|---|
| 2026-11-15 | 创建 | Task 1 固化基线；17 个必需功能族全部有 P-ID；未验证/延期项已单列 |
