# 04 并行开发计划

## 1. 工作包与执行者

按技术栈切成三个工作包，每个由一个 agent 负责（一个 agent 串行做完也可以，顺序不变）：

| 工作包 | 执行者 | 负责的任务 | 拥有的路径（只有它能改） |
| --- | --- | --- | --- |
| **WP-R（Rust 引擎）** | Agent R | S0 T0.3 的 Rust 基准；S1 全部；S3 T3.4（剪贴板方言）；S3 T3.7 中所有 **B 类**修复；S5 T5.1–T5.6、T5.8 | `crates/md-core/**`、`crates/markdown/**`、根 `Cargo.toml` 的 `members` |
| **WP-W（绑定与构建）** | Agent W | S2 T2.1（P0）、T2.2–T2.4、T2.6、T2.7、T2.10（指纹部分）、T2.11、T2.13（Web 部分）、T2.14；S5 T5.7、T5.9 | `crates/md-wasm/**`、`clients/mica_flutter/rust/**`、`clients/mica_flutter/web/index.html`、`scripts/**`、`justfile`、`.github/workflows/**`、`rust-toolchain.toml`、`.gitignore`、`AGENTS.md` 的依赖清单 |
| **WP-D（Dart 编辑器）** | Agent D | S0 T0.1、T0.2、T0.3（Dart 部分）、T0.4、T0.5；S2 T2.5、T2.8、T2.9（Dart 部分）、T2.12、T2.13（桌面部分）；S3 除 T3.4 外全部；S4 全部 | `clients/mica_flutter/lib/**`、`clients/mica_flutter/test/**`、`clients/mica_flutter/integration_test/**` |

共享文件的规则：

| 文件 | 规则 |
| --- | --- |
| `Cargo.lock` | 谁改了依赖谁更新；冲突时**不手改**，合并后在仓库根 `cargo check --workspace` 重新生成 |
| `scripts/file-size-budget.txt` | 只能把数字往下收紧；谁让行数下降谁收紧 |
| `docs/single-source/stages/*.md` | 每个执行者只追加自己负责的任务的「执行记录」；改方案正文须审核方同意 |
| `docs/single-source/02-architecture.md` | 冻结后只有审核方能改；执行者提「契约问题」 |
| `AGENTS.md` | W 只改依赖清单一行（S2 T2.2）；D 只改 Markdown 规则一条（S4 T4.12） |

## 2. 依赖图

```
                    ┌──────────── S0（D：T0.1 T0.2 T0.3 T0.4 T0.5；R：T0.3-Rust）────────────┐
                    │                                                                        │
  R: S1 T1.1 → T1.2 → T1.3 → T1.4 → T1.5 → T1.6 → T1.7 → T1.8 → T1.9 → T1.10 ══▶ 【契约冻结 F1】
                                                                                     │
  W: S2 T2.1(P0，Day 1 起，独立) ─────────────────────────────▶ 判定 ─┐               │
                                                                      ▼               ▼
                                         W: T2.2 → T2.3 → T2.4 → T2.6 → T2.7 → T2.11 → T2.14
                                                            │
  D:                                    T2.5(接口+wire 可在 F1 后立刻开始) → T2.8 → T2.9 → T2.12 → T2.13
                                                                                     │
                                                                      【S2 完成 F2】◀┘
                                                                            │
  R: S3 T3.4（剪贴板方言，F1 后即可开始，T3.5 之前完成）──────────────────┐ │
                                                                          ▼ ▼
  D: S3 T3.1 → T3.2 → T3.3 ─────────────────────────────────────────▶ T3.5 → T3.6 → T3.7 ⇄（R 修 B 类）→ T3.8 → T3.9 → T3.10 → T3.11
                                                                                                         │
  D: S4 T4.1 → T4.2 → T4.3（F2 后即可，与 S3 并行；只改 table.dart / render 单元格） ─────────────┐        │
                                                                                                  ▼        ▼
                                                                    D: T4.4 → T4.5 → … → T4.12（S3 合并后）
  R: S5 T5.1–T5.6、T5.8（F1 后任何时候）      W: S5 T5.7、T5.9（F2 后）
```

冻结点：

| 冻结点 | 内容 | 判定人 |
| --- | --- | --- |
| **F1 契约冻结**（S1 结束） | `mica_md` 公共 API（02 §2）、线格式（§3）、Schema（§6） | 审核方确认 S1 执行记录后在 02 顶部标注 |
| **F2 门面冻结**（S2 结束） | Dart `MarkdownCodec` 接口（02 §5） | 同上 |
| **P0 判定**（S2 T2.1） | wasm 阈值是否达标；不达标走 S2 的「退出方案」 | 审核方（必要时问用户） |

## 3. 冲突热点与顺序约束

