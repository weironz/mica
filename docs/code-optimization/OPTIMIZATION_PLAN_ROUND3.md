# 第三轮代码结构优化：长尾收敛

日期：2026-10-16。开发基线 `41809fc` + 第二轮未提交工作区（[第一轮](OPTIMIZATION_PLAN.md) / [第二轮](OPTIMIZATION_PLAN_ROUND2.md)）。第二轮解决了三个巨石文件并建立三层门禁；本轮处理其「值得做」清单的 2、3 两项与全部「可做可不做」项——都是长尾，单项收益不大，合起来把结构债清到只剩 god class 一项（那项等机会主义重构）。

原则沿用第二轮：**纯移动优先，行为性重构只做「提取子函数不改语义」级别**，每步以既有测试验证。

## 执行计划

### 1. 拆 `transfer.rs` 的跨域测试模块（3,929 → 约 1,100 行）

`transfer.rs` 尾部的 `mod tests`（约 2,800 行）测的根本不是 transfer：图片导出往返、搜索片段、etag、markdown_update 家族、回收站级联全是别的域。拆法：

- **纯测试按域归位**：`markdown_update_*` → `crud.rs`；etag/snippet/like_pattern → `search.rs`；图片往返/blob href → `export.rs`；transfer 自己的四个测试（`transfer_rewrites`、`dedup_sibling_name`、`page_link_targets`、`topo_order`）留在原处。
- **DB 集成测试整体独立**：嵌套的 `mod parent_guard_pg`（约 2,100 行，Postgres 后端、依赖共享 seed fixture）搬到 `documents/pg_tests.rs` 作为 `#[cfg(test)]` 模块——它是跨域集成套件，按域硬拆会把共享 fixture 撕碎，独立成文件已解决「挤在 transfer.rs」的问题。
- 归位后清理第二批为跨模块测试加的 `pub(super)`/`pub(crate)`（能收回的收回；`pg_tests.rs` 仍跨域，其触达的保留）。

### 2. 拆 `editor/editor.dart`（7,094 行）

沿用 `main.dart` 的 part 方案。按声明映射拆为 `editor_hooks.dart`（Scroll/Outline/Find/ActiveBlock/Command hooks + findTextMatches）、`editor_ai.dart`（_InlineAiDialog/_AiResult/_SlashOption/ai 常量）、`editor_dialogs.dart`（_CodeTitleDialog/_LanguagePicker/_ImageEditDialog），`editor.dart` 保留 `MicaEditor`/`_MicaEditorState` 主体。完成后视余量更新棘轮预算（低于软上限则删条目）。

### 3. 模块化 `mcp-server/src/lib.rs`（3,015 行）

37 个工具方法挤在一个 `impl MicaMcp` 里。Rust 允许同 crate 多个 impl 块，按工具族拆：`args.rs`（约 25 个 Args DTO）、`results.rs`（tool_result/ack 家族）、`markdown_windows.rs`（unwrap/window/pick_anchor/section + latex 守卫）、`tools_read.rs` / `tools_write.rs` / `tools_organize.rs` / `tools_trash.rs` / `tools_admin.rs`（impl 块分文件），`lib.rs` 留 MicaMcp 定义、ServerHandler、serve_stdio 与测试。

### 4. 复杂度热点收敛（不改行为）

- `routes/import.rs::run_import`（认知复杂度 190，约 395 行）：按阶段提取子函数（计划构建 / workspace-or-folder 落位 / 事务写入 / 收尾广播）。
- `routes/ws.rs::handle_client_message`（143，约 262 行）：按消息类型提取分支处理函数。

验收：提取后两函数本体降到「分发表 + 调用」量级；api-server 测试全绿。

### 5. 样板收敛

- `batch_trash/restore/purge/move` 四个 wrapper 100% 同构（认证 → check_batch_ids → 调内部批量函数 → 组装响应），宏化到一个 `batch_view_handler!`。
- `ensure_workspace_member` 在 `documents/mod.rs` 与 `workspaces.rs` 各有一份（真实重复），合并为一份。

### 6. 覆盖率量化（只测不改）

装 cargo-llvm-cov + llvm-tools-preview，跑 workspace 覆盖，记录盲区清单进本文档实施记录，作为后续补测依据；不引入 CI 门禁（第二轮已论证 flaky）。

## 实施记录

实施日期：2026-10-16/17，全部完成。逐项结果（验证口径：`cargo clippy --all-targets -- -D warnings` 零告警 + 对应 crate 测试全绿）：

### 1. transfer.rs 测试拆分 —— 完成

