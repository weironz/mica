# 08 — 长文档虚拟滚动、滚动条、页面宽度与视觉节奏

> **适用范围**：桌面端、Web、移动端。

## 核心不变量

1. **视口虚拟化是刚性架构红线**：文档画布只挂载与排版视口附近（及少量预渲染缓冲区）的顶层块，面对 141KB+ / 2,000+ 块超长文档，切页、滚动、选区、打字均须保持毫秒级响应，不得加载出大面积空白。
2. **信号隔离铁律**：虚拟列表的 `count` 与 `keys` 严禁订阅正文打字或选区信号，只订阅结构变更信号 `structure_rev`；普通打字只触发当前单块的细粒度 `block_rev(id)`，兄弟块零重建。
3. **编辑期绝不无故跳滚**：在视口内可见行按 `Enter`、`Backspace`、打字或上下移动光标时，页面滚动位置必须纹丝不动；只有目标行真正移出视口上下边界时才做最小幅度滚动。

---

## 能力与交互规格

### 虚拟滚动与滚动策略
- **`CAP-08-01` 超长文档瞬时切页与流畅滚动**：打开或粘贴 100KB–200KB 级长文档（含大量表格、代码块、标题），切页响应平滑无明显冻结；快速下拉滚动时及时呈现内容，不出现持久空白区域。
- **`CAP-08-02` 区分 `Jump` 与 `KeepVisible` 两种滚动语义**：
  - `Jump`（切换页面回顶、点击右侧大纲跳转、页内查找命中）：主动将目标块定位到视口清晰位置（第 0 块滚至 `0.0` 以展示页面标题，其余块定位到视口上方约 `20%`）。
  - `KeepVisible`（打字、`Enter`、`Backspace`、`Delete`、粘贴、撤销/重做、方向键跨块）：优先实测**当前光标行**相对视口的垂直范围 `[caret_rel_top, caret_rel_bottom]`（原子块回退到行外框），只要光标已在视口 `[12px, vh - 16px]` 内可见则**完全不滚动**（即使超高代码块/长段落顶部已滚出视口上方被遮住一半，在底部回车也绝不把顶部跳出来）；仅当光标真正越出视口上下边界时才按最小偏移量平滑微调 `scroll_offset_y`。
- **`CAP-08-03` 窗口最右侧自适应比例滚动条**：
  - 垂直滚动条必须固定在**编辑器视口最右边缘**（绝不可贴在居中正文栏右边缘）。
  - 滚动条轨道宽约 `14px`、圆角滑块宽约 `8px`、最小滑块长度不小于 `56px`（防止长文档缩成肉眼难见的细线）；滑块长度按 `viewport_height / content_height` 随窗口尺寸与页面总长度实时自适应调整，支持拖拽滑块与点击轨道跳转，内容未溢出时自动隐藏。

### 页面版式与宽度调节
- **`CAP-08-04` 标题与正文紧凑视觉节奏**：页面标题（`30px SemiBold`）与正文第 0 块之间保持清爽紧凑间距（首块不叠加 H1 的大顶边距），避免出现 40–50px 的断层大空隙。
- **`CAP-08-05` 11 档页面宽度调节（设置页统一配置）**：
  - 正文栏居中排版，支持 11 档离散步进：`680px` 到 `1880px`（步长 `120px`，默认 `800px`，`1880px` 代表全宽），统一在「设置 → 外观 → 页面宽度」中调节并持久化，同时不缩小虚拟列表宽度（滚动条始终留在窗口最右边缘）。
  - 正文左右空白边缘不再挂载拖拽页宽抓手，避免与正文边缘空白框选冲突或误触发调宽。
- **`CAP-08-06` 右下角悬浮字数徽章**：在视口右下角（为右侧滚动条留出间距）悬浮展示 `{words} 词 · {chars} 字`，0 字空页自动隐藏。

---

## 历史踩坑与反例

