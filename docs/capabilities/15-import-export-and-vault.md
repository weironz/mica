# 15 — 导入导出、数据互通与 Markdown Round-Trip

> **适用范围**：桌面端、Web、CLI / MCP。

## 核心不变量

1. **Markdown Round-Trip 是绝对不变量**：保持 CommonMark 0.31.2 / GFM 与 Mica 扩展方言（Callout、数学公式、Mermaid、`mica-board`、Front-Matter、硬换行）双向兼容；写侧输出规范化子集，修改解析或序列化必须过 [规范记分牌](../architecture/commonmark-scoreboard.md) 与回归测试。
2. **单一互通引擎（`crates/interchange`）**：所有 `.mica.zip`、Markdown ZIP、Notion 导入导出、文件夹递归导入均由 `crates/interchange` 统一编解码，桌面/Web/CLI/服务端共享同一逻辑。
3. **无锁定数据主权**：用户任何时候均可一键导出为开放标准 Markdown + 图片目录或完整 `.mica.zip` 备份，绝不绑架用户数据。

---

## 能力与交互规格

### 导出能力
- **`CAP-15-01` 单页多格式导出**：页面 `⋯` 菜单支持导出为：
  - **ZIP（含图片资源）**：标准 Markdown 正文 + 相对路径图片附件包。
  - **独立 HTML**：Warm-Paper 排版样式、自动转义标题、内嵌样式可离线浏览器打开。
  - **PDF**：桌面端/浏览器支持打印级 PDF 导出（纯 Rust 无内嵌浏览器内核时导出可打印 Warm-Paper HTML 供一键另存 PDF）。
- **`CAP-15-02` 文件夹与整工作区导出**：支持两种模式：
  - **Mica 无损备份（`.mica.zip`）**：完整保留目录层级、页面 ID/图标/属性/白板 JSON 与图片附件，支持 100% 无损恢复。
  - **标准 Markdown ZIP（`.zip`）**：按目录树结构展开为标准文件夹与 `.md` 文件，安全清洗非法文件名与路径穿越字符。

### 导入能力
- **`CAP-15-03` 多源导入（文件 / 文件夹 / ZIP / Notion）**：
  - 支持多选 `.md` 文件导入、本地目录递归导入（自动映射为 Mica 文件夹与页面树，支持根包裹）、`.mica.zip` 备份还原、普通 Markdown ZIP 导入、Notion 导出 ZIP 解析导入（自动清理 Notion 哈希后缀并修复内部链接）。
  - 桌面端使用原生系统文件/文件夹/另存为对话框，支持 Headless/自动化测试降级路径。

### Vault 本地文件模式与外部集成
- **`CAP-15-04` 开放接口（CLI / HTTP API / MCP）**：文档读写同时通过 `mica-cli`、REST API 与 `mica-mcp` 暴露给外部脚本与 AI Agent（如 Claude Code / Antigravity），所有写入口遵守同一份树规则与 Markdown 规范。

---

## 验收清单

| 编号 | 优先级 | 适用端 | 纯 Rust 状态 | 验证入口 |
| --- | --- | --- | --- | --- |
| `CAP-15-01` 单页 ZIP / HTML / PDF 导出 | P0 | 桌面/Web | ✅ | `transfer::tests::standalone_html_wraps_warm_paper_document_and_escapes_title` + `html_and_workspace_bundle_export_round_trip` |
| `CAP-15-02` 文件夹/工作区 `.mica.zip` 与 Markdown ZIP 导出 | P0 | 桌面/Web | ✅ | `transfer::tests::mica_zip_backup_and_markdown_zip_round_trip` |
| `CAP-15-03` Markdown / 文件夹 / ZIP / Notion 导入 | P0 | 桌面/Web | ✅ | `transfer::tests::import_folder_walks_nested_markdown_and_wraps_root` + `markdown_export_and_import_round_trip` |
| `CAP-15-04` CommonMark / GFM Round-Trip 记分牌 | P0 | 全端/后端 | ✅ | `crates/md-core` 测试套件与 [commonmark-scoreboard.md](../architecture/commonmark-scoreboard.md) |

---

## 演进记录

- **2026-11-23**：沉淀导入导出、`.mica.zip` 无损备份、Notion 导入与 Markdown 往返不变量规格。
