# 第二轮代码结构优化：巨石文件拆分

日期：2026-10-16。开发基线 `41809fc`（v0.13.52）。本文档是 [第一轮优化计划](OPTIMIZATION_PLAN.md)（2026-09-29，风险修复导向）之后的第二轮，聚焦**代码结构债务**：巨石文件拆分与防劣化门禁。第一轮解决「会不会坏」，本轮解决「还能不能维护」。

## 评估结论（为什么做这一轮）

基于仓库知识图谱 + 源码抽查的全量评估，结论是**不是屎山，但有三处明确的巨石热点**。健康面与债务面的证据如下，供后续复查对照。

健康面（不需要动的部分）：

- 分层干净：`api-server`/`cli`/`mcp-server`/`interchange` → `app-core` → `mica-core`/`infra`/`markdown`，依赖方向单一，无跨 crate 循环依赖。全库 31 个调用环中，除 `mermaid.min`/`yjs_bundle` 等第三方压缩产物外，真实代码的环均为解析器互递归与断线重连这类领域固有模式。
- 测试充足且分布均匀：785 个 Rust 测试，每个 crate 均有覆盖（markdown 120、mica-core 150、api-server 246 等）。
- 错误处理纪律好：`src/` 下 1019 处 `.unwrap()` 甄别后仅 **9 处**在生产路径，其余 1010 处位于内联 `#[cfg(test)]` 模块。
- 无大段复制粘贴：图谱相似度边集中在小的成对样板（`batch_trash/restore/purge`、`ensure_xxx` 校验、CLI `list_xxx`），属轻微样板而非重复实现。
- TODO/FIXME 欠账接近零（crates 1 处、Flutter 0 处）。

债务面（本轮对象）：

| 文件 | 拆分前规模 | 问题 |
| --- | --- | --- |
| `clients/mica_flutter/lib/main.dart` | 13,177 行 / 约 1,347 个方法 | `_WorkspaceShellState`（338–7423 行，约 7,100 行）与 `_WorkspaceViewState`（8780–13177 行，约 4,400 行）两个巨型 State 类，标准 god class |
| `crates/api-server/src/routes/documents.rs` | 8,196 行 / 102 个函数 | 69 个 HTTP handler 挤在单文件，胖路由模块（内部模式其实一致） |
| `crates/markdown/src/lib.rs` | 6,339 行 | 手写 CommonMark/GFM 引擎全部内联：`import_markdown_inner` 单函数 1,359 行、认知复杂度 649（全仓最高） |

次级（本轮不动，列入观察）：`editor/editor.dart` 7,094、`ui/dialogs.dart` 5,850、`mcp-server/src/lib.rs` 3,015、`routes/auth.rs` 2,444、`editor/render.dart` 3,805。

## 原则：纯移动，不改逻辑

本轮全部拆分只做**物理移动**：把整函数/整类搬到新文件，补齐可见性（`pub(crate)`/`pub(super)`）与模块声明，不改任何执行逻辑、不重命名、不调整行为。理由：

1. 三个热点各有测试兜底（markdown 120 个、api-server 246 个、Flutter 端 analyze + 单测），纯移动的回归面最小且可验证；
2. 重写（尤其 markdown 引擎）风险与收益不成比例——`import_markdown_inner` 的复杂度是「完整 CommonMark/GFM 规范合规」的固有复杂度，代码注释引用规范用例编号，质量受控。**本轮明确不重写解析器**，只拆文件边界。

## 执行计划与验收

每项验收标准：既有测试全绿 + lint 无新增告警 + 行为零变化（不引入任何逻辑 diff）。

### 1. 拆分 `main.dart`（Flutter 外壳）

方法：沿用本仓库既有的 `part`/`part of` 约定（`ui/dialogs.dart`、`ui/widgets.dart` 已是 `main.dart` 的 part，私有标识符同库可见，无需改访问性）。把顶层声明按下表搬到 `main_*.dart` part 文件，`main.dart` 保留常量、`main()`、`MicaApp` 与 part 指令，成为约 300 行的入口。

