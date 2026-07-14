<!-- codebase-memory-mcp knowledge graph -->

## MCP Tools: codebase-memory-mcp

**IMPORTANT: This project has a knowledge graph (project `D-codes-mica`).
ALWAYS use the codebase-memory-mcp tools BEFORE Grep/Glob/Read to explore
the codebase.** The graph is faster, cheaper (fewer tokens), and gives you
structural context (callers, callees, data flow, tests) that file scanning
cannot. (code-review-graph was uninstalled 2026-07-15 — this is now the ONLY
code graph; ignore any lingering references to it.)

### When to use graph tools FIRST

- **Find a symbol** (function/class/route): `search_graph` (name_pattern / label / qn_pattern) instead of Grep
- **Text search across code**: `search_code` (graph-augmented grep) instead of raw Grep
- **Read one symbol's source**: `get_code_snippet` (qualified_name) — precise range, cheaper than Read
- **Call chains / impact**: `trace_path` (mode=calls|data_flow|cross_service) instead of hand-tracing imports
- **Complex relationships**: `query_graph` (Cypher — callers/callees/imports/tests)
- **Architecture questions**: `get_architecture` (aspects) instead of reading many files
- **Code review**: `detect_changes` for a risk-scored diff analysis

Fall back to Grep/Glob/Read **only** when the graph doesn't cover what you need.

### Key Tools

| Tool | Use when |
| ------ | ---------- |
| `search_graph` | Find functions/classes/routes by name / label / qualified-name |
| `search_code` | Graph-augmented text search (grep replacement) |
| `get_code_snippet` | Exact source of a symbol by qualified name |
| `trace_path` | Call chains / data flow / cross-service paths |
| `query_graph` | Cypher patterns — callers, callees, imports, tests |
| `get_architecture` | High-level structure (e.g. aspects=['all']) |
| `detect_changes` | Risk-scored review of what changed |
| `index_repository` | (Re)index — MANUAL, see Workflow #1 |
| `index_status` / `list_projects` | Check freshness / list indexed projects |

### Workflow

