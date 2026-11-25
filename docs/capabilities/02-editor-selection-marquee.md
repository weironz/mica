# 02 — 文本选区、跨块拖选与多块框选（Marquee）

> **适用范围**：桌面端（鼠标为主）、Web（鼠标/触摸）、移动端（触摸手柄，详见 `16`）。

## 核心不变量

1. **选区绘制绝不重建文本节点**：拖动选区或改变选区起止点只能触发轻量级选区高亮层重绘，严禁重建段落文本 span 或引发虚拟列表重排。
2. **拖选路径无死区、不卡阻**：鼠标向左/右页边距偏移、向上/向下跨越表格、代码块、分割线、图片、Mermaid、公式、内嵌白板时，跨块拖选必须连续顺滑，不得中断或卡死。
3. **点击外区必释放**：选中内容后，点击正文以外空白区域、点击页面标题或按 `Esc`，必须立刻清除所有选中高亮并关闭浮动菜单。

---

## 能力与交互规格

### 文本选区（字符级）
- **`CAP-02-01` 单击 / 双击选词 / 三击选段**：
  - 单击放置折叠光标；双击选中光标所在单词/中文词段；三击选中整段（`0..len`）。
  - 支持 `Shift+Click` 扩展选区、`Shift+方向键`/`Mod+Shift+方向键` 键盘扩选、`Mod+A` 全选文档。
- **`CAP-02-02` 跨块自由上下拖选不卡元素**：
  - 拖选监听必须覆盖整行全宽容器（含左右页边距），鼠标滑出正文列左侧时自动贴齐该行行首，滑出右侧贴齐行尾。
  - 跨越非纯文本块（表格、代码块、分割线、图片、公式、Mermaid、白板）时，按方向将其纳入选区并渲染统一半透明选区底色，继续向上/向下拖动到相邻正文块时无缝续接字符级选区。
- **`CAP-02-03` 拖选边缘自动滚动**：按住左键拖选接近视口顶部或底部安全带（约 `44px`）时，以受控节流速率自动向上/向下滚动视口并同步延伸选区。
- **`CAP-02-04` 选区释放**：
  - 点击正文左/右空白页边距、顶部页面标题输入框，立即清除文本选区、块框选选区和表格多选选区，并隐藏浮动菜单。
  - 按 `Esc` 将范围选区折叠到当前焦点位置（或清空块框选/表格多选）。
  - 点击顶栏或浮动气泡菜单上的格式化按钮（如加粗、标题）时，**不得**提前清空选区。

### 多块框选（Block Marquee）
- **`CAP-02-05` 触发区域覆盖页边距与正文内空白**：
  - 从页面左/右留白页边距、左侧 `24px` 块把手槽、最后一块下方空白处按住左键拖动超过阈值（`4px`），启动蓝色半透明矩形框选。
  - 从正文列内部的空白区域（如短行文字右侧空白、空段落空白、块间空白）按住左键拖动超过 `4px`，同样平滑升级为多块框选（纯单击不拖动仍正常放置行尾光标）。
  - 右侧滚动条带（`16px`）与左右页宽调节抓手（`12px`）优先归滚动条与页宽拖拽，不误触发框选。
- **`CAP-02-06` 连续块范围命中与高亮**：框选矩形与任意顶层块相交时，取最小到最大命中的连续顶层块区间 `[lo, hi]`，为区间内每个块绘制 `6px` 圆角整块选区底色。
- **`CAP-02-07` 多块批量键盘操作**：框选结束后保持高亮，支持 `Mod+C`（复制规范 Markdown + HTML）、`Mod+X`（剪切并删除）、`Delete`/`Backspace`（一次性删除所选块并保持文档非空）、`Esc`（取消框选）。

---

## 历史踩坑与反例

