# 01 — 光标、聚焦、中文 IME 输入与基础打字

> **适用范围**：桌面端（Windows / macOS / Linux）、Web、移动端。  
> **技术栈无关契约**：无论底层使用自绘文本画布、系统原生文本代理还是 DOM `contenteditable`，均须满足本规格。

## 核心不变量

1. **坐标系统统一为 UTF-16 偏移**：光标位置 `(block_index, utf16_offset)` 与行内 marks 严格对齐，绝不在 Emoji 代理对或组合字素簇中间劈开光标。
2. **焦点归属单一权威**：任意时刻，页面标题、正文 IME 代理、代码块编辑器、表格单元格、查找栏、弹窗输入框至多只有一个持有键盘焦点；隐藏光标绝不与内嵌编辑器双光标并存。
3. **打字期间绝不重置平台输入缓冲**：严禁在每个字符提交后同步清空平台 IME 代理控件的值，防止 `WM_CHAR`/`EN_CHANGE` 或 IME 提交竞态造成字符双写或吞符号。

---

## 能力与交互规格

### 光标显示与定位
- **`CAP-01-01` 光标闪烁与失焦隐藏**：正文聚焦且选区折叠时，光标立即常亮呈现，随后以约 `530ms` 周期平滑闪烁；光标移动或打字时重置为常亮（不出现“正在打字却隐身”）；焦点离开正文（点击标题、代码块、表格、侧边栏、页边距或切窗口）时立即隐藏光标。
- **`CAP-01-02` 精确点击命中与字形校准**：点击文字任意位置时，光标落到最近的字符边界（按平台真实行宽比例校准），CJK、英文混排、等宽行内代码、Emoji 代理对均无明显偏移；点击行末右侧空白落在行尾，点击行首左侧落在偏移 0。
- **`CAP-01-03` 连续上下方向键保持视觉列**：在多行折行段落内按 `↑`/`↓` 时，维持期望视觉 `x` 坐标（`preferredX`），穿过短行到达下一长行时尽量回到原列；左右移动或点击后清除 `preferredX`。

### 中文 IME 与连续打字
- **`CAP-01-04` 连续打字零重复、零吞字符**：快速输入普通字母（如 `test`）、标点与特殊符号（特别是反引号 `` ` ``、单双引号、中文标点）时，字符逐一准确落盘，绝不出现首字双写（如 `ttest`）或反引号无响应。
- **`CAP-01-05` 中文 / CJK 组合预编辑（Composition）**：拼写中文（如微软拼音、搜狗、macOS 拼音）时，实时在光标处渲染带下划线的预编辑文本（preedit）；组合期间 `Enter`/`Backspace`/`Esc` 归输入法候选窗消费，不误删正文、不误拆分段落；上屏后预编辑消失、正文一次性落盘。
- **`CAP-01-06` 跨块选中后直接中文输入**：选中跨多块内容后直接用中文 IME 拼写上屏，能完整替换跨块选区并保留新输入内容，不双写预编辑文本（对齐原版 `ProseInputSession` 语义）。

### 块级键盘结构规则
- **`CAP-01-07` `Enter` / `Shift+Enter` 行为**：
  - 普通段落/标题内按 `Enter`：在光标处拆分块；光标在非空块偏移 `0` 按 `Enter` 时，在上方插入空段落并保留下方原块格式。
  - 列表/待办/引用块非空行按 `Enter`：延续同类型与同缩进；空列表/空待办/空引用按 `Enter`：退出当前格式降级为普通段落。
  - `Shift+Enter`：块内插入硬换行（按 Mica 约定以 `\` + `\n` 存储）。
- **`CAP-01-08` `Backspace` 阶梯退化规则**：
  - 嵌套列表偏移 `0` 按 `Backspace`：先减一层缩进（`outdent`）。
  - 顶层列表/待办/引用/空标题偏移 `0` 按 `Backspace`：先剥离块样式转为普通段落，不直接与上一段合并。
  - 普通段落偏移 `0` 按 `Backspace`：与上一段落合并（若上一块是原子块如图片/分割线，空行先删空行并选中原子块，防止一键误删图片）。
- **`CAP-01-09` 页面标题与首行双向切换（回车下移 / 回退聚焦标题末尾）**：
  - 在页面标题按 `Enter`：若正文首行已有内容（或为非普通段落块），在索引 `0` 插入一个新空普通段落并将原首行下移至索引 `1`，光标落在新空行 `(0, 0)`；若文档本身为空白页（仅 1 个空段落），直接聚焦 `(0, 0)`。
  - 光标折叠在正文第 0 块普通段落偏移 `0` 按 `Backspace`（Windows 空代理下含 `Delete`）：若第 0 块为空且下方有块，先清理该首行空行让正文上移，随后将焦点交给页面标题并将光标定位到**标题末尾**（`set_selection(end, end)`）；若第 0 块非空或为唯一块，直接跳至标题末尾。
- **`CAP-01-10` 点击正文底部空白自动新建下一行**：点击正文最后一块下方的空白区域时，若末块已是空段落则直接聚焦它；若末块非空或为非文本块（表格、代码块、图片、分割线、公式等），自动在文档末尾追加一个新空段落并聚焦。
- **`CAP-01-11` 空白文档无占位提示文案**：新建空白页面直接呈现干净编辑画布，不显示「输入正文，或输入 / 唤出命令...」类冗余占位文字。

### Markdown 即时输入规则（Input Rules）
- **`CAP-01-12` 行首块级输入规则**：行首输入 `# `..`###### ` 转 H1–H6；`- ` / `* ` / `+ ` 转无序列表；`1. ` 转有序列表；`[] ` / `[ ] ` / `[x] ` 转待办；`> ` 转引用；`---` / `***` 转分割线；`` ``` ``（含可选语言名）自动转换为代码块并将焦点直接移入代码块编辑器；`$$` 转块级公式。
- **`CAP-01-13` 行内配对输入规则**：输入 `**text**`、`*text*`、`~~text~~`、`` `code` `` 闭合触发后即时转为对应行内 mark，不残留原始语法符号。

