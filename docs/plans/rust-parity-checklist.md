# Mica 纯 Rust 客户端（`clients/mica_rust`）vs 原版 Flutter（`main:clients/mica_flutter`）1:1 功能对齐总表

> 基准：`main:clients/mica_flutter/lib/`（291 个源文件）+ `l10n/app_zh.arb` + `docs/features/`  
> 目标：`clients/mica_rust/src/`（100% 全栈纯 Rust，L1 + L2 + L3 `nsh-core`/`nsh-base`）  
> 状态图例：`[ ]` 缺失 / 偏差，`[x]` 已按原版源码 1:1 还原并通过验证。  
> 验证记录：`cargo clippy --all-targets -- -D warnings` 0 告警；`cargo test` **107 / 107 通过**（88 单元测试 + 3 端到端 Headless 测试 + 16 SQLite/云端同步集成测试）；`scripts/check-file-size.sh` 与 `scripts/check-ignored-sources.sh` 均通过；Windows 实机 DPI 截图验收通过。

---

## Domain 1：Shell 顶栏、左上角应用菜单、账号菜单、切换世界、标签页与快捷键

- [x] **1.1 主题色板与阴影 Token 1:1 校准** (`mica_theme.rs`)：暗色 `surface_overlay=#252525`、`code_bg=#202020`、`accent=#4C8DF6`、`accent_hover=#6BA1F8`、`accent_wash=rgba(0x4C8DF6,0.12)`、`on_primary=#191919`；补全 `status.{warning,success,danger}_wash`、`inline_code_bg/ink`、`comment_highlight/active`、`alert_accents`（5 种）、`CodeTokens`（6 色）、`favorite_amber=#F5B400`、`copy_done=#16A34A`。
- [x] **1.2 无边框 34px 统一顶栏** (`lib.rs`)：全宽 `surface.sunken` 无分隔线，左集群最小宽度 `max(sidebar_w + 7, 84)`，左内边距 `8px`，`30×30` M-logo 按钮（22px 靛紫渐变 Logo）+ `2px` + `30×30` 侧栏折叠按钮（19px 图标）。
- [x] **1.3 左上角应用菜单 (`app_menu.rs`)**：点击左上角 M 图标弹出级联菜单（`surface.overlay`、`r=9`、`p=4`）：`文件`（新建页面 `Ctrl+N` / 新建标签页 `Ctrl+T` / 关闭当前标签页 `Ctrl+W` / 设置 `Ctrl+,`）、`视图`（✓ 收起/展开左侧栏、✓ 收起/展开右侧栏）、`历史`（后退/前进）、`帮助`（关于 Mica / 检查更新）；含独立 `420px` `关于 Mica` 弹窗。
- [x] **1.4 底部账号菜单 (`sidebar.rs` / `app_menu.rs`)**：常驻行 `28×28` 头像（本地模式首字 `本`，名称 `本地工作区`，副标题 `这台设备`）；浮层采用 `surface.sunken` 外壳（`r=14, p=5`）内嵌堆叠 `272px` `surface.overlay` 卡片（`r=10`）：身份卡 + 操作卡（`设置 Ctrl+,` / `切换世界…` / `检查更新 v...`）+ 会话卡（`退出登录` 或 `登录云端…`）。
- [x] **1.5 切换世界与登录分屏页 (`world_switcher.rs` / `net.rs` / `storage.rs`)**：全窗口分屏 `SignInScreen`（`>=900px` 左深色径向渐变 `SignInHero` + 右 `420px` `SignInPane`；顶部避开系统标题栏的 `× 返回` 按钮）；`云端账户` / `本地模式` 分段切换；多服务器持久列表（`servers`/`cloudOrigin`/`activeOrigin` + 按源持久凭据）；自动 `/api/health` 探测与重试；可展开服务器列表（`已登录` 徽章、删除确认、添加服务器弹窗）；`SignedInCard`（免密切换）与 `AuthFormCard`（登录/注册切换 + 显示名称 + 忘记密码 + 内联错误）；本地模式「在此设备上使用 / 开始使用」。
- [x] **1.6 浏览器式标签栏、单标签历史与全量应用快捷键 (`lib.rs`)**：每标签独立 `TabHistory`（上限 50）、`max_w=200` 横向滚动、关闭 `×` 仅悬停/激活显示且单标签隐藏、关闭后定位右邻（`activeIndexAfterClose`）、`+` 弹出 2 项菜单（`新建页面` / `打开已有页面…`）、标签持久化 `openTabs`、侧栏单点页面默认在当前标签导航；补齐 `Ctrl+N`、`Ctrl+F`、`Ctrl+T`、`Ctrl+W`、`Ctrl+Tab`、`Ctrl+Shift+Tab`、`Esc`、`F2`。

