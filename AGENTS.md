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

## 架构与数据不变量

- **Rust-first 数据面**：核心文档模型、CRDT、同步与数据处理放在 Rust；Dart 负责 UI 与编辑器热路径。Markdown 语法以 `crates/markdown` 为权威，Dart 镜像必须同步更新。
- **渲染扩展走机制**：通过 `AtomicBlockRenderer` 注册表扩展，避免继续向 `render.dart` 堆分支。见 [渲染架构](docs/architecture/render-architecture.md)。
- **Markdown round-trip 是不变量**：保持 CommonMark / GFM 与现有方言兼容，写侧输出规范化子集；改解析或导出时检查 [规范记分牌](docs/architecture/commonmark-scoreboard.md) 及对应回归测试。
- **folder 是唯一容器，page 是叶子**：服务端写 `parent_view_id` 必须经过 `documents::ensure_parent_accepts_children`；父节点须为同工作区、未删除的 folder，保留 `views_parent_must_be_folder` DB 触发器兜底。本地库由 mica-core `ensure_valid_parent` 判定（含防环），不在 FFI 包装层。
- **业务规则只有一个权威判定点**：唯一替换、上传配额、父节点合法性等，由持有写锁或事务的数据层最终判定；UI 和调用方的预检只改善交互。删除、回收、改密等操作在同一个事务边界内完成全部必要的状态更新，不靠调用方记得补步骤。服务端与本地库各有一份的规则（恢复、页树），改一边必须同步另一边。适用于关系型状态；CRDT 文档正文只能在写入入口的锁里判定。见 [开发教训 §2](docs/dev/lessons.md)。
- 编辑器使用 marks-over-plain-text；硬换行按现有反斜杠加换行约定存储。块级嵌套保持扁平模型：`data.indent` 为列表层级，`data.quote` / `qbreak` 为引用深度/分组，`data.li` 为 item 容器子块；HTML 导出重建嵌套。代码字体使用 `kMonoFont`。
- 快捷键变更同步编辑器/应用级处理、设置面板和 [快捷键清单](docs/features/shortcuts.md)；按当前符号定位实现，不照搬旧文件位置。

## 代码结构与大小门禁

- API 路由放入 `crates/api-server/src/routes/<域>/`，共享 helper / DTO 在对应 `mod.rs` 再导出；Markdown 引擎按 block / inline / html / export_md / entities 管线归位，公开 API 留在 `lib.rs`。
- Flutter 的 `part` 可整理同库代码，但不减少库耦合或库行数；需要真正拆分时建立独立 library。
- 门禁以 [scripts/check-file-size.sh](scripts/check-file-size.sh) 和 [预算表](scripts/file-size-budget.txt) 为准：Dart 按 library（root + 全部 part），Rust 按文件，包含桌面 FFI crate。
- 普通单元硬上限 8,000 行、软上限 4,000 行；预算登记的存量单元跳过硬限，但**只许减不许增**，修改前先看预算。
- 增量检查 `--diff <base>` 还限制：超过 2,000 行的存量单元净增不得超过 300 行；同库各 part 合并计量。
- 生成代码等豁免须在 `EXEMPT_RE` 或预算表显式登记并说明理由。提交前运行 `just check-structure`；背景见 [拆分计划](docs/reviews/code-optimization/OPTIMIZATION_PLAN_ROUND2.md)。

## 依赖与平台边界

- 核心数据面优先自研、最小化第三方依赖；平台粘合层和复杂领域渲染可用成熟库。新增依赖须有明确豁免并说明理由，不能因已有依赖而无限扩展。
- 已批准的依赖及用途：
  - 传输与解析：`http`、`web_socket_channel`、`xml`、`html`；数学渲染：`flutter_math_fork`、Rust `ratex-*`。
  - 桌面集成：`window_manager`、`screen_retriever`、`tray_manager`、`file_picker`、`pasteboard`。
  - 图形与导出：Rust `merman`、`webview2-com`，Dart `flutter_svg`；Web CRDT 对端：JS `yjs`。
- 平台专用依赖用条件导入 / `cfg` 隔离，禁止进入不支持的平台 bundle；`yjs` 只进入 Web，`webview2-com` 只用于 Windows。
- 托盘仅 Windows 支持；注册失败时降级 `minimize()`，不可 `hide()` 导致窗口无法找回。Mermaid 配色沿用 host theme profile，保留 SVG CSS 属性展开的兼容处理。

