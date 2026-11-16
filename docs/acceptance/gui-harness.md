# 桌面验收自动化

需要真实指针、键盘和屏幕像素的验收统一由 `scripts/gui-acceptance.ps1` 驱动。

## 为什么不当成普通测试跑

这些测试会移动真实指针、注入真实点击。默认 `cargo test` 不跑它们（全部标了默认不运行），
以免打断正在使用电脑的人。

## 两层让出

| 层 | 覆盖时机 | 阈值 |
|---|---|---|
| 天枢运行时护栏 | agent 发起命令的那一刻 | `RIVET_CU_YIELD_MS`，默认 1200ms |
| 测试内闸门 `InputGate` | 测试运行的每一秒 | `IDLE_THRESHOLD`，2 秒 |

两层都必须通过才会接管指针。任一层判定用户在用，就跳过，**一个输入事件都不注入**。

## 用法

```powershell
# 只看要跑什么
powershell -File scripts/gui-acceptance.ps1 -WhatIf

# 真正跑
powershell -File scripts/gui-acceptance.ps1

# Release 性能探针
powershell -File scripts/gui-acceptance.ps1 -Release
```

## Exit code

| code | 含义 |
|---|---|
| 0 | 全部通过 |
| 1 | 至少一项失败 |
| 2 | 至少一项因用户占用而跳过，且无失败 |

## 结果文件

`.rivet/scratch/gui-acceptance.json`，每项含 `Target` / `State` / `Detail`。
`State` 只可能是 `Passed` / `Skipped` / `Failed`（预览模式为 `WhatIf`）。

## 结论行前缀

| 前缀 | 含义 |
|---|---|
| `PLNK_PASS:` | 通过 |
| `PLNK_FAIL:` | 失败 |
| `PLNK_SKIP:` | 因用户占用跳过 |

## 不要把 SKIP 当 PASS

exit 2 表示「没验成」，不是「验过了」。需要真实验收结论时，应在用户离开电脑时重跑。

## 新增桌面测试时

1. 保留 `#[ignore]`，否则无人看守的 `cargo test` 会动真实指针。
2. 在测试体第一行调用 `acquire_desktop!();`。
3. 若测试需要自己的清理，在此之后建立，`Drop` 会先恢复指针再执行你的清理。