---

## Domain 2：侧边栏页面树、右键菜单、拖拽排序、多选、收藏、Emoji 与回收站

- [x] **2.1 侧边栏几何与持久化 (`sidebar.rs` / `storage.rs`)**：默认 `280px`、拖拽范围 `220..480px`、持久化宽度；行高 `30px`、左缩进 `2 + depth*16`、固定 `18px` 展开箭头列（悬停显隐）、`18×18` Emoji/类型图标、`14px` 标题、悬停右侧动作条（页：`⋯`；文件夹：`⋯` `+` `📁+`）、定位创建（文件夹内 / 页面正下方 / 根目录）、`RenameDialog`（`400px`）。
- [x] **2.2 目录树右键菜单 (`sidebar.rs`)**：
  - 单页菜单（`在新标签页打开` / `重命名` / `设置图标` / `创建副本` / `移动到工作区` / `复制到工作区` / 分割线 / 红色 `删除`）。
  - 文件夹菜单（`新建子页面` / `新建子文件夹` / `重命名` / `设置图标` / `创建副本` / `展开/收起子项` / `备份(Mica 格式)` / `导出为 Markdown` / `导入文件(.md/.zip)` / `导入文件夹` / `移动到工作区` / `复制到工作区` / 分割线 / 红色 `删除`）。
  - 多选批量菜单（`移动 N 项到工作区…` / `复制 N 项到工作区…` / 红色确认 `删除 N 项`）。
  - 收藏行右键菜单（`取消收藏`）；空白区点击清除多选并聚焦根目录。
- [x] **2.3 树拖拽排序与跨父重排 (`tree_dnd.rs` / `sidebar.rs` / `storage.rs`)**：`30% / 40% / 30%` 三段落点区（`before`/`into`/`after`）；`folder` 唯一容器 + 防环校验（页面 `into` 显示警告色 `blocked` 反馈）；`after` 横向拖拽按 `reparent_level_for` 弹出层级；8px 空心圆 + 2px 线 `DropIndicator` + `into` 2px 框选高亮；底部空白区根拖放；跟随指针预览卡（单卡 / 三层堆叠多选卡 + `N 项`）；通过 `LocalStore::reorder_views`（10 位补零步进 10）单事务提交。
- [x] **2.4 目录树多选 (`tree_dnd.rs` / `sidebar.rs`)**：`Shift+Click` 同父范围连选（跨父降级切换）、`Ctrl/Cmd+Click` 切换、普通点击设锚点、折叠裁剪不可见选中、`Esc` 清空、多选 `1px text_primary` 描边、`independent_selection` 去重子孙后批量拖拽与批量删除。
- [x] **2.5 收藏区 1:1 对齐 (`sidebar.rs` / `storage.rs`)**：按收藏先后顺序 `Vec<Uuid>` 持久化，限高 `180px` 独立滚动，左留 `20px` 对齐树图标列，渲染页面 Emoji/文档图标而非星星，右键「取消收藏」。
- [x] **2.6 Emoji 图标选择器 (`emoji_picker.rs` / `storage.rs`)**：`360×420px` 弹窗、搜索框（中英双语关键词 + Enter 选首项）、`移除图标`、7 分类 8 列 `38×38` 网格；树行/收藏/回收站统一渲染 `view.icon`；本地 SQLite 持久化 `icon`。
- [x] **2.7 回收站子树聚合与永久删除 (`settings_view.rs` / `storage.rs`)**：调用 `trash_view_subtree` / `restore_view_subtree` / `purge_view_subtree` / `purge_all_trash`；聚合仅列子树根 + 原路径 + 含子文件夹/子页数 + 删除时间；支持刷新、恢复、确认永久删除、清空回收站；挂载进设置「回收站」标签页（侧栏底部保留入口直达）。

---

## Domain 3：编辑器核心、原版代码块风格、正文框选、光标锚定斜杠菜单、拖拽调宽与表格