`transfer.rs` 3,929 → 1,240 行。23 个纯测试按域归位（`markdown_update_*` 家族 + outline → crud.rs；etag/snippet/like_pattern → search.rs；图片往返/blob href/percent_decode/safe_segment → export.rs；transfer 自己的 4 个测试留原处）；约 2,100 行 Postgres 集成测试（原嵌套 `mod parent_guard_pg`）整体独立为 `documents/pg_tests.rs`（`#[cfg(test)]` 模块，mod 名保留以便 `cargo test parent_guard_pg` 过滤命令继续有效）；`backlink_scan_cost` 基准测试随迁。验证：api-server 240 测试全绿（与拆分前逐数一致）。

### 2. editor.dart 拆分 —— 完成

7,094 → `editor.dart` 6,133 + `editor_hooks.dart` 210（5 个 Hook 类 + `findTextMatches`/`pasteMergesInline`/`PageLinkTarget`）+ `editor_ai.dart` 322（`_InlineAiDialog`/`_SlashOption`/slash 菜单常量）+ `editor_dialogs.dart` 440（代码标题/语言选择/图片编辑三个对话框）。验证：`flutter analyze` 零 error/warning，1,631 测试全绿。棘轮预算同步收紧 7,094 → 6,133。

### 3. mcp-server 模块化 —— 完成

3,015 → 9 个文件：`lib.rs`（MicaMcp/ServerHandler/serve_stdio/测试）+ `args.rs` + `markdown_windows.rs` + `results.rs` + 五个 tools 域文件（read/write/organize/trash/admin，182-501 行）。**关键发现**：rmcp 的 `#[tool_router]` 宏只收集它所标注的那一个 impl 块里的 `#[tool]` 方法——直接拆 impl 会静默丢工具（正是 ci.yml 里记录过的那类协议事故）。方案：每个 tools 文件用 `#[tool_router(router = <域>_router, vis = "pub(crate)")]` 生成自己的路由，`MicaMcp::full_tool_router()` 用 `ToolRouter::merge` 合并五段。实施中还真实发生过一次 `#[tool_handler]` 属性被切边界丢掉——`dead_code` lint（字段无人读）当场暴露，已修复。验证：34 个 `#[tool]` 注册数与原始一致、30 测试全绿；协议级最终验收依赖 CI 的 mcp-conformance job（本地无官方 inspector）。

### 4. 复杂度热点收敛 —— 完成

- `run_import`：395 → 238 行，提取 8 个子函数（`import_mode`/`cancel_requested`/`resolve_target_workspace`/`update_job`/`wanted_external_urls`/`prefetch_external_images`/`upload_archive_image`/`rehost_external_image`），所有原注释随逻辑迁移。取消语义保持：预取中取消返回 `Ok(false)` 由调用方退出。
- `handle_client_message`：260 → 44 行（信封解析 + 七路分发表），提取 `ClientCtx` 上下文结构 + 6 个分支处理器（presence/document.update/sync.bootstrap/sync.pull/sync.push/ping）。`document.update` 与 `sync.push` 里逐字重复的权限闸门合并为 `require_write_access`（该重复本来就是注释里自称 "Same re-check as" 的复制粘贴）。

### 5. 样板收敛 —— 完成

- `batch_view_handler!` 宏（documents/mod.rs）收敛 batch-trash/restore/purge 三个同构 wrapper（认证 → check_batch_ids → 域批量语句 → affected/skipped 组装），文档注释经宏的 `$(#[$meta])*` 保留。`batch_move_views` 有真实额外逻辑（父视图校验 + 事务 + 无 skipped 语义），不套宏，保持手写。
- 真实重复消除：`workspace_role` 与 `ensure_workspace_member` 在 documents/mod.rs 与 workspaces.rs 各有一份逐字节相同的实现——canonical 版移至 workspaces.rs（`pub(crate)`），documents/mod.rs 以 `pub(crate) use` 再导出，8 个引用文件的路径零变化。

### 6. 覆盖率量化 —— 完成

工具：cargo-llvm-cov 0.9.1 + llvm-tools-preview（`just coverage`）。**注意**：本机运行无 Postgres，api-server 的 DB 集成测试（pg_tests 家族）走 skip 路径，下列 api-server 数字是**下界**，CI 带 Postgres 时会显著更高；另 rustc 1.98.0 对 mcp-server 测试目标插桩会触发编译器崩溃（0xc0000409），该 crate 本机无法测量。

总量（region 覆盖率，排除 mcp-server）：48,182 regions / 58.21%。

核心逻辑层覆盖优秀，与第二轮「测试分布健康」的判断互相印证：

| 文件 | 覆盖 |
| --- | --- |
| markdown/block.rs · markdown/inline.rs | 96.0% · 96.6% |
| interchange/import.rs · export_tree.rs | 97.8% · 96.0% |
| cli/export.rs · mica-core/doc.rs | 94.3% · 91.4% |

