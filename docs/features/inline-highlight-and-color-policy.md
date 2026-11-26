# 行内高亮（`==高亮==`）与颜色策略调研及实现

> **决策日期**：2026-11-26  
> **决策结论**：**不引入 Notion 式的任意字体色 / 背景色调色盘**；在保留块级 5 种 GFM Callout 语义强调的基础上，只增加单一语义行内高亮（`==高亮==` ↔ `<mark>`）。

---

## 1. 背景与共同前提

Notion / 飞书文档等块编辑器在选中文本或块菜单中提供「10 种字体颜色 × 10 种背景颜色」的任意调色盘。在评估 Mica 是否应该跟进时，必须先对齐 Mica 的四条核心前提：

1. **Markdown Round-Trip 是硬不变量**：文档在 Vault 模式（本地 `.md`）、Markdown 导出/导入、Git 版本控制、AI / MCP 工具读写之间必须无损往返且源码可读。
2. **CommonMark 0.31.2 记分牌零回归**：任何方言扩展不得破坏官方 641 例（`641/641`）。
3. **浅色 / 深色双主题自适应**：文档内容不应写死绝对色值，切换主题后必须保持可读对比度。
4. **最小充分解**：避免为低频装饰性需求引入复杂的 CRDT mark 参数、迁移负担与跨平台兼容问题。

---

## 2. 同类产品横向调研

| 产品阵营 | 代表产品 | 颜色 / 高亮方案 | Markdown 互操作表现 |
| --- | --- | --- | --- |
| **私有块数据库型** | Notion、飞书文档、语雀 | 10×2 任意字体色 + 背景色调色盘（块级 + 行内），存在私有 JSON / 富文本属性里 | 导出 Markdown 时颜色全部丢失，或退化成大段 `<span style="...">` 脏 HTML |
| **Markdown 原生知识库** | Obsidian、Typora、Bear、iA Writer | **刻意不做任意字体/背景色调色盘**；只提供**单一语义行内高亮 `==text==`**（渲染为 `<mark>`），辅以块级 Callout | 源码极度干净，社区生态事实标准（Obsidian / Typora / Bear / Critiq / Pandoc / Markdown-it-mark 均原生识别） |
| **开发者平台** | GitHub / GFM | 支持 5 种块级 GFM Alerts（`NOTE`/`TIP`/`IMPORTANT`/`WARNING`/`CAUTION`）；不支持任意行内颜色，且 HTML 消毒器会剥离 `style="color:..."` | 纯标准 Markdown / GFM 兼容，语义化强 |

### 刻意不采用「任意字体/背景色调色盘」的理由

1. **破坏 Markdown 源码可读性与 Round-Trip**：CommonMark / GFM 均无行内任意颜色语法；若强行序列化为 `<span style="color: #E03E3E; background: #FBF3DB">...</span>`，会把 Vault 中的 `.md` 文件与 Git diff 污染得面目全非，也干扰 AI 模型阅读与编辑。
2. **破坏深浅主题与视觉秩序**：硬编码色值在浅色/深色模式下极易出现对比度崩坏；若改成 20 套语义色名（`red`/`pink`/`purple`…），又会产生私有语法，脱离 Mica 就变成乱码或裸 HTML。
3. **80% 的真实需求已有更优语义解**：
   - **整段提示/警示/重点框** → 已有 **5 种 GFM Callout**（`> [!NOTE]` / `TIP` / `IMPORTANT` / `WARNING` / `CAUTION`），自带标准语义配色且与 GitHub / Obsidian 互通。
   - **行内划重点** → 仅需一个**荧光笔高亮**（`==高亮==`），无需 10 种花花绿绿的文字色。

---

## 3. 最终设计：`==高亮==` 语义与边界规则

### 3.1 语法与渲染映射

| 层面 | 表示形式 |
| --- | --- |
| Markdown 源码 | `==高亮内容==` |
| CRDT / Block Schema (`Mark::kind`) | `"highlight"`（新增于 `mica_md::schema::marks::HIGHLIGHT`） |
| HTML 导出 & 公开分享 `/s/{token}` | `<mark>高亮内容</mark>` |
| 富文本剪贴板 (`CF_HTML` / `text/html`) | 写入 `<mark>...</mark>`；粘贴 `<mark>...</mark>` 还原为 `==...==` |
| 客户端主题色板（浅色 / 深色） | 浅色 `rgba(253, 224, 71, 0.42)`（暖黄荧光笔）；深色 `rgba(234, 179, 8, 0.30)`（暖琥珀透底） |

### 3.2 防误伤与边界规则

为了让代码比较表达式、数学等式、英文与中文混排都不会被误识别为高亮，解析器（L1）与输入即转换规则（L2）采用完全一致的边界约束：

1. **精确双等号**：仅当连续 `=` 长度为 **2**（`==`）时才开闭；单个 `=`（`a=b`）与 3 个及以上 `===`（`a === b`、`===分割===`）整段保持字面量。
2. **内侧非空白**：开标记 `==` 后第一个字符、闭标记 `==` 前最后一个字符都**不能是空白字符**，因此 `if a == b and c == d` 与 `== 空格 ==` 永远不会误触；同时紧贴中文标点或汉字的 `中文==重点==后面` 正常生效。
3. **代码与转义保护**：行内代码 `` `==code==` `` 内的 `==` 不参与配对；反斜杠转义 `\==literal==` 保持字面量。
4. **无损往返转义**：序列化（`escape_inline`）只在纯文本出现「精确长度 2 且后接非空白字符的 `==`」时输出 `\==`；普通 `a = b`、`a == b`、`===`、Base64 尾部 `==` 均不加多余反斜杠。
5. **嵌套组合**：支持与粗体、斜体、删除线、链接自由嵌套（如 `**==粗体高亮==**`、`==[链接](url)==`）；与行内代码/行内公式共存时保持代码/公式自身的底色识别度。

