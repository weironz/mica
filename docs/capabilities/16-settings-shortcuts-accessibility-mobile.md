# 16 — 设置中心、快捷键体系、主题外观与移动端体验

> **适用范围**：桌面端、Web、移动端（iOS / Android，阅读优先 + 完整编辑）。

## 核心不变量

1. **快捷键三处同步**：任何快捷键新增或调整，必须同步更新应用/编辑器按键处理、设置页「快捷键」面板和权威清单 [`docs/features/shortcuts.md`](../features/shortcuts.md)。
2. **外观偏好即改即生效并跨重启持久化**：语言、浅/深色主题、界面缩放、页面宽度、正文字号、字体族、格式栏与页面标题显隐，修改后实时刷新全界面并持久化保存。
3. **三端能力连续性**：桌面端与 Web 端保持同等核心地位；移动端以极致阅读体验优先，同时保留全功能编辑、白板查看、搜索与离线访问能力。

---

## 能力与交互规格

### 设置中心与主题外观
- **`CAP-16-01` 9 标签设置中心与多端自适应**：桌面默认 `1120×840` 双栏布局，窄视口（`<720px`）自动切换为**顶部横向滚动胶囊导航 + 全宽单列详情滚动区**（杜绝手机端右侧页面被挤成半屏无法操作）；Web 端自动精简隐藏桌面专属能力（如诊断/崩溃日志、关闭窗口行为、本地目录路径框与离线镜像缓存、被浏览器劫持的标签页快捷键说明，并保留 `.md` 上传按钮）。左侧/顶部导航覆盖 9 个标签页：
  - 通用：**外观**、**快捷键**
  - 账户：**账户**、**API 令牌**
  - 工作区：**数据**（导入导出与本机/云端存储）、**使用统计**（工作区/页面/字数/存储/知识健康度/写作趋势与 180 天热力图，云端与本地双模式同构；亦支持从左下角账号菜单一键直达）、**回收站**
  - 其他：**AI 服务商**、**诊断**（仅桌面端）
  - 底部固定「关于 v<ver>」弹窗入口。
- **`CAP-16-02` 外观设置与实时预览**：
  - 支持语言（跟随系统 / 中文 / English）、主题（跟随系统 / 浅色 / 深色）、界面字号（小 / 标准 / 大）、页面宽度 11 档滑块与重置、正文字号（`13..22px`）、正文字体（系统默认 / Serif / Mono）、格式工具栏开关、页面标题开关。
  - 提供实时 `MicaAppearancePreview` 预览卡直观反馈排版变化。
- **`CAP-16-03` 统一主题色板**：浅色纸感底色与深色护眼色板（如 `surface_overlay=#252525`、`code_bg=#202020`、`accent=#4C8DF6`、`favorite_amber=#F5B400`、`copy_done=#16A34A`、5 色 Callout 与 6 色代码高亮 Token）在主界面、白板、Mermaid、公式、灯箱之间保持高度一致。

### 快捷键与全局缩放体系
- **`CAP-16-04` 完整应用/编辑器快捷键与 `Ctrl+滚轮` 全局缩放**：覆盖文件/标签（`Mod+N`/`T`/`W`/`Tab`/`,`）、搜索/查找（`Mod+Shift+F`/`Mod+F`）、撤销重做（`Mod+Z`/`Y`/`Shift+Z`）、行内格式（`Mod+B`/`I`/`E`/`Shift+S`/`K`）、缩进（`Tab`/`Shift+Tab`）、块选与剪贴板（`Mod+A`/`C`/`X`/`V`）、退出关闭（`Esc`）、重命名（`F2`），并在桌面与 Web 端统一支持 `Ctrl+滚轮` / `Ctrl+=` / `Ctrl+-` / `Ctrl+0` 进行 `75%–150%`（步进 `5%`）界面与正文字号实时缩放（短暂显示百分比角标）；所有快捷键在设置页以键帽胶囊形式完整展示。

