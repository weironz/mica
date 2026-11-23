# 06 — 代码块与语法高亮

> **适用范围**：桌面端、Web、移动端。

## 核心不变量

1. **代码块编辑态与正文焦点互斥**：当代码块处于编辑态时，正文隐藏 IME 代理必须失焦，`Backspace`、`Delete`、`Tab`、`Enter`、方向键、`Mod+A`/`C`/`V` 全部归代码编辑器独占（除非触发明确的块级边界规则，如 `(0,0)` 按 Backspace 删除代码块）。
2. **代码编辑控件采用非受控模式（Uncontrolled）**：编辑态内绝不可用受控 `.value(...)` 在每次 `on_change` 同步写回重建编辑器内部 rope/光标，否则会导致 `Backspace`/`Delete` 失效或光标跳回开头。
3. **顶层 `overlay()` 浮层永不被下方块遮挡**：语言选择器、AI 代码助手菜单、`⋯` 更多菜单必须渲染在应用最顶层浮层，绝不可被下方的内嵌画板、表格、Mermaid 或其他兄弟块裁切遮盖。

---

## 能力与交互规格

### 视觉与懒挂载性能
- **`CAP-06-01` 原版无框暖灰卡片**：静息态无外框、无常驻厚重顶栏；背景使用暖灰 `#F7F6F3`（暗色 `#202020`），圆角 `6px`，内边距 `px=14 py=12`，等宽字体 `14px/22px`，默认关闭自动换行（`wrap=false`）和行号（`lineNumbers=false`）。
- **`CAP-06-02` 紧凑高度零底部死行**：单行代码块仅占 1 行高度（`min_rows(1)`，`scroll_beyond_last_line = None`），绝不出现多余的空白底部死行。
- **`CAP-06-03` 大文档懒挂载**：非聚焦代码块以轻量等宽文本预览渲染，保障 141KB+ 含数十个代码块的文档毫秒级滚动；用户点击代码块或通过 `` ``` `` 输入规则创建时，即时挂载完整代码编辑器并自动聚焦。

### 工具条与弹层菜单
- **`CAP-06-04` 悬停浮现右上角工具条**：仅在鼠标悬停、正在编辑、弹层打开或复制确认期间（`1600ms` 绿勾反馈）在右上角浮现工具条：`[语言胶囊 "rust ▾"]`、`[AI 助手]`、`[复制]`、`[⋯ 更多]`。
- **`CAP-06-05` 可搜索语言选择器（顶层浮层）**：点击语言胶囊弹出可搜索语言列表（`auto` 置顶 + 涵盖全部常用语言与 `mermaid`/`mica-board` 切换），通过顶层 `overlay()` 锚定触发按钮，上方有画板/表格也绝不被遮挡。
- **`CAP-06-06` 更多菜单与持久化属性**：支持切换「自动换行 / 取消换行」、「显示行号 / 隐藏行号」、「添加/编辑底部居中标题」（显示在代码块下方外部）及红色「删除代码块」；`language`、`wrap`、`lineNumbers`、`title` 均持久化在块 `data` 并通过 CRDT 同步。
- **`CAP-06-07` AI 代码助手菜单**：提供「解释代码」「优化代码」「查找问题」「自定义提问」4 项预设，未配置 AI 服务商时给出清晰轻提示。

### 键盘编辑与边界退出
- **`CAP-06-08` 完整代码编辑能力**：代码块内支持自由输入、多行编辑、正常 `Backspace`/`Delete` 删字、`Tab`/`Shift+Tab` 缩进、自动缩进与括号闭合、选区复制粘贴。
- **`CAP-06-09` 空代码块或光标在最左侧 `(0, 0)` 按 `Backspace` / `Delete` 删除代码块**：当光标折叠位于代码块第 0 行第 0 列（最左侧）时按 `Backspace`，或代码块内容为空时按 `Backspace` / `Delete`，直接删除该代码块并将焦点平滑交回相邻正文块（支持 `Mod+Z` 一键撤销恢复）；当代码块非空时，`Delete` 保持代码内向前删字语义。
- **`CAP-06-10` 语法高亮单一真相源**：语言识别与高亮配色遵循主题 Token（浅/深色自动切换），粘贴误标语言的修复仅作用于粘贴不覆盖用户手动选定的语言。

---

## 历史踩坑与反例

- **反例 1（受控 `.value(...)` 导致 Backspace 失效）**：`nsh_base::editor()` 传入了受控 `.value(live_src.get())`，按 Backspace 时外部旧值覆盖内部编辑状态导致删除键完全无效。改用 `.default_value(...).auto_focus(true)` + `update_block_source_live` 彻底根治。
- **反例 2（画板遮挡上方代码块的语言下拉菜单）**：下拉菜单作为代码块内部 `absolute` 子元素渲染，遇到下方紧邻的 `canvas` 内嵌画板（后绘制层叠序更高）时被画板拦腰截断。必须改用顶层 `overlay().anchor(...)`。
- **反例 3（代码块内按 Backspace 误删正文）**：正文全局 `backspace` 绑定未在 `focused_code_block.is_some()` 时注销，导致代码块内按键被外层劫持。
- **反例 4（`nsh_base::editor()` 获焦后 LIFO 键位表覆盖导致空代码块 Backspace/Delete 无效）**：`nsh-core::Keymap::find_with` 采用逆序查找（后注册者优先）。`editor_view.rs` 在设置 `focused_code_block` 时先注册了 `backspace`，一帧后子组件 `nsh_base::editor()` 收到 OS Focus 触发其内部 `effect` 在队尾追加了自身的 `backspace`/`delete`，彻底遮蔽了外层删块绑定，且空代码块未绑定 `delete`。根治方案：在 `editor().on_focus`（同步 + 微任务 `spawn`）、`on_change` 与 `on_selection_change` 中递增 `code_key_rev` 重新排到键位表队尾，`on_blur` 清空避免干扰标题/语言输入，并对空代码块同时绑定 `backspace` 与 `delete`（`app_flows::regression_code_block_backspace_and_delete_after_live_focus` 覆盖全路径）。

---

## 验收清单

| 编号 | 优先级 | 适用端 | 纯 Rust 状态 | 验证入口 |
| --- | --- | --- | --- | --- |
| `CAP-06-01` 无框暖灰卡片与默认配置 | P1 | 全端 | ✅ | `block_views.rs::render_code_block_view` |
| `CAP-06-02` `min_rows(1)` 零底部死行 | P1 | 全端 | ✅ | `block_views.rs::render_code_block_view` |
| `CAP-06-03` 懒挂载编辑态与 `` ``` `` 自动聚焦 | P0 | 全端 | ✅ | `app_flows::regression_typing_backtick_caret_focus_and_code_block_backspace` |
| `CAP-06-04` / `05` 悬停工具条与顶层防遮挡语言选择 | P0 | 桌面/Web | ✅ | `app_flows::virtualized_editor_code_overlay_mermaid_lightbox_and_page_switch` |
| `CAP-06-06` / `07` 换行/行号/标题/AI/删除 | P1 | 桌面/Web | ✅ | `app_flows::regression_capability_gaps_closed`（换行/行号/AI/删除 + `code-menu-title` 底部内联标题编辑并持久化 `code-block-caption`） |
| `CAP-06-08` 代码块内输入与 Backspace/Delete | P0 | 全端 | ✅ | `app_flows::regression_typing_backtick_caret_focus_and_code_block_backspace` |
| `CAP-06-09` 光标在 `(0,0)` 或空块按 Backspace/Delete 删除代码块 | P0 | 桌面/Web | ✅ | `app_flows::regression_code_block_backspace_and_delete_after_live_focus` + `app_flows::regression_fifteen_editor_ux_fixes` |
| `CAP-06-10` 语法高亮主题适配 | P1 | 全端 | ✅ | `block_views::render_highlighted_code_preview`（L1 `mica_md::highlight::highlight_code` 6 色主题 token 渲染，>12KB 自动降级保 60fps）+ `app_flows::regression_capability_gaps_closed` |

---

## 演进记录

- **2026-11-23**：沉淀顶层 `overlay()` 防遮挡、非受控编辑器修复 Backspace、懒挂载与 `(0,0)` 按 Backspace 删除代码块规则（对应 `3f033c30` / `72d84877` / `ff104b87`）。
- **2026-11-23（补齐迭代）**：补齐非聚焦预览态基于 L1 `md-core` 的 6 色语法高亮与底部居中标题内联编辑（`CAP-06-06` / `10`）。
- **2026-11-23（根因修复）**：修复 `nsh_base::editor()` 获焦后 LIFO 键位表覆盖外层 `backspace`/`delete` 导致空代码块无法按后退/删除键删除的根因，新增 `code_is_empty` + `code_key_rev`（`on_focus` 同步+微任务重排、`on_blur` 释放）并补齐 `Delete` 删空块及 4 场景真实焦点回归测试。