盲区清单（本地口径，按缺口排序，供补测排期）：

| 文件 | 覆盖 | 说明 |
| --- | --- | --- |
| routes/workspaces.rs | 5.6% | DB handler，CI 有 Postgres 兜底 |
| routes/ai.rs | 14.7% | AI 代理路由，缺单测 |
| routes/auth.rs | 19.4% | DB 依赖 + refresh_pg 在 CI 跑 |
| app-core/store.rs | 19.6% | DB 写路径 |
| routes/documents/export.rs | 20.4% | zip/blob 组装，pg_tests 在 CI 覆盖 |
| routes/import.rs | 30.2% | 本轮已提取 8 个子函数，纯逻辑部分可补 |
| cli/main.rs | 31.3% | 命令分发 |
| routes/ws.rs | 33.2% | 本轮已拆 6 处理器，可测分发表 |
| routes/files.rs | 41.1% | 预签/完成上传 |

结论：纯逻辑（解析/导出/引擎）覆盖 90%+ 不需要补；缺口集中在 DB/IO handler——其中大部分在 CI 的 Postgres 环境被 pg_tests 真实覆盖，真正值得补的是 ai.rs（无任何 DB 测试也无线测）与两个已拆小的分发层（import.rs 子函数、ws.rs 处理器现在是纯函数化的好测形状）。不引入 CI 覆盖率门禁（第二轮已论证 flaky），本清单作为后续补测依据。

## 7. God-class 试点：干跑分析 + import 域 controller 化（同批追加）

第三轮主体完成后，按讨论对两个 god class 做了零风险干跑分析，并完成第一个域的 controller 化试点。

### 干跑分析（只读）

方法→字段使用矩阵（花括号配对提取，覆盖约 78% 方法；数字用于排期，非精确账）：

- **`_WorkspaceShellState`**（7,090 行）：46 存储字段、~250 方法、12 个语义域。22/46 字段跨域共享，但热度集中：`_session`（11 域）、`_selectedWorkspace`（9 域）、`_message`（7 域）、`_workspaces`（6 域）、`_viewsByWorkspace`（5 域）——**4 个枢纽字段是跨域耦合的根源**，直接按域拆会在 33 处接线上反复踩坑；正确顺序是先收枢纽、再拆域。
- **`_WorkspaceViewState`**（4,978 行 = widget 580 + State ~4,400）：29 字段只有 7 个跨域且多为单向读取，navtree 域（16 方法/8 字段）自洽度最高。真正的耦合面是 `WorkspaceView` widget 的 **103 个回调字段**——shell 的 controller 化会自然缩小这个回调电池。
- 难度比约 view:shell = 1:3。抽取顺序建议：import 域试点 → view 的 navtree → shell 收枢纽字段 → 逐域拆分。

### 试点实施（import 域）

- **特征测试先行**：`test/import_controller_test.dart` 20 个测试钉死原行为——`cleanArchiveName` 的 Notion ID 清洗、rehost 的 URL 级取数记忆化/块级重贴/不可重试短路、watch 状态机的 done/total 发布、取消时报已落地数、错误回退文案、二段取图先于完成通知、rehost 开关跳过二段、取消失败留任务运行。测试先于重构对控制器合同跑绿。
- **`lib/import/import_controller.dart`**（246 行，新独立库）：全部远程调用以函数注入（不依赖 `MicaApi`，测试直接编排）；持有 jobId/progress 状态；对话框/_snackbar/忙态包装留在 State，经 5 个钩子回调（onError/onCancelled/onWorkspacesLoaded/onLanded/onFinished）回投——onLanded 被 await，保持「先选工作区再取图」的原顺序。
- **State 适配层**：`_initImportController` 接线 + `_syncImportState` 把控制器状态同步进 `_importProgress`/`_importJobId`（侧栏进度行与取消按钮读点零改动）；`_watchImportJob`（96 行）与 `_cleanArchiveName` 删除；`_cancelImport`/`_rehostImportFailures` 变薄适配；requireSession 的摆放保持「缺会话 → `_message`」而非崩溃。`_importMarkdownAsPage` 深缠 tab 状态，属 views 域，未纳入本试点。

### 验证

`flutter analyze` 零 error/warning；**1,651 测试通过 / 0 失败**（原 1,631 + 20 个特征测试）；三层门禁通过，棘轮预算收紧 7,090 → 7,007。净效果：main_shell.dart −83 行，换来一个 246 行、无 context 依赖、可直接单测的控制器与一份行为契约。试点证明方法论可行；后续按干跑结论的顺序推进（下一个：view 的 navtree）。

### 第二步：view 的 navtree（SidebarTreeController）

