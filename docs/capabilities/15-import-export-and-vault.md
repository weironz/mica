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
  - **ZIP（含图片资源）**：标准 Markdown 正文 + 相对路径图片附件包；文档内的 `mica-board` 画板自动渲染为**自包含矢量 `.svg`**（内嵌 `<metadata id="mica-board-v1">` Base64 源码）写入 `assets/`，正文改为标准 `![画板标题](assets/xxx.svg)` 方便外部工具直接预览。
  - **独立 Markdown（`.md`）**：单文件无伴生 `assets/` 目录时，保留标准围栏代码块 ` ```mica-board `，避免死链且保证数据完整。
  - **独立 HTML**：Warm-Paper 排版样式、自动转义标题、内嵌样式可离线浏览器打开，画板与 Mermaid 无头渲染为内联 `<svg>`。
  - **PDF**：桌面端/浏览器支持打印级 PDF 导出（纯 Rust 无内嵌浏览器内核时导出可打印 Warm-Paper HTML 供一键另存 PDF）。
- **`CAP-15-02` 文件夹与整工作区导出**：支持两种模式：
  - **Mica 无损备份（`.mica.zip`）**：完整保留目录层级、页面 ID/图标/属性、`.mica.json` 原生块、原始 ` ```mica-board ` 围栏块与图片附件，支持 100% 无损恢复。
  - **标准 Markdown ZIP（`.zip`）**：按目录树结构展开为标准文件夹与 `.md` 文件，安全清洗非法文件名与路径穿越字符；所有内嵌画板自动导出为 `assets/<标题>.svg` 自包含矢量图并在 `.md` 中改写为标准图片引用，同时保证外部工具可见图、导回 Mica 可编辑。

### 导入能力
- **`CAP-15-03` 多源导入（文件 / 文件夹 / ZIP / Notion）**：
  - 支持多选 `.md` 文件导入、本地目录递归导入（自动映射为 Mica 文件夹与页面树，支持根包裹）、`.mica.zip` 备份还原、普通 Markdown ZIP 导入、Notion 导出 ZIP 解析导入（自动清理 Notion 哈希后缀并修复内部链接）。
  - **自包含画板 SVG 自动无损还原**：无论通过服务端 ZIP 导入、桌面 ZIP 导入、解压后的本地文件夹导入、还是单文件 `.md` 导入，遇到指向自包含 Mica 画板 `.svg`（含 `<metadata id="mica-board-v1">`）的相对路径图片引用时，自动提取源码并还原为可编辑的 ` ```mica-board ` 块（不重复上传预览图）；画板编辑器本身也支持直接导出/重导自包含 `.svg`。普通 SVG 或被第三方剥离 metadata 的 SVG 安全降级为普通图片。
  - 桌面端使用原生系统文件/文件夹/另存为对话框（支持 Headless 降级路径）；Web 端使用浏览器原生 `Blob` + `<a download>` 下载与 `<input type="file">` + `FileReader` 选择上传，并在云端模式下直接通过服务端 API 持久化导入的文件夹与页面。

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
- **2026-11-24**：实现「自包含画板 SVG」全链路互通——`crates/mica-board` 支持 `<metadata id="mica-board-v1">` 嵌入与提取；`crates/interchange` 在纯 Markdown ZIP（工作区/文件夹/单页）导出时将 `mica-board` 块物化为 `assets/<标题>.svg` 标准图片引用，并在 ZIP、本地目录及单文件 `.md` 导入时自动无损还原为可编辑 ` ```mica-board ` 块；Mica 备份（`.mica.zip`）保持原生 ` ```mica-board ` 不变；画板独立 SVG 导出/导入同步支持无损往返。
- **2026-11-24**：打通 Web 端浏览器原生导出下载（Markdown / HTML / ZIP / 画板 SVG / 画板 JSON）与 `<input type="file">` 导入（`.md` / `.zip` / `.mica.zip` / 自包含画板 `.svg` / draw.io），修复云端页面 `view.id != object_id` 导出子树 ZIP 匹配问题，并支持云端导入直接写入服务端工作区。
