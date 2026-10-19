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

<!-- 代码结构约定：防单文件膨胀 -->
## 代码结构约定（防单文件膨胀）

背景与先例见 [docs/reviews/code-optimization/OPTIMIZATION_PLAN_ROUND2.md](docs/reviews/code-optimization/OPTIMIZATION_PLAN_ROUND2.md)。改动代码前先过一遍：

- **新代码的去处，不是「往现有大文件里续写」**：
  - API 路由进 `crates/api-server/src/routes/<域>/` 子模块（先例：`routes/documents/` 的 search/crud/export/transfer/trash/share/images）；共享 helper 与 DTO 放对应 `mod.rs` 并 `pub use` 再导出。
  - Flutter 拆大库文件用 `part` / `part of`（先例：`main.dart` 及其 `main_*.dart`，`editor/render.dart`）；私有标识符同库可见，纯移动零风险。
  - markdown 引擎按管线归位：block / inline / html / export_md / entities；公开 API 留在 `lib.rs`。
- **门禁三层**（`just check-structure` = CI `structure` job = `scripts/check-file-size.sh`）：
  1. 绝对限：Rust `crates/**/src` 与 Flutter `lib/` 单文件硬上限 8,000 行（失败），软上限 4,000 行（警告）。
  2. 棘轮：`scripts/file-size-budget.txt` 登记的文件**只许减不许增**——动这些文件（editor.dart、main_shell.dart、ui/dialogs.dart、main_workspace_view.dart）前先看预算，涨一行 CI 就红。
  3. 增量：超过 2,000 行的存量文件一次改动净增 >300 行会被拦下。要加大量逻辑时开新文件，不要续写。
- **豁免是登记制**：生成代码等豁免必须写进脚本 `EXEMPT_RE` 或预算文件并附理由；没有「惯例豁免」。提交前跑 `just check-structure` 自查。
