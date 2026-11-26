# 11 — 应用外壳、多标签页、目录树与拖拽排序

> **适用范围**：桌面端、Web、移动端（抽屉适配见 `16`）。

## 核心不变量

1. **`folder` 是唯一容器，`page` 是叶子**：无论 UI 拖拽、右键菜单、API 还是本地 SQLite，`parent_view_id` 只能指向同工作区未删除的 `folder`，严禁 page 挂子节点或形成父子环；权威判定统一归属 `mica_core::tree_rules` 与数据库触发器（UI 预检只做交互提示）。
2. **单事务重排与排序步进**：树拖拽移动与同级排序通过 `reorder_views`（10 位补零步进 10）单事务提交，不产生乱序或重号。
3. **标签页与单标签历史隔离**：每个标签页维护独立前进/后退栈，点击侧栏页面默认在当前激活标签页内导航而非泛滥新开标签。

---

## 能力与交互规格

### 应用顶栏、应用菜单与多标签页
- **`CAP-11-01` 无边框 34px 统一顶栏、桌面应用图标与 M 应用菜单**：桌面端采用隐藏系统原生标题栏的无边框 34px 顶栏，左上角为 M 图标级联菜单（`文件`/`视图`/`历史`/`帮助` + 关于弹窗与检查更新）与侧栏折叠按钮，避开 Windows/macOS 系统窗口控制按钮区；Windows 可执行文件内嵌多尺寸（16–256px）整数资源 ID `1`（`RT_GROUP_ICON`）图标，保证任务栏、Alt+Tab、快捷方式与安装程序一致显示 Mica 标志。
- **`CAP-11-02` 浏览器式多标签页**：支持单标签上限 50 步前进/后退历史、标签最大宽 `200px` 横向滚动、关闭 `×` 仅悬停或激活显示且单标签时隐藏、关闭后聚焦右邻、`+` 弹出「新建页面 / 打开已有页面…」、重启持久恢复 `openTabs`、快捷键 `Mod+N`/`Mod+T`/`Mod+W`/`Mod+Tab`/`Mod+Shift+Tab`。

### 侧边栏目录树与多选
- **`CAP-11-03` 可调宽侧边栏与定位创建**：默认宽 `280px`、支持 `220..480px` 拖拽并持久化；行高 `30px`、层级缩进 `depth * 16px`、固定展开箭头列、Emoji/类型图标、悬停操作条；支持定位创建（文件夹内 / 页面下方 / 根目录）与重命名（含 `F2`）。
- **`CAP-11-04` 完整上下文右键菜单**：
  - 单页：新标签打开、重命名、设置图标、创建副本、移动/复制到工作区、删除。
  - 文件夹：新建子页/子文件夹、重命名、设置图标、创建副本、展开/收起子项、Mica 备份、导出 Markdown、导入文件/文件夹、移动/复制到工作区、删除。
  - 多选：批量移动/复制到工作区、批量删除确认。
- **`CAP-11-05` 30 / 40 / 30 拖拽排序、边缘自动滚动与跨层级弹出**：落点区按高度 `30%`（之前）/ `40%`（放入）/ `30%`（之后）划分；页面不支持 `into` 并显示明确禁止反馈；在 `after` 区域横向左拖可按祖先层级弹出（`reparent_level_for`）；支持底部空白区根拖放与多选堆叠预览卡；拖拽光标靠近或越过目录树视口上下边缘（`44px` 触发带）时以 60fps 平滑渐进自动滚动列表并实时刷新光标下方的落点指示线，松手 / `Esc` / 移出侧栏立即停止计时器。
- **`CAP-11-06` 目录树多选**：`Shift+Click` 同父范围连选、`Mod+Click` 独立切换、`Esc` 清空、折叠父节点自动裁剪不可见选中项；批量拖拽/删除前自动执行 `independent_selection` 去重子孙节点。

