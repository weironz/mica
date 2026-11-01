# 01 背景：双实现是什么、为什么会这样、为什么这么改

> 本文收录 2026-10-26 的全部讨论（「100+ issue 修复复盘 → 五条优化项 → 第 4 条」一路推导下来），
> 以及 2026-07-21、2026-08-07 两次早期评估的结论。执行 agent **不需要重新论证**这里的结论；
> 如果发现事实错误，在 PR 里指出。

## 1. 「双实现」通俗地说

同一套「Markdown 怎么解析、怎么写回」的规则，仓库里写了两遍：

- **Rust 版** `crates/markdown`（约 9,400 行）：服务端在用（导入导出、MCP 写入、HTML/PDF 导出），是**权威**。
  CommonMark 0.31.2 读侧 641/641、GFM 24/24、方言（脚注、front matter、Pandoc 数学、callout、details）。
- **Dart 版**（Flutter 客户端）：Rust 版的**镜像**，主要在
  - `lib/editor/markdown.dart`（1,492 行，`markdownToBlocks` 一个函数就 913 行，镜像 Rust `import_markdown`）
  - `lib/editor/marks.dart`（2,689 行，其中约 1,900 行是行内解析器 `parseInline`、序列化器 `inlineToMarkdown`、转义、HTML）
  - `lib/editor/controller.dart` 里的块 → Markdown（`_rangeMarkdown` 等约 270 行）和块 → HTML（`selectionHtml` 约 190 行）
  - `lib/editor/table.dart`、`list_serialization.dart` 的 GFM 表格与列表编号

镜像的意思是：每改一条语法规则，两边都要改，还要保证改得一样。

## 2. 为什么会分叉：三个约束同时成立

| # | 约束 | 为什么成立 | 推出 |
| --- | --- | --- | --- |
| C1 | 服务端必须能解析 Markdown | MCP 写入（`mica_update_document` 的 append/insert_at/replace_all）、ZIP/Notion 导入、HTML/PDF 导出都在服务端。MCP 写入是 Mica 的差异化功能，不能要求调用方先跑一个 Flutter 客户端 | Rust 版必须存在，且是权威 |
| C2 | 客户端是另一种语言，Flutter Web 没有 FFI | Flutter 选型 | 客户端无法直接调 Rust（当时认为） |
| C3 | **编辑器在绘制时也要解析 Markdown** | 表格单元格的内存模型是 Markdown 原文（`TableData.rows: List<List<String>>`），绘制时每一帧 `parseInline(raw)`（`render.dart:1914`） | 客户端必须有本地的、很快的解析器 → 只能是原生 Dart |

C1 + C2 ⇒「两种语言都需要解析器」；C3 ⇒「客户端那份必须是 Dart」。三个叠在一起，双实现几乎是必然的。

**关键洞察**：C2 已经被证伪（Flutter Web 可以加载 wasm，见 §11 的实测）；C3 是**架构上的越界**——
Markdown 本该只在边界出现，却渗进了编辑器的内存模型。去掉 C3，Markdown 就只在边界被调用，
调用频率是「每次用户操作一次」，跨语言开销可忽略，单一实现就成立了。

## 3. 代价：数据

### 3.1 修两遍

| 时间段 | 改 Dart 镜像的提交 | 其中同一提交也改了 Rust |
| --- | --- | --- |
| 2026-07-01 以来 | 32 | 17 |
| 2026-09-01 以来 | 9 | **8** |

（统计方法：`git log --since=… -- crates/markdown/src` 与 `-- lib/editor/markdown.dart lib/editor/marks.dart` 的提交交集。）

另有一批专门修「双表示漂移」的提交：`c3627aae`（escapeBlockLeader 镜像 Rust）、`6491ff2d`（引用标签 case-fold 漂移）、
`0f918aed`（嵌套链接 / title 转义漂移）、`d70bc66f`（空链接标签）、`fab6cfd9`（清空漂移清单 + 序列化器丢链接）。

### 3.2 两边错得一样，只能修两遍

2026-10-26 那一轮修复里，#96（Unicode 缩进导致解析不前进）和 #98（引用块代码围栏规则与主解析器分叉）
在 Rust（`11c8a7dc`）和 Dart（`37f03278`，`markdown.dart` + `marks.dart`）各修了一次。
对拍抓不到这种 bug，因为两边的答案是一样的（一样错）。

### 3.3 跨引擎往返

AI 编辑：Dart `documentMarkdown()` 把文档序列化成 Markdown 发给服务端 → 服务端 `ai_edits.rs` 用 Rust
`import_markdown_fragment` 解析回来做 diff。序列化和解析不是同一个引擎，任何一处不一致都会变成「AI 改了它没打算改的块」。

