# plnk 发布流程

plnk 跟随 gInk-mod 仓库用 Git 管理。版本号只有一个来源：`plnk/Cargo.toml` 中 `[package]` 的 `version`。`plnk/build.rs` 用它生成 exe 的 VERSIONINFO，`scripts/package-plnk.ps1` 在不传 `-Version` 时也从这里读取，因此不要在其他地方手写版本号。

发布产物（`plnk-v<版本>-win64` 目录和同名 zip）统一放在仓库的 `release\` 目录；该目录已被 `.gitignore` 忽略，不会提交。桌面只放一个指向最新包的快捷方式，不放包目录或 zip。打包脚本会自动把启动文件夹的自启动快捷方式和桌面快捷方式指向新包；具体规则见第 7 步。

## 版本策略

- 每次升级至少升 patch 位：0.1.0 → 0.1.1 → 0.1.2。
- 功能里程碑升 minor 位：0.1.x → 0.2.0，例如新增选项窗口、完整语言界面等成组功能。
- 功能完全对齐旧版 gInk 后升 1.0.0。
- 新版本生成新的 `plnk-v<版本>-win64` 目录和 zip，不复用旧版本目录名。
- 每个发布包必须能对应一个 Git 提交：包内 `VERSION` 文件的 `Commit=` 行记录构建时的 `git rev-parse --short HEAD`。若构建时 Git 不可用会写 `Commit=unknown`，这样的包不能作为可追溯的正式发布包。

## 发布步骤

1. 修改代码并自测：
   - `cargo fmt --check`
   - `cargo clippy --all-targets -- -D warnings`
   - `cargo test --manifest-path plnk/Cargo.toml`
2. 提交（或先提交）功能改动，保证工作区干净。`scripts/bump-version.ps1` 会拒绝在 `plnk/Cargo.toml` 之外存在未提交改动时运行，避免发布包与 Git 提交对不上。
3. 升版本：
   `powershell -ExecutionPolicy Bypass -File scripts/bump-version.ps1 -Version 0.1.1`
   脚本只改 `plnk/Cargo.toml` 的 `[package] version`，并刷新 `plnk/Cargo.lock`；之后提交这次版本变更。
4. 重新构建 Release：
   `cargo build --release --locked --manifest-path plnk/Cargo.toml`
5. 打包：
   `powershell -ExecutionPolicy Bypass -File scripts/package-plnk.ps1`
   默认读取 `plnk/Cargo.toml` 的版本，在仓库的 `release\` 目录（不存在会自动创建）生成 `release\plnk-v<版本>-win64` 和同名 zip，并在包内 `VERSION` 写入 `Commit=<短哈希>`。需要输出到其他位置时用 `-Destination <目录>`，但不要把发布包放到桌面。打包成功后脚本会自动调用 `scripts/update-plnk-shortcuts.ps1` 更新快捷方式；CI 或试跑时可加 `-SkipShortcuts` 跳过这一步。
6. 验证包：
   `powershell -ExecutionPolicy Bypass -File scripts/verify-plnk-package.ps1`
   默认验证 `release\plnk-v<当前 Cargo.toml 版本>-win64`；也可用 `-Package <路径>` 直接指定包目录，或用 `-Version <x.y.z> -Destination <目录>` 组合指定。验证过程会在包内生成 `data\`，验证结束后应删除，保持发布目录干净（zip 在验证前生成，本来就不含 `data`）。
7. 更新快捷方式：**已自动化，无需手动操作**。`scripts/package-plnk.ps1` 在打包完成后会自动调用 `scripts/update-plnk-shortcuts.ps1 -PackagePath <新包目录>`：
   - 开始菜单启动文件夹中的自启动快捷方式 `plnk.lnk` 始终指向最新的 `release\plnk-v<版本>-win64\plnk.exe`（工作目录为该包目录，最小化启动），因此自启动永远跟随最新版本。
   - 桌面已有的 `plnk.lnk` 或 `gInk.lnk` 只有在当前目标位于仓库 `release\plnk-v*` 目录内时才会被重定向到新包；指向旧版 gInk 或其它文件的快捷方式不会被改动。两个快捷方式都不存在时会创建 `plnk.lnk`。
   - 桌面上只保留这一个快捷方式，**不要把包目录或 zip 复制到桌面**；不要手工修改系统启动项。
   只有包目录被移动或删除、需要手工修复快捷方式时，才直接运行 `scripts/update-plnk-shortcuts.ps1 -PackagePath <包目录>`。当前工作区状态：启动文件夹快捷方式为 `plnk.lnk`，桌面快捷方式为 `gInk.lnk`，两者都指向最新包。确认旧包不再使用后再删除 `release\` 中的旧目录和 zip。

## 可追溯与回滚

发布包可追溯：用 `VERSION` 的 `Commit=` 找到对应提交，在 `git worktree` 或 `git checkout` 到该提交后重新执行第 4–6 步，即可复现同一个版本号。不要手工修改包内文件再压缩，否则包哈希与 `Commit=` 记录会失真。
