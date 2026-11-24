# Mica 项目协作规则

## 协作与决策

- 对话与项目文档用中文；面向公众的 README 用英文。
- 按需求实现最小充分解，不为假想需求预先加层、加开关。复杂决策先验证共同前提，再研究同类产品在相同约束下的实现及刻意不用的方案；说明取舍。
- 计划获准后连续执行到完成，常规决策按推荐项推进；如实报告未验证或做不到的部分。
- 可用子代理并行处理独立任务；需要边修改边维护清单的批量工作，由单个 agent 顺序完成，避免中断留下悬空引用。

## 代码探索：图谱优先

- 统一使用 **code-review-graph**，探索代码前优先查图谱；MCP 工具名、参数以当前会话为准，跨仓库查询传当前仓库的 `repo_root`。
- 找符号用 `semantic_search_nodes`，查调用/导入/测试关系用 `query_graph`，架构概览用 `get_architecture_overview` / `list_communities`。
- 审查先用 `detect_changes` + `get_review_context`；影响分析用 `get_impact_radius` / `get_affected_flows`，测试覆盖查 `tests_for`。
- 本机 Git/编辑 hooks 可增量刷新图谱；查询前检查新鲜度，必要时运行 `code-review-graph update`，未建图时运行 `code-review-graph build`。
- MCP 未接入时可用 CLI；图谱不可用或覆盖不足，以及查字符串、配置、脚本等非代码内容时，回退到 `rg` / 文件读取。

## 全栈纯 Rust 三层架构与数据不变量

- **100% 全栈纯 Rust（L1 + L2 + L3，零 Dart / Flutter / FFI 桥接）**：
  - **L1 数据与渲染引擎**（`crates/mica-core`, `crates/md-core`, `crates/mica-board`, `crates/api-types`, `crates/interchange`, `crates/api-server`, `crates/cli`, `crates/mcp-server`, `crates/md-wasm` SDK）：CRDT 文档核心只有一份（`mica-core::MicaDoc`），Markdown 解析/序列化/HTML/公式(`ratex-*`)/图表(`merman`) 只有一份（`crates/md-core`）。
  - **L2 无头客户端内核**（`crates/mica-board::session`, `crates/mica-sync`, `crates/mica-editor`, `crates/mica-client`）：纯 Rust 零 UI 依赖，双目标支持原生与 `wasm32`。
  - **L3 原生 UI 壳（`clients/mica_rust`）**：路线二-B（自有 `mica-editor` + `nsh-core` / `nsh-base default-features=false` 原生 `virtual_list` + 原生 IME 代理 + `nsh::Canvas`），直接调用 L1 + L2 Rust API，覆盖桌面与 WebAssembly。
- **渲染扩展走机制**：编辑器原子块通过 [atomic_blocks.rs](clients/mica_rust/src/atomic_blocks.rs) 注册表扩展（公式 `ratex`、Mermaid `merman`、内嵌白板 `mica-board`、代码块、图片），避免向 `editor_view.rs` 堆分支；`virtual_list` 的 `count` 和 `keys` 严禁订阅正文编辑信号，只订阅 `structure_rev`，块内更新走 per-block 细粒度 `Signal`。
- **Markdown round-trip 是不变量**：保持 CommonMark / GFM 与现有方言兼容，写侧输出规范化子集；改解析或导出时检查 [规范记分牌](docs/architecture/commonmark-scoreboard.md) 及对应回归测试。
- **folder 是唯一容器，page 是叶子**：服务端写 `parent_view_id` 必须经过 `documents::ensure_parent_accepts_children`；父节点须为同工作区、未删除的 folder，保留 `views_parent_must_be_folder` DB 触发器兜底。本地库由 `mica-core` `ensure_valid_parent` 与 `mica_core::tree_rules` 判定（含防环）。
- **业务规则只有一个权威判定点**：唯一替换、上传配额、父节点合法性等，由持有写锁或事务的数据层最终判定；UI 预检只改善交互。服务端与本地库共用的树规则放 `mica_core::tree_rules`，SQL 规则改动须让 `contracts/tree-rules.json` 两侧回放都通过。见 [开发教训 §2](docs/dev/lessons.md)。
- 编辑器使用 marks-over-plain-text（UTF-16 偏移）；硬换行按反斜杠加换行约定存储；块级嵌套保持扁平模型（`data.indent` / `data.quote` / `qbreak` / `data.li`），HTML 导出重建嵌套。
- 快捷键变更同步编辑器/应用级处理、设置面板和 [快捷键清单](docs/features/shortcuts.md)。

## 代码结构与大小门禁

- API 路由放入 `crates/api-server/src/routes/<域>/`，共享 helper / DTO 在对应 `mod.rs` 再导出；Markdown 引擎（`crates/md-core`）按 `parse/`、`serialize/`、`html/` 管线归位，公开 API 在 `api.rs` 并由 `lib.rs` 再导出；`crates/markdown` 只做门面，不放语法逻辑。
- 门禁以 [scripts/check-file-size.sh](scripts/check-file-size.sh) 和 [预算表](scripts/file-size-budget.txt) 为准：按 Rust 单文件扫描 `crates/*/src` 与 `clients/mica_rust/src`。
- 普通单元硬上限 8,000 行、软上限 4,000 行；预算登记的存量单元只许减不许增；增量检查 `--diff <base>` 限制超过 2,000 行的存量文件净增不得超过 300 行。提交前运行 `just check-structure`。

