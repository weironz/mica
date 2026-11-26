# 04 — 块类型、行内格式、斜杠菜单与浮动工具栏

> **适用范围**：桌面端、Web、移动端。

## 核心不变量

1. **扁平块级模型**：块级嵌套保持扁平模型（`data.indent` / `data.quote` / `qbreak` / `data.li`），HTML 导出与视图按语义重建层级，避免深层树节点带来的选区灾难。
2. **单层卡片渲染**：引用块与 GFM Callout 必须单层渲染（唯一左竖条 + 可选淡色底洗），严禁出现“外框套内框”的双层竖线。
3. **工具栏状态实时反映选区**：顶栏格式条与浮动气泡菜单的激活高亮必须实时跟踪当前光标/选区的块类型与行内 marks。

---

## 能力与交互规格

### 块类型与行内格式
- **`CAP-04-01` 全量文本块类型**：支持正文段落、`H1`–`H6`、无序列表（按缩进层级切换 `•`/`◦`/`▪`）、有序列表（按同级连续计算可见序号）、待办清单（可点击勾选框切换 `checked`）、引用块、5 种 GFM Callout（`NOTE`/`TIP`/`IMPORTANT`/`WARNING`/`CAUTION`，各自独立主题色与图标）、水平分割线。
- **`CAP-04-02` 列表缩进与容器附着**：支持 `Tab` 缩进、`Shift+Tab` 反缩进（最多 8 级）；支持把代码块/表格/引用等容器块通过 `data.li` 附着在列表项下并保持 Markdown 往返一致。
- **`CAP-04-03` 全量行内 Marks**：支持加粗（`Mod+B`）、斜体（`Mod+I`）、删除线（`Mod+Shift+S`）、高亮（`Mod+Shift+H` / `==高亮==` ↔ `<mark>`，浅/深色主题自适应底色；刻意不引入任意 10×2 字体/背景色调色盘，详见 [行内高亮与颜色策略](../features/inline-highlight-and-color-policy.md)）、行内代码（`Mod+E`，暖灰底色而非刺眼红色）、下划线、超链接（`Mod+K`，编辑/移除浮窗，支持悬停/点击打开）、行内数学公式。

### 斜杠命令菜单（`/`）
- **`CAP-04-04` 22 项光标锚定斜杠菜单**：
  - 行首或空格后输入 `/` 弹出 `248px` 宽、`max_h 288px` 垂直菜单，锚定在当前光标附近并自动避让视口边缘。
  - 完整涵盖 22 项：问 AI、正文、H1–H3、无序/有序/待办、引用、5 种 Callout、代码块、数学公式、画板、思维导图、流程图、表格、分割线、图片。
  - 支持中英文关键字、类型名、缩写（如 `h1`/`todo`/`note`/`code`）实时过滤，支持 `↑`/`↓` 循环、`Enter` 插入（只删除 `/query` 片段，保留前面已有文字）、`Esc` 关闭且不丢正文焦点。

### 浮动选区菜单、顶栏格式条与页内查找
- **`CAP-04-05` 贴近被选内容的浮动气泡菜单**：
  - 当产生非折叠文本选区时，在**被选内容正上方**（上方不足时自动翻转到**正下方**，水平居中并夹紧在视口内）弹出浮动菜单，严禁写死固定在窗口顶部；滚动页面时自动跟随选区位置。
  - 包含：加粗、斜体、删除线、高亮、行内代码、链接、H1–H3、正文、无序/有序/待办、引用、代码块、以及启用 AI 时的「用 AI 改写所选」。
- **`CAP-04-06` 响应式顶栏格式条**：受设置「格式工具栏」开关控制；宽屏单行内嵌在页面顶栏中，窄屏自动折落为干净的 `36px` 第二行，不挤压左侧面包屑与复制路径按钮。
- **`CAP-04-07` 页内查找与高亮（`Mod+F`）**：右上角弹出查找条，实时高亮全文匹配项（当前激活项橙色、其余黄色），显示 `当前/总数`（或「无结果」），支持 `Enter` 下一个、`Shift+Enter` 上一个、`Esc` 关闭并清除高亮同时归还正文焦点。

---

## 历史踩坑与反例

