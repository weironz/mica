<!-- code-review-graph MCP tools -->
## MCP Tools: code-review-graph

**IMPORTANT: This project has a knowledge graph. ALWAYS use the
code-review-graph MCP tools BEFORE using Grep/Glob/Read to explore
the codebase.** The graph is faster, cheaper (fewer tokens), and gives
you structural context (callers, dependents, test coverage) that file
scanning cannot.

### When to use graph tools FIRST

- **Exploring code**: `semantic_search_nodes` or `query_graph` instead of Grep
- **Understanding impact**: `get_impact_radius` instead of manually tracing imports
- **Code review**: `detect_changes` + `get_review_context` instead of reading entire files
- **Finding relationships**: `query_graph` with callers_of/callees_of/imports_of/tests_for
- **Architecture questions**: `get_architecture_overview` + `list_communities`

Fall back to Grep/Glob/Read **only** when the graph doesn't cover what you need.

### Key Tools

| Tool | Use when |
| ------ | ---------- |
| `detect_changes` | Reviewing code changes — gives risk-scored analysis |
| `get_review_context` | Need source snippets for review — token-efficient |
| `get_impact_radius` | Understanding blast radius of a change |
| `get_affected_flows` | Finding which execution paths are impacted |
| `query_graph` | Tracing callers, callees, imports, tests, dependencies |
| `semantic_search_nodes` | Finding functions/classes by name or keyword |
| `get_architecture_overview` | Understanding high-level codebase structure |
| `refactor_tool` | Planning renames, finding dead code |

### Workflow

1. The graph auto-updates on file changes (via hooks).
2. Use `detect_changes` for code review.
3. Use `get_affected_flows` to understand impact.
4. Use `query_graph` pattern="tests_for" to check coverage.

<!-- Project principles & state — distilled from the Linux dev machine's agent
     memory (2026-06-06) so any session on any machine starts with them. -->

## 项目原则(长期有效,优先级高于默认习惯)

1. **In-house 优先**:最小化第三方依赖,宁可自研(编辑器整个是自绘的)。引入依赖需明确豁免(现有豁免:flutter_math_fork;window_manager——桌面窗口大小/位置/最小尺寸;file_picker——桌面文件对话框(开/存/目录),只在非 web 的 `_stub` 变体里用。均属标准边角/平台粘合层,自研要背三套平台原生层,用成熟包反而对;条件导入隔离,不入 web bundle)。in-house 该留给核心数据面(CRDT/文档模型/同步),不是平台粘合层。
2. **Rust-first 数据面**:数据处理一律在 Rust 后端;Dart 只做 UI 和编辑器热路径。Markdown 语法逻辑两端必须同步(Rust `crates/markdown` 是权威,Dart `lib/editor/marks.dart`/`markdown.dart` 镜像)。
3. **渲染架构红线**:新渲染能力先抽象机制(AtomicBlockRenderer 注册表),严禁往 `render.dart` 堆 if 分支。见 `docs/render-architecture.md`。
4. **Markdown 方言原则**:CommonMark 0.31.2 底座(读侧 641/641=100%)+ GFM 扩展(24/24)+ 方言(脚注、front matter、Pandoc 数学约定);写侧输出规范化子集,round-trip 是不变量。记分牌:`docs/commonmark-scoreboard.md`,回归地板在 `commonmark_scoreboard.rs`。
5. **修复纪律**:每个 bug 修复配回归测试 + 实测验证(web 端用 playwright-cli 截图);提交信息写根因,不写流水账。

## 当前状态(2026-06-06)

- **Web 端功能稳定**,Markdown 规范线已闭环。
- **桌面端启动**:Flutter Desktop 原生,Windows 优先。完整决策与 M1-M3 计划在 **`docs/desktop-plan.md`**(新会话先读它)。
- 图片存储:image block 存 `file_id`(非 URL),sha256 去重,加载时解析 URL;导出保留原文件名。
- 开发环境备忘(stale bundle、幽灵会话、DB 取证)也在 `docs/desktop-plan.md`。

## 架构速记

- 编辑器:单 RenderBox 自绘画布(`render.dart`),marks-over-plain-text 模型,IME 走 TextInputClient;硬换行存储约定 = 文本里的 `\`+换行。
- 块级嵌套是扁平模型:`data.indent`(列表层级)、`data.quote`/`qbreak`(引用深度/分组)、`data.li`(item 容器子块),HTML 导出端重建嵌套。
- 代码字体:web 上 `'monospace'` 族名不解析,一律用 `kMonoFont`(打包的 Roboto Mono,`model.dart`)。