### 移动端体验（阅读优先 + 全功能编辑）
- **`CAP-16-05` 窄屏响应式布局、抽屉导航与溢出工具栏**：视口宽度 `<768px` 时，左右侧边栏自动转为可滑出抽屉（Drawer），顶部切换为紧凑标签切换器，正文自动适配屏幕宽度（不显示桌面页宽抓手）；`<720px` 格式栏支持横向滑动触控不裁切图标，`<640px` 顶部页眉仅保留标题与核心操作、将次高频入口收纳入 `⋯` 菜单。
- **`CAP-16-06` 移动端触摸选区、操作条与复制自动收起**：
  - 长按或双击中文字符时按字素安全规则精确选中 1–2 个汉字（而非一口气选到下一个标点符号），拉丁词与数字选整词（对齐原版 `touchWordBounds`）。
  - 选中后在两端显示可拖动水滴手柄，并在上方弹出触摸友好操作条（剪切 / 复制 / 粘贴 / 全选），避免软键盘与工具栏互遮；点击「复制」后立即清空选区并自动收起浮动菜单与水滴手柄，不残留浮层遮挡阅读。
- **`CAP-16-07` 移动端阅读舒适度**：长文档流畅惯性滚动、点击图片与 Mermaid 支持双指捏合/灯箱缩放查看、离线 SQLite 随时秒开历史笔记。

---

## 历史踩坑与反例

- **反例 1（手机端打开设置页，右侧详情页面只能显示一半）**：设置弹窗沿用固定宽度双栏行布局，在 `<720px` 屏幕左侧边栏挤压右侧内容区。根治方案：`<720px` 自动切换为顶部横向滚动胶囊导航 + 全宽单列垂直滚动。
- **反例 2（手机端顶栏与格式栏图标过多被截断、无法看到完整按钮）**：窄屏未折叠次要页眉动作、格式工具栏也未提供横向滚动。根治方案：`<640px` 页眉收敛入 `⋯` 菜单，`<720px` 格式栏开启横向滚动。
- **反例 3（移动端点击触摸选区「复制」后浮动操作条与水滴手柄未自动退出）**：复制回调仅写剪贴板未清空选区状态，导致气泡菜单一直遮挡正文。
- **反例 4（Web 端设置页暴露仅桌面端可用的诊断/托盘/本地路径及已无效的浏览器快捷键）**：未按 `allow_local` / `wasm32` 条件精简设置项并遗漏 `.md` 上传入口。
- **反例 5（打开设置页后按 `Esc` 无法自动关闭）**：应用根壳在挂载早期静态注册的全局 `escape` 快捷键被后挂载的 `editor_view` `escape` 绑定按 `nsh_core::keymap::Keymap` 逆序命中规则遮蔽，导致即使设置了 `pass_through: true` 也无法回到应用级浮层处理。根治方案：设置弹窗与关于弹窗挂载时直接注册局部 `When::Always` 的 `escape` 处理，且应用级浮层 `escape` 绑定改为由浮层打开状态驱动的响应式 `effect` 动态置顶。

---

## 验收清单

| 编号 | 优先级 | 适用端 | 纯 Rust 状态 | 验证入口 |
| --- | --- | --- | --- | --- |
| `CAP-16-01` / `02` / `03` 9 标签设置中心、多端自适应与色板 | P0 | 全端 | ✅ | `settings_view::tests` + `usage_stats_view::tests` + `app_flows::regression_usage_statistics_dashboard_and_live_updates` + `wave2_settings_tests::regression_settings_mobile_responsive_layout_shows_full_width_detail_pane` + `wave2_settings_tests::regression_settings_web_mode_hides_desktop_only_controls_and_exposes_avatar_actions` + `storage::tests::prefs_round_trip_and_persist_to_json_file` |
| `CAP-16-04` 快捷键体系、`Ctrl+滚轮` 全局缩放与设置页清单 | P0 | 桌面/Web | ✅ | [docs/features/shortcuts.md](../features/shortcuts.md) + `zoom_controller::tests::snap_zoom_clamps_and_quantizes_to_five_percent_steps` + `app_flows::regression_ctrl_wheel_and_shortcuts_zoom_within_bounds` + `app_flows::regression_escape_closes_settings_and_layered_about_modal` |
| `CAP-16-05` 移动端 `<768px` 抽屉、`<640px` 溢出菜单与 `<720px` 横滑工具栏 | P1 | 移动端/窄屏 | ✅ | `responsive_shell.rs` + `app_flows::regression_narrow_viewport_drawers_and_touch_bar` + `app_flows::regression_mobile_editor_toolbar_scrolls_format_bar_and_moves_overflow_to_more_menu` |
| `CAP-16-06` 触摸选区、水滴手柄、操作栏与复制后自动收起 | P1 | 移动端/Web | ✅ | `touch_selection::tests` + `app_flows::regression_narrow_viewport_drawers_and_touch_bar` + `on_long_press` / `touch-handle-start` / `touch-handle-end` |
| `CAP-16-07` 移动端流畅阅读与灯箱缩放 | P1 | 移动端 | 🟡 | 虚拟滚动、触摸防误弹键盘（滚动滑动不触发 `request_focus`，仅干脆单击落光标唤起输入法）与多指捏合状态机（`touch_selection::LightboxGesture` + `touch_selection::tests`）已落地，待下一版发版后 iOS/Android 实机复测 |