| 新 part 文件 | 内容（原行区间） | 约行数 |
| --- | --- | --- |
| `main_shell.dart` | `_WorkspaceShellState` + `_activeEditorFlush`（338–7433） | ~7,100 |
| `main_workspace_view.dart` | `WorkspaceView` + `_WorkspaceViewState`（8202–13177） | ~4,980 |
| `main_panels.dart` | `_RenameDialog`、`_BacklinksPanel`、`_HomeNavRow`、`_WordCountBadge`、`_ToolsTab`、`_ToolsTabButton`、`_CommentsButton`、`_SyncBadge`、`_DropIndicator`（7434–7898、8045–8137） | ~660 |
| `main_tree_helpers.dart` | `pagePathSegments`、`ancestorPathSegments`、`WorldCardMode`、树宽常量、`DropIndicatorState`、`reparentLevelFor`、`rootDropOrder`（7899–8044、8138–8201） | ~200 |

验收：`flutter analyze` 无 error/warning（info 维持基线水平），`flutter test` 全绿。

说明：两个巨型 State 类**跨文件不可再拆**（Dart 类声明不能跨文件），把 god class 真正改造成 controller+widgets 属于行为性重构，超出本轮「纯移动」边界，留待第三轮。

### 2. 拆分 `documents.rs`（API 路由）

方法：`routes/documents.rs` → `routes/documents/mod.rs`（共享 helper + `pub use` 再导出，外部 `routes::documents::xxx` 引用路径不变）+ 域子模块：

| 子模块 | 内容 |
| --- | --- |
| `search.rs` | `list_views`、`search_workspace`、`search_all_workspaces`、`search_views`、`backlinks`、`graph` |
| `crud.rs` | 文档/文件夹 CRUD、`bootstrap_document`、`apply_document_update`、`document_outline`、`batch_read_documents` |
| `trash.rs` | 回收站单体与批量操作（trash/restore/purge 家族） |
| `transfer.rs` | `move/reorder/transfer/clone` 家族、`TransferPlan`/`TransferRow`、批量移动 |
| `images.rs` | `drop_image`、`rehost_image` |
| `export.rs` | 全部导出（markdown/html/zip 家族、blob href 处理、manifest） |
| `share.rs` | 分享链接家族、`SHARE_CSP`、`render_share_shell` |

验收：`cargo test -p mica-api-server` 全绿；`cargo clippy --workspace --all-targets -- -D warnings` 干净；外部文件对 `routes::documents::*` 的引用零修改。

### 3. 拆分 `markdown` 引擎（`lib.rs` → 模块）

方法：按解析/渲染管线拆为同 crate 内模块，公开 API（`pub` 项签名）不变：

| 模块 | 内容 |
| --- | --- |
| `lib.rs` | 公共 API（`DocumentSnapshotPayload`、`Block`、`import_markdown*`、`export_*`、`block_index` 等）+ 模块声明 |
| `block.rs` | 块级解析：`import_markdown_inner` 与块分类/缩进/fence/quote/alert 等辅助 |
| `inline.rs` | 行内解析：`parse_inline_memo`、emphasis/双指针扫描、autolink、链接引用定义 |
| `entities.rs` | `named_entity`、`parse_entity` |
| `html.rs` | HTML 渲染与净化（`export_html` 内部、`strip_unsafe_attrs`、tagfilter） |
| `export_md.rs` | Markdown 回写（`append_markdown_*`、表格序列化、escape 家族） |

验收：`cargo test -p mica-markdown` 全绿（含 conformance/commonmark_scoreboard 等 20 个集成测试文件）；clippy `-D warnings` 干净。

### 4. CI 门禁：防巨石回潮

方法：新增行数门禁脚本（bash，零依赖，CI 与本地 `just` 均可跑）+ 接入 `ci.yml`：

- 阈值：Rust `src/` 与 Dart `lib/` 单文件 **≤ 8,000 行**（硬失败，本期拆分后最大剩余文件 `editor.dart` 7,094 行留有裕量）；**> 4,000 行**输出警告清单。
- 已知例外显式登记在脚本 allowlist（如生成文件），新增例外必须改脚本留下痕迹。
- 复杂度门禁不引入新 clippy restriction lint（存量代码会爆）：维持现有 `clippy -D warnings`，把 `import_markdown_inner` 登记为本轮已知复杂度豁免。

验收：门禁在当前代码上通过；人为注入超限文件能失败（本地验证）。

### 5. 膨胀提前规避：从「事后拦截」到「事前预防」（同日补充）

第 4 项的盲区：绝对限只看行数不看增量——一个 3,900 行的文件每次 PR 涨 100 行永远触发不了警告；4 条 WARN 也只是日志，拦不住那几个文件继续变大。补三层：

