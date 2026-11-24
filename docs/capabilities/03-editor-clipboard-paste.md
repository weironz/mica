# 03 — 剪贴板、富文本复制粘贴与格式保真

> **适用范围**：桌面端（Windows / macOS / Linux）、Web、移动端。

## 核心不变量

1. **同一套 Markdown 引擎是粘贴与复制的唯一语法来源**：HTML → Markdown 转换、Markdown 解析、规范化序列化全部经由 `crates/md-core`，UI 层不自造正则语法解析器。
2. **双通道富剪贴板写入**：在 Mica 内复制/剪切文档内容时，同时写入带完整格式的 Markdown 纯文本通道与语义 HTML 通道（Windows 按 `CF_HTML` 封包），既保证本应用内 100% 无损往返，又保证粘贴到 Word / 飞书 / 邮件 / Notion 时保留富文本样式。
3. **粘贴修复启发式只作用于粘贴**：针对 LLM（ChatGPT/Claude）双围栏与误标语言的自动修复仅在粘贴时运行，绝不修改已存储文档、文件导入或导出结果。

---

## 能力与交互规格

### 复制与剪切
- **`CAP-03-01` 同应用复制/剪切粘贴 100% 保留 Markdown 结构**：在页面内选中任意单块或多块（含标题、加粗/斜体/行内代码/删除线/链接、嵌套列表、待办勾选状态、引用与 GFM Callout、表格、带语言标签的代码块、Mermaid、公式、内嵌白板），执行 `Mod+C` / `Mod+X` 后粘贴到同页面或其他页面，所有块类型与行内格式完整保留。
- **`CAP-03-02` 整页复制 Markdown**：页面右上角 `⋯` 菜单「复制页面内容」将整页导出为规范 Markdown 写入剪贴板，并给出状态提示。
- **`CAP-03-03` 跨应用导出 HTML 兼容**：复制内容粘贴到外部富文本编辑器（Word、飞书、浏览器编辑器）时，通过 `HtmlProfile::Clipboard` 输出干净语义 HTML。

### 跨应用粘贴（含 v0.14.11 / 浏览器 / Word / Excel）
- **`CAP-03-04` Windows `CF_HTML` 精确字节偏移切片**：读取 Windows 剪贴板 `"HTML Format"` 时，必须按头部 `StartFragment:` / `EndFragment:` 的 **UTF-8 字节偏移**精确切片（辅以 `<!--StartFragment-->`..`<!--EndFragment-->` 兜底），严禁将周围页面上下文一并粘贴；若头部损坏则安全降级回纯文本。
- **`CAP-03-05` 从 v0.14.11（Flutter）及其他应用粘贴保留渲染效果**：从 v0.14.11、Chrome/Edge、Notion、飞书、Word 复制富文本粘贴进 Mica 时，能完整还原标题、加粗、斜体、链接、引用、列表、代码块与表格，不退化为纯文本。
- **`CAP-03-06` 表格与预览位图优先级仲裁**：从 Excel / Google Sheets 复制表格时，剪贴板往往同时携带 `<table>` HTML 与预览位图；若 HTML 包含真实数据 `<table>` 且不含 `<img>`，必须优先粘贴为 GFM 表格而非图片；反之若 HTML 只是布局表包裹 `<img>`（如 Word 复制一张图），则优先粘贴为图片。
- **`CAP-03-07` 纯文本无格式粘贴（`Mod+Shift+V`）**：强制以字面纯文本插入，不触发 Markdown 或 HTML 富文本解析；在代码块内部粘贴时亦始终按原样保留换行与缩进，不拆出新块。

### 智能粘贴识别与图片粘贴
- **`CAP-03-08` 剪贴板图片直接粘贴**：
  - 粘贴系统截图（Windows 截图工具/QQ/微信截图，优先读 `"PNG"` 格式保留透明通道，回退 `CF_DIB` 位图）或网页“复制图片”时，自动落盘（桌面写入本机附件目录生成 `file:///` URI，Web 使用 data/blob 或云端上传）并插入图片块。
  - 从文件管理器复制本地图片文件（`.png`/`.jpg`/`.jpeg`/`.gif`/`.webp`/`.bmp`/`.svg`）后 `Mod+V`，自动识别并插入图片块。