## 验证、构建与发布

- 跨 await 写共享状态、或持有锁/连接/临时文件的改动，按 [异步评审清单](docs/dev/async-review-checklist.md) 逐条过：结果绑定账号/服务器/会话代际（写入绑定文档版本），提交前检查归属，延迟闭包入队时捕获数据，资源走 finally / RAII。
- Bug 修复配回归测试，并通过真实运行路径验证；Web UI 用 Playwright 检查并截图，桌面问题须在桌面验证。提交说明写根因。
- 工具链以 `.fvmrc` / `rust-toolchain.toml` 为准；升级时同步构建镜像与 CI，验证桌面 Cargokit 实际使用的工具链。不要写死本机安装路径或对正常构建额外限流。
- Windows 的 `just` recipe 使用 Git Bash；网络受限时按环境配置 Flutter / Pub 镜像。Windows runner C++ 按 warning-as-error 处理，注释保持 ASCII，环境变量读取用 `GetEnvironmentVariableW`。
- 按改动范围运行测试；共享 Rust crate 变更还需检查 `clients/mica_flutter/rust`，它不属于根 workspace。DB 测试必须连接可用 Postgres；静默跳过不算通过。
- 门禁要证明测试**真的跑了**：指定测试用 `scripts/lib-receipt.sh` 的回执（通过数低于下限即失败），DB 测试走 `mica_infra::test_database_url()`，浏览器套件走 `scripts/browser-tests.sh`，FFI 测试用 `initCheckedNativeRust` 校验库的源码指纹；被 `.gitignore` 藏起的源码文件由 `scripts/check-ignored-sources.sh` 拒绝。见 [开发教训 §3](docs/dev/lessons.md)。
- 改动完成后自动推送 `main`，无需逐步请示；**发版须等用户明确要求**。用户要求发版后，版本 bump → 门禁 → tag / 构建 → 部署 → 验证连续做完。
- 默认递增补丁号，minor 由用户决定；版本一致性与锁文件由现有脚本校验。发布步骤以 [发布文档](docs/ops/release.md) 为准，命令见 `just --list`，不得绕过发布门禁。
- 不假定 push 会触发 CI；检查 `.github/workflows/` 的当前触发方式，手动触发后核对运行结果。部署后验证 `/api/health` 版本，并冒烟测试本版实际改动。
- 发版时同步路线图：未完成项留在 [roadmap.md](docs/plans/roadmap.md)，完成项移入 [roadmap-done.md](docs/plans/roadmap-done.md)；明确不做的条目删除，理由写入提交信息。

## 配置与生产运维

- 改配置时验证未设置、空字符串和非法值；检查 Compose `environment` 显式传递了所需变量。保留敏感默认配置的运行时告警，部署细节见 [部署文档](docs/ops/deploy.md)。
- 已发布的 SQL 迁移冻结，**包括注释与原始 LF 字节**；变更另加迁移并确保纳入 Git。新增迁移由 `crates/infra/build.rs` 跟踪，无须手动 touch 源码。
- 带迁移部署必须先有经内容校验的还原点，不能用周期 backup sidecar 代替；备份管线用 `pipefail`，仅 `gzip -t` 不能证明 dump 有效，失败须中止部署。部署配置须来自目标 tag。
- 普通 Mica 发版不得重建共享 Traefik 或覆盖其独立 slice；基础设施升级与应用发版分开。见 [Traefik 工作集保护](docs/ops/traefik-working-set.md)。
- API 部署与 `mica-cli` / MCP 客户端升级分别生效；修改 MCP 代理后须更新用户实际调用的二进制并重连验证。

## 文档维护

- 文档入口为 [docs/README.md](docs/README.md)；新增文档按用途归类并补索引，评审与修复记录放 `docs/reviews/`。
- 修改相关模块前阅读 [开发教训](docs/dev/lessons.md) 的对应部分；历史细节与事故经过保留在专题文档或 Git 中。
- 本文件只维护长期规则和必要入口；版本/功能完成快照、机器状态、安装路径、临时故障处理和可由脚本直接查询的清单不在此重复维护。
