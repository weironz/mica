# 05 — 表格编辑、列宽自适应与多单元格选择

> **适用范围**：桌面端、Web、移动端。

## 核心不变量

1. **单元格使用与段落同源的 `text + marks` 模型**：表格在块模型中为单一 `table` 块，每个单元格在内存中保存纯文本与 UTF-16 行内 marks，通过 `mica_md::decode_cell` / `encode_cell` 与 GFM Markdown 双向无损编解码。
2. **列宽比例驱动上下行严格对齐**：同一列在表头与所有数据行中的计算宽度必须完全一致，绝不出现竖线错位。
3. **表格嵌套焦点与正文隔离**：编辑单元格或选中多格区域时，正文隐藏 IME 代理失焦，键盘按键归表格独占。

---

## 能力与交互规格

### 列宽与布局
- **`CAP-05-01` 内容自适应列宽**：
  - 未手动拖拽列宽时，根据整张表每列内容的视觉宽度（区分 CJK 宽字符与 ASCII 窄字符，表头略加权）自动分配各列占比：内容少的短列（如「项」「编号」「优先级」）保持紧凑，内容多的长列自动占据更多宽度（例如 2 列键值表自动约 18%/82%），严禁无脑均分导致大面积空白。
  - 若用户在 `model.widths` 中保存了自定义列宽比例，优先尊重用户调整值。
- **`CAP-05-02` 默认贴合页宽与拖拽超宽横向滚动**：默认创建的任意列数表格（`table_width == 1.0`）一律自适应当前页面内容宽度、不默认超出页面宽度且隐藏横向滚动条；当用户向右拖拽最右侧边缘或把中间列拖超相邻列最小宽而超出页面宽度（`table_width > 1.0`，最高 `4.0` 倍页宽）时，实时开启独立横向滚动并在表格下方展示比例自适应横向滚动条，向左拖回页宽内后自动隐藏。
- **`CAP-05-03` 单元格所见即所得无跳变编辑**：展示态与点击聚焦编辑态共享同一套自动换行富文本渲染（含加粗、斜体、行内代码、删除线、链接、表头加粗与灰底色），仅叠加闪烁光标、浅蓝选区高亮与透明光标锚定 IME 代理；点击多行或表头单元格绝不塌陷为单行、绝不截断、绝不出现白底框或丢粗体，编辑文本时自动偏移保留已有行内 marks。

### 单元格编辑与多格选择
- **`CAP-05-04` 自由上下左右拖动选择多个单元格**：
  - 在任意单元格按下鼠标左键并向上、下、左、右任意方向拖动跨越到其他单元格时，自动退出单格输入态并实时高亮矩形选区 `[r0..=r1, c0..=c1]`（覆盖 `selection_bg` 背景）。
  - 当继续向上拖出表格顶部或向下拖出表格底部进入相邻块（含标题页头与页尾）时，自动清空表内多格态并无缝升级为跨块文档选区（锚定在表格块并继续向周围标题/正文/代码块扩选），绝不卡死在表格边界。
  - 单纯点击（不跨格拖动）则进入该单元格的实时编辑态；支持从已聚焦的单元格直接拖出跨格多选。
- **`CAP-05-05` 多单元格批量操作**：选中矩形多格区域后，支持 `Mod+C`（通过 `TableModel::copy_area` 输出 TSV + HTML + Markdown，可直接粘贴进 Excel/飞书或 Mica 其他表格）、`Mod+X`（复制并清空选中格）、`Delete`/`Backspace`（批量清空选中格内容）、`Esc`（取消多格选区）。
- **`CAP-05-06` 单元格键盘导航与实时编辑**：
  - 编辑单个单元格时输入实时同步到 CRDT，不触发整表卸载重建丢焦。
  - 支持 `Enter` / `Tab` / `Shift+Tab` 在单元格间顺序切换，末行末格按 `Tab` 自动追加新行并聚焦首列；到达边界可顺滑退出到上下相邻正文块。
- **`CAP-05-07` 表格结构操作与全高列宽拖拽**：
  - 表格左上角提供整表锚点菜单（整表左/中/右对齐、自适应列宽、复制表格、删除表格）；每列顶部与每行左侧紧贴外沿提供行/列锚点胶囊，点击整行/整列高亮并弹出顶层 `overlay()` 菜单（上/下/左/右移动、前/后插入、列对齐、清空、删除）。
  - 表格右侧与底部紧贴网格外沿提供 `+` 扩展列/扩展行横竖条，不遮挡表头文字。
  - 所有列分隔线及表格最右外边界在**整张表全高（覆盖全部数据行而非仅首行）**均可悬停高亮并左右拖拽调整列宽/表宽：**拖动过程中单元格宽度、文字折行、表头列胶囊与横向滚动条 60fps 实时跟随变化**，松手时通过单次原子 `TableOp::SetLayout` 持久化（支持单步撤销）。

---

## 历史踩坑与反例

- **反例 1（所有列 `.flex_1()` 均分）**：2 列表格第一列只有 2–4 个字（「项」「版本」「日期」），却硬占 50% 宽度，留下大片难看空白，同时右侧长内容被迫折成四五层。
- **反例 2（单元格彼此孤立无法框选）**：每个单元格仅放一个独立 `button()` + `input()`，拖动鼠标被困在单个单元格内，无法向上下左右框选多个单元格。
- **反例 3（跨格拖动与跨块拖动冲突 / 卡死表格边界）**：外层 `virtual_list` 行上的跨块文本拖选若直接忽略 `table_drag_anchor`，会劫持表内跨格拖拽；反之若一刀切 `return`，则从表格中间向上拖到顶部或向下拖到底部时会卡死在表格边界无法继续选到周围正文。正确判定为：指针仍在同一表格块内时放行表内多格选择；一旦指针越出表格进入不同块（或页头/页尾），立即将 `table_drag_anchor` 升级为锚定在表格块的 `drag_anchor` 跨块选区。