- **`CAP-03-09` 裸 URL / 图片 URL / 公式行智能粘贴**：
  - 选中文本后粘贴裸 `http(s)://` URL：直接为选中文字添加超链接（Notion 风格）；无选区时插入可点击链接。
  - 粘贴裸图片 URL：直接插入图片块。
  - 粘贴单行 `$$...$$` 或 `\[...\]`：直接插入块级数学公式。
- **`CAP-03-10` LLM 粘贴双围栏与误标语言修复**：粘贴来自聊天窗口的嵌套双围栏（如外层 `` ``` `` 包着 `` ```mermaid ``）自动解包；强语法特征明显与错误标签矛盾的代码块（如 Python 被标为 `bash`）自动纠正语言标签。

---

## 历史踩坑与反例

- **反例 1（`nsh-platform-windows v0.6.1` 未读 `CF_HTML`）**：底层框架在 `Ctrl+V` 时只填充了 `PasteEvent.text`、`files`、`uris`，`PasteEvent.html` 永远为空，导致从 v0.14.11、浏览器、Word 粘贴全部丢失格式。已在 [clipboard_platform.rs](file:///c:/data/codes/mica-will-laptop/clients/mica_rust/src/clipboard_platform.rs) 通过 Win32 FFI 补读 `"HTML Format"`。
- **反例 2（复制写 `plain` 丢失 Markdown）**：`Copy` 时将剥离语法的 `payload.plain` 写入纯文本通道，遇到只读纯文本通道的场景就会把全部 Markdown 结构丢光。
- **反例 3（忽略 `PasteEvent.files` / `uris`）**：只判断 `!p.text.is_empty() || !p.html.is_empty()`，导致纯图片截图 `Ctrl+V` 彻底无响应。

---

## 验收清单

| 编号 | 优先级 | 适用端 | 纯 Rust 状态 | 验证入口 |
| --- | --- | --- | --- | --- |
| `CAP-03-01` 同应用复制/剪切粘贴无损 | P0 | 全端 | ✅ | `editor_bridge::tests::markdown_and_html_paste_round_trip_headings_quotes_lists_tables_and_images` + `app_flows::regression_fifteen_editor_ux_fixes` |
| `CAP-03-02` 整页复制 Markdown | P1 | 全端 | ✅ | `editor_view.rs::page-menu-copy-md` |
| `CAP-03-03` 跨应用导出 HTML | P1 | 桌面/Web | ✅ | `mica-editor::paste_markdown_copy_cut_and_set_link` |
| `CAP-03-04` Windows `CF_HTML` 字节偏移切片 | P0 | Windows | ✅ | `clipboard_platform::tests::extract_cf_html_fragment_slices_exact_utf8_byte_offsets` |
| `CAP-03-05` v0.14.11 / 浏览器富文本粘贴 | P0 | 桌面/Web | ✅ | `app_flows::regression_fifteen_editor_ux_fixes` |
| `CAP-03-06` 表格 vs 图片剪贴板仲裁 | P1 | 桌面/Web | ✅ | `clipboard_platform::tests::clipboard_html_table_vs_image_discrimination` |
| `CAP-03-07` `Mod+Shift+V` 纯文本粘贴 | P1 | 桌面/Web | ✅ | `editor_view.rs::EditorKeyAction::PastePlain`（`mod+shift+v` 绑定 `paste_clipboard(..., true)`） |
| `CAP-03-08` 剪贴板截图与图片文件粘贴 | P0 | 桌面/Web | ✅ | `app_flows::regression_fifteen_editor_ux_fixes` + `clipboard_platform::tests` |
| `CAP-03-09` 裸 URL / 图片 URL / 公式粘贴 | P1 | 全端 | ✅ | `mica-editor::paste_heuristics_detect_urls_formulas_and_fence_repairs` |
| `CAP-03-10` LLM 双围栏与语言修复 | P2 | 全端 | ✅ | `mica-editor::paste_heuristics_detect_urls_formulas_and_fence_repairs` |

---

## 演进记录

- **2026-11-23**：完整沉淀剪贴板双通道、`CF_HTML` 字节偏移切片、v0.14.11 互操作、截图/文件图片粘贴与表格/图片仲裁规则（对应 `ff104b87`）。
- **2026-11-23（补齐迭代）**：在 `editor_view.rs` 补齐 `Mod+Shift+V` 纯文本字面粘贴快捷键绑定（`CAP-03-07`）。