---

## 演进记录

- **2026-11-23**：沉淀设置中心、快捷键体系、主题外观与移动端（含 `touchWordBounds` 触摸选区）规格。
- **2026-11-23（补齐迭代）**：新增 `responsive_shell.rs` 与 `touch_selection.rs`，补齐 `<768px` 响应式左右覆盖抽屉、窄屏标签切换器、CJK 1–2 字触摸选词算法与窄屏紧凑触摸选区操作条（`CAP-16-05` / `06`）。
- **2026-11-24（使用统计与可观测性 #191）**：设置中心扩展为 9 标签新增「使用统计」（同步在左下角账号菜单提供直达入口），接入 L1 统一 `mica_core::word_count` / `usage_stats`、本地 SQLite 小时桶活跃表与云端 `/api/stats/overview` `/api/stats/activity` + Prometheus `/metrics` 业务指标。
- **2026-11-28（真机体验修复 `CAP-16-07` & Web 离线 PWA 外壳）**：① 正文/代码块/图片在触摸指针（`Touch`/`Pen`）下不再于 `on_press_down` 提前 `request_focus()`，改为仅干脆单击 `on_press` 时才聚焦 IME，根治长文档下滑刚触屏就弹出输入法；② 放大灯箱改为 `TouchClaim::on_start().block_native(true)` + 基于指针 ID 的 `LightboxGesture` 状态机，支持以双指中点为锚点的顺滑缩放与单/双指切换零跳动，修复双指缩放在区域内来回跳动；③ 新增构建期内容哈希版 `sw.js`（HTML 首页 Network-First 离线兜底、`?v=` 静态 JS/WASM Cache-First、`/api/*` `/ws/*` `/s/*` 严格直通）支持 Web 端离线按 F5 刷新秒开本机缓存。
- **2026-11-28（移动工具栏、移动设置适配、Web 精简、正文右键与全局缩放）**：① `<640px` 顶部页眉仅保留标题与高频动作并把次高频入口收入 `⋯` 菜单，`<720px` 格式栏支持横向滑动触控不裁切；② 移动端 `<720px` 设置弹窗改为顶部横向胶囊导航 + 全宽详情滚动区与单列表单；③ 触摸选区点击「复制」后立即清选区并收起操作条与水滴手柄；④ Web 端隐藏诊断、窗口关闭行为、本地目录导入/路径框/镜像缓存及浏览器占用的快捷键说明，补齐 `.md` 上传按钮；⑤ 桌面与 Web 端统一支持正文右键菜单（复制/剪切/添加评论/粘贴/纯文本粘贴/全选）及 `Ctrl+滚轮`、`Ctrl+=/−/0` 全局缩放（75%–150%）。
- **2026-11-28（修复 `Esc` 无法关闭设置页与分级弹窗关闭）**：修复 `nsh_core::keymap` 逆序绑定被 `editor_view` 静态 `escape` 遮蔽的问题，确保打开设置页后（含焦点位于设置页输入框内时）按 `Esc` 立即关闭设置弹窗，以及打开「关于」等应用弹窗后按 `Esc` 稳定退出。
