# 单一实现专项：Markdown 引擎只留一份（S0–S5）

> 状态：**全部完成，G4 通过，S0–S5 已并回 main**（2026-10-29；2026-10-26 立项）。未发版：crate / npm 发布与 App 发版都等用户批准（见 [S5 发布手册](stages/S5-sdk-release.md)）。
> 审核方：本仓库的维护 agent，在审核停点 G1–G3 抽查、全部完成后（G4）按 [05-review-checklist.md](05-review-checklist.md) 逐条审核。
> 本目录是这个专项的**唯一权威文档**；其他文档里与之冲突的描述，以本目录为准。

## 最终结果（2026-10-29）

| 阶段 | 停点 | 并回 main | 要点 |
| --- | --- | --- | --- |
| S0 基线与安全网 | 随 G1 | `8d78ba6c` | 差分对拍、性能与启动基线、剪贴板快照 |
| S1 引擎拆成 SDK 核心 | G1 ✅（2026-10-27，契约冻结） | `8d78ba6c` | `crates/md-core`（`mica-md`）+ `mica-markdown` 门面，消费方零改动 |
| S2 绑定与门面 | G2 ✅ / M2 ✅（2026-10-27） | `2b0dedee` | 桌面 FFI、Web wasm（opt-3）、Dart `MarkdownCodec`；wasm 上限 256 KiB、复制 ≤ 16 ms 由用户定 |
| S3 编辑器边界 | G3 ✅（2026-10-28） | `6cc2e526` | 粘贴 / 复制 / 导出 / AI 走引擎，Dart Markdown 镜像删除 |
| S4 表格单元格 | G4 ✅（2026-10-29） | `6cc2e526` | 单元格内存模型改为「文本 + 标记」，Dart 行内解析删除；`encode_cell` 对任意单元格无损 |
| S5 SDK 交付 | G4 ✅（2026-10-29） | `6cc2e526` | 文档、Schema、示例、API / 健壮性测试、npm 包、许可证审计、`just sdk-check`；`publish = false` |

- 最终树门禁（审核方 Linux）：`local-ci.sh all` ALL GREEN——Rust 1160 + 213 passed、Flutter 2030 passed、wasm 203,003 B、Chrome 浏览器套件全绿。
- `clients/mica_flutter/lib` 中不再有任何 Markdown 语法实现；`AGENTS.md` 的规则已改为「Markdown 只有一份实现」。
- **未做 / 待用户决定**：App 发版、crate / npm 首次发布（手册见 [S5](stages/S5-sdk-release.md) 文末，用户本人执行）；
  对外发布前给 SDK 的 HTML 导出加「严格」档（建议）；Mica 内部粘贴优先读 Markdown（可选改进，见 G3 记录）。
- 审核方没有 Windows，Windows 集成测试与「旧版客户端读新表格」以执行方回执为准。
- 每个停点的核对过程、裁决与审核方修复都在 [reviews.md](reviews.md)；各阶段执行过程在 `stages/` 的执行记录里。

## 一句话

Markdown 的解析、序列化、HTML 渲染**只在 Rust 里写一份**（`crates/markdown` 拆出的 SDK 核心），
服务端/CLI/MCP 原生调用、桌面走 FFI、Web 走 wasm；Flutter 编辑器只操作「块 + 标记」，
**只在边界上**（粘贴、复制、导入、导出、AI 输入输出）调用这个引擎，编辑器热路径不再解析 Markdown。
引擎按 SDK 的形状组织，具备对外发布的条件（发布本身由用户拍板）。

## 为什么做

- 现在同一套 Markdown 规则写了两遍：Rust（权威）+ Dart（镜像，约 4,000 行）。2026-09 以来改 Dart 镜像的
  提交 9 个，其中 8 个同一提交也改了 Rust —— **每个解析 bug 修两遍**（#96、#98 是最近的例子）。
- 现有防线（共享黄金文件对拍）只能抓「两边不一样」，抓不到「两边错得一样」，也省不掉「修两遍」。
- 详细推导、同类产品调研、数据见 [01-background.md](01-background.md)。

