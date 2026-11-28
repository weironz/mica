# 07 — 图片、数学公式（LaTeX）与 Mermaid 图表

> **适用范围**：桌面端、Web、移动端。

## 核心不变量

1. **原子块通过统一注册表扩展**：图片、数学公式、Mermaid 图表、内嵌白板属于可扩展原子渲染块（对应 `AGENTS.md` 渲染红线），不得在主编辑器循环中堆砌散落分支。
2. **单一纯 Rust 渲染引擎**：数学公式唯一使用 `ratex-*`（通过 `crates/md-core`），Mermaid 唯一使用 `merman`（通过 `crates/md-core`，配色跟随宿主机主题并保留 SVG CSS 属性展开兼容）。
3. **两步防误删保护**：光标在原子块下一空行按 `Backspace` 时，先删除空行并选中原子块，再按一次才删除原子块，防止误删图片/图表。

---

## 能力与交互规格

### 图片块（Image）
- **`CAP-07-01` 多入口插入与云端/本地上传**：支持通过 `/` 斜杠菜单、顶栏图片按钮（桌面原生文件选择器 & Web `<input type="file">`）、系统剪贴板截图/网页复制图片 `Mod+V`、文件管理器复制图片文件 `Mod+V`、粘贴裸图片 URL 插入图片块；云端空间走 `files/presign → PUT → files/complete` 上传并写入 `data.file_id` + `data.name`（桌面同时镜像本地 CAS 供离线读取），本地空间存入内容寻址 CAS，同工作区 Mica `/blob` 链接直接复用 `file_id`。
- **`CAP-07-02` `file_id` 解析、自然宽高比、对齐与拖拽缩放**：统一解析 `data.file_id`（优先本地 CAS 镜像，未命中拼接永久 `/api/workspaces/{ws}/files/{file_id}/blob/{name}` 直链）、服务端相对 `/api/...` 路径以及外链 `data.url`（外链左上角显示「外链」徽章，加载失败提示域名）；通过本地文件头探测 + `Image::on_load` 获取真实自然宽高比，严禁硬编码固定宽高强行拉伸；悬停右上角显示紧凑深色半透明纯图标浮条（全屏放大 / 复制图片 / 循环切换对齐 / 编辑图注 / `⋯` 更多菜单，悬停每个图标显示文字 Tooltip），右边缘提供拖拽缩放手柄实时调整并持久化 `data.width`。
- **`CAP-07-03` 单击选中、右侧点击光标换行、无框灯箱、位图复制与菜单**：单击图片或其右侧空白区域选中整块并获得编辑器焦点（蓝框高亮并在图片最右侧显示闪烁光标 `image-trailing-caret`，按 `Enter` 直接在下方插入新段落换行，支持 `Mod+C` 复制位图、`Mod+X` 剪切、`Backspace`/`Delete` 删除）；双击图片或点击「全屏查看」直接以无边框、无多余背景填充的全视口暗色聚光灯箱展示图片（小图保持 1:1 清晰尺寸、大图等比贴合视口，底部极简缩放胶囊 + 右上角悬浮关闭按钮，点击图片外暗色遮罩可关闭）；`⋯` 浮条按钮与右键菜单共享完整操作项（存储位置提示、全屏查看、编辑图注、左/中/右三段对齐选择、替换图片、转存到 Mica、复制图片、剪切图片、粘贴替换、复制公开链接、在浏览器打开、下载/另存为与删除）；多块/选区复制时将 `file_id` 解析为永久 `/blob` 链接写入 Markdown 与 HTML。

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
- **反例 4（忽略 `data.file_id` 导致历史云端图片全部显示「图片地址为空」）**：`v0.15.0` 切流初期仅读取 `data.url`/`data.src`，未解析云端/本地 CAS 存储的 `data.file_id` 与相对 `/api/.../blob` 路径，且选区复制传入空 `image_urls`、缺少位图复制与右键菜单。
- **反例 5（右键菜单/悬停工具条内直接 `spawn()` 导致所有异步图片操作被瞬间取消）**：`nsh-core::spawn` 将任务取消句柄绑定到当前 `Owner::current()`；图片右键菜单处于 `when(menu_sig.is_some())` 子作用域下，点击「复制图片 / 剪切图片 / 替换图片… / 粘贴替换 / 下载图片 / 转存到 Mica」时先执行 `menu_sig.set(None)` 关闭菜单，下一拍子 `Owner` 被销毁并立即取消刚创建的异步任务，导致复制图片无法写入剪贴板（无法粘贴到 Mica、Word 或任何应用）、下载/替换无响应且提示 Toast 定时消失任务被取消而常驻。根治方案：`ImageContext` 捕获应用根级 `owner: Owner::current()` 并通过 `ctx.spawn(...)` 统一派生所有异步图片与 Toast 任务；同时 Windows `CF_DIB` 将透明通道预合成到白底避免 Paint/旧版 Word 变黑、桌面「粘贴替换」兼容资源管理器复制的图片文件（`CF_HDROP`）、Web 端复制同源/data 图片在点击手势内同步触发 `set_clipboard_image` 避免丢失用户激活。
- **反例 6（悬停文字栏遮挡图片且切换对齐时浮条闪烁消失）**：早期悬停工具条采用 7 个中文文字胶囊横向排列，宽度超过 `250px` 遮挡图片右上角内容；且 `hovered` 存放在行内局部 `signal` 中，点击对齐触发 `block_rev` 重建后 `hovered` 重置为 `false`，用户无法连续点击切换左/中/右对齐。根治方案：改用紧凑深色半透明 5 图标胶囊 + Tooltip，并将 `hovered_block` 状态上移至 `ImageContext` 跨块重建保持稳定。
- **反例 7（放大图片套用固定灰白弹窗外壳导致四周大量空白边距）**：早期图片与 Mermaid 共用 `86%×82%` `surface_overlay` + 46px 顶栏弹窗，导致图片放大后四周出现大块无用灰白背景。根治方案：图片灯箱改为全编辑区无框透明舞台直接浮于暗色聚光遮罩之上，小图保持 1:1 清晰尺寸、大图按视口余量等比例贴合居中。

