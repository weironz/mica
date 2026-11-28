# Mica 文档索引

按用途分文件夹;每个文件夹里按主题列出。**先读哪几份**:

- 想知道还要做什么 → [`plans/roadmap.md`](plans/roadmap.md)(权威待办,只放还要做的)
- 查产品功能与交互验收基线（跨技术栈）→ [`capabilities/README.md`](capabilities/README.md)（16 份独立产品能力规格，含编辑器核心编辑验收清单）
- 改代码前 → [`dev/lessons.md`](dev/lessons.md)(踩过的坑)与仓库根 [`AGENTS.md`](../AGENTS.md)
- 发版 / 上线 → [`ops/release.md`](ops/release.md)、[`ops/sdk-release.md`](ops/sdk-release.md)、[`ops/deploy.md`](ops/deploy.md)
- 代码评审与修复记录 → [`reviews/`](#reviews--代码评审与修复)
- Markdown 引擎的结构与 SDK 契约 → [`single-source/`](single-source/README.md)(单一实现专项,S0–S5 已完成并随 v0.14.0 上线)

| 文件夹 | 放什么 |
| --- | --- |
| [`capabilities/`](#capabilities--产品功能与交互能力权威清单) | **技术栈无关的产品能力与交互验收规格**（总索引 + 16 份独立特性文档，覆盖编辑器 9 大核心模块及白板/外壳/搜索/同步/导入导出/移动端；每次新增特性、优化体验或跨栈重构同步落地） |
| [`architecture/`](#architecture--架构与核心机制) | 整体架构、编辑器与渲染、同步、Markdown 引擎、local-first 系列 |
| [`features/`](#features--功能设计) | 单个功能的设计与决策(评论、版本历史、分享、导入导出、快捷键……) |
| [`plans/`](#plans--规划与路线图) | 路线图与存档、阶段计划、迁移评估、同类产品对标 |
| [`ops/`](#ops--部署发版与运维) | 部署、发版、备份与容灾、基础设施升级、生产事件定位 |
| [`dev/`](#dev--开发工具与接入) | 开发环境、CLI / HTTP API / MCP、踩坑记录 |
| [`performance/`](#performance--性能分析) | 性能分析与优化记录 |
| [`reviews/`](#reviews--代码评审与修复) | 全项目代码评审、修复计划与验证 |
| [`single-source/`](#single-source--单一实现专项) | 专项:Markdown 引擎单一实现(背景、契约、分阶段代码级方案、并行计划、审核清单) |
| `design/` | 设计稿 01–19(`.dc.html`),采纳记录见 [`features/design-adoption.md`](features/design-adoption.md) |
| `brand/` | 品牌标志 |
| `assets/` | 文档里引用的截图与基准数据 |

新增文档时放进对应文件夹并在这里补一行;文件名带日期的是某一时刻的快照(评审、实测、事件定位),
不随代码更新,结论以后续文档或代码为准。

项目规则已统一到根 `AGENTS.md`；历史文档中的根 `CLAUDE.md` 及旧条款编号仅作追溯，当前规则以上述入口为准。设计目录内的独立 `CLAUDE.md` 不属于此次合并范围。

## capabilities — 产品功能与交互能力权威清单

与技术栈解耦的产品行为契约与验收清单，入口见 [`capabilities/README.md`](capabilities/README.md)。每次新增特性、交互优化、关键边界修复或未来切换 UI 技术栈时，均以本目录为验收基线并持续更新。

| 文档 | 内容 |
| --- | --- |
| [README.md](capabilities/README.md) | **能力库总索引**：维护纪律、编号规则（`CAP-XX-YY`）、优先级、三端状态总表 |
| [01-editor-caret-ime-typing.md](capabilities/01-editor-caret-ime-typing.md) | 编辑器：光标闪烁与命中、CJK IME、连续打字、Markdown 输入规则、首行回退标题、底部新建行 |
| [02-editor-selection-marquee.md](capabilities/02-editor-selection-marquee.md) | 编辑器：文本选区、跨块自由拖选、页边距与正文内空白框选（Marquee）、点击外区释放选区 |
| [03-editor-clipboard-paste.md](capabilities/03-editor-clipboard-paste.md) | 编辑器：双通道无损复制粘贴、Windows `CF_HTML` 与 v0.14.11 互操作、截图/文件图片粘贴 |
| [04-editor-blocks-and-marks.md](capabilities/04-editor-blocks-and-marks.md) | 编辑器：全量块类型与 5 种 Callout、行内 marks、22 项斜杠菜单、贴近选区的浮动气泡栏、`Ctrl+F` 查找 |
| [05-editor-tables.md](capabilities/05-editor-tables.md) | 编辑器：GFM 表格自适应列宽、上下左右多单元格拖选与批量复制/清空、单格编辑与导航 |
| [06-editor-code-blocks.md](capabilities/06-editor-code-blocks.md) | 编辑器：无框暖灰代码块、顶层 `overlay()` 语言选择器、`min_rows(1)`、`(0,0)` 按 Backspace 删除代码块 |
| [07-editor-media-math-mermaid.md](capabilities/07-editor-media-math-mermaid.md) | 编辑器：图片自然比例与灯箱、RaTeX 公式与原子选区保护、Mermaid 主题渲染与全屏缩放 |
| [08-editor-scroll-layout-virtualization.md](capabilities/08-editor-scroll-layout-virtualization.md) | 编辑器：长文档虚拟化与信号隔离、`Jump`/`KeepVisible` 零跳滚、窗口右缘自适应滚动条、11 档页宽 |
| [09-editor-undo-crdt-collab.md](capabilities/09-editor-undo-crdt-collab.md) | 编辑器：单栈 CRDT 撤销/重做、远端合并本地脏块保护与 `SelectionReconcile`、离线容错 |
| [10-whiteboard-and-embedded-board.md](capabilities/10-whiteboard-and-embedded-board.md) | 无限白板（飞书式工具坞/图形与架构图标库/检查器/导图）与内嵌单层画板三路合并 |
| [11-sidebar-tree-tabs-shell.md](capabilities/11-sidebar-tree-tabs-shell.md) | 无边框顶栏、应用菜单、多标签历史、目录树 `folder` 唯一容器与 30/40/30 拖拽、多选、收藏、Emoji、子树回收站 |
| [12-page-chrome-outline-properties-backlinks.md](capabilities/12-page-chrome-outline-properties-backlinks.md) | 面包屑与路径复制、标题联动、Front-Matter 属性面板、右侧大纲/评论、反链、双栏块级 Diff 版本历史 |
| [13-search-graph-and-ai.md](capabilities/13-search-graph-and-ai.md) | 全局搜索（工作区/文件夹/页面 + 高亮）、力导向知识关系图谱、多入口 AI 助手与服务商配置 |
| [14-workspace-cloud-sync-sharing.md](capabilities/14-workspace-cloud-sync-sharing.md) | 本地离线 Vault 与多云端世界切换、工作区管理与成员邀请、公开分享 `/s/{token}`、API 令牌 |
| [15-import-export-and-vault.md](capabilities/15-import-export-and-vault.md) | `.mica.zip` 备份、Markdown ZIP、Notion/文件夹导入、HTML/PDF 导出、CommonMark/GFM Round-Trip |
| [16-settings-shortcuts-accessibility-mobile.md](capabilities/16-settings-shortcuts-accessibility-mobile.md) | 8 标签设置中心与外观预览、快捷键体系、主题色板、移动端阅读优先与触摸选区适配 |

## architecture — 架构与核心机制

| 文档 | 内容 |
| --- | --- |
| [architecture.md](architecture/architecture.md) | **系统现状架构（100% 全栈纯 Rust）**：前后端进程边界、L1/L2/L3 crate 地图、存储/同步/搜索与部署 |
| [editor.md](architecture/editor.md) | 编辑器设计原则 |
| [editor-engine.md](architecture/editor-engine.md) | 编辑器引擎设计(自绘画布、marks 模型) |
| [render-architecture.md](architecture/render-architecture.md) | 块渲染架构(`AtomicBlockRenderer` 注册表,渲染红线) |
| [sync-and-api.md](architecture/sync-and-api.md) | 同步、历史与 API 草案 |
| [search.md](architecture/search.md) | 全文搜索:进程内正文索引、可锁定内存、开关与观测点、各版本根因 |
| [commonmark-scoreboard.md](architecture/commonmark-scoreboard.md) | CommonMark 0.31.2 / GFM 记分牌 |
| [local-first-plan.md](architecture/local-first-plan.md) | Local-first 统一改造(option C) |
| [phase2-offline-crdt.md](architecture/phase2-offline-crdt.md) | Phase 2:离线优先 + CRDT 实施方案 |
| [local-first-p2-design.md](architecture/local-first-p2-design.md) | P2:云文档离线编辑 |
| [local-first-p3-design.md](architecture/local-first-p3-design.md) | P3:溶解双模式(本地 / 已连云共存) |
| [fullstack-rust-architecture.md](architecture/fullstack-rust-architecture.md) | **全栈 Rust 三层标杆架构与分阶段开发计划**（L1/L2/L3、路线二-B 主权在我、Phase 0–3 路线图，分支 `feat/fullstack-rust-architecture`，跟踪 #172） |
| [ui-gpui-evaluation.md](architecture/ui-gpui-evaluation.md) | Rust UI 壳选型调研与复审记录：前期 GPUI / gpui-kit / Native Shell 四关实测，及 2026-11-23 在“桌面=Web+移动端偏阅读”约束下对 GPUI / Dioxus / Makepad / Slint / Xilem 的横向评估与留在 `nsh` 的拍板结论 |

## features — 功能设计

| 文档 | 内容 |
| --- | --- |
| [shortcuts.md](features/shortcuts.md) | 键盘快捷键(权威清单) |
| [comments-plan.md](features/comments-plan.md) | 评论 / 建议锚点设计 |
| [version-history-plan.md](features/version-history-plan.md) | 页面版本历史 |
| [page-title-plan.md](features/page-title-plan.md) | 页面标题:从视图属性改为文档的一部分 |
| [page-properties.md](features/page-properties.md) | 页面属性 / 标签 |
| [sharing.md](features/sharing.md) | 页面分享(Publish-to-web) |
| [workspace-invitations.md](features/workspace-invitations.md) | 工作区邀请:添加成员改为对方接受后加入 |
| [export-import.md](features/export-import.md) | 导出 / 导入设计 |
| [cross-workspace-move.md](features/cross-workspace-move.md) | 跨工作区移动 / 复制 |
| [vault-mode.md](features/vault-mode.md) | Vault 模式(本地 Markdown 文件) |
| [ai-block-editing.md](features/ai-block-editing.md) | AI 当前页定向修改 |
| [password-reset.md](features/password-reset.md) | 密码找回 |
| [editor-virtualization-plan.md](features/editor-virtualization-plan.md) | 长文档虚拟化 |
| [mermaid.md](features/mermaid.md) | Mermaid 图表渲染（纯 Rust `merman` 引擎、三端链路、主题/CSS 内联与门禁） |
| [whiteboard.md](features/whiteboard.md) | 无限画板与思维导图架构 RFC（方案 C：Rust `mica-board`、CRDT Schema、WS 双通道、Markdown/SVG 导出与探针结论） |
| [usage-statistics.md](features/usage-statistics.md) | 使用统计与 Prometheus 可观测性设计（Issue #191），附交互原型 [usage-statistics-demo.html](features/usage-statistics-demo.html) |
| [world-switcher-demo.html](features/world-switcher-demo.html) | 桌面端居中多主机环境切换弹窗、左下角快速切换菜单与 Web 居中单卡片登录页交互原型（对应 `CAP-14-01` / `CAP-14-02`） |
| [inline-highlight-and-color-policy.md](features/inline-highlight-and-color-policy.md) | 行内高亮（`==高亮==` / `<mark>`）与颜色策略调研（为什么不做 Notion 式任意字体/背景色调色盘及三层实现） |
| [design-adoption.md](features/design-adoption.md) | 设计稿(`design/` 01–19)采纳记录 |

## plans — 规划与路线图

| 文档 | 内容 |
| --- | --- |
| [roadmap.md](plans/roadmap.md) | **路线图:剩余功能与优化点(权威)** |
| [roadmap-done.md](plans/roadmap-done.md) | 已完成条目存档(只进不出) |
| [rust-parity-checklist.md](plans/rust-parity-checklist.md) | 纯 Rust 客户端（`clients/mica_rust`）1:1 功能对齐与验收清单 |
| [notion-gap-analysis.md](plans/notion-gap-analysis.md) | Notion 对标分析 |
| [ui-visual-refresh.md](plans/ui-visual-refresh.md) | 界面与字体观感改造施工说明(Windows 桌面优先,待实施) |
| [desktop-plan.md](plans/desktop-plan.md) | 桌面端技术方案与开发计划 |
| [mvp-plan.md](plans/mvp-plan.md) | MVP 开发计划 |
| [window-manager-migration.md](plans/window-manager-migration.md) | window_manager 迁移评估(结论:不迁) |

## ops — 部署、发版与运维

| 文档 | 内容 |
| --- | --- |
| [release.md](ops/release.md) | **发版与构建**(含 Actions 不可用时的本地发布链) |
| [sdk-release.md](ops/sdk-release.md) | **对外 SDK 发布手册**(`mica-board` / `mica-md` crates.io 与 `mica-md` npm 发版、避坑与验收) |
| [release-0.14.3-validation.md](ops/release-0.14.3-validation.md) | 0.14.3 发行物、生产与隔离栈验收;入口 404 缺口及修复边界 |
| [traefik-working-set.md](ops/traefik-working-set.md) | 共享入口最终配置、缺页 I/O 根因、部署保留方式与空闲 TLS / 目录验收 |
| [deploy.md](ops/deploy.md) | 部署、凭据、管理员、删账号 |
| [aliyun-esa-cdn.md](ops/aliyun-esa-cdn.md) | 阿里云边缘安全加速（ESA）CDN 接入配置、WASM 缓存与 WebSocket 避坑、验收命令与跨区实测基线 |
| [metrics.md](ops/metrics.md) | **Prometheus `/metrics` 指标字典、网络隔离边界、PromQL 配方与告警参考** |
| [backup.md](ops/backup.md) | 备份与恢复 |
| [dr-plan.md](ops/dr-plan.md) | 容灾方案 |
| [dr-drill.md](ops/dr-drill.md) | 容灾恢复演练工作表 |
| [upgrade-infra.md](ops/upgrade-infra.md) | Postgres 大版本 / RustFS 手动升级 |
| [bucket-provisioning-plan.md](ops/bucket-provisioning-plan.md) | 建桶由谁做 |
| [cd-plan.md](ops/cd-plan.md) | CD 与 provisioning 拆分 |
| [legacy-orphan-audit.md](ops/legacy-orphan-audit.md) | 历史孤儿对象盘点 |
| [backup-validation-2026-10-16.md](ops/backup-validation-2026-10-16.md) | 生产备份验收(v0.13.51–52) |
| [host-pressure-2026-10-16.md](ops/host-pressure-2026-10-16.md) | 生产主机 Swap / I/O 定位 |

## dev — 开发、工具与接入

| 文档 | 内容 |
| --- | --- |
| [lessons.md](dev/lessons.md) | **踩过的坑**(和根 `AGENTS.md` 一起读) |
| [async-review-checklist.md](dev/async-review-checklist.md) | 异步结果归属与资源释放的评审清单 |
| [nsh-dependency-governance.md](dev/nsh-dependency-governance.md) | `nsh` 0.x 依赖精确锁版本、升级检查表与上游已知问题登记（P2-2） |
| [bootstrap.md](dev/bootstrap.md) | 新开发机从零搭建(Windows) |
| [dev-environment.md](dev/dev-environment.md) | 开发环境与辅助工具 |
| [api.md](dev/api.md) | HTTP API |
| [cli.md](dev/cli.md) | mica-cli |
| [mcp-server.md](dev/mcp-server.md) | MCP 服务设计 |
| [mcp-connect.md](dev/mcp-connect.md) | 接入 Claude Code / Claude Desktop |

## performance — 性能分析

| 文档 | 内容 |
| --- | --- |
| [search-latency-2026-10-16.md](performance/search-latency-2026-10-16.md) | 生产全局搜索偶发延迟定位(及 2026-10-19 的修复) |
| [general-latency-2026-10-23.md](performance/general-latency-2026-10-23.md) | 新建与目录切换卡顿：Mica 与 Traefik 工作集保护；最终空闲 TLS 约 30 ms，保留早期慢样本与验收边界 |
| [large-workspace-performance.md](performance/large-workspace-performance.md) | 大工作区目录性能 |
| [workspace-switch-latency.md](performance/workspace-switch-latency.md) | 工作区切换延迟 |
| [mcp-performance.md](performance/mcp-performance.md) | MCP 性能与 token 优化(权威) |
| [web-cold-start-2026-11-16.md](performance/web-cold-start-2026-11-16.md) | 网页端首次打开白屏：不是字体；启动画面、预加载与 Brotli 预压缩，首帧约快 30% |
| [`benchmarks/README.md`](../benchmarks/README.md) | **纯 Rust L1+L2 性能基准与万篇压测套件**（`just bench` / `just bench-10k`，含 [2026-11-25 全栈 Rust 基线与优化实录](../benchmarks/baselines/2026-11-25-fullstack-rust-baseline.md)） |

## reviews — 代码评审与修复

全项目代码评审(问题清单 + 修复状态)按日期排列;`code-optimization/` 是结构性优化的计划与验证。

| 文档 | 内容 |
| --- | --- |
| [fullstack-rust-branch-review-2026-11-24.md](reviews/fullstack-rust-branch-review-2026-11-24.md) | `feat/fullstack-rust-architecture` 分支评审与后续实施计划：已定方向（全栈 Rust、界面层留在 `nsh`、Flutter 冻结维护）、拆分合入与预览渠道里程碑、P0/P1/P2 任务（缓存键 #111、Web 异步与离线、自动更新等）、Web 首屏实测与 W1–W6 优化、切换闸门；复现脚本在 `reviews/assets/fullstack-rust-2026-11-24/` |
| [fullstack-rust-p0-p2-verification-2026-11-24.md](reviews/fullstack-rust-p0-p2-verification-2026-11-24.md) | **全栈 Rust 架构评审（P0–P2）落地与验证总台账**（含全部 commit 映射、完整 CI 与 Release dry-run 实测回执及合并前真人签收项） |
| [fullstack-rust-ime-browser-verification-2026-11-24.md](reviews/fullstack-rust-ime-browser-verification-2026-11-24.md) | 全栈 Rust 分支输入法与多浏览器验证记录与真人签收矩阵（P1-2） |
| [fullstack-rust-p0-spike-2026-11-22.md](reviews/fullstack-rust-p0-spike-2026-11-22.md) | 全栈 Rust Phase 0 全景实证与 L2 代码边界摸底（NSH 纯 Rust 瘦身 Web 1.39 MiB Brotli、路线二-B 2,000 块/500 节点基准、Board/Sync/Editor 抽离蓝图） |
| [wasm-size-budget-2026-11-17.md](reviews/wasm-size-budget-2026-11-17.md) | Markdown 引擎 wasm 体积预算：为什么设、业界做法、高亮器入引擎（gh#178）后的实测与调到 288 KiB 的决定、以后怎么调 |
| [redundancy-and-home-review-2026-11-10.md](reviews/redundancy-and-home-review-2026-11-10.md) | 双实现盘点、首页与三轮实施评估;前三项(Mermaid、公式、兼容层)已实施并审核;第二轮评估与用户决定;交给他人实施的待办清单 |
| [crdt-web-wasm-spike-2026-11-01.md](reviews/crdt-web-wasm-spike-2026-11-01.md) | Web 端 CRDT 换成 mica-core wasm 的可行性验证:可行,净增约 194 KB gzip,待用户决定 |
| [editor-p2-undo-cache-design.md](reviews/editor-p2-undo-cache-design.md) | #79 本地操作撤销与 #62 预览/图片缓存循环：源码调研、设计与验证 |
| [editor-formatting-2026-10-24.md](reviews/editor-formatting-2026-10-24.md) | 列表跨块格式与表格多余边框：根因、近期 issue 边界、193 项回归与浏览器验证 |
| [database-dns-2026-10-24.md](ops/database-dns-2026-10-24.md) | 工作区操作数据库错误：DNS 证据、Mica 专用连接修复与部署预检 |
| [code-review-2026-10-21.md](reviews/code-review-2026-10-21.md) | 第三轮全项目评审(重构后):P1×8、工具实跑结果、旧条目核对、精简估算 |
| [code-review-2026-10-19.md](reviews/code-review-2026-10-19.md) | 第二轮全项目评审(与 10-18 互补) |
| [code-review-2026-10-18.md](reviews/code-review-2026-10-18.md) | 全项目评审,A–D 级问题与逐条修复、决策、验证 |
| [code-review-2026-07-20.md](reviews/code-review-2026-07-20.md) | 2026-07-20 全项目评审 |
| [code-optimization/ARCHITECTURE_REVIEW_2026-10-18.md](reviews/code-optimization/ARCHITECTURE_REVIEW_2026-10-18.md) | 架构调整评审与修复 |
| [code-optimization/OPTIMIZATION_PLAN.md](reviews/code-optimization/OPTIMIZATION_PLAN.md) | 代码库优化计划(第一轮) |
| [code-optimization/OPTIMIZATION_PLAN_ROUND2.md](reviews/code-optimization/OPTIMIZATION_PLAN_ROUND2.md) | 第二轮:巨石文件拆分 |
| [code-optimization/OPTIMIZATION_PLAN_ROUND3.md](reviews/code-optimization/OPTIMIZATION_PLAN_ROUND3.md) | 第三轮:长尾收敛 |
| [code-optimization/OPTIMIZATION_PLAN_ROUND4.md](reviews/code-optimization/OPTIMIZATION_PLAN_ROUND4.md) | 第四轮:10-21 评审的分阶段修复与结构收口计划 |
| [code-optimization/UI_EDITING_EXPERIENCE_PLAN.md](reviews/code-optimization/UI_EDITING_EXPERIENCE_PLAN.md) | UI 与编辑舒适感方案 |
| [code-optimization/UI_EDITING_EXPERIENCE_VALIDATION.md](reviews/code-optimization/UI_EDITING_EXPERIENCE_VALIDATION.md) | UI 与编辑舒适感:实施与验证 |
| [code-optimization/SIDEBAR_QUICK_RENAME.md](reviews/code-optimization/SIDEBAR_QUICK_RENAME.md) | 目录树双击快速重命名 |

## single-source — 单一实现专项

| 文档 | 内容 |
| --- | --- |
| [README.md](single-source/README.md) | 专项总览:目标、阶段总表、已拍板的决定、执行规则 |
| [01-background.md](single-source/01-background.md) | 双实现是什么、为什么分叉、同类产品、理想架构、其他双份与导出格式讨论 |
| [02-architecture.md](single-source/02-architecture.md) | **契约**:Rust 公共 API、跨语言线格式、Dart 门面、块/标记 Schema、性能预算 |
| [03-inventory.md](single-source/03-inventory.md) | 立项时的清单(历史):每个 Dart 镜像函数的去向、调用点、测试处理、构建接入点 |
| [04-parallel-plan.md](single-source/04-parallel-plan.md) | 工作包、依赖图、冲突热点、分支合并协议、日程、进度表 |
| [05-review-checklist.md](single-source/05-review-checklist.md) | 审核清单 |
| [stages/](single-source/stages/) | S0 基线 · S1 引擎拆分 · S2 绑定 · S3 编辑器边界 · S4 表格单元格 · S5 SDK 交付 |
| [history/markdown-boundary-comparison.md](single-source/history/markdown-boundary-comparison.md) | 2026-08-07 AppFlowy / AFFiNE 源码级对照(原 `architecture/`) |
| [history/rust-migration-assessment-2026-07-21.md](single-source/history/rust-migration-assessment-2026-07-21.md) | 2026-07-21 Rust 化评估与两个 wasm spike(原 `plans/`) |