- **反例 4（点击单元格切换成单行白底 `input()`）**：聚焦单元格时把换行富文本替换成单行不透明输入框，导致多行文字瞬间塌陷截断、表头出现白底且丢失粗体与行内格式。必须保持单一富文本展示层，仅叠加光标/选区与透明 IME 代理。
- **反例 5（列宽拖拽条仅挂在首行表头）**：把 `ColResize` 句柄只放在 `r == 0` 的单元格内，用户在第 2..N 行悬停竖线时既无箭头提示也不能拖动调宽。列分隔线拖拽层必须覆盖表格网格全高（`top_0..bottom_0`）。
- **反例 6（单元格点击与格内拖选始终跳到尾部）**：① `cell_box.on_press_down`/`on_mouse_move` 把窗口级 `e.page_position` 传入 `hit_offset` 后却减去 `text_box.on_layout` 的父级局部坐标 `frame.origin`，导致算出的 `lx/ly` 远超格宽并恒落到 `max_off`；② 鼠标抬起时触发的 `on_press` 又无条件将 `desired_sel`/`visual_sel` 覆写回 `(end, end)` 冲掉格内选区；③ 缺少 `TextLayout` 折行兜底与聚焦时 `set_selection` 同步。根治方案：统一用 `cell_box` 局部坐标 `e.position` 命中、接入 `synthesize_wrapped_lines` 兜底、支持双击选词与三击全选、指针按下后拦截 `on_press` 尾部覆写，并在透明 IME 代理 `on_focus` 时同步 `desired_sel`。

---

## 验收清单

| 编号 | 优先级 | 适用端 | 纯 Rust 状态 | 验证入口 |
| --- | --- | --- | --- | --- |
| `CAP-05-01` 内容自适应列宽 | P0 | 全端 | ✅ | `table_view::tests::adaptive_table_column_widths_give_long_content_more_space` |
| `CAP-05-02` 默认贴合页宽与拖拽超宽横向滚动 | P0 | 全端 | ✅ | `table_view::tests::wide_five_column_table_fits_page_width_by_default_and_overflows_only_after_drag` + `app_flows::regression_table_default_fits_page_live_column_drag_and_right_edge_resize` |
| `CAP-05-03` 单元格所见即所得无跳变编辑与 marks 保留 | P0 | 全端 | ✅ | `table_view::tests::splice_cell_marks_preserves_and_shifts_bold_marks_across_edits` + `app_flows::regression_table_wysiwyg_chrome_handles_expand_bars_and_hscroll` |
| `CAP-05-04` 自由上下左右拖选多单元格 | P0 | 桌面/Web | ✅ | `table_view.rs::render_table_cell` |
| `CAP-05-05` 多格复制/剪切/清空 | P0 | 桌面/Web | ✅ | `mica-editor::table_model_crud_alignment_navigation_and_area_copy` + `editor_view.rs` |
| `CAP-05-06` 单元格编辑与 Tab/Enter 导航 | P0 | 全端 | ✅ | `mica-editor::table_ops_through_editor_session_and_undo` + `app_flows::regression_capability_gaps_closed` |
| `CAP-05-07` 全高实时列宽/右缘拖拽与左上锚点/右下扩展条 | P1 | 全端 | ✅ | `table_view::tests::compute_live_column_drag_handles_inner_dividers_overflow_and_right_edge` + `mica-editor::table_layout_supports_overflow_width_and_atomic_undo` + `app_flows::regression_table_default_fits_page_live_column_drag_and_right_edge_resize` |

---

## 演进记录

- **2026-11-23**：沉淀内容自适应列宽、自由上下左右多单元格拖选与批量复制/清空规格（对应 `ff104b87`）。
- **2026-11-23（补齐迭代）**：补齐单元格键盘导航（`Tab`/`Shift+Tab`/`Up`/`Down`/`Escape` 走 `TableModel::navigate`）、悬停条 `− 行`/`− 列`/`对齐` 切换与表头列分隔线拖拽调宽（`CAP-05-06` / `07`）。
- **2026-11-24（全面对齐）**：独立拆出 `clients/mica_rust/src/table_view.rs`，统一所见即所得无跳变单元格编辑与行内 marks 保留、整表全高列宽/表宽拖拽、自适应横向滚动条，以及对齐旧版 Mica 的左上整表/行/列锚点菜单与右/底 `+` 扩展条。
- **2026-11-27（单元格中间定位光标与格内拖选修复）**：修复 `table_view.rs` 命中坐标混用 `page_position` 与父级局部 `frame.origin` 及 `on_press` 松手将选区强行拉回尾部的问题，补齐双击选词、三击全选、`TextLayout` 折行兜底与 `on_focus` 选区同步，并新增无头回归测试 `regression_table_cell_mid_click_places_caret_in_middle_and_drag_selects_substring`。
- **2026-11-27（默认贴合页宽、实时列宽跟随与最右边缘拖拽）**：① 移除 4+/5+ 列表格默认撑超页面宽度的强行扩展，默认一律按 `avail` 贴合页面宽度；② 扩展 `TableModel::table_width` 支持 `0.15..=4.0` 倍率并新增单步原子 `TableOp::SetLayout`，根治旧版像素值与 `clamp(0.15, 1.0)` 错配导致最右边缘拖拽完全无效的缺陷；③ 将 `resize_layer` 移至 `middle_row` 并预留 4px 右缘命中区，结合 `live_col_pxs` 响应式信号实现拖动中 60fps 实时重排内容与横向滚动条显隐，补齐单元及 `drag_and_hold` 端到端回归测试。