---

## 验收清单

| 编号 | 优先级 | 适用端 | 纯 Rust 状态 | 验证入口 |
| --- | --- | --- | --- | --- |
| `CAP-07-01` 多入口插入与云端/本地上传 | P0 | 桌面/Web | ✅ | `mica_client::net::tests::build_and_parse_file_blob_url_round_trip` + `image_blocks.rs` |
| `CAP-07-02` `file_id` 解析、自然宽高比、紧凑浮条、对齐与拖拽缩放 | P0 | 全端 | ✅ | `image_blocks::tests::{resolve_cloud_file_id_builds_permanent_blob_url,resolve_server_relative_blob_path_prepends_origin_and_recognizes_file_id,set_image_source_by_id_preserves_width_align_and_caption}` + `app_flows::regression_image_compact_hover_toolbar_and_frameless_lightbox` |
| `CAP-07-03` 选中/右侧光标换行/无框灯箱/位图复制/选区复制与菜单 | P0 | 全端 | ✅ | `block_views::tests::compute_image_lightbox_fit_preserves_small_images_and_fits_large_without_card_margin` + `image_blocks::tests::build_document_image_urls_maps_file_ids_for_selection_copy` + `clipboard_platform::tests::windows_clipboard_image_prepares_both_png_and_dib` + `app_flows::regression_code_auto_lang_fast_backspace_image_caret_and_indent_buttons` + `app_flows::regression_image_context_menu_actions_survive_menu_close` + `app_flows::regression_image_compact_hover_toolbar_and_frameless_lightbox` |
| `CAP-07-04` 块级公式渲染与点击编辑 | P1 | 全端 | ✅ | `block_views.rs::render_math_block_view` |
| `CAP-07-05` 行内公式原子渲染与光标保护 | P1 | 全端 | ✅ | L2 内核 `math_atom_run_detection_and_caret_snapping` + L3 点击行内 `$...$` 弹出 `editor_positioning::inline_math_popover_view` 实时 RaTeX SVG 预览与 `SetInlineMath` 编辑/删除（正文行内流式胶囊展示） |
| `CAP-07-06` / `07` / `08` Mermaid 主题渲染、灯箱缩放与源码编辑 | P0 | 全端 | ✅ | `app_flows::virtualized_editor_code_overlay_mermaid_lightbox_and_page_switch` |

---

## 演进记录

- **2026-11-23**：沉淀 Mermaid 全屏灯箱、自然宽高比图片渲染与 RaTeX 公式规格（对应 `3f033c30` / `ff104b87`）。
- **2026-11-23（补齐迭代）**：新增点击行内 `$...$` 公式弹出的实时 RaTeX SVG 预览与源码编辑浮层（`inline-math-popover`，直连 `EditorCommand::SetInlineMath`，`CAP-07-05`）。
- **2026-11-25**：全面恢复对齐 `v0.14.11` 的图片管线与交互（`crates/mica-client` 接入 `upload_image`/`import_image_url`/`resolve_files`/`download_url_bytes`；`clients/mica_rust/src/image_blocks.rs` 补齐 `file_id` + 相对 `/api/.../blob` 解析、桌面 CAS 镜像、`on_load` 自然宽高比、单击选中/双击灯箱、悬停工具条、右边缘拖拽缩放、完整右键菜单、Windows `PNG`+`CF_DIB` / Web 位图复制与选区 `image_urls` 映射）。
- **2026-11-27（图片右侧点击显示光标并回车换行）**：在 `image_blocks.rs` 为图片外层容器补充 `Cursor::Text` 与 `on_press_down` 聚焦选中、在图片右侧渲染闪烁尾随光标 `image-trailing-caret`，点击图片或图片右侧空白后即可按 `Enter` 在下方插入新段落（`CAP-07-03`）。
- **2026-11-27（图片右键菜单异步任务存活根治）**：`ImageContext` 捕获根级 `Owner` 并将全部异步任务（复制/剪切/下载/替换/粘贴替换/转存/Toast 自动消失及 `Ctrl+C`）统一经由 `ctx.spawn` 执行，根治关闭右键菜单或移出悬停条时异步任务被取消导致「复制图片」无法粘贴到 Mica/Word 及其他菜单项失效的问题；同步补齐 Windows `CF_DIB` 白底合成、桌面「粘贴替换」`CF_HDROP` 支持、Web 手势内同步复制与图注弹窗自动聚焦（`CAP-07-03`）。
- **2026-11-28（图片悬停浮条紧凑图标化与无框放大灯箱）**：参考 AppFlowy / AFFiNE / Notion，将 `image_blocks.rs` 图片右上角悬停浮条由 7 个宽文字按钮替换为深色半透明纯图标胶囊（全屏放大 / 复制图片 / 循环切换对齐 / 编辑图注 / `⋯` 更多菜单，配合悬浮 Tooltip），并在菜单中补齐三段式对齐切换（`CAP-07-02`）；重构 `block_views.rs` 图片放大灯箱，去掉 `86%×82%` 灰白大弹窗外壳与顶栏空白边距，改为全视口透明舞台直接浮于暗色聚光遮罩之上（小图 1:1、大图等比贴合视口、点击空白遮罩关闭、顶部悬浮 `×` 与底部极简缩放胶囊，`CAP-07-03`）。