## 4. 现有防线：挡住了什么，挡不住什么

- 导入方向：`test/markdown_conformance_test.dart` —— Dart `markdownToBlocks` 对 Rust 生成的 `.blocks.json` 黄金文件
  （`crates/markdown/tests/fixtures/conformance/`，105 个文件）。
- 导出方向：`test/markdown_export_conformance_test.dart` —— Dart 序列化对 Rust `export(import(md))`。
- 若干专项 parity 测试：`link_parity_test`、`math_code_span_test`（「Mirrors crates/markdown/tests/…」）等。

**挡得住**：两边结果不同（漂移）。**挡不住**：两边错得一样；fixture 没覆盖到的输入；以及「修两遍」本身的成本。

## 5. 同类产品怎么做（核实过的事实）

| 模式 | 代表 | 做法 | 为什么它们没有分叉 |
| --- | --- | --- | --- |
| 同一种语言 | Outline（ProseMirror） | 服务端 `server/editor` 与客户端 `@shared/editor` 用同一套扩展定义生成 schema / parser / serializer | 前后端都是 JS |
| Markdown 只在客户端 | AppFlowy、AFFiNE | 服务端只存块 / CRDT，从不解析 Markdown；客户端一个 adapter（详见 [history/markdown-boundary-comparison.md](history/markdown-boundary-comparison.md)） | 服务端不需要解析（与 Mica 的 C1 相反） |
| 一个引擎编译到多端 | 思源 Lute（Go）、Logseq mldoc（OCaml） | Lute 提供 Go 包与 `lute.min.js`（README 致谢 GopherJS，**JS 构建方式未在首页写明**）；mldoc 用 js_of_ocaml 构建 JS（`make js`、`mldoc-js`） | 一份源码，多个产物 |

共同点：**编辑器内部操作结构化模型，Markdown 只在边界出现**。区别只在边界编解码器用什么语言、跑在哪里。
Mica 的约束（C1：服务端必须解析）决定了编解码器必须是 Rust，于是只剩「一个引擎编译到多端」这条路。
这正是思源 Lute 的形态，也和本仓库「Rust-first 数据面」原则一致。

未核实的点：思源 / Logseq 前端是否用这份 JS 产物；AFFiNE 是否有 `.affine` 原生工作区文件。不影响结论。

