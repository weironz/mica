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

冻结点（与 §4 的审核停点对应）：

| 冻结点 | 内容 | 在哪个停点确认 |
| --- | --- | --- |
| **F1 契约冻结**（S1 结束） | `mica_md` 公共 API（02 §2）、线格式（§3）、Schema（§6） | G1 |
| **P0 判定**（S2 T2.1） | wasm 阈值是否达标；不达标走 S2 的「退出方案」 | G2 |
| **F2 门面冻结**（S2 结束） | Dart `MarkdownCodec` 接口（02 §5） | M2（并回检查） |

## 3. 冲突热点与顺序约束

| 热点 | 涉及任务 | 约束 |
| --- | --- | --- |
| `lib/editor/controller.dart` | S3 T3.5/T3.6（序列化）、S4 T4.5（单元格写入） | S3 的 T3.5、T3.6 合并之后，S4 才开始 T4.4 及以后 |
| `lib/editor/editor.dart` | S3 T3.2/T3.3/T3.5、S4 T4.5 | 同上 |
| `lib/editor/marks.dart` | S2 T2.5（剪走 `Mark`）、S3 T3.6、S4 T4.7 | 按阶段顺序；S3 删除时保留 S4 才删的函数 |
| `lib/main.dart` 库行数 | S2 T2.8（+1 行）、S3 T3.2（`main_shell.dart` 一处替换） | 每个 PR 自己保证净变化 ≤ 0 |
| `crates/md-core/src/html/` | S3 T3.4（R）、S3 T3.7 的 B 类修复（R） | 都在 WP-R 内，串行 |
| `clients/mica_flutter/rust/src/frb_generated.rs` 等生成物 | S2 T2.4 | 只有 W 运行 codegen；其他人需要新 FFI 函数时提给 W |

## 4. 分支、审核停点与合并协议

### 4.1 分支结构

```
main ─────────●──────────────●────────────────────────────────●──────▶   始终可发版
               \   (每周 merge main → single-source)          ↑
                \              ↑ G1 后 S1 并回        ↑ M2 后 S2 并回   ↑ G4 后 S3+S4+S5 并回
single-source ───●────●────●────●────●────●────●────●────●────●──▶   集成分支（长期）
                  ↑    ↑    ↑    ↑
            ss/<wp>/<task> 任务分支，PR 合进 single-source
```

1. **集成分支 `single-source`**：从 main 拉出，整个专项期间长期存在。**所有任务分支都合进它，不直接合进 main。**
   这样 main 在专项期间始终可以发版（用户修别的 bug、发补丁版本不受影响）。
2. **任务分支**：每个任务（或一组相邻任务）一个，`ss/<wp>/<task>`，例如 `ss/r/t1.4-block-ids`、`ss/d/t3.5-copy-via-codec`，
   从 `single-source` 拉出，PR 合回 `single-source`。
3. **同步 main**：至少**每周一次**把 main merge 进 `single-source`（由当周第一个开工的执行者做，PR 标题 `sync: main → single-source`）。
   专项要做约 5 周，`editor.dart`、`controller.dart` 这些文件 main 上也可能有人改，拖久了冲突会很难解。
4. **分段并回 main**（每次都是 `single-source` → main 的 PR，merge commit）：

| 并回 | 时机 | 为什么可以早并 |
| --- | --- | --- |
| S1（+ S0） | **G1 通过后** | 纯重构，行为不变；早并减少漂移 |
| S2 | **M2 通过后** | 只建门面和构建链，编辑器仍用旧代码，行为不变 |
| S3 + S4 + S5 | **G4 通过后**一起 | S3/S4 改用户可见行为，中间状态（块级镜像已删、行内镜像还在）不应进入可发版的 main |

   并回 main 前必须在 `single-source` 的当前头上跑 `bash scripts/local-ci.sh all` 全绿。

### 4.2 审核停点

执行者到了停点：在 §6 进度表把该停点标为「⏸ 待审核」，在对应阶段文档的执行记录里写好停点材料，**然后停止该工作包**
（其他工作包里不依赖这个停点的任务可以继续）。执行 agent 无法直接通知审核方——**由用户转告**（例如在审核方的会话里说「审核 G1」）。
审核方拉取 `single-source`，按下表核对，把结论写进 [reviews.md](reviews.md)：通过，或列出必须修改的项。通过后才能继续。