## 阅读顺序（给执行 agent）

1. [01-background.md](01-background.md) —— 为什么、和别人怎么做、为什么选这条路（**先读，避免重新论证**）。
2. [02-architecture.md](02-architecture.md) —— 目标架构与**契约**：Rust 公共 API 签名、跨语言 JSON 线格式、
   Dart 门面接口、块/标记 Schema、性能预算。**契约冻结后改动须经审核方同意。**
3. [03-inventory.md](03-inventory.md) —— 现状清单：Dart 镜像逐函数的去向、调用点、35 个测试文件的处理方式。
4. [04-parallel-plan.md](04-parallel-plan.md) —— 工作包划分、依赖图、谁改哪些文件、合并规则、日程。
5. 各阶段详细方案（代码级）：
   - [stages/S0-baseline.md](stages/S0-baseline.md) —— 基线与安全网（差分对拍、性能基线）
   - [stages/S1-core-sdk.md](stages/S1-core-sdk.md) —— Rust 引擎按 SDK 形状拆分（行为不变）
   - [stages/S2-bindings.md](stages/S2-bindings.md) —— 绑定：桌面 FFI + Web wasm + Dart 门面
   - [stages/S3-editor-boundary.md](stages/S3-editor-boundary.md) —— 编辑器边界改走引擎，删除 Dart 块级解析与序列化
   - [stages/S4-table-cells.md](stages/S4-table-cells.md) —— 表格单元格改用「文本 + 标记」内存模型，删除 Dart 行内解析
   - [stages/S5-sdk-release.md](stages/S5-sdk-release.md) —— SDK 化交付（文档、示例、npm 包、发布流水线）
6. [05-review-checklist.md](05-review-checklist.md) —— 审核方的验收清单（执行方自检也用它）。
7. [reviews.md](reviews.md) —— 各停点的审核结论（审核方写，执行方读）。

## 阶段总表

| 阶段 | 目标 | 前置 | 粗估 | 风险 | 完成标志 |
| --- | --- | --- | --- | --- | --- |
| S0 | 差分对拍工具 + 性能基线 | 无 | 2–3 天 | 低 | 对拍报告生成；基线数字入档 |
| S1 | `crates/md-core`（SDK 核心）+ `mica-markdown` 变门面，**消费方零改动** | S0 基线 | 5–7 天 | 低 | 全部现有测试、记分牌、黄金文件逐字节不变 |
| S2 | FFI 新接口 + wasm crate + 构建链 + Dart `MarkdownCodec` 门面 | S1 契约冻结（P0 验证可提前） | 6–8 天 | 中 | 两个平台的门面通过同一套契约测试；P0 阈值达标 |
| S3 | 粘贴/复制/AI/导出改走门面；删 `markdownToBlocks`、`_rangeMarkdown`、`selectionHtml` 等 | S2 | 5–7 天 | 中 | 对拍差异清零或逐条记录；编辑器行为测试全绿 |
| S4 | `TableData` 改为 `TableCell(text, marks)`；删 Dart 行内解析/序列化 | S2（可与 S3 部分并行） | 4–6 天 | 中低 | `lib/` 中不再有任何 Markdown 语法实现 |
| S5 | 公开 API 文档、Schema 规范、示例、npm 包、发布流水线（发布需用户批准） | S1 | 5–7 天 | 低 | `cargo publish --dry-run` / `npm pack` 在 CI 通过 |

合计约 6–7 周（单个 agent 串行）；按 [04-parallel-plan.md](04-parallel-plan.md) 三个 agent 并行约 5 周。

> **更正**：讨论阶段（见 01 §8）曾说 S4「要做文档数据迁移、风险高」。调研代码后确认**存储层早已统一**：
> Rust `encode_table_cell` 与 Dart `TableData._encodeCell` 都把带格式的单元格写成 `{text, marks}`，只有
> Dart 的**内存模型**还是 Markdown 原文字符串。所以 S4 **不需要数据迁移**，只改 Dart 内存模型。

