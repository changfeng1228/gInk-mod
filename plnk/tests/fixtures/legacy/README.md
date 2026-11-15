# 旧版配置 fixtures（`plnk/tests/fixtures/legacy/`）

**用途：** 为 Task 2 的 `Settings::from_legacy` 提供 golden fixtures，固定旧版 INI 的默认值、缺项/非法值回退、注释与未知键保留、热键往返。  
**状态：** 目录与规则由 Task 1 定义；实际 fixture 文件由 Task 2 创建（本文件不复制用户配置）。  
**来源：** 仓库 `bin/` 出厂模板、当前工作树旧版源码、用户当前运行配置的只读、脱敏副本。旧版配置目录绝不被写入。

## 1. 计划目录结构

```text
plnk/tests/fixtures/legacy/
  README.md                      # 本文件：规则与清单
  default/                       # 干净安装的出厂配置
    config.ini                   # 程序实际读取的配置（出厂值）
    config_default.ini           # 模板原件；程序不读取，用于证明 build.ps1 的复制来源
    pens.ini
    hotkeys.ini
    expected.json                # 期望的 Settings 字段（见 §4）
  current/                       # 当前用户配置的脱敏副本
    config.ini
    pens.ini
    hotkeys.ini
    expected.json                # 期望导入后的 Settings 与警告
  malformed/
    missing-file/                # 三个文件都不存在
    missing-keys/                # 文件存在但键大量缺失
    invalid-values/              # 颜色/宽度/布尔/浮点/热键非法
    syntax/                      # 分节、注释、空行、重复键、未知键、值内含 =
    encoding/                    # UTF-8 BOM、CRLF/LF、非 ASCII
    hotkeys/                     # F1–F12、NumPad、组合键、None 往返
    expected.json                # 每个子目录的期望结果与警告键名
```

Task 2 可以拆成多个 `expected.json`；唯一要求是每个 fixture 都能独立说明“输入 → 期望 Settings/警告 → 是否允许写旧路径”。

## 2. `default/` 的内容

- `config.ini`：干净安装时程序实际读取的内容；来源为 `bin/config.ini`（开发配置，CRLF）或 `bin/config_default.ini`（模板，LF）；两者键值相同、行尾不同，需在 provenance 中说明选择了哪个并保持字节一致。
- `config_default.ini`：`bin/config_default.ini` 的原样副本；程序自身不读取；用于区分“出厂模板”和“代码缺文件默认”。
- `pens.ini`、`hotkeys.ini`：`bin/` 出厂模板的原样副本。
- `expected.json`：按 `docs/acceptance/legacy-baseline.md` §4.1–§4.3 断言：
  - 10 个笔槽的 Enabled、RGB、Alpha（不透明度）、Width；
  - 所有 `config.ini` 键的布尔/数值/路径；
  - 所有热键键的解析结果；
  - `CloseOnSnap = blankonly`、`ToolbarHeight = 0.05`、`SnapshotBasePath = %USERPROFILE%/Pictures/gInk/`；
  - `Pin_On_Snap` 与 `AutoScroll` 不改变任何行为（保留原始行，不产生 Settings 字段；`Pin_On_Snap` 是已知缺陷 D-005）。

## 3. `current/` 的内容与脱敏

`current/` 来自用户当前运行配置的只读副本，只保留 `config.ini`、`pens.ini`、`hotkeys.ini`。规则：

1. **不得包含**：用户名、机器名、邮箱、绝对用户路径、截图/PNG/JPG、剪贴板图像、`crash.txt`、`gInk_debug.log`、任何崩溃转储或调试日志。
2. **路径**：把任何展开后的用户绝对路径替换为 `%USERPROFILE%/...` 或 `<redacted>/...`；不得保留盘符下的个人目录。
3. **非身份行为值可以保留**：语言文件名（如 `zh-cn`）、布尔值、颜色、宽度、Alpha、自定义热键字符串。若团队认为自定义热键也需脱敏，可替换为结构合法的等价字符串，并在 `expected.json` 同步；测试只依赖结构、键名和解析结果。
4. **保留解析相关字节**：键的大小写、空白、注释、未知键、行尾、BOM 等，凡是解析/保存测试依赖的部分必须原样保留。
5. **不整体复制目录**：只复制上述三个 INI；不复制 `lang/`、图标、旧 exe 或任何截图。
6. **不写旧路径**：Task 2 的导入测试必须断言没有写入旧配置目录；fixtures 是只读输入。
7. **provenance**：在 `expected.json` 或单独的 `PROVENANCE.md` 记录来源类别（仓库模板/用户配置只读副本）、日期、构建/工作树状态；不要写用户绝对路径。

当前用户配置的相对特征（不得写成绝对路径）：语言不是 `en-us`，全局/清空/贴图热键与出厂值不同；`config.ini` 中没有 `Pin_On_Snap`（旧版 `SaveOptions` 不会补写缺失键，D-015）。`current/expected.json` 应断言这些自定义值在导入后保留，同时 `Pin_On_Snap` 不产生行为。