1. **棘轮预算**：`scripts/file-size-budget.txt` 登记当前超软上限的 4 个文件及基线行数，CI 规则变为**只许减不许增**（涨 1 行失败；降了提示收紧基线；条目指向不存在的文件也失败，逼着改名/删除后同步）。警告清单从「参考信息」变成只收敛不扩张的目标。
2. **增量门禁**：`check-file-size.sh --diff <base>`（CI 传 `merge-base(origin/main, HEAD)`）对超过 2,000 行的**存量**文件，一次改动净增 >300 行即失败。在「往大文件续写」的决策点拦截，而非等绝对限触发。新建文件豁免增量门禁（拆分产出的新文件不误伤，由绝对限管理）。
3. **约定注入**：`AGENTS.md` 增设「代码结构约定」节——新代码的去处（路由域子模块 / Dart part 文件 / markdown 管线模块）、三层门禁规则、豁免登记制。本仓库开发高度依赖 AI 辅助，把边界写进 agent 指令等于在每个会话开始时注入约束。

验收：棘轮违规（基线调低 1 行）、增量违规（+400 行进 2,500 行文件）、新文件豁免（新建 3,000 行文件不误伤）、过期预算条目四条路径均有反向验证。

## 实施记录

实施日期：2026-10-16，基线 `41809fc`。全部按「纯移动」完成，验证命令与结果如下。

### 1. main.dart 拆分 —— 完成

| 文件 | 行数 | 内容 |
| --- | --- | --- |
| `main.dart`（保留） | 341 | 常量、`main()`、`MicaApp`、`WorkspaceShell` + part 指令 |
| `main_shell.dart` | 7,090 | `_WorkspaceShellState` + `_activeEditorFlush` |
| `main_workspace_view.dart` | 4,978 | `WorkspaceView` + `_WorkspaceViewState` |
| `main_panels.dart` | 513 | 重命名/反链/工具条/同步徽章/拖放指示器等 9 个独立 widget |
| `main_tree_helpers.dart` | 267 | 路径段、`WorldCardMode`、树宽常量、`reparentLevelFor`、`rootDropOrder` |

验证：`flutter analyze` 零 error/warning（136 条 info 全部为存量风格提示，集中在 test/tool 文件）；`flutter test` **1,631 通过 / 2 跳过 / 0 失败**。

实施中发现并处理：本地 `lib/l10n/app_localizations*.dart` 生成文件过期（缺少 `updaterAbortedBody` 等新键，属本地环境问题而非代码问题），已 `flutter gen-l10n` 重新生成后分析恢复干净。

### 2. documents.rs 拆分 —— 完成

`routes/documents.rs`（8,196 行）→ `routes/documents/` 目录 8 个文件：

| 文件 | 行数 | 域 |
| --- | --- | --- |
| `mod.rs` | 636 | 共享 DTO、fetch/ensure/normalize 家族、`pub use` 再导出（外部 `routes::documents::*` 路径零变化） |
| `transfer.rs` | 3,929 | move/reorder/transfer/clone 家族 + 原 `tests` 模块（约 2,800 行跨域测试，见「已知残留」） |
| `export.rs` | 1,260 | markdown/html/zip 导出全家 + blob href 处理 + manifest |
| `crud.rs` | 1,094 | 文档/文件夹 CRUD、bootstrap/apply、outline、batch read + `markdown_update_tests` |
| `search.rs` | 652 | 列表/搜索/反链/图 |
| `trash.rs` | 425 | 回收站单体与批量操作 |
| `share.rs` | 182 | 公开分享 |
| `images.rs` | 167 | 图片 drop/rehost |

除物理移动外的唯一变化是可见性：被跨模块引用的私有 helper 提升为 `pub(super)`（约 20 处），被跨模块测试触达的 DTO 字段提升为 `pub(crate)`（`SearchResult`、`GraphResponse` 家族、`ExportManifestResponse` 家族、`DocumentOutlineResponse` 家族）。

验证：`cargo check/clippy -p mica-api-server --all-targets -- -D warnings` 零告警；`cargo test -p mica-api-server` **240 通过 / 6 忽略（DB 集成）/ 0 失败**。

### 3. markdown 引擎拆分 —— 完成

`lib.rs`（6,339 行）→ 6 个文件，公开 API 签名不变（`math_run_spans` 经 `pub use` 保持原路径）：

| 文件 | 行数 | 职责 |
| --- | --- | --- |
| `lib.rs` | 907 | 公共 API、cfg 双实现的 math/mermaid、`block_index`/`is_descendant`、内联测试 |
| `block.rs` | 2,094 | 块级解析（`import_markdown_inner` + 块分类/缩进/fence/quote/alert/HTML 块判定） |
| `inline.rs` | 1,849 | 行内解析（`parse_inline_memo`、emphasis/autolink/链接引用定义） |
| `html.rs` | 890 | HTML 渲染与净化（tagfilter、`strip_unsafe_attrs`） |
| `export_md.rs` | 441 | Markdown 回写（escape 家族、表格序列化） |
| `entities.rs` | 212 | 命名实体表 |