| 停点 | 时机 | 停的是谁 | 审核方核对什么（对应 05 的条目） | 为什么必须停 |
| --- | --- | --- | --- | --- |
| **G1 契约冻结** | S1 T1.10 完成 | WP-R 停；WP-W 的 T2.2 起、WP-D 的 T2.5 起等待 | 公共 API 与 02 §2 一致；`facade_surface.rs` 覆盖全；消费方 diff 为空（A5、A6）；新测试都证明过会失败（B6）；基准 ±5% | 契约冻结后 S2–S5 全部基于它，之后改契约代价最大 |
| **G2 wasm 判定** | S2 T2.1 P0 出结果 | WP-W 停（T2.2 起） | P0 每项阈值的实测数字与方法；不达标时把「退出方案 vs 继续优化」的选择**交给用户** | 决定 Web 走 wasm 还是保留 Dart 镜像，后续路径完全不同 |
| **M2 并回检查** | S2 全部完成 | **不停工**：分支上 S3/S4 可以继续；只是 S2 并回 main 要等它 | 构建链在全部 Web 入口生效（A7、03 §4）；三平台契约测试回执（B2–B4）；编辑器行为没变（S0 差分棘轮未动）；启动耗时（D1–D2） | 改了 CI 和构建链，进 main 前要有人看过 |
| **G3 删除之前** | S3 的 T3.4、T3.5、T3.7、T3.8 完成（已切到引擎，**Dart 旧实现还在**），T3.6 之前 | WP-D 停（T3.6、S4 T4.4 起）；WP-R 只做 B 类修复 | 剪贴板移植验收（逐字节）回执；T3.7 分诊表每条有结论、B 类全修（C1）；往返测试（B7）；把「用户可见变化」「待用户决定」整理给**用户** | 删 Dart 代码是单向门；用户可感知的行为变化要用户拍板 |
| **G4 最终审核** | S4、S5 全部完成 | 全部 | [05-review-checklist.md](05-review-checklist.md) 全部条目 + F 节抽查 | 收尾，通过后 `single-source` 并回 main |

不设停点的部分：S0（只加测试和工具）、S2 除 P0 外（行为不变，M2 并回前看）、S5（不碰编辑器）。这些出了问题，G4 也能兜住。

### 4.3 每个 PR 的要求

1. 合并前：先把 `single-source` 的最新头 merge 进任务分支；`bash scripts/local-ci.sh all` 全绿，回执（`receipt:` 行）贴进 PR 描述。
2. 合并方式：**merge commit**（不 rebase、不 squash，保留每个任务的提交，审核方按任务号追溯）。
3. 涉及 Windows 的任务（T2.14、T3.10、T4.11）附 Windows 运行证据。
4. PR 描述包含：任务号、做了什么、**偏离方案之处及原因**、契约问题（若有）、删除/新增行数。
5. 合并顺序遵守 §2 的依赖图和 §4.2 的停点；被依赖方没合并、或停点没通过时，依赖方的 PR 保持 draft。
6. **不发版**。专项期间不执行 `just release` / `just ship`（发版由用户决定，AGENTS.md）。main 上的发版不受影响——它不含专项的未完成部分。
7. 每个阶段的最后一个 PR 合并后，执行者补全该阶段文档末尾的「执行记录」，并更新 §6 进度表。

## 5. 日程（工作日，三个 agent 并行）

| 周 | Agent R | Agent W | Agent D |
| --- | --- | --- | --- |
| 1 | S1 T1.1–T1.6；T0.3-Rust | S2 T2.1（P0 验证，出判定） | S0 全部 |
| 2 | S1 T1.7–T1.10 → **F1** | T2.2–T2.4、T2.6（F1 后） | T2.5（F1 后）、准备 T2.9 黄金文件读取 |
| 3 | S3 T3.4（剪贴板方言） | T2.7、T2.10、T2.11、T2.13（Web）、T2.14 | T2.8、T2.9、T2.12、T2.13（桌面）→ **F2**；S3 T3.1–T3.3；S4 T4.1–T4.3 |
| 4 | T3.7 的 B 类修复（随到随修）；S5 T5.1–T5.3 | S5 T5.7、T5.9 | S3 T3.5–T3.11 |
| 5 | S5 T5.4–T5.6、T5.8 | 支持 Windows 验证、CI 收尾 | S4 T4.4–T4.12 |

预计 5 周（含分诊和真实路径验证的不确定性）。单个 agent 串行约 6–7 周。

停点大致落在：G2 ≈ 第 1 周末，G1 ≈ 第 2 周中，M2 ≈ 第 3 周末，G3 ≈ 第 4 周中，G4 ≈ 第 5 周末。
每个停点的审核周转按 1 个工作日计（含用户转告）；G2、G3 需要用户拍板时，按用户回复时间顺延。
停点期间被阻塞的工作包可以做不依赖它的任务（见 §2），例如 G1 期间 WP-D 继续 S0、WP-W 继续 P0。

## 6. 进度表（执行者勾选）

状态：☐ 未开始 · ▶ 进行中 · ⏸ 待审核 · ✅ 完成 / 通过 · ✖ 审核要求修改（见 reviews.md）