## 4. 必须固化的默认值差异

Task 2 的 `shipped_and_missing_file_defaults_are_distinct` 需要以下差异（详见 `docs/acceptance/legacy-baseline.md` §4）：

| 项 | 出厂 `bin/` 值 | 代码缺文件/缺键默认 |
|---|---|---|
| Pen3 颜色 | 250,180,55 | 235,180,55 |
| Pen7 颜色 | 240,140,200 | 250,140,200 |
| `Toolbar_Height` | 0.05 | 0.06 |
| `Allow_Dragging_Toolbar` | False | true |
| `Allow_Hotkey_In_Pointer_Mode` | False | true |
| `Hotkey_Eraser` | E | None |
| `Hotkey_InkVisible` | V | None |
| `Hotkey_Undo` | Ctrl + Z | None |
| `Hotkey_Redo` | Ctrl + Y | None |
| `Hotkey_Snapshot` | Ctrl + Alt + S | None |
| `Pen0_Hotkey` … `Pen9_Hotkey` | 0 … 9 | None |
| `Hotkey_Global` | Ctrl + Alt + G | Ctrl + Alt + G |
| `Hotkey_SnapPin` | Ctrl + Alt + Shift + P | Ctrl + Alt + Shift + P |

不得把差异“统一”为同一个默认值；两种场景必须分别可测。

## 5. `malformed/` 的用例清单

| 子目录 | 输入重点 | 期望 |
|---|---|---|
| `missing-file/` | 三个 INI 都不存在 | 代码缺文件默认；导入不失败、不创建/写入旧路径 |
| `missing-keys/` | 只保留 `Language_File`、一个 `Pen1_*` 组、空 `hotkeys.ini` | 每个缺失键回退到代码默认；有效键保留；产生带键名的警告（新要求 P-172） |
| `invalid-values/` | RGB 300/-1、Alpha 999、Width 10/5000、`Toolbar_Height=abc`、`Canvas_Cursor=7`、布尔 `maybe`、`PEN10_RED`（D-023） | 单个非法值回退该键并警告；其他键继续导入；不把 `PEN10_*` 映射到 Pen1；不写旧文件 |
| `syntax/` | `[SECTION]`、`- % ' / ! # [` 开头注释、空行、重复键、小写键、未知键、值内含第二个 `=` | 分节/注释跳过；键 trim 后大写；重复键后者生效；未知行保留；含第二个 `=` 的行跳过 |
| `encoding/` | UTF-8 无 BOM、UTF-8 BOM、CRLF/LF、非 ASCII 值 | 均按 UTF-8 解析；BOM 不进入首个键名；行尾不改变语义 |
| `hotkeys/` | `None`、`Ctrl + Alt + G`、`F1`…`F12`、NumPad、`Ctrl + Z`、`Ctrl+Y`、非法 `Ctrl +`、`F13` | 合法热键完整往返；F10–F12 修正 D-001；NumPad 修正 D-002；非法值整键回退并警告（D-003） |

所有 fixture 的 `expected.json` 必须列出：解析后的字段、警告的键名和数量、是否允许保存、是否触碰旧路径。

## 6. 与 Task 2 测试的对应关系

| Task 2 测试 | fixture |
|---|---|
| `legacy_defaults_match_gink` | `default/` |
| `shipped_and_missing_file_defaults_are_distinct` | `default/` 对比 `malformed/missing-file/` |
| `current_pen_and_hotkey_values_survive_import` | `current/` |
| `comments_and_unknown_lines_survive_save` | `malformed/syntax/`、`current/` |
| `malformed_values_fall_back_without_writing_legacy` | `malformed/invalid-values/`、`malformed/missing-keys/` |
| `all_hotkey_families_roundtrip` | `malformed/hotkeys/` |

31 个语言文件的解析属于 Task 9（`all_31_language_files_parse_as_utf8`），不放入本目录；本目录只保留 `Language_File` 键值。

## 7. 脱敏检查清单（提交前逐项确认）

- [ ] 没有用户名、机器名、邮箱、绝对用户路径。
- [ ] 没有截图、剪贴板图像、崩溃转储、调试日志。
- [ ] 没有整体复制旧配置目录；只有三个 INI 的必要内容。
- [ ] 保留了解析/保存测试依赖的大小写、空白、注释、未知键、行尾和 BOM。
- [ ] `current/` 的用户自定义热键/语言值已确认不含身份信息，或已替换并同步 `expected.json`。
- [ ] 所有导入测试只读旧路径；没有写入、重命名、删除旧配置。
- [ ] `expected.json` 中的警告键名与旧版源码的键名一致（如 `Pen3_Red`、`Toolbar_Height`）。
- [ ] provenance 只写来源类别、日期、工作树状态，不写用户绝对路径。

## 8. 变更记录

| 日期 | 变更 | 说明 |
|---|---|---|
| 2026-11-15 | 创建 | Task 1 定义 fixture 结构、默认值差异、malformed 用例与脱敏规则；实际文件留给 Task 2 |