1. **The graph is NOT auto-updated** (no file-change hook — that was code-review-graph's).
   After a substantial change, or if `search_graph` misses a just-added symbol, re-run
   `index_repository(repo_path="D:/codes/mica", mode="full")`. Modes: `full` (incl.
   similarity/semantic edges, slow) / `moderate` / `fast`. Check freshness with `index_status`.
2. Use `search_graph` / `search_code` / `get_code_snippet` for exploration.
3. Use `trace_path` / `query_graph` for callers/callees/impact and test coverage.
4. Use `detect_changes` for code review.

<!-- Project principles & state — distilled from the Linux dev machine's agent
     memory (2026-06-06) so any session on any machine starts with them. -->

## 项目原则(长期有效,优先级高于默认习惯)

1. **In-house 优先**:最小化第三方依赖,宁可自研(编辑器整个是自绘的)。引入依赖需明确豁免(现有豁免:#1 flutter_math_fork——数学渲染;#2 window_manager——桌面窗口大小/位置/最小尺寸;#3 file_picker——桌面文件对话框(开/存/目录);#4 pasteboard——桌面富剪贴板图片读写;#5 merman——纯 Rust headless mermaid 引擎(FFI,桌面/移动离线渲 mermaid,同 flutter_math_fork 类的复杂领域渲染器,自研不现实);#6 flutter_svg——把 merman 的 SVG 栅格成 ui.Image;#7 yjs(JS 库,web-only)——web 端 CRDT 引擎,是 Rust `yrs` 核心的 JS 对端,**和 yrs 在 update/state-vector/lib0 v1 编码层字节兼容**(已实证),web 跑不了我们的 Rust FFI 故用它实现"一个权威引擎全平台跑";仅 web bundle(`tool/yjs` esbuild 打成 `web/yjs_bundle.js`),不入桌面;另 `xml`/`html` 为纯 Dart 解析库。除 flutter_math_fork/yjs 外均只在非 web 的 `_stub`/`_web` 变体里用,条件导入隔离不入 web bundle。均属标准边角/平台粘合层或复杂领域渲染器,自研要背三套平台原生层或重写一个引擎,用成熟包反而对)。in-house 该留给核心数据面(CRDT/文档模型/同步),不是平台粘合层。merman 的 mermaid SVG 主题用 CSS,纯 Dart 渲染器不解析 → 自研了 `mermaid_svg_inline.dart` 把 CSS 拍平进属性(merman 文档把这列为 host 边界)。
2. **Rust-first 数据面**:数据处理一律在 Rust 后端;Dart 只做 UI 和编辑器热路径。Markdown 语法逻辑两端必须同步(Rust `crates/markdown` 是权威,Dart `lib/editor/marks.dart`/`markdown.dart` 镜像)。
3. **渲染架构红线**:新渲染能力先抽象机制(AtomicBlockRenderer 注册表),严禁往 `render.dart` 堆 if 分支。见 `docs/render-architecture.md`。
4. **Markdown 方言原则**:CommonMark 0.31.2 底座(读侧 641/641=100%)+ GFM 扩展(24/24)+ 方言(脚注、front matter、Pandoc 数学约定);写侧输出规范化子集,round-trip 是不变量。记分牌:`docs/commonmark-scoreboard.md`,回归地板在 `commonmark_scoreboard.rs`。
5. **修复纪律**:每个 bug 修复配回归测试 + 实测验证(web 端用 playwright-cli 截图);提交信息写根因,不写流水账。
6. **难决策先调研同类产品**:面对没有明显正确/完美方案的架构或技术决策(或自己的方案有明显代价/取舍)时,拍板前先去同类开源/闭源产品扒一遍实现,当作兜底手段——去别人那里找灵感,而不是凭自己的假设硬推。重点不是"它支不支持",而是"在和我们**相同约束**下它具体怎么实现、又刻意**没用什么**"(排除法往往最有信息量;闭源就从 pubspec/依赖清单、CHANGELOG、issue 等公开痕迹推断)。最该警惕的是自己脑子里"必须 X 才能 Y"那类前提,正是它最值得被别人的实现证伪——给出"几选一"前先自问这些选项的**共同前提**验证过没有。手段:派调研子代理 + GitHub MCP 读对方真实源码。本项目参照系:AppFlowy(Flutter 原生同构)、AFFiNE(web/Yjs 对照)。〔教训:mermaid 桌面渲染曾基于"服务端渲 mermaid 必须 headless 浏览器"的错误前提,差点选 Kroki/Node/Chrome;扒 AppFlowy+AFFiNE 后才发现纯 Rust 渲染器(mermaid-rs-renderer/merman)这条离线+跨平台+无浏览器的更优路径。〕

## 当前状态(2026-06-06)

- **Web 端功能稳定**,Markdown 规范线已闭环。
- **桌面端启动**:Flutter Desktop 原生,Windows 优先。完整决策与 M1-M3 计划在 **`docs/desktop-plan.md`**(新会话先读它)。
- 图片存储:image block 存 `file_id`(非 URL),sha256 去重,加载时解析 URL;导出保留原文件名。
- 开发环境备忘(stale bundle、幽灵会话、DB 取证)也在 `docs/desktop-plan.md`。

## 架构速记

- 编辑器:单 RenderBox 自绘画布(`render.dart`),marks-over-plain-text 模型,IME 走 TextInputClient;硬换行存储约定 = 文本里的 `\`+换行。
- 块级嵌套是扁平模型:`data.indent`(列表层级)、`data.quote`/`qbreak`(引用深度/分组)、`data.li`(item 容器子块),HTML 导出端重建嵌套。
- 代码字体:web 上 `'monospace'` 族名不解析,一律用 `kMonoFont`(打包的 Roboto Mono,`model.dart`)。