## 依赖与平台边界

- 核心数据面优先自研、最小化第三方依赖；复杂领域渲染与原生 UI 壳复用成熟 Rust crate。
- 已批准的关键依赖：`ratex-*`（数学渲染）、`merman`（Mermaid 渲染）、`yrs`（CRDT）、`nsh-core` / `nsh-base`（`default-features = false`）/ `nsh-platform-{windows,macos,linux,web}`（L3 原生 UI 壳）、`wasm-bindgen` + `wasm-bindgen-cli`（仅 `crates/md-wasm` 对外 `mica-md-js` SDK，`scripts/check-md-wasm-toolchain.sh` 校验）。
- 平台专用依赖用 `cfg` / target 依赖隔离，禁止进入不支持的平台。Mermaid 配色沿用 host theme profile，保留 SVG CSS 属性展开的兼容处理。

## 验证、构建与发布

- 跨 await 写共享状态、或持有锁/连接/临时文件的改动，按 [异步评审清单](docs/dev/async-review-checklist.md) 逐条过：结果绑定账号/服务器/会话代际，提交前检查归属，资源走 RAII。
- Bug 修复配回归测试，并通过真实运行路径验证；提交说明写根因。
- 工具链以 `rust-toolchain.toml` 为准；升级时同步构建镜像与 CI。不要写死本机安装路径或对正常构建额外限流。
- Windows 的 `just` recipe 使用 Git Bash。
- 按改动范围运行测试；共享 Rust crate 变更须同时验证 `clients/mica_rust`（独立 workspace）。DB 测试必须连接可用 Postgres；静默跳过不算通过。
- 门禁要证明测试**真的跑了**：指定测试用 `scripts/lib-receipt.sh` 的 `cargo_receipt`（通过数低于下限即失败），DB 测试走 `mica_infra::test_database_url()`，被 `.gitignore` 藏起的源码文件由 `scripts/check-ignored-sources.sh` 拒绝。见 [开发教训 §3](docs/dev/lessons.md)。
- 改动完成后自动推送 `main`，无需逐步请示（特性分支开发除外）；**发版须等用户明确要求**。用户要求发版后，版本 bump → 门禁 → tag / 构建 → 部署 → 验证连续做完。
- 默认递增补丁号，minor 由用户决定；版本一致性与两份锁文件（`Cargo.lock` + `clients/mica_rust/Cargo.lock`）由 `scripts/manifest-version.sh` 和 `scripts/release-tree-check.sh` 校验。发布步骤以 [发布文档](docs/ops/release.md) 为准，命令见 `just --list`，不得绕过发布门禁。
- **GitHub Actions 使用官方 runner，保持手动入口**：正式发布从 `main` 手动启动一次 `Ship`，后续 CI、打标签、发布、部署和验证由工作流接续。额度用完或 Actions 不可用时走本地流程（`just ci`、`just release`、`just ship` 等）。常规推送 `main` 前运行本地 CI（`just ci` / `just ci rust` / `just ci client`）。
- 部署后验证 `/api/health` 版本，并冒烟测试本版实际改动。
- 发版时同步路线图：未完成项留在 [roadmap.md](docs/plans/roadmap.md)，完成项移入 [roadmap-done.md](docs/plans/roadmap-done.md)；明确不做的条目删除，理由写入提交信息。

## 配置与生产运维

- 改配置时验证未设置、空字符串和非法值；检查 Compose `environment` 显式传递了所需变量。保留敏感默认配置的运行时告警，部署细节见 [部署文档](docs/ops/deploy.md)。
- 已发布的 SQL 迁移冻结，**包括注释与原始 LF 字节**；变更另加迁移并确保纳入 Git。新增迁移由 `crates/infra/build.rs` 跟踪，无须手动 touch 源码。
- 带迁移部署必须先有经内容校验的还原点，不能用周期 backup sidecar 代替；备份管线用 `pipefail`，仅 `gzip -t` 不能证明 dump 有效，失败须中止部署。部署配置须来自目标 tag。
- 普通 Mica 发版不得重建共享 Traefik 或覆盖其独立 slice；基础设施升级与应用发版分开。见 [Traefik 工作集保护](docs/ops/traefik-working-set.md)。
- API 部署与 `mica-cli` / MCP 客户端升级分别生效；修改 MCP 代理后须更新用户实际调用的二进制并重连验证。

## 文档维护

- 文档入口为 [docs/README.md](docs/README.md)；新增文档按用途归类并补索引，评审与修复记录放 `docs/reviews/`。
- **产品能力与交互契约权威库为 [docs/capabilities/README.md](docs/capabilities/README.md)**（与技术栈解耦）：新增产品特性、交互优化或修复关键边界体验时，同步更新对应能力文档的条款、边界反例、状态与演进记录；未来任何重构或换壳须按该清单验收。
- 修改相关模块前阅读 [开发教训](docs/dev/lessons.md) 的对应部分；历史细节与事故经过保留在专题文档或 Git 中。
- 本文件只维护长期规则和必要入口；版本/功能完成快照、机器状态、安装路径、临时故障处理和可由脚本直接查询的清单不在此重复维护。