- [x] **3.1 原版代码块风格 1:1 还原 (`node_views.rs` / `mica_rich_bridge.rs`)**：
  - 静息态无外框、无常驻 34px 顶栏；纯 `#F7F6F3`（暗 `#202020`）圆角 `6px`、`px=14 py=12`、`14px/22px` 等宽字、默认不换行 (`wrap=false`)、默认隐藏行号 (`lineNumbers=false`)。
  - 仅悬停（或复制后 1600ms 绿勾 `#16A34A`）在右上角浮现工具条：`[语言胶囊 "rust ▾"]` + `[问 AI]` + `[复制]` + `[⋯]`（`mermaid`/`mica-board` 附 `[预览 | 代码]` 切换）。
  - 点击语言胶囊弹出 `240×312px` 可搜索 `LanguagePicker`（`auto` 置顶 + L1 `mica_md::highlight::code_languages()` 全部语言 + 键盘操作）。
  - `⋯` 菜单：`自动换行/取消换行`、`显示行号/隐藏行号`、`添加标题/移除标题`（`CodeTitleDialog`，标题居中显示在代码块下方外部 24px 区域）、红色 `删除代码块`。
  - Schema & CRDT 完整保留 `wrap`、`lineNumbers`、`title` 属性并经回归测试验证。
- [x] **3.2 拖拽正文左右边界调宽 (`page_width.rs` / `editor_view.rs` / `storage.rs`)**：11 档离散步进 `680..1880px`（步长 `120px`，默认 `800px`，`1880` 为全宽）；左右 `12px` `ColResize` 抓手，悬停/拖拽亮起 `2px border.strong` 引导线，对称 `2×Δx` 实时吸附并在顶部居中显示 `{px} px` / `全宽 · {px} px` 浮窗，双击恢复 800px，持久化 `pageWidth` 并与设置页滑块联动。
- [x] **3.3 光标锚定斜杠命令菜单 (`editor_view.rs` / `editor_slash.rs`)**：
  - 彻底移除顶在页面上方的横排胶囊栏，改为通过 `bridge.caret_coords()` 定位在光标正下方 `(left, bottom + 6)` 并做视口边界夹紧的 `248px` 宽、`max_h 288px` 垂直浮层菜单。
  - 补全原版全部 22 项（`问 AI`、`正文`、`H1–H3`、无序/有序/待办、引用、5 种 GFM Callout `注意/提示/重要/警告/小心`、`代码`、`数学公式`、`画板`、`思维导图`、`流程图`、`表格`、`分割线`、`图片`），支持中英文/kind/alert 过滤、行中空格后触发、仅删除 `/query` 片段、`↑/↓/Enter/Esc` 键盘选择不丢焦。
- [x] **3.4 正文多块框选效果 (`block_marquee.rs` / `editor_view.rs` / `mica_rich_bridge.rs`)**：从页面左右留白、左侧拖拽槽或末块下方空白处按住左键拖拽（>4px），绘制半透明蓝色选框，命中的顶层块渲染 `6px` 圆角整块选区底色，支持 `Ctrl+C`、`Ctrl+X`、`Delete`/`Backspace` 批量删除及 `Esc` 取消。
- [x] **3.5 编辑器顶栏、可选格式栏、浮动选区菜单、页内查找与标题 (`editor_view.rs`)**：
  - 顶栏右侧严格对齐：`上次编辑`、右侧栏开关、`白板`、`共享`、`复制链接`（绿勾反馈）、`收藏星标 #F5B400`、`⋯` 页面菜单（`✓ 评论`、`✓ 属性`、`复制页面内容`、`导出(ZIP,含图片)`、`导出为 HTML`、`导出为 PDF`（桌面纯 Rust 导出为可打印 HTML 版本）、`导入 Markdown…`、`版本历史`、`恢复到上一个检查点`）。
  - 格式栏受设置「格式工具栏」开关控制，补齐 `链接`、`代码块`、`图片`；字数徽章改为右下角悬浮 `right:12 bottom:8`（0 字隐藏）。
  - 浮动选区气泡菜单扩充至原版全集（行内格式 + `链接` + `代码块` + `H1–H3` + `正文`/列表/待办/引用 + `用 AI 改写所选`）；新增 `Ctrl+F` 页内查找条。
  - 行内 `code` 样式修正为暖灰底 `#44423D`/`#D4D4D4`（非红色），光标颜色为 `accent` 蓝；支持 GFM Callout（`data.alert` 5 色左条与标题）与 `ImageNodeView` 真实图片渲染/对齐/缩放/说明。

---

## Domain 4：设置页 1:1 尺寸与 8 标签页、工作区菜单与概览/成员、导入导出与搜索