按干跑结论的顺序执行。实施前的再勘察修正了一个假设：多选状态机早已是独立模型（`ui/tree_selection.dart`），分页树也在 `PageTreeCache` 里——真正还长在 State 上的是**展开/焦点/揭示**状态（`_expandedViewIds`/`_focusedNavId`/`_rootFocused`/`_pendingRevealId`，约 34 处引用）。

- **特征测试先行**：`test/sidebar_tree_controller_test.dart` 15 个测试——文件夹 toggle 即「定位」（收起也换焦点）、focusRoot 的幂等无通知、行点击的防抖动守卫、revealAncestors 只开祖先不开自身、expandForChildOf 两者都开、revealFolder 对未知 id 的静默、**父链成环不挂死**（`seen` 守卫）、pending 揭示的两种路径、120 帧超时自清、工作区切换清空。
- **`lib/ui/sidebar_tree_controller.dart`**（168 行）：纯状态机，无 context、无 api 依赖（`ancestorIds` 复用 models.dart 的公开函数）。五个公开语义：toggle/focusRoot/locateRow/focusView(null 可)/reveal 家族 + pending 揭示计数。
- **State 侧**：五个旧字段降级为同步视图模型（`_syncTreeState`，initState 用无 setState 的 Quiet 变体——initState 期间 setState 非法）；六个方法变一行适配（`_revealFolder`/`_revealAncestors`/`_expandForChildOf`/`_revealInTree`/`_focusRoot`/`_toggleViewExpand`，后者保留 `_pruneSelectionToVisible` 调用）；`_flushPendingReveal` 的滚动数学原样保留，计数逻辑走控制器；didUpdateWidget 的开卷定位与工作区切换清空改走控制器（didUpdateWidget 内 setState 自身合法）。`_revealAttempts` 字段因再无读取方而删除。
- **验证**：analyze 零 error/warning；**1,666 测试通过 / 0 失败**（+15）；门禁通过，预算收紧 4,978 → 4,953。

实施备注：main_workspace_view.dart 因含 Edit 工具无法解码的字节序列，全程用 awk/perl 行级编辑完成——这也解释了为何该文件此前无法用常规工具读取。

### 第三步：shell 的会话枢纽收敛（SessionController）

干跑分析认定的「硬仗」：`_session`（11 域）/`_selectedWorkspace`（9 域）/`_workspaces`（6 域）/`_viewsByWorkspace`（5 域）四个枢纽字段的 62 处直接赋值。实施时把被动容器改为**行为正确的转移集**——这些赋值簇本身是会话域行为：

- **特征测试先行**：`test/session_controller_test.dart` 18 个测试，钉死两条**故意不同**的选择回退规则：列表重取时「选中项 id 仍在则存活、消失则落空（不落到首个）」vs 工作区删除时「落到首个剩余」；以及过期仅清会话（工作区列表存活供本地世界回退）vs 登出全清；addCreated 的追加+选中+首树播种；seedViews 的「服务器新鲜条目压过镜像」合并次序。
- **`lib/session/session_controller.dart`**（162 行）：restore/signIn/adoptSession/expireSession/signOut/replaceWorkspaces（生存规则）/setWorkspaces（朴素赋值，与生存规则区分——导入完成/世界迁移路径原本就没有选择意见）/select/applyRename/addCreated/remove（返回 wasSelected 供调用方关页）/replaceViews/seedViews。
- **接线**：62 处直接赋值全部改走控制器转移（审计确认仅剩字段声明与同步方法两个合法残留）；四个旧字段降级为 `_syncSessionState` 维护的视图模型副本，build 读取点零改动；`_endExpiredSession`/登出/重建的四元组各归一个转移。中途发现并规避了一个数据回退陷阱：只要还有一处绕过控制器的视图表直写，store 的陈旧副本就会在下一次转移时覆写它——因此 21 处视图写点一次全量接线，未留半迁移状态。
- **验证**：analyze 零 error/warning；**1,684 测试通过 / 0 失败**（+18）；门禁通过，预算收紧 7,007 → 6,969。

至此 god-class 攻坚三步完成（import 域 → 侧栏树 → 会话枢纽），累计 53 个特征测试。剩余的逐域拆分（auth/sync/views 域的方法体搬迁）从此是低耦合搬运——它们的状态访问全部经控制器。

## 遗留与边界

- `pg_tests.rs` 的 DB 集成测试在本地无数据库时走 skip 路径（与拆分前行为一致），真实执行在 CI 的 Postgres 上——本次拆分依赖编译 + skip 路径 + CI 兜底验证。
- mcp-server 拆分的协议级验收（工具注册/capability 声明）以 CI mcp-conformance job 为准。
- `editor.dart` 6,133 行：part 拆分已到纯移动的极限，进一步是 `_MicaEditorState` 的 controller 化（与 main_shell 的 god class 同类，见第二轮「已知残留」）。