- **反例 1（向上拖选卡在表格/分割线/页边距）**：仅在正文 `text_box` 内绑定 `on_mouse_move`，鼠标移到表格、分割线或左右留白时事件链断裂导致拖选卡死。必须将跨块 `on_mouse_move` 提升到整行全宽外层容器及 header/footer。
- **反例 2（正文内框选失效）**：`starts_marquee_in_margin` 仅允许页边距且把 `content_bottom` 误传为整个视口高度 `vh`，导致底部和正文内空白处全部无法起框选。
- **反例 3（点击空白处选中不释放）**：页边距 `on_press_down` 只清了 marquee 没调 `bridge.clear_selection()`，导致蓝色文本高亮和顶部菜单一直挂在屏幕上。
- **反例 4（`nsh-core` 中 `PressDown` 不向父级冒泡导致正文内无法起框选）**：虽然子元素（短行右侧留白、空行、底部空白区）设置了 `body_blank_marquee_ready = true`，但 `PressDown` 不会冒泡到外层 `mica-rich-editor`，导致外层的 `marquee_origin` 始终为 `None`、行矩形也未快照，只能在正文外边缘起框选。根治方案：子元素及行外层容器统一写入 `marquee_origin_page = Some(e.page_position)`，外层通过 `effect` 自动快照可见块矩形，并在 `on_mouse_move` 中用 `e.position - (e.page_position - origin_page)` 零误差还原局部起点。
- **反例 5（Web 端跨块拖选选不全且被代码块挡住中断）**：① 内层 `text_box`/`code_text_box` 算完精确字符命中后，同帧冒泡到外层 `row.on_mouse_move` 又用错位的 `text_frame.origin` 覆盖选区；② 拖选进入代码块时 `on_mouse_enter` 动态弹出右上角 `code-block-toolbar` 截断 Web `elementFromPoint` 命中；③ Web 端无 `setPointerCapture`，移出 `root` 后 `pointermove`/`mouseUp` 断流。根治方案：内层命中打 `inner_drag_hit` 帧内去重戳、外层 `row_text_origin_offset` 精确计入缩进与代码块内边距并贴齐右边界行尾、拖选期间抑制代码块悬浮工具栏、并在 `mica-rich-editor` 通过被动 `on_touch` 接收 `window` 级鼠标移出与松手事件持续驱动自动滚屏与释放。

---

## 验收清单

| 编号 | 优先级 | 适用端 | 纯 Rust 状态 | 验证入口 |
| --- | --- | --- | --- | --- |
| `CAP-02-01` 单/双/三击与键盘扩选 | P0 | 桌面/Web | ✅ | `mica-editor::word_range_selects_latin_identifiers_and_cjk_segments` |
| `CAP-02-02` 跨块向上/下拖选不卡元素 | P0 | 桌面/Web | ✅ | `app_flows::regression_typing_backtick_caret_focus_and_code_block_backspace` |
| `CAP-02-03` 拖选边缘自动滚屏 | P1 | 桌面/Web | ✅ | `editor_positioning::tests::edge_autoscroll_speed_ramps_smoothly_near_and_past_edges` + `editor_view.rs` 60fps `set_interval` 连续像素滚屏与选区同步推进 |
| `CAP-02-04` 点击正文外/标题/Esc 释放选区 | P0 | 桌面/Web | ✅ | `editor_view.rs` 页边距/标题 `on_press_down`/`on_focus` |
| `CAP-02-05` 页边距与正文内空白启动框选 | P0 | 桌面/Web | ✅ | `block_marquee::tests::starts_marquee_margin_grip_gutter_scrollbar_rules` + `editor_view.rs` |
| `CAP-02-06` 连续块范围命中与高亮 | P1 | 桌面/Web | ✅ | `block_marquee::tests::hit_block_range_returns_contiguous_bounds` |
| `CAP-02-07` 多块批量复制/剪切/删除 | P0 | 桌面/Web | ✅ | `editor_view.rs` marquee shortcuts + `editor_bridge.rs::copy_block_range_payload` |

---

## 演进记录

- **2026-11-23**：沉淀跨块全宽拖选、正文内空白区左键框选升级、点击外区即时释放选区等契约（对应 `72d84877` / `ff104b87`）。
- **2026-11-23（根因修复）**：修复 `nsh-core` 中 `PressDown` 不向父级冒泡导致正文范围内空白区、行间隙、列表前缀槽与底部区域均无法启动框选的根因，改为通过 `marquee_origin_page` 统一透传起点并自动快照块矩形。