- [x] **4.1 设置对话框原版尺寸与 8 标签页壳 (`settings_view.rs` / `storage.rs`)**：
  - 尺寸从 `780×540` 还原为 **`1120×840`（`max_w 94%` / `max_h 92%`）**，通栏顶栏（`设置` + 关闭），`200px` 左侧分组导航（`通用`：外观/快捷键；`账户`：账户/API 令牌；`工作区`：数据/回收站；`其他`：AI 服务商/诊断；底部固定 `关于 v<ver>` 打开关于弹窗），右侧 `28px` 内边距可滚动详情页。
  - **外观**：语言（跟随系统/中文/English）、主题（跟随系统/浅色/深色）、界面字号（小/标准/大）、页面宽度 11 档滑块+重置、正文字号（13..22px）、字体（系统/Serif/Mono）、格式工具栏/页面标题开关、实时 `MicaAppearancePreview` 预览卡，持久化设置。
  - **快捷键**：完整 3 组 25 行表（左侧键帽胶囊）+ 平台/IME 说明。
  - **数据**：虚线 `MicaPickZone`（原生文件选择器导入 ZIP 工作区）、`.mica.zip` 与 Markdown `.zip` 导出、本机存储统计。
  - **回收站**：完整子树回收站面板（同 2.7）。
  - **账户 / API 令牌 / AI 服务商 / 诊断**：完整 1:1 表单与交互（云端接口已通过 `net::tests` 本地 mock HTTP 验证）。
- [x] **4.2 工作区切换菜单、设置弹窗、成员邀请与概览页 (`workspace_views.rs` / `sidebar.rs`)**：
  - 侧栏顶栏还原：`WorkspaceSelector` + `关系图` 按钮 + `工作区设置` 按钮。
  - `320px` 宽工作区下拉菜单（>8 项自动搜索过滤、页面数统计、每行 `⋯` 菜单：`重命名`、`导出(Mica 备份 / Markdown ZIP)`、`导入(文件 / 文件夹)`、`删除`；底部 `+ 新建工作区` + `导入工作区(ZIP / Notion)`）。
  - `440px` `工作区设置` 弹窗（角色、ID、存储用量、重命名、成员管理与邀请）+ `OverviewPane` + `HomeScreen`。
- [x] **4.3 真实可用导入导出与原生文件选择 (`transfer.rs` / `transfer_dialog.rs`)**：
  - 使用 `nsh_core::prelude::{pick_paths, pick_save_path}` 原生 OS 文件/目录/另存为对话框（零额外 `rfd` 依赖，保留 Headless/测试降级路径）。
  - 接入 `crates/interchange`（`mica-interchange`）实现真正的 `.mica.zip`（无损备份）与 `.zip`（Markdown + assets）单页/文件夹/整工作区导出，以及 `.mica.zip` / Markdown ZIP / Notion ZIP / 多 `.md` 文件 / 本地文件夹递归导入。
  - 实现 `440px` 跨工作区移动/复制弹窗 `TransferDialog` + `240px` `DestinationTreePicker`。
- [x] **4.4 搜索弹窗 1:1 对齐 (`search_view.rs`)**：`600×440` 尺寸、空输入态显示「其他工作区快捷芯片（最多 8 个）+ 最近编辑（最多 6 篇）」；搜索时顶部显示工作区名称匹配（最多 5 条）+ 固定 `82px` 3 行结果（含**文件夹搜索**与完整祖先路径、关键词高亮片段）；`↑/↓` 首尾循环、`Ctrl+Enter` 新标签打开、`initialQuery` 支持标签跳转。

---

## Domain 5：右侧工具栏、页面属性、版本历史 Diff、评论、分享、关系图谱与 AI