- **反例 1（引用套两层）**：外层 `RichEditor` 的 blockquote 样式与自定义 quote 视图各自画一条左竖线，粘贴引用出现双竖线。切换到 `mica-editor` 自渲染后必须保证唯一单层左条。
- **反例 2（浮动菜单固定在顶部）**：将气泡菜单绝对定位写死 `.top(8px)` 居中，导致用户滚动到页面中部/底部选中文字时，菜单远远飘在屏幕最顶端，极难操作。
- **反例 3（列表嵌套代码块视觉贴左且粘贴被拍平）**：L3 视图层未消费 `data.li` 缩进偏移（应为 `26 + 24 * li` px 对齐列表正文列），且 AI 对话/富文本 HTML 中包裹在 `<div class="code-block">` 内的 `<pre>` 被 `direct_inline` 拍平成列表首行纯文本；同时在列表续行直接输入 `` ``` `` 或 `/code` 未自动继承/附着 `data.li`。
- **反例 4（正文/表格链接点击仅进入打字模式无法打开）**：`block_views.rs` 与 `table_view.rs` 仅将 `link` mark 渲染为蓝字下划线，未挂载像素级链接命中与点击跳转；必须做到：单击链接文字区域直接打开外链（`open_url`）或跳转 `mica://page/<uuid>` 内部页并支持悬停手形光标，同时点击链接右侧空白区、`Alt+单击`、双击/三击及拖选划词仍保持正常编辑与选区体验不误触跳转。

---

## 验收清单

| 编号 | 优先级 | 适用端 | 纯 Rust 状态 | 验证入口 |
| --- | --- | --- | --- | --- |
| `CAP-04-01` 全量文本块与 5 种 Callout | P0 | 全端 | ✅ | `block_views::tests::callout_kind_and_svg_viewbox_parsing` + `app_flows` |
| `CAP-04-02` 列表缩进与容器附着 | P0 | 全端 | ✅ | `mica-editor::list_indent_outdent_and_todo_toggle` + `mica-editor::list_nested_code_blocks_via_fence_slash_indent_and_html_paste` + `mica-md::list_nested_wrapped_code_blocks_roundtrip_with_li` + `block_views::tests::list_continuation_inset_matches_list_text_column` |
| `CAP-04-03` 行内 marks 与链接编辑 | P0 | 全端 | ✅ | `mica-editor::mark_apply_toggle_remove_and_normalize_merge_adjacent` + `mica-markdown::highlight` + `mica-editor::highlight_toggle_paste_and_markdown_roundtrip_work_end_to_end` |
| `CAP-04-04` 22 项斜杠菜单与过滤 | P0 | 全端 | ✅ | `editor_slash::tests` + `editor_positioning::schedule_slash_anchor_update`（`measure()` 实时锚定光标下方/上方） |
| `CAP-04-05` 贴近选区的浮动气泡菜单 | P0 | 桌面/Web | ✅ | `editor_view.rs::schedule_bubble_anchor_update`（待桌面真机复测各种滚动位置） |
| `CAP-04-06` 响应式顶栏格式条 | P1 | 桌面/Web | ✅ | `editor_view.rs` |
| `CAP-04-07` `Mod+F` 页内查找高亮 | P1 | 桌面/Web | ✅ | `mica-editor::find_replace_and_replace_all_in_one_undo_batch` |

---

## 演进记录

- **2026-11-23**：沉淀单层引用、22 项斜杠菜单、贴近选区动态定位浮动气泡栏及响应式格式条规格（对应 `3f033c30` / `ff104b87`）。
- **2026-11-23（补齐迭代）**：将 `/` 斜杠菜单锚点升级为 `editor_positioning::schedule_slash_anchor_update` 动态测量光标屏幕位置并自动上下避让（`CAP-04-04`）。
- **2026-11-24（列表嵌套代码块修复）**：补齐 L3 `data.li` 左缩进（`26 + 24 * li` px）、L1 HTML 粘贴支持 `<div class="code-block"><pre>` 卡片嵌套与语言头回退、L2 列表续行输入 `` ``` `` / `~~~` / `/code` 自动附着 `data.li` 及 `visible_ordinal` 外层续块截断子列表序号修复（`CAP-04-02`）。
- **2026-11-26（行内高亮 `==高亮==` 与颜色策略定案）**：经对标 Notion/飞书与 Obsidian/Typora/GitHub，拍板不引入任意字体/背景色调色盘，全栈三层新增单一语义行内高亮（`==高亮==` ↔ `<mark>`，`Mod+Shift+H`，工具栏/浮动气泡/HTML 导出与公开分享 CSS/剪贴板粘贴互操作；`CAP-04-03`、`CAP-04-05`）。
- **2026-11-27（正文与表格超链接点击跳转修复）**：在 `block_views.rs` 与 `table_view.rs` 接入像素级 `hit_link_in_lines` 命中判定、`Cursor::Pointer` 悬停光标、`open_url` 外链打开与 `OpenMicaPageCallback` 内部 `mica://page/<uuid>` 跳转；单击链接直接打开，拖选/双三击/`Alt+单击`/点击行尾空白区不误触跳转（`CAP-04-03`）。
