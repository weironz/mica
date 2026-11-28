# 12 — 页面顶栏、属性、大纲、反向链接、评论与版本历史

> **适用范围**：桌面端、Web、移动端。

## 核心不变量

1. **面包屑复制的是可读路径，复制链接才复制 `mica://page/<uuid>`**：二者职责严格分离，绝不混用。
2. **属性面板直接读写文档根块 `front_matter`**：不另起旁路字段，所有类型推断与 YAML 读写由 L1 `mica_md::properties` 统一完成，导出 Markdown 自动带回 Front-Matter。
3. **版本历史只读预览与块级 Diff**：查看历史版本绝不污染当前活动编辑会话，只有用户点击「恢复」并经二次确认后才写入新版本。

---

## 能力与交互规格

### 页面顶栏与标题
- **`CAP-12-01` 可选中的面包屑与真实路径复制**：顶部左侧显示 `工作区 > 文件夹 > … > 页面标题`（严格依据页面真实父文件夹链向上推导，无嵌套文件夹时不回退重复拼接工作区名；支持鼠标划词选择文本），紧邻「复制页面路径」按钮复制人类可读的 `Workspace/Folder/...` 路径并给出绿勾反馈；右侧「复制链接」按钮复制 `mica://page/<uuid>`。
- **`CAP-12-02` 页面标题联动与 `新页面` 统一约定**：新建页面默认名称、空标题兜底与正文顶部 `30px SemiBold` 标题输入框的灰色占位符统一为 `新页面`（英文 `New page`），同时向下兼容云端或本地库中的历史 `"未命名页面"` / `"无标题"` / `"Untitled"` 占位值，点击标题即可直接输入无需先删默认字样；标题与侧边栏树节点、标签页标题、面包屑实时同步；按 `Enter` 焦点平滑跳入正文第 0 块（单 tick 回车去重防 Web 端插入多余空行），正文第 0 块偏移 `0` 按 `Backspace` 焦点回退到标题（详见 `01`）；受设置「页面标题」开关控制显隐。
- **`CAP-12-03` 页面 `⋯` 菜单（顶层浮层锚定）**：挂载在顶层 `overlay()` 并通过触发按钮位置锚定弹出，点击菜单外部空白、再次点击 `⋯` 按钮或按 `Esc` 立即自动收起；集中提供 `✓ 评论`、`✓ 属性`、「复制页面内容」、「导出(ZIP,含图片)」、「导出为 HTML」、「导出为 PDF」、「导入 Markdown…」、「版本历史」、「保存检查点」、「移入回收站」。

### 属性、大纲、反向链接与评论
- **`CAP-12-04` YAML Front-Matter 属性面板**：挂载在标题正下方（由 `✓ 属性` 开关控制）；支持文本、数字、复选框、日期、标签列表 5 类属性及自动类型推断；支持折叠为紧凑摘要；点击标签直接拉起全局 `#tag` 搜索。
- **`CAP-12-05` 右侧可调宽双栏边栏（大纲 / 评论）**：默认宽 `300px`（`220..480px` 可拖宽）；「大纲」实时提取文档所有 `H1–H6` 标题并按层级缩进，点击一键平滑跳转对应块；「评论」按未解决优先展示锚点评论卡片、支持引用定位、孤儿/已解决标记、行内回复、解决/重开与删除。
- **`CAP-12-06` 页底反向链接（Backlinks）**：挂载在正文末尾，自动列出工作区内通过 `[[WikiLink]]` 或 `mica://page/<id>` 指向当前页的文档，点击直接跳转；0 条反链时完全隐藏不占空间。

### 版本历史与检查点
- **`CAP-12-07` 双栏块级 Diff 版本历史**：`980×640` 弹窗，左侧展示只读预览与块级 Diff（绿增 / 黄改 / 红删幽灵块 + 顶部图例），右侧按「今天 / 更早」分组展示时间线；支持手动命名保存检查点与二次确认恢复。

---

## 历史踩坑与反例

- **反例 1（面包屑缺少父目录时回退拼接工作区名导致 `compute > compute > ...` 重复显示）**：根目录页面或树缓存尚未同步时错误将工作区名当作文件夹补进面包屑前缀。根治方案：严格按 `views` 中的真实 `parent_view_id` 链解析父文件夹（`breadcrumb_folder_resolver_never_falls_back_to_workspace_name`）。
- **反例 2（页面 `⋯` 菜单挂在局部条件节点下导致点击页面其他区域不会自动收起）**：早期未使用顶层全屏透明遮罩承接外点关闭，打开菜单后点击正文或空白处无法关闭。根治方案：统一挂入 `overlay()` 全视口可点击透明背景遮罩、配合按钮 `ElementRef` 锚定与 `Esc` 监听。

---

## 验收清单

| 编号 | 优先级 | 适用端 | 纯 Rust 状态 | 验证入口 |
| --- | --- | --- | --- | --- |
| `CAP-12-01` / `02` / `03` 页面顶栏、路径复制、标题联动与页面菜单 | P0 | 全端 | ✅ | `cloud_sync::tests::breadcrumb_folder_resolver_never_falls_back_to_workspace_name` + `app_flows::regression_recycle_bin_hidden_copy_path_appearance_and_board_click_edit` + `app_flows::regression_page_more_menu_dismisses_on_outside_click_anchor_and_escape` + `editor_view::tests::untitled_page_titles_render_empty_input_for_gray_placeholder` + `app_flows::regression_untitled_placeholder_folder_rename_and_batch_delete` |
| `CAP-12-04` Front-Matter 属性面板 | P1 | 全端 | ✅ | `property_panel::tests` |
| `CAP-12-05` / `06` 右侧大纲/评论双标签与页底反链 | P1 | 桌面/Web | ✅ | `comment_panel::tests` + `right_sidebar.rs` |
| `CAP-12-07` 双栏块级 Diff 版本历史 | P1 | 桌面/Web | ✅ | `version_history::tests` + `storage::tests::version_snapshots_increment_count_and_load_readonly_markdown` |

---

## 演进记录

- **2026-11-23**：沉淀页面顶栏路径复制、属性面板、右侧大纲/评论、反向链接与块级 Diff 版本历史规格。
- **2026-11-28（`1d28c14e` / `ecd2f266`）**：① 将页面右上角 `⋯` 菜单迁移至顶层 `overlay()` 并按 `ElementRef` 锚定定位，支持点击外部区域、再点按钮或按 `Esc` 立即收起（`CAP-12-03`）；② 修正面包屑父目录解析逻辑，根页面不再重复拼接工作区名（`CAP-12-01`）。
- **2026-11-28（`bbea5159` / `abfa6d86`）**：新建/未命名页面（`""` / `"新页面"` / `"未命名页面"` / `"无标题"` / `"Untitled"`）在正文顶部 `page-title-input` 中统一映射为空输入并显示灰色占位符 `新页面`，用户无需先删除默认文字即可直接键入正式标题；新建页面默认名称及清空标题兜底文案统一为 `"新页面"`（同时保留对历史 `"未命名页面"` / `"无标题"` / `"Untitled"` 的兼容识别，`CAP-12-02`）。