---

## 历史踩坑与反例（切勿重犯）

- **反例 1（`ttest` 与吞反引号）**：在 `selection_sig` 的响应式副作用中每次输入都清空隐藏 `input()` 的 `value`，与 Windows `WM_CHAR`/`EN_CHANGE` 竞争，导致连续键入产生 `ttest` 并且 `` ` `` 被吞掉。应允许缓冲自然累积至 `SINK_LIMIT`（如 64B），仅在点击跳光标、结构按键、粘贴或失焦时清空。
- **反例 2（切页残留焦点锁死正文快捷键）**：从含聚焦代码块的页面切换到另一页面后，未清理旧 `block_id` 的 `focused_code_block`，导致新页面上 `Ctrl+A` 等全部正文快捷键失效。结构变更时必须按当前存活 `block_id` 裁剪焦点状态。
- **反例 3（光标滞后一帧）**：输入字符后先算光标位置，下一帧平台 `TextLayout` 回调才更新行宽，导致光标停在 `ttes|t`。必须在 `TextLayout` 回调行几何变化时主动通知光标层重绘。

---

## 验收清单

| 编号 | 优先级 | 适用端 | 纯 Rust 状态 | 验证入口 |
| --- | --- | --- | --- | --- |
| `CAP-01-01` 光标闪烁与失焦隐藏 | P0 | 桌面/Web | ✅ | `app_flows::regression_typing_backtick_caret_focus_and_code_block_backspace` |
| `CAP-01-02` 点击命中与字形校准 | P0 | 全端 | ✅ | `text_geometry::tests::caret_selection_and_hit_test_work_for_cjk_ascii_and_emoji_surrogates` |
| `CAP-01-03` 连续上下方向键保持列 | P1 | 桌面/Web | ✅ | `editor_positioning::move_caret_vertical_preserving_x`（跨行+跨块保持 `preferred_x`）+ `text_geometry::tests::multi_line_vertical_navigation_and_selection_span_lines` |
| `CAP-01-04` 连续打字与反引号不丢失 | P0 | 桌面/Web | ✅ | `app_flows::regression_typing_backtick_caret_focus_and_code_block_backspace` |
| `CAP-01-05` CJK IME 预编辑与上屏 | P0 | 全端 | ✅ | `editor_positioning::schedule_ime_sink_pos_update`（实时跟随光标定位原生 IME 代理）+ `mica-editor::ime_update_handles_cjk_composition_commit_and_input_rules` |
| `CAP-01-06` 跨块选区中文 IME 替换 | P1 | 桌面/Web | 🟡 | L2 内核支持，L3 实机 IME 专项验证待补 |
| `CAP-01-07` Enter / Shift+Enter 规则 | P0 | 全端 | ✅ | `mica-editor::insert_text_split_and_backspace_merge_with_undo_redo` |
| `CAP-01-08` Backspace 阶梯退化 | P0 | 全端 | ✅ | `mica-editor::list_indent_outdent_and_todo_toggle` |
| `CAP-01-09` 第一行 Backspace 回退标题 | P1 | 桌面/Web | ✅ | `editor_view.rs::try_backspace_to_title`（待真机复测光标落标题末尾） |
| `CAP-01-10` 底部点击自动新建空行 | P0 | 全端 | ✅ | `app_flows::regression_fifteen_editor_ux_fixes` |
| `CAP-01-11` 无冗余空页占位提示 | P1 | 全端 | ✅ | `app_flows::regression_fifteen_editor_ux_fixes` |
| `CAP-01-12` / `13` Markdown 输入规则 | P0 | 全端 | ✅ | `mica-editor::input_rules_match_headings_lists_quotes_code_and_inline_marks` |

---

## 演进记录

- **2026-11-23**：基于 `main:clients/mica_flutter/lib/editor/` 与 `clients/mica_rust`（`ff104b87`）初版沉淀，纳入首行回退标题、底部新建行、移除占位提示、修复 `ttest`/吞反引号及光标闪烁规则。
- **2026-11-23（补齐迭代）**：新增 `editor_positioning.rs`，补齐跨块 `preferred_x` 垂直方向键保持列（`CAP-01-03`）与隐藏原生 IME 代理实时跟随光标定位（`CAP-01-05`）。
