# 单一实现专项：Markdown 引擎只留一份（S0–S5）

> 状态：**G1 通过、契约已冻结，S0+S1 已并回 main；S2 集成验证中，M2 未通过**（2026-10-27 更新；2026-10-26 立项）。G2 附条件通过。执行方：三个并行工作包，协调者汇总。
> 审核方：本仓库的维护 agent，在审核停点 G1–G3 抽查、全部完成后（G4）按 [05-review-checklist.md](05-review-checklist.md) 逐条审核。
> 本目录是这个专项的**唯一权威文档**；其他文档里与之冲突的描述，以本目录为准。

## 当前交付与审核入口（2026-10-27）

> **M2 审核结论（2026-10-27，详见 [reviews.md](reviews.md)）**：**要求修改，4 条**。两项预算由用户改定（wasm 上限 256 KiB；复制 10k 改为 ≤ 16 ms 绝对预算），
> Export HTML 只要求 Clipboard 跨端逐字节，FFI 边界现在不改。要做的是：改构建脚本门槛并记 Linux 体积、补测 Web 复制、改正「全部来自 FFI 边界」的归因、
> 把 `scripts/md-wasm-evidence/` 移出集成分支。M2 不停工，S3 可继续。下文是审核前执行方的快照。

> **审核与执行状态（2026-10-27）**：审核方已在 [reviews.md](reviews.md) 记录 G1 复核通过，并冻结 02 §2 / §3 / §5；S0+S1 已在 `8d78ba6c` 并回 main。S2 当前只在 `single-source` 集成与验证，不代表 M2 通过。
> G2 已由审核方 **附条件通过**：正式构建用 opt-3，M2 用正式产物重测。审核方结论只写入 [reviews.md](reviews.md)。

| 工作包 | 可复核提交 / 分支 | 当前状态 |
| --- | --- | --- |
| S0 初始交付 | `single-source` 的 `c6446d5` | 已集成；其中对拍往返输入与性能采样存在问题，以修正提交为准 |
| S0 修正与补测 | `e122680`，merge `5d8feb7` | 已集成；[S0 执行记录](stages/S0-baseline.md) 含 Flutter VM 2159 passed / 2 skipped、原生指纹与对拍 5 passed、Web release 与字体校验结果 |
| S1 引擎拆分与审查修复 | `8b5f824`，merge `f2bcb70` | G1 已复核通过、契约冻结，S0+S1 已并回 main；[S1 执行记录](stages/S1-core-sdk.md) 保留四项修订与门禁 |
| S2 绑定、门面与跨端修复 | 工作包 merge `2e5eb2e8` / `a5a5f1e2` / `7923e72a`；wire `942565a3`、Clipboard `ff8fcf19`、性能 `4f3cc4a3`、安全层融合 `492a9a42` | 已进入集成分支。Web 正式产物与真实门面已达标；桌面 AOT 仍有 serialize／html 两项超预算，正在归因收尾。Clipboard 提前按 T3.4 移植；Export 保留既有 feature 行为的契约矛盾单独交审。**M2 未通过** |
| T2.1 P0 | 原 spike `d48ed4f`；优化实验 `refs/heads/ss/w/p0-costs` @ `68ae07f`；核心 `6594f12` 以 `20ecde0` 进入 S1 | G2 已附条件通过；spike 不进集成分支，正式构建 opt-3 且 M2 须重新测量。详见 [审核记录](reviews.md) |

S0 修正后的差异记录为 4336 条（旧值 4887 作废）；其中 Rust 自身往返记录 754 条（旧值 1305 混入解析差异），记录数不是独立 bug 数。Chrome 测试基线实际为 dartdevc，不能与 P0 的 production dart2js 数字直接相除。

**G1 复核入口：**审核方已裁决原七项契约问题；其中 I3 保留现有 URL / 邮箱自动链接与首尾空白例外，S4 再由用户决定是否改变存储行为。四项修订的提交与回执详见 [S1 执行记录](stages/S1-core-sdk.md#2026-10-27--g1-第四项集成头-rust-门禁)，交审核方快速复核；执行方不自行冻结契约。

**G2：**用户选择继续优化后，审核方已附条件通过 wasm 路线。正式产物须使用 opt-3，保持 200 KB gzip、60 ms 初始化和三档 ≤ Dart ×1.2 预算，M2 用正式十 API 产物重新验证；P0 诊断产物数据不能代替 M2 证据。实验归因和原始数据入口见 [S2 执行记录](stages/S2-bindings.md#执行记录)。

**S2 当前验收快照（执行方核对过的数据，M2 判定仍归审核方）：**

- Web 正式十 API 产物：gzip **204,107 B**（≤204,800）、初始化 p50 **14.0 ms**（≤60）、真实 `MarkdownCodec` 三档比值中位 **1.073 / 0.523 / 0.569**，每轮 ≤1.2；SHA 与 gold 重放证据见 [S2 执行记录](stages/S2-bindings.md#执行记录)。
- Linux 复核实测（`9e604c43` 树，Docker Debian 12 正式构建）：gzip **205,269 B**（wasm 202,525 + JS 2,744，≤262,144 新门槛），wasm SHA `d1bdfd3d…`，JS 与 Windows 产物逐字节相同。
- Chrome 生产构建 Web 复制 10k：真实门面 `serializeBlocks + renderClipboardHtml` 中位 **4.4 ms ≤ 16 ms**。
- 桌面 release（最新核心 `492a9a42` 链，指纹 `ac687532799855bc`，预热口径）：init p50 **2 ms**、缺 DLL 降级路径真实通过、AOT parse 87/10k/100k **0.873 / 0.640 / 0.616**、边界调用 0.0124 ms、VM 契约 42/0/0。
- 桌面仍超预算 2 项，性质不同（已按 M2 复核更正归因）：AOT `serialize.10k` **1.534×** —— Rust wire 本体与同轮 AOT Dart 基本持平（1.449 vs 1.459 ms），超标来自 `encodeBlocks` 0.42 ms + FFI/FRB 边界约 0.37 ms；AOT `html.10k` **3.712×** —— Rust Clipboard render 本体已是 Dart 的 **2.16×**（1.542 vs 0.715 ms），渲染本体是主因、边界是次要增量。缩小边界需变更冻结 wire 形状（批量/二进制），缩小渲染本体需继续优化 Rust 路径；两者均交审核方裁决。
- 待审核契约项：Export HTML 保留 rich feature 语义，与「十函数跨端逐字节」的冻结描述冲突；候选方案是跨端字节矩阵仅约束 Clipboard。

按 [04 §4.2](04-parallel-plan.md#42-审核停点) 等待 M2；不合入 main，不发版。

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
