# 07 — 图片、数学公式（LaTeX）与 Mermaid 图表

> **适用范围**：桌面端、Web、移动端。

## 核心不变量

1. **原子块通过统一注册表扩展**：图片、数学公式、Mermaid 图表、内嵌白板属于可扩展原子渲染块（对应 `AGENTS.md` 渲染红线），不得在主编辑器循环中堆砌散落分支。
2. **单一纯 Rust 渲染引擎**：数学公式唯一使用 `ratex-*`（通过 `crates/md-core`），Mermaid 唯一使用 `merman`（通过 `crates/md-core`，配色跟随宿主机主题并保留 SVG CSS 属性展开兼容）。
3. **两步防误删保护**：光标在原子块下一空行按 `Backspace` 时，先删除空行并选中原子块，再按一次才删除原子块，防止误删图片/图表。

---

## 能力与交互规格

### 图片块（Image）
- **`CAP-07-01` 多入口插入图片**：支持通过 `/` 斜杠菜单、顶栏图片按钮（原生文件选择器）、系统剪贴板截图/网页复制图片 `Mod+V`、文件管理器复制图片文件 `Mod+V`、粘贴裸图片 URL 插入图片块。
- **`CAP-07-02` 自然宽高比渲染与对齐**：自动探测图片真实宽高（PNG/JPEG/GIF/BMP/WebP）并在合理视口范围内保持原宽高比呈现，严禁硬编码固定宽高强行拉伸变形；悬停支持左/中/右对齐、底部图片说明（`alt`）展示与删除；支持持久化自定义宽度。
- **`CAP-07-03` 全屏缩放灯箱（Lightbox）**：点击图片打开全屏灯箱预览，支持 `−` / `+` / `100%` 缩放与滚动查看，按 `Esc` 或「关闭」返回原位置。

### 数学公式（LaTeX / RaTeX）
- **`CAP-07-04` 块级数学公式（`math_block`）**：使用 `mica_md::render::render_math_svg` 渲染高清矢量公式，支持点击展开 LaTeX 源码输入框实时预览编辑，回车收起；浅/深色主题自动切换前景色与柔和底色。
- **`CAP-07-05` 行内数学公式与原子选区保护**：行内 `$...$` 公式作为原子 run 渲染；左右方向键跨过整个公式，禁止光标落入公式隐藏源码内部盲打；光标紧贴公式右侧按 `Backspace` 删除整段公式。

### Mermaid 图表
- **`CAP-07-06` 主题同步矢量渲染**：语言为 `mermaid` 的代码块渲染为 Mermaid SVG 预览卡片，实时注入当前宿主机浅/深色 `MermaidHostTheme`；语法错误时优雅降级显示源码而不崩溃。
- **`CAP-07-07` 点击全屏放大与缩放查看**：点击 Mermaid 图表卡片（或悬停工具条「放大」）打开全屏缩放灯箱，支持 `40%–400%` 缩放、滚动平移与灯箱内展开源码实时同步修改。
- **`CAP-07-08` 卡片内联源码切换与复制**：悬停显示「放大」「复制」「源码 / 收起源码」按钮；展开源码使用非受控代码编辑器并互斥正文焦点，编辑实时刷新图表。

---

## 历史踩坑与反例

- **反例 1（Mermaid 无法点击、无法放大）**：早期接入 `RichEditor` 的非交互静态卡片，丢掉了点击触发全屏灯箱的回调，复杂架构图在正文列宽里缩成一团无法看清。
- **反例 2（图片固定 `560×300` 拉伸变形）**：未读取本地图片文件头宽高，竖图和超宽截图全部被压扁或拉长。
- **反例 3（SVG 内部 `<style>` 在部分光栅器失效）**：Mermaid 导出的 SVG 若依赖 CSS class 选择器，在非浏览器光栅器中会丢失颜色和线条，必须保留 `md-core` 的 CSS 属性内联展开处理。

---

## 验收清单

| 编号 | 优先级 | 适用端 | 纯 Rust 状态 | 验证入口 |
| --- | --- | --- | --- | --- |
| `CAP-07-01` 多入口插入与剪贴板粘贴图片 | P0 | 桌面/Web | ✅ | `app_flows::regression_fifteen_editor_ux_fixes` |
| `CAP-07-02` 自然宽高比与对齐/删除 | P1 | 全端 | ✅ | `clipboard_platform::tests::probe_png_and_bmp_dimensions` + `block_views.rs` |
| `CAP-07-03` 图片全屏缩放灯箱 | P1 | 全端 | ✅ | `block_views.rs::render_lightbox_modal` |
| `CAP-07-04` 块级公式渲染与点击编辑 | P1 | 全端 | ✅ | `block_views.rs::render_math_block_view` |
| `CAP-07-05` 行内公式原子渲染与光标保护 | P1 | 全端 | ✅ | L2 内核 `math_atom_run_detection_and_caret_snapping` + L3 点击行内 `$...$` 弹出 `editor_positioning::inline_math_popover_view` 实时 RaTeX SVG 预览与 `SetInlineMath` 编辑/删除（正文行内流式胶囊展示） |
| `CAP-07-06` / `07` / `08` Mermaid 主题渲染、灯箱缩放与源码编辑 | P0 | 全端 | ✅ | `app_flows::virtualized_editor_code_overlay_mermaid_lightbox_and_page_switch` |

---

## 演进记录

- **2026-11-23**：沉淀 Mermaid 全屏灯箱、自然宽高比图片渲染与 RaTeX 公式规格（对应 `3f033c30` / `ff104b87`）。
- **2026-11-23（补齐迭代）**：新增点击行内 `$...$` 公式弹出的实时 RaTeX SVG 预览与源码编辑浮层（`inline-math-popover`，直连 `EditorCommand::SetInlineMath`，`CAP-07-05`）。