| 热点 | 涉及任务 | 约束 |
| --- | --- | --- |
| `lib/editor/controller.dart` | S3 T3.5/T3.6（序列化）、S4 T4.5（单元格写入） | S3 的 T3.5、T3.6 合并之后，S4 才开始 T4.4 及以后 |
| `lib/editor/editor.dart` | S3 T3.2/T3.3/T3.5、S4 T4.5 | 同上 |
| `lib/editor/marks.dart` | S2 T2.5（剪走 `Mark`）、S3 T3.6、S4 T4.7 | 按阶段顺序；S3 删除时保留 S4 才删的函数 |
| `lib/main.dart` 库行数 | S2 T2.8（+1 行）、S3 T3.2（`main_shell.dart` 一处替换） | 每个 PR 自己保证净变化 ≤ 0 |
| `crates/md-core/src/html/` | S3 T3.4（R）、S3 T3.7 的 B 类修复（R） | 都在 WP-R 内，串行 |
| `clients/mica_flutter/rust/src/frb_generated.rs` 等生成物 | S2 T2.4 | 只有 W 运行 codegen；其他人需要新 FFI 函数时提给 W |

## 4. 分支与合并协议

1. 每个任务（或一组相邻任务）一个分支：`ss/<wp>/<task>`，例如 `ss/r/t1.4-block-ids`、`ss/d/t3.5-copy-via-codec`。
2. 合并方式：PR → main，**merge commit**（不 rebase、不 squash，保留每个任务的提交，审核方按任务号追溯）。
3. 合并前必须：
   - 基于最新 main（先把 main merge 进分支，解决冲突）；
   - `bash scripts/local-ci.sh all` 在合并后的分支上全绿，回执（`receipt:` 行）贴进 PR 描述；
   - 涉及 Windows 的任务（T2.14、T3.10、T4.11）附 Windows 运行证据；
   - PR 描述包含：任务号、做了什么、**偏离方案之处及原因**、契约问题（若有）、删除/新增行数。
4. 合并顺序遵守 §2 的依赖图；被依赖方没合并时，依赖方的 PR 保持 draft。
5. **不发版**。整个专项期间不执行 `just release` / `just ship`（发版由用户决定，AGENTS.md）。
6. 每个阶段的最后一个 PR 合并后，执行者在该阶段文档末尾补全「执行记录」，并在本文 §6 的进度表里勾选。

## 5. 日程（工作日，三个 agent 并行）

| 周 | Agent R | Agent W | Agent D |
| --- | --- | --- | --- |
| 1 | S1 T1.1–T1.6；T0.3-Rust | S2 T2.1（P0 验证，出判定） | S0 全部 |
| 2 | S1 T1.7–T1.10 → **F1** | T2.2–T2.4、T2.6（F1 后） | T2.5（F1 后）、准备 T2.9 黄金文件读取 |
| 3 | S3 T3.4（剪贴板方言） | T2.7、T2.10、T2.11、T2.13（Web）、T2.14 | T2.8、T2.9、T2.12、T2.13（桌面）→ **F2**；S3 T3.1–T3.3；S4 T4.1–T4.3 |
| 4 | T3.7 的 B 类修复（随到随修）；S5 T5.1–T5.3 | S5 T5.7、T5.9 | S3 T3.5–T3.11 |
| 5 | S5 T5.4–T5.6、T5.8 | 支持 Windows 验证、CI 收尾 | S4 T4.4–T4.12 |

预计 5 周（含分诊和真实路径验证的不确定性）。单个 agent 串行约 6–7 周。

## 6. 进度表（执行者勾选）

| 阶段 | 任务 | 状态 | PR / 提交 | 执行者 |
| --- | --- | --- | --- | --- |
| S0 | T0.1–T0.5 | ☐ | | D（T0.3-Rust：R） |
| S1 | T1.1–T1.10 | ☐ | | R |
| — | F1 契约冻结 | ☐ | | 审核方 |
| S2 | T2.1 P0 判定 | ☐ | | W → 审核方 |
| S2 | T2.2–T2.14 | ☐ | | W / D |
| — | F2 门面冻结 | ☐ | | 审核方 |
| S3 | T3.1–T3.11 | ☐ | | D（T3.4、B 类：R） |
| S4 | T4.1–T4.12 | ☐ | | D |
| S5 | T5.1–T5.12（发布除外） | ☐ | | R / W |
| — | 最终审核（05） | ☐ | | 审核方 |

## 7. 交给审核方的东西

全部完成后，执行者（或最后一个完成的 agent）在 `docs/single-source/README.md` 顶部把状态改为「开发完成，待审核」，
并附一张汇总表：每个阶段的 PR 列表、门禁回执、执行记录链接、未解决事项（「待用户决定」汇总、已知差异、超预算项）。
审核方按 [05-review-checklist.md](05-review-checklist.md) 逐条核对。