- [x] **5.1 右侧可调宽双标签工具栏 + 页底反向链接 (`right_sidebar.rs` / `editor_view.rs`)**：右侧栏默认 `300px`（`220..480px` 拖拽改宽，`surface.base`），支持 `大纲` / `评论` 双标签与顶部版本历史入口；`BacklinksPanel` 挂载在编辑器正文下方（0 条时完全隐藏）。
- [x] **5.2 页面属性面板 (`property_panel.rs` / `mica_rich_bridge.rs`)**：直调 L1 `mica_md::properties`（文本/数字/复选框/日期/标签列表 + 自动类型推断 + 点击标签跳 `#tag` 搜索 + 折叠摘要），持久化到根块 `data["front_matter"]`，由页面菜单 `✓ 属性` 控制显隐。
- [x] **5.3 980×640 双栏版本历史与块级 Diff (`version_history.rs`)**：左栏只读预览 + 块级 Diff 引擎（绿增/黄改/红删幽灵块 + 顶部图例），右 `300px` 按「今天/更早」分组的时间线 + `保存检查点` 命名弹窗 + `恢复` 二次确认框；本地走 `SqliteLocalStore`、云端走 REST。
- [x] **5.4 评论面板 (`comment_panel.rs`)**：右侧栏 `评论` 标签页（未解决优先、引用卡片点击定位、孤儿/已解决标记、行内回复、解决/重开、删除）与云端评论接口。
- [x] **5.5 公开分享弹窗 (`share_dialog.rs`)**：`440px` 弹窗、`公开访问` 开关、可复制 `/s/{token}` 链接框 + 绿勾反馈，对接 `/share` 接口。
- [x] **5.6 900×640 工作区关系图谱 (`graph_dialog.rs`)**：纯 Rust Fruchterman–Reingold 力导向布局（`nsh::Canvas` 绘制、缩放 `0.2x–4x`、平移、26px 命中测试、悬停连边高亮、点击打开页面、未链接页计数）。
- [x] **5.7 全局与行内 AI 弹窗 (`ai_dialog.rs`)**：`520px` 全局 AI 弹窗（新建页面 / 当前页面 / 新建工作区）、`/ai`、选区浮窗 `用 AI 改写所选`（`480px` + 4 预设 + 替换/插到下面）与代码块 AI 菜单（解释/优化/查错/自定义），对接云端 AI 接口。

---

## Domain 6：无限白板 1:1 飞书式布局与内嵌白板三路合并

- [x] **6.1 飞书式 50px 顶栏 + 左侧垂直 10 工具坞 + 图形/架构图标库飞出层 (`board_view.rs`)**：50px 顶栏（可编辑标题、Undo/Redo、Auto Layout、导入 draw.io/Markdown、导出 SVG/Markdown/JSON、节点连线计数、蓝色 `完成`）；左侧垂直 10 工具坞（`V/H` + 12 图形库飞出 + 20 技术架构图标库飞出 + `S/T/L/P/F/M`）；左下快捷键提示胶囊；右下缩放胶囊（`-` / `{zoom}%` 点击归 100% / `+` / `Fit`）。
- [x] **6.2 顶部居中上下文节点 & 连线检查器 + 完整画布绘制 (`board_view.rs`)**：拖拽/缩放/编辑时自动隐藏；节点模式 8 填充色 + 6 描边色 + 实虚线 + 字号 ±1.5 + 导图子/同级 + 置顶/置底 + 克隆 + 删除；连线模式 折线/曲线/直线 + 6 描边色 + 虚线 + 箭头 + 编辑标签 + 删除；补齐 8 缩放手柄、单选/悬停 4 锚点、导图折叠计数徽章、20 个架构图标矢量绘制、圆柱底部弧线、平滑贝塞尔自由画笔、画布世界坐标锚定多行编辑框、全部快捷键直通 `BoardCommand::KeyDown` 及 `UiEffect` 闭环。
- [x] **6.3 内嵌白板三路合并与主题同步 (`node_views.rs` / `atomic_blocks.rs` / `lib.rs`)**：点击文档内 `mica-board` 块的「编辑白板」打开对应块 JSON 的全屏白板弹窗，点「完成」时通过 `mica_board::merge_board_json` 三路合并写回该块；`render_board_card` 传入宿主机 `BoardSvgTheme`。

---

## 如实说明的平台与环境边界

1. **云端协作后端交互**：账户登录/注册、分享、评论、API 令牌、成员邀请、AI 流式生成等全部客户端 UI 与 HTTP 契约均已实现，并已在 `net::tests` 中针对本地 mock HTTP 服务器通过自动化测试；尚未连接线上生产 Mica Server 进行真实外网联调。
2. **PDF 导出**：纯 Rust 客户端未内嵌 Chromium/WebView2，页面菜单「导出为 PDF」会通过原生另存为导出可直接打印为 PDF 的独立 Warm-Paper HTML 文档。
3. **响应式极窄窗口（`<768px` 移动端抽屉）**：当前桌面客户端已完整支持宽/中屏响应式布局（如 World Switcher `>=900px` 分屏切换、Settings `max_w 94% / max_h 92%`），尚未专门适配移动端触摸抽屉。