### 收藏、Emoji 图标与子树回收站
- **`CAP-11-07` 收藏区**：按收藏先后顺序持久化，独立限高滚动，显示页面自身 Emoji/文档图标，支持右键取消收藏。
- **`CAP-11-08` Emoji 图标选择器**：支持中英双语搜索（Enter 选首项）、7 分类网格与移除图标，目录树、收藏区、回收站统一渲染。
- **`CAP-11-09` 子树聚合回收站**：删文件夹时聚合为子树根条目（展示原路径、所含子文件夹/子页数、删除时间），支持一键恢复整棵子树、确认永久删除与清空回收站；入口统一收口于设置页（不冗余占用主侧栏底部常态高度，按用户偏好控制显隐）。

---

## 历史踩坑与反例

- **反例 1（page 当成容器或产生环）**：仅靠前端 UI 阻止还不够，必须在本地存储与服务端写锁内由 `mica_core::tree_rules` 最终兜底。
- **反例 2（多选同时选了父文件夹和子页面导致重复操作）**：未先做 `independent_selection` 去重，批量移动或删除时二次操作已移除的子节点报错。
- **反例 3（Windows 桌面端任务栏显示默认空白窗口图标）**：`nsh-platform-windows` 在注册 `WNDCLASSEXW` 时按整数资源 ID `1`（`MAKEINTRESOURCEW(1)`）查找可执行文件图标；若未嵌入 PE 图标资源或用了字符串资源名，任务栏与 Alt+Tab 会回退为 Windows 默认应用图标。

---

## 验收清单

| 编号 | 优先级 | 适用端 | 纯 Rust 状态 | 验证入口 |
| --- | --- | --- | --- | --- |
| `CAP-11-01` / `02` 无边框顶栏、桌面图标、应用菜单与多标签 | P0 | 桌面/Web | ✅ | `app_flows::flutter_parity_app_menu_world_switcher_graph_version_diff_and_width_grips` + `windows_app_icon` |
| `CAP-11-03` / `04` 侧边栏几何、右键菜单与定位创建 | P0 | 全端 | ✅ | `storage::tests::views_enforce_folder_parent_and_purge` + `app_flows` |
| `CAP-11-05` / `06` 30/40/30 拖拽排序与树多选 | P0 | 桌面/Web | ✅ | `tree_dnd::tests`（8 项全绿） + `storage::tests::reorder_views_renumbers_reparents_and_enforces_tree_rules` |
| `CAP-11-07` / `08` 收藏区与 Emoji 选择器 | P1 | 全端 | ✅ | `emoji_picker::tests` |
| `CAP-11-09` 子树聚合回收站 | P1 | 全端 | ✅ | `settings_view::tests::trash_root_entries_collapse_subtrees_and_resolve_live_path` + `storage::tests::trash_restore_promote_purge_and_build_trash_entries` |

---

## 演进记录

- **2026-11-23**：沉淀应用外壳、多标签页、目录树 30/40/30 拖拽、多选与子树回收站规格。
- **2026-11-25**：补齐 `clients/mica_rust` Windows PE 整数 ID `1` 图标资源嵌入（`build.rs` + `assets/app_icon.ico` + `mica.iss` `SetupIconFile`）及真机 Win32 `LoadIconW`/`LoadImageW` 回归测试。
- **2026-11-26**：补齐侧边栏目录树拖拽排序时的边缘自动滚动（`virtual_list` 挂载 `ElementRef` + `on_scroll`/`on_layout` 实时几何测量 + 16ms 渐进滚屏循环 + 滚屏期间落点指示线同步刷新与 RAII 取消守卫），解决长列表拖到视口外无法自动跟滚的问题。
- **2026-11-27**：修复云端删除复活与标题中间输入丢字——① `on_batch_trash` 改为单次原子 `POST /views/batch-trash`，`on_purge_all_trash` 接入 `DELETE /trash`；② `CloudSessionHandles` 引入 `views_refresh_gen` 代际丢弃与 RAII `PendingViewOps` 在途变更屏障，确保连续/批量删除期间收到的中间态 `views_changed` 延后到全部变更落库后再单次刷新，杜绝已删项复活；③ 页面标题输入改为 400ms 防抖 + 失焦/回车立即提交，并在聚焦或待提交期间屏蔽远端 `views_changed` 覆盖 `active_title`。