---

## 4. 三层架构实现地图

### L1 — 数据与 Markdown 引擎（`crates/md-core`、`crates/markdown`）

- [schema.rs](../../crates/md-core/src/schema.rs) & [SCHEMA.md](../../crates/md-core/SCHEMA.md)：新增 `marks::HIGHLIGHT = "highlight"` 并纳入 `marks::ALL`（受 `schema_doc_sync` 门禁校验）。
- [inline.rs](../../crates/md-core/src/parse/inline.rs)：
  - `parse_inline_with_defs`：解析精确 `==...==`（紧跟 `~~` 删除线，位于扩展自动链接之后、反引号代码跨度跳过保护之内）。
  - `escape_inline`：仅对精确双等号后接非空白字符转义为 `\==`，确保纯文本无损往返。
  - `render_span`：`"highlight" => format!("=={body}==")`。
- [html.rs](../../crates/md-core/src/html/html.rs)：`html_span` 导出 `<mark>{body}</mark>`。
- [clipboard.rs](../../crates/md-core/src/html/clipboard.rs)：剪贴板 HTML 导出 `<mark>...</mark>`。
- [paste.rs](../../crates/md-core/src/html/paste.rs)：`inline_marks` 识别 `<mark>`、`wrap_marks` 包裹为 `==...==`（不盲目猜测普通 `<span>` 的 `background-color` CSS，避免从外部网页粘贴整段带底色文本时误高亮）。
- [crates/markdown/src/lib.rs](../../crates/markdown/src/lib.rs)：`static_page_css` 增加 `mark{background:#fef08a;color:inherit;padding:.05em .2em;border-radius:3px;}`，同时覆盖独立 HTML 导出与公开分享页 `/s/{token}`。

### L2 — 无头编辑器内核（`crates/mica-editor`）

- [rules.rs](../../crates/mica-editor/src/rules.rs)：两字符行内输入规则加入 `("==", "highlight")`，键入闭合 `==` 即转为高亮；同步内置 `===` 与首尾空白防误触守卫。
- [paste.rs](../../crates/mica-editor/src/paste.rs)：单行纯文本粘贴白名单 `single_line_paste_keeps_marks` 放行 `"highlight"`。
- `ToggleMark`、`active_marks_for_selection`、Undo/Redo、跨块选区与 CRDT 同步均已按通用 mark 机制工作，无需额外分支。

### L3 — 纯 Rust 原生 UI 壳（`clients/mica_rust`）

- [mica_theme.rs](../../clients/mica_rust/src/mica_theme.rs)：`MicaPalette` 新增 `highlight_bg`（浅色暖黄 / 深色暖琥珀）。
- [block_views.rs](../../clients/mica_rust/src/block_views.rs)：`InlineRun` 新增 `highlight: bool`，`render_inline_span` 在非 code/math span 上应用 `highlight_bg`（兼容带链接与加粗/斜体/删除线组合）。
- [editor_bridge.rs](../../clients/mica_rust/src/editor_bridge.rs)：`active_marks` 上报 `"highlight"`，驱动工具栏与气泡菜单激活态。
- [mica_icons.rs](../../clients/mica_rust/src/mica_icons.rs)：新增 `highlight_icon` 荧光笔图标。
- [editor_view.rs](../../clients/mica_rust/src/editor_view.rs)：
  - 快捷键：`Ctrl + Shift + H`（macOS `⌘ + Shift + H`，与 Notion / Typora / Bear 一致）。
  - 顶栏格式工具栏：新增 `ed-highlight` 按钮。
  - 选区浮动气泡菜单：新增 `bubble-highlight` 按钮。
- [settings_view.rs](../../clients/mica_rust/src/settings_view.rs)：快捷键列表同步 `Ctrl + Shift + H`（高亮）与 `Ctrl + Shift + S`（删除线）。
- [transfer.rs](../../clients/mica_rust/src/transfer.rs)：客户端独立 HTML 导出 CSS 同步 `mark` 样式。

---

## 5. 回归验证清单

- CommonMark 0.31.2 记分牌：`cargo test -p mica-markdown --test commonmark_scoreboard`（硬门：**641/641 100.0%**）。
- L1 高亮专项测试：[crates/markdown/tests/highlight.rs](../../crates/markdown/tests/highlight.rs)（解析、往返、CJK、嵌套、`a == b`/`===`/`\==`/行内代码防误触、`<mark>` HTML 导出与粘贴）。
- L1 Schema 同步门禁：`cargo test -p mica-md --test schema_doc_sync`。
- L2 编辑器规则与端到端会话测试：[crates/mica-editor/tests/lists_quotes_tables_and_paste.rs](../../crates/mica-editor/tests/lists_quotes_tables_and_paste.rs)（输入规则、`ToggleMark`、单行粘贴、Markdown 导出）。
- L3 渲染与导出测试：`clients/mica_rust` `block_views` 单元测试与 `app_flows` E2E 测试。