模块内私有项批量提升 `pub(crate)`（模块间耦合本来就是一张网，逐个甄别无收益）；`InlineMark`/`ParsedInline` 字段同步提升。**未重写任何解析逻辑。**

验证：`cargo test -p mica-markdown` **117 通过 / 0 失败**；`--all-features` **120 通过 / 0 失败**（render 特性下的 3 个额外测试）；两套特性 `clippy -D warnings` 均零告警。

### 4. CI 门禁 —— 完成

- 新增 `scripts/check-file-size.sh`：Rust `src/` 与 Flutter `lib/` 单文件硬上限 8,000 行（失败）、软上限 4,000 行（警告清单）；生成代码（`lib/src/rust/`、`lib/l10n/app_localizations*`）显式豁免，新增豁免必须改脚本留痕。
- `ci.yml` 新增 `structure` job（秒级、仅需 checkout、排在最前）；`just check-structure` 本地同款。
- 已做注入式反向验证：伪造 8,001 行文件时正确失败；当前树通过并输出 4 条 WARN（`editor.dart` 7,094、`main_shell.dart` 7,090、`ui/dialogs.dart` 5,850、`main_workspace_view.dart` 4,978）。
- 复杂度门禁维持既有 `cargo clippy --workspace --all-targets -- -D warnings`，不引入 restriction lint（存量会爆）；`import_markdown_inner` 为登记在案的已知复杂度豁免。

### 5. 膨胀提前规避 —— 完成

- **棘轮**：`scripts/file-size-budget.txt` 登记上述 4 个文件及 2026-10-16 基线；`check-file-size.sh` 默认模式即校验。反向验证：把 `editor.dart` 基线调低 1 行 → 正确 FAIL（+1 棘轮违规）；调高 10 行 → 正确输出「可收紧基线」提示；预算条目指向不存在的文件 → 正确 FAIL（过期条目）。
- **增量**：`--diff <base>` 模式 + `ci.yml` 传 `merge-base(origin/main, HEAD)`（`fetch-depth: 0`）。临时仓库反向验证：2,500 行文件 +400 → FAIL；小文件 +400 → 放行；已提交的全新 3,000 行文件 → 放行（新文件豁免，拆分不误伤）。
- **约定**：`AGENTS.md` 新增「代码结构约定（防单文件膨胀）」节，含新代码去处与门禁规则；`justfile` 的 `check-structure` 注释同步三层防线说明。
- 实现备注：增量门禁基于**已提交**的 diff（`git diff --numstat`，CI 语义正确）；本地未提交的工作区改动不在其视野内，本地自查以绝对限 + 棘轮为准。

### 全量回归

- `cargo clippy --workspace --all-targets -- -D warnings`：**0 error**。
- `cargo test`（CI 同款 8 包列表）：全部通过（明细见上文各项）。
- `flutter analyze` + `flutter test`：通过（见第 1 项）。
- `cargo fmt`：仓库本就未启用（存量 938 处与本次无关的差异，CI 无 fmt 步骤）；新文件为原样搬移，风格与原文件一致。

## 已知残留（第三轮候选）

1. `_WorkspaceShellState`（7,090 行）与 `_WorkspaceViewState`（4,978 行）仍是单文件巨型 State 类——Dart 类声明不能跨文件，真正解耦需要 controller+widgets 的行为性重构，超出本轮纯移动边界。
2. `transfer.rs` 内的跨域 `tests` 模块（约 2,800 行）测的是 search/export/trash 等多个域，可按域拆进各模块。
3. 软上限警告清单的四个文件（`editor.dart`、`main_shell.dart`、`ui/dialogs.dart`、`main_workspace_view.dart`）。
4. `editor/editor.dart` 7,094 行是下一个最大的单文件，拆分模式可复用本轮的 part 方案。

## 风险与回滚

- 全部为纯移动改动，任一项出问题可单独 `git revert`，互不依赖（按 1→4 顺序独立提交）。
- 拆分期间如基线测试本身有失败，先停下核对是否为存量问题（2026-10-16 基线：`flutter analyze` 500 条均为 info 级且集中在 test 文件；Rust 基线结果见下）。