来源：[88250/lute](https://github.com/88250/lute)、[logseq/mldoc](https://github.com/logseq/mldoc)、
[Outline server/editor/index.ts（2022 镜像）](https://igit.yuuza.net/yuuza/outline/src/commit/01a482552adfbffee585b6188094bc4aedb9f0dc/server/editor/index.ts)、
[ProseMirror/prosemirror-markdown](https://github.com/ProseMirror/prosemirror-markdown)。

## 6. 有没有完美方案

没有零代价的方案，但有干净的方向。分界线**不在「前端 / 后端」，而在「编辑模型 / Markdown 编解码」**：

- 编辑器（Dart）：只认「块 + 标记」（marks-over-plain-text，已是现状），负责交互、渲染、输入规则。
- 编解码器（Rust，唯一实现）：Markdown ⇄ 块、块 → HTML。
- 边界：粘贴、复制、导入、导出、AI 输入输出、MCP 写入。这些都是「每次用户操作一次」的调用。

输入快捷规则（打 `**` 自动加粗、`# ` 变标题）属于编辑器交互，不是解析器，留在 Dart（`controller.dart applyInputRules`）。

## 7. 理想架构（从零设计）

```
            ┌──────────── Markdown 引擎（Rust，唯一实现）────────────┐
            │ md-core   Markdown ⇄ 块文档（扁平块 + 标记），块 → HTML │
            │           可选 render：mermaid / 数学 → SVG            │
            │ markdown  Mica 适配：页面标题、资产、文本查找替换、MCP 防护 │
            └──────┬───────────────┬───────────────┬──────────────────┘
               原生调用          FFI（frb）     wasm（wasm-bindgen）
          服务端 / CLI / MCP      桌面            Web
                                   │               │
            ┌──────────────────────┴───────────────┴───────────────────┐
            │ Dart 门面 MarkdownCodec（一个接口，两个实现）             │
            │ 编辑器：只操作块 + 标记；只在边界调用门面                 │
            └──────────────────────────────────────────────────────────┘
```

## 8. 方案演进（为什么现在是这个版本）

1. **第一版（2026-10-26 上午）**：块级收到 Rust，行内留两份 + 差分测试。理由：表格单元格每帧解析，跨边界太贵。
2. **零基思考**：问题根源是 C3（编辑器热路径解析 Markdown）。去掉 C3 → 行内也能收 → 彻底单一实现。
   当时估计 S4 要改单元格存储格式、做文档迁移，风险高。
3. **代码调研后更正**（写本文时）：
   - 存储层早已统一：Rust `encode_table_cell`（`crates/markdown/src/export_md.rs`）与 Dart `TableData._encodeCell`
     都把带格式单元格写成 `{text, marks}`，无格式单元格写成字符串（旧格式也一直兼容读）。**S4 只改 Dart 内存模型，不迁移数据。**
   - Dart 的重复不止解析器：`controller.dart` 还有整套「块 → Markdown」「块 → HTML」序列化（复制、AI 输入、`documentMarkdown`），
     一并收进 S3。
   - 核心解析器唯一用到随机数的是块 id（`block.rs:654 Uuid::new_v4()`），改成调用方注入即可；
     wasm 用顺序 id（Dart 插入时本来就重新分配 id），**不需要 getrandom**。7 月 spike 的「编不过」阻塞点因此消失。

## 9. 仓库里其他「双份」与后续专项（不在本专项范围）

| 双份 | 现状 | 收成单份的做法 | 风险 | 建议 |
| --- | --- | --- | --- | --- |
| ~~接口数据结构~~ | **Rust 侧已收(2026-11-01)**:`crates/api-types`(`mica-api-types`)定义 User / Auth / Workspace / View / Token / BatchOutcome / ExportManifest / ImageFailure,服务端(`sqlx` feature 派生 `FromRow`)与 CLI 共用;MCP 本就按 JSON 值读。`ImportJob` 留在服务端:它是带持久化的服务端状态,CLI 刻意把 status 当字符串读,旧 CLI 才能容忍新状态。**Dart 不生成**:`api/models.dart` 带客户端语义(icon 去空白、时间转本地、缺省值、`withAvatarVersion` 等),生成器要么丢掉这些、要么自己长成一套方言;Dart 继续由 `contracts/*.json` 黄金契约守 | — | — | Rust 侧已完成;Dart 维持契约守护 |
| ~~服务端 / 本地库业务规则~~ | **已收(2026-11-01)**:能写成纯函数的判定进 `mica_core::tree_rules`——父节点合法性 `judge_parent`(拒绝理由 `ParentRefusal` 一份措辞;唯一的策略差异「看不到的父节点」显式成 `MissingParent::Refuse/Accept`)和同级重名 `dedup_sibling_name`(原来两份逐字相同)。在两边都是一条 SQL 的规则(整棵回收、只恢复同一批、恢复时把孤儿抬到顶层)不搬:改成取事实→判定→写会多出往返和锁窗口;改为 `contracts/tree-rules.json` 场景文件,本地库(`mica-core/tests/tree_rules.rs`)与服务端(`tree_rules_pg_tests.rs`)各自回放,改一边另一边红(已用变异验证) | — | — | 已完成 |
| ~~CRDT 引擎~~ | **已收（2026-11-01）**：Web 改跑 mica-core 的 wasm（`crates/doc-wasm`，`scripts/build-doc-wasm.sh`，gzip 约 251 KB，硬预算 256 KiB），与桌面共用 `SyncDocReplica` 适配与 `BlockOpMirror` op 回放；删除 yjs 与 Dart 镜像（`mica_ydoc` / `marks_diff` / `text_diff` / `yjs_interop` / `tool/yjs` 含一份 JS 版撤销定位）。旧浏览器里 yjs 写的 IndexedDB 文档照常读取，由 `mica-core/tests/legacy_yjs_state.rs` 用 yjs 真实写出的状态钉住。见 [验证与迁移记录](../reviews/crdt-web-wasm-spike-2026-11-01.md) | — | — | 已完成 |
| ~~页面属性(front matter)~~ | **已收(2026-10-31)**:线格式加 `properties_json`,门面加 `properties`,`lib/editor/properties.dart` 只剩类型与薄封装,解析/写回只在 `md-core/src/properties.rs` | — | — | 已完成 |
| 同步状态机 | 已合并为一份 `cloud_sync_session.dart`（7 月评估的丁-1，已完成） | — | — | 无需再做 |

**不该收的**：存储层（Postgres / SQLite / IndexedDB，平台决定，共享的是接口约定 `CloudDocStore`）；
多层防线（API 校验 + DB 触发器、UI 预检 + 数据层判定，刻意冗余）；平台胶水（`_io` / `_web` 条件导入）。

### 9.1 导出 / 备份格式（讨论结论，未排期）

现状更像 Notion：对用户可见的只有通用格式（Markdown ZIP）。导出包从 2026-08-30 起已经同时写
`<页面>.mica.json`（原生块数据 + id + 树 + 图标，manifest v2）。**N1 已做（2026-11-01）**：导入我们自己的导出包时，
`.md` 未被改动（sidecar 里的 `md_fingerprint` 对得上）的页面直接用 `.mica.json` 的块建文档——块 id、图片宽度等不经 Markdown 的属性保住，
图片按 `assets` 映射重新上传，`mica://page/<旧 id>` 按 manifest 的 id 改指新页；`.md` 被手改过、或 11-01 之前的导出包（无指纹），照旧走 Markdown。
服务端与本地导入共用 `mica_interchange::native_payload`。此前导入只走通用 Markdown 解析 → 页面 id、评论、编辑历史、分享链接丢失（页面 id、评论、历史、分享链接 N1 仍不带）；备份只能整库恢复，不能单工作区原样恢复
（见 `docs/ops/backup.md`「What this restores, and what it does not」）。

同类：思源（磁盘上 `.sy` 即 JSON，另有 `.sy.zip` / Data 导出）、AppFlowy（工作区 ZIP，官方称可原样恢复）都有「原生无损格式 + 通用格式」两层；
Notion 只有通用格式。

建议方向：原生包 `.mica.zip`（块 JSON 而非 CRDT 二进制；不导出分享令牌、成员、账号）+ 通用 Markdown/HTML/PDF + 实例灾备。
步骤 ~~N1 给 `.mica.json` 补导入器~~（已做）→ ~~N2 加评论与页面属性（v3）~~（已做，2026-11-01）→
~~N3 界面区分「备份」与「导出 Markdown」~~（已做，2026-11-01：工作区/文件夹菜单「导出」分「备份(Mica 格式,可完整还原)」
下载 `<名>.mica.zip`、「导出为 Markdown」下载 `<名>.zip` 不含 sidecar；服务端 `?format=backup|markdown`，缺省为备份，
CLI 镜像不变）→ N4 单工作区恢复。本地↔云端迁移没有改格式：它本来就逐页回放原生块（不经 Markdown），
再套一层 ZIP 只会多一次编解码，故不并入。

与本专项的关系：原生包导入**不经过 Markdown 解析**，Markdown 只处理外来数据 —— 与 D1 同一原则。

## 10. SDK：能不能、值不值

- **能**：本专项把引擎拆成 SDK 形状（核心 + 适配 + 绑定）。仅内部就已有五个使用方（服务端、CLI、MCP、桌面、Web），这一步现在就有收益。
- **对外发布另行决定**：发布即长期承诺（API 稳定、semver、文档、issue）。通用 Markdown 解析器赛道拥挤
  （Rust：pulldown-cmark、comrak、markdown-rs）；Mica 的差异在「Markdown ⇄ 块文档、保证往返一致」
  （记分牌、往返不变量、中文强调、方言可控），定位接近 Lute 自称的「结构化 Markdown 引擎」——小众但真实。
- 2026-08-07 的结论是「暂不发布：没有外部需求驱动，而代价是永久的」（[history/markdown-boundary-comparison.md](history/markdown-boundary-comparison.md) §5）。
  本专项不推翻它：S5 做到「随时可发布」，发布由用户拍板。

## 11. 早期评估的实测数据（仍然有效）

摘自 [history/rust-migration-assessment-2026-07-21.md](history/rust-migration-assessment-2026-07-21.md)：

| 测量（2026-07-21） | Dart | wasm（含 JSON 序列化） | 备注 |
| --- | --- | --- | --- |
| 裸 wasm-bindgen，Node/V8，87 字符 | 0.059 ms | 0.029 ms | 对照组是 Dart VM，不公平 |
| frb wasm，真实 Chrome，87 字符 | 0.085 / 0.046 ms | 0.093 / 0.079 ms | 同一浏览器对照，基本打平 |
| frb wasm，真实 Chrome，7,118 字符 | 4.22 / 6.38 ms | 5.73 / 5.43 ms | 打平 |
| 产物体积（frb，wasm-opt） | — | 180 KB 原始 / 78 KB gzip + JS 6 KB gzip | 现有 web 产物 59 MB |

当时没证明的：Flutter Web 里的 interop 开销（本专项 S2 P0 实测）；stable 工具链免 `-Z build-std` 的路径（本专项用 wasm-bindgen 直出，不用 frb web）；
现有 FFI crate 整体编不进 web（依赖 `libsqlite3-sys`）→ 必须另建 web 专用 crate（本专项 `crates/md-wasm`）。

7 月结论：「可行但暂不推进；触发条件应是又出现了 fixture 抓不到的漂移，或要新增一个平台」。§3.2 即这个触发条件。