| 阶段 | 任务 | 状态 | PR / 提交 | 执行者 |
| --- | --- | --- | --- | --- |
| S0 | T0.1–T0.5 | ✅ 修正已集成；桌面精确启动指标的限制已记录 | S0 修正 merge `5d8feb7`；Rust 基准 `9af0de7` 随 S1 集成 | D（T0.3-Rust：R） |
| S1 | T1.1–T1.10 | ✅ 四项修订完成，集成头 Rust 门禁通过 | merge `f2bcb70`；核心优化 `20ecde0`、I3 测试 `88a46af`、FFI lock `349f19a`；[回执](stages/S1-core-sdk.md#2026-10-27--g1-第四项集成头-rust-门禁) | R / 协调者 |
| **G1** | 契约冻结审核 → S0+S1 并回 main | ✅ 通过（2026-10-27）：契约已冻结，S0+S1 已并回 main | [reviews.md](reviews.md) G1 四项修订已交付 | 审核方 |
| S2 | T2.1 P0 | ✅ 成本归因及 opt-3 硬门槛复测已交付 | 核心 `6594f12` 已经 `20ecde0` 单独进入 S1；实验 `ss/w/p0-costs` 与 spike 未集成 | W |
| **G2** | wasm 判定 | ✅ 通过（附条件：opt-3、M2 用正式产物重测，见 [reviews.md](reviews.md)） | [决定与验证范围](reviews.md)、[复测记录](stages/S2-bindings.md#执行记录) | W / 审核方 |
| S2 | T2.2–T2.14 | ✅ M2 复核通过并并回 main（`2b0dedee`） | [reviews.md](reviews.md) M2 记录 | W / D |
| **M2** | 并回检查 → S2 并回 main | ✅ 通过（2026-10-27）：S2 已并回 main（[reviews.md](reviews.md)） | | 审核方 |
| S3 | T3.1–T3.5、T3.7、T3.8 | ✅ 完成：编辑器已切到引擎，G3 附条件通过（3 条必须修改由审核方完成，`9add59c9`） | `e7b4a533`、`71b27bc2`、`7ef6bbba`、`3b6c477f`；[执行记录](stages/S3-editor-boundary.md#执行记录) | D（T3.4、B 类：R） |
| **G3** | 删除之前审核（用户可见变化交用户） | ✅ 通过（附条件，2026-10-28）：可执行 T3.6；3 条必须修改在 S3 内完成（[reviews.md](reviews.md)） | | 审核方 + 用户 |
| S3 | T3.6、T3.9–T3.11 | ✅ T3.6/T3.9 完成（Dart 镜像删除，`flutter analyze` 0 error/0 warning，2,076 passed）；T3.10 Web 与 Windows 桌面真路径均通过（桌面集成 1/0，wire 契约 10/0）；T3.11 门禁 `ALL GREEN` | 提交见 S3 执行记录；[T3.6/T3.9](stages/S3-editor-boundary.md#t36-删除2026-10-28)、[T3.10](stages/S3-editor-boundary.md#t310-真实路径验证2026-10-28)、[T3.11](stages/S3-editor-boundary.md#t311-门禁2026-10-28) | D |
| S4 | T4.1–T4.12 | ✅ 全部完成（表格单元格 `TableCell(text, marks)` 模型；`marks.dart` 2,455→373 行，行内解析器/序列化器删除；差分工具与基线删除；兼容性 9 用例、性能基准达标；门禁 `ALL GREEN`；Web + Windows 真路径验证通过） | `ss/d/t4-table-cells`：`7759ba60`（T4.1–T4.5）、`ce4ad1c9`（T4.7–T4.9）、`1b1bdcea`（T4.8/T4.10）、`6a65343e`（T4.11）；[执行记录](stages/S4-table-cells.md#执行记录) | D |
| S5 | T5.1–T5.12（发布除外） | ✅ 全部完成（mica-md 随时可发布：compat 模块、README/SCHEMA/CHANGELOG/THIRD_PARTY、4 示例、public_api/schema_doc_sync/never_panic 三守门测试均证可失败、npm 包 13 文件 412 kB、`just sdk-check` ALL GREEN；Q1–Q6 用户拍板） | `ss/r/t5-sdk`：`30170178`（T5.1–T5.9）+ 文档提交；[执行记录](stages/S5-sdk-release.md#执行记录)、[发布手册](stages/S5-sdk-release.md#发布手册t512用户批准后执行不自动) | D（代 R/W） |
| **G4** | 最终审核 → `single-source` 并回 main | ✅ 通过（2026-10-29），已并回 main（[reviews.md](reviews.md)） | | 审核方 |

## 7. 交给审核方的东西

每个停点的材料写在对应阶段文档的执行记录里；审核结论写在 [reviews.md](reviews.md)。
全部完成后，执行者（或最后一个完成的 agent）在 `docs/single-source/README.md` 顶部把状态改为「开发完成，待审核（G4）」，
并附一张汇总表：每个阶段的 PR 列表、门禁回执、执行记录链接、未解决事项（「待用户决定」汇总、已知差异、超预算项）。
审核方按 [05-review-checklist.md](05-review-checklist.md) 逐条核对。