## 已拍板的决定（不再重新论证）

| # | 决定 | 来源 |
| --- | --- | --- |
| D1 | 方向：**Markdown 只出现在边界，Rust 是唯一实现** | 用户 2026-10-26 |
| D2 | 引入 wasm（仅 Web 端），新增依赖 `wasm-bindgen`（Rust crate）+ `wasm-bindgen-cli`（构建工具），登记进依赖豁免清单 | 用户 2026-10-26 |
| D3 | 按 SDK 形状重构；**对外发布另行批准**（S5 只做到「随时可发布」） | 用户 2026-10-26 |
| D4 | 不新增中立 AST 层：SDK 的模型就是现有的**扁平块 + 标记**模型（理由见 02 §1.2） | 本方案 |
| D5 | Web 绑定用 `wasm-bindgen` + `dart:js_interop`，**不用** flutter_rust_bridge 的 web 构建（理由见 02 §4.3） | 本方案 |
| D6 | 表格单元格**存储格式不变**（已是 `{text, marks}` / 旧字符串兼容），只改 Dart 内存模型 | 本方案 |
| D7 | 行为差异以 Rust 为准；Dart 侧刻意的粘贴启发式（去 front matter、LLM 双围栏修复、代码语言纠错）保留为引擎调用前后的 Dart 预/后处理 | 本方案 |

## 不在本专项范围（已讨论、未排期）

见 [01-background.md §9](01-background.md)：接口数据结构单一来源、服务端/本地库业务规则纯函数模块、
CRDT（Web 换 yrs wasm）、原生无损导出格式 `.mica.zip`（N1–N4）。本专项建好的 wasm 通道是 CRDT 统一的前提。

## 本目录收录的历史文档

| 文档 | 原位置 | 说明 |
| --- | --- | --- |
| [history/markdown-boundary-comparison.md](history/markdown-boundary-comparison.md) | `docs/architecture/` | 2026-08-07 AppFlowy / AFFiNE 源码级对照：为什么服务端必须能解析 Markdown |
| [history/rust-migration-assessment-2026-07-21.md](history/rust-migration-assessment-2026-07-21.md) | `docs/plans/` | 2026-07-21 Rust 化评估：两个 wasm spike 的实测数据、「可行但暂不推进」的结论与触发条件 |

两份历史文档的结论**被本专项更新**：触发条件「出现 fixture 抓不到的漂移」已满足（两边同错只能修两遍，见 01 §3）。

## 给执行 agent 的工作规则

1. **先读 01、02，不要重新论证已拍板的决定**；发现契约有问题，写进 PR 描述的「契约问题」一节，等审核方回复，不要自行改契约。
2. 遵守仓库根 `AGENTS.md` 的全部规则，尤其：修复配回归测试并先证明测试会失败；门禁要出回执（`scripts/lib-receipt.sh`）；
   `main.dart` 库与其他登记单元的行数只许减不许增（`scripts/check-file-size.sh`）；提交信息写根因。
3. 每个任务（如 `T2.3`）一个提交或一组提交，提交信息首行带任务号：`feat(md-core): T1.4 injectable block ids`。
4. 每个阶段结束交付一份**阶段报告**，追加到该阶段文档末尾的「执行记录」一节：做了什么、偏离方案的地方及原因、
   门禁回执、对拍报告、性能数字、删除/新增行数。审核方以这份记录为入口。
5. 不确定的产品行为（例如某个粘贴结果该不该变）不要猜：列进「待用户决定」表，先按 Rust 行为实现。
6. **在集成分支 `single-source` 上开发**，任务分支合回它，不直接合进 main（[04 §4.1](04-parallel-plan.md)）。
7. **到审核停点必须停**：G1（S1 完成）、G2（wasm 判定）、G3（S3 删 Dart 代码之前）、G4（最终），另有不停工的并回检查 M2（S2 完成）。
   到点后在进度表标「⏸ 待审核」并停下该工作包，由用户转告审核方；审核结论写在 [reviews.md](reviews.md)（[04 §4.2](04-parallel-plan.md)）。
