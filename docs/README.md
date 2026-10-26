# Mica 文档索引

按用途分文件夹;每个文件夹里按主题列出。**先读哪几份**:

- 想知道还要做什么 → [`plans/roadmap.md`](plans/roadmap.md)(权威待办,只放还要做的)
- 改代码前 → [`dev/lessons.md`](dev/lessons.md)(踩过的坑)与仓库根 [`AGENTS.md`](../AGENTS.md)
- 发版 / 上线 → [`ops/release.md`](ops/release.md)、[`ops/deploy.md`](ops/deploy.md)
- 代码评审与修复记录 → [`reviews/`](#reviews--代码评审与修复)
- 进行中的专项 → [`single-source/`](single-source/README.md)(Markdown 引擎只留 Rust 一份实现,S0–S5)

| 文件夹 | 放什么 |
| --- | --- |
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

## architecture — 架构与核心机制

| 文档 | 内容 |
| --- | --- |
| [architecture.md](architecture/architecture.md) | 系统现状架构:crate 地图、存储/同步/搜索、本地优先与部署 |
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
| [design-adoption.md](features/design-adoption.md) | 设计稿(`design/` 01–19)采纳记录 |

## plans — 规划与路线图

| 文档 | 内容 |
| --- | --- |
| [roadmap.md](plans/roadmap.md) | **路线图:剩余功能与优化点(权威)** |
| [roadmap-done.md](plans/roadmap-done.md) | 已完成条目存档(只进不出) |
| [notion-gap-analysis.md](plans/notion-gap-analysis.md) | Notion 对标分析 |
| [desktop-plan.md](plans/desktop-plan.md) | 桌面端技术方案与开发计划 |
| [mvp-plan.md](plans/mvp-plan.md) | MVP 开发计划 |
| [window-manager-migration.md](plans/window-manager-migration.md) | window_manager 迁移评估(结论:不迁) |

## ops — 部署、发版与运维

| 文档 | 内容 |
| --- | --- |
| [release.md](ops/release.md) | **发版与构建**(含 Actions 不可用时的本地发布链) |
| [traefik-working-set.md](ops/traefik-working-set.md) | 共享入口最终配置、缺页 I/O 根因、部署保留方式与空闲 TLS / 目录验收 |
| [deploy.md](ops/deploy.md) | 部署、凭据、管理员、删账号 |
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

## reviews — 代码评审与修复

全项目代码评审(问题清单 + 修复状态)按日期排列;`code-optimization/` 是结构性优化的计划与验证。

| 文档 | 内容 |
| --- | --- |
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
| [03-inventory.md](single-source/03-inventory.md) | 现状清单:每个 Dart 镜像函数的去向、调用点、测试处理、构建接入点 |
| [04-parallel-plan.md](single-source/04-parallel-plan.md) | 工作包、依赖图、冲突热点、分支合并协议、日程、进度表 |
| [05-review-checklist.md](single-source/05-review-checklist.md) | 审核清单 |
| [stages/](single-source/stages/) | S0 基线 · S1 引擎拆分 · S2 绑定 · S3 编辑器边界 · S4 表格单元格 · S5 SDK 交付 |
| [history/markdown-boundary-comparison.md](single-source/history/markdown-boundary-comparison.md) | 2026-08-07 AppFlowy / AFFiNE 源码级对照(原 `architecture/`) |
| [history/rust-migration-assessment-2026-07-21.md](single-source/history/rust-migration-assessment-2026-07-21.md) | 2026-07-21 Rust 化评估与两个 wasm spike(原 `plans/`) |