- **反例 1（非虚拟化编辑器打开 141KB 文档彻底卡死）**：早期采用 `nsh-base::RichEditor` 无视口虚拟化，粘贴 141KB Checklist 文档后切页延迟数秒、鼠标拖选卡顿、下拉出现空白。切回 `mica-editor + virtual_list` 彻底根治。
- **反例 2（滚动条贴在正文右侧边缘）**：把 `PageWidthFrame` 套在滚动容器外面，导致滚动条跟着 `800px` 正文栏漂在屏幕中间。正确结构是 `virtual_list` 铺满 100% 宽，内部每一行自行居中 `max_w(page_width)`。
- **反例 3（回车/删除导致上面内容向上滚动）**：`dispatch` 在 `SplitBlock`/`DeleteBackward` 时无条件调用 `scroll_to_index(idx, 20%)`，每次在屏幕中下部按回车都把当前行猛拉到顶部 20%。
- **反例 4（Windows 默认滚动指示器仅 3px 宽）**：未挂载自定义比例 `Scrollbar`，长文档右侧仅剩一条几乎不可见的极细短线。
- **反例 5（超高代码块顶部滚出视口一半，在末尾按回车导致被遮住的上半部分突然跳出来）**：`ScrollMode::KeepVisible` 按整块 `row_ref` 的 `rel_top < 12px` 判断越界并执行 `scroll_to_index(idx, 10%)`，忽略了光标本身位于代码块底部且已在视口内。根治方案：按精确光标矩形 `[caret_rel_top, caret_rel_bottom]` 与 `compute_keep_visible_scroll_target` 计算最小滚动偏移量，光标在视口内时严格零滚动。

---

## 验收清单

| 编号 | 优先级 | 适用端 | 纯 Rust 状态 | 验证入口 |
| --- | --- | --- | --- | --- |
| `CAP-08-01` 长文档虚拟化与信号隔离 | P0 | 全端 | ✅ | `editor_bridge::tests::typing_bumps_only_single_block_rev_never_structure_rev` + `large_document_loads_fast_and_undo_redo_stay_single_stack` |
| `CAP-08-02` `Jump` vs `KeepVisible` 编辑零跳滚 | P0 | 全端 | ✅ | `editor_bridge.rs::ScrollMode` + `editor_view.rs`（待长文档真机复测） |
| `CAP-08-03` 窗口最右侧自适应比例滚动条 | P0 | 桌面/Web | ✅ | `app_flows::regression_fifteen_editor_ux_fixes`（`id:editor-vertical-scrollbar`） |
| `CAP-08-04` 标题与正文紧凑间距 | P1 | 全端 | ✅ | `editor_view.rs` + `block_views.rs`（`idx == 0 => mt 2px`） |
| `CAP-08-05` 11 档页面宽度调节（设置页统一配置） | P1 | 全端 | ✅ | `settings_view::tests::page_width_stops_match_flutter_11_stops` + `app_flows::flutter_parity_app_menu_world_switcher_graph_version_diff_and_width_grips`（验证正文不再挂载拖拽抓手） |
| `CAP-08-06` 右下角字数徽章 | P2 | 全端 | ✅ | `editor_view.rs::editor-word-count` |

---

## 演进记录

- **2026-11-23**：确立全宽 `virtual_list` 虚拟化架构、`Jump`/`KeepVisible` 分离、自适应比例滚动条与首块紧凑间距规格（对应 `3f033c30` / `ff104b87`）。
- **2026-11-24**：移除编辑器左右边缘拖拽调整正文宽度的抓手，正文宽度统一由设置页 11 档滑块调节，左右空白区无缝支持块级框选。
- **2026-11-27**：`ScrollMode::KeepVisible` 升级为按活动光标行精确垂直区间 `[caret_rel_top, caret_rel_bottom]` 与 `compute_keep_visible_scroll_target` 最小偏移滚动，彻底消除超高代码块顶部滚出视口一半时在末尾回车导致被遮住顶部跳出的问题（`CAP-08-02`）。
