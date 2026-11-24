# Mica 产品功能与交互能力权威清单（Capabilities Spec）

> **定位**：与技术栈无关的 Mica 产品行为契约与验收规格库。  
> **基准来源**：综合 `main:clients/mica_flutter`（v0.14.11，291 个源文件沉淀的交互细节）、`crates/mica-core` / `md-core` / `mica-editor` / `mica-board` 引擎契约，以及纯 Rust 客户端（`clients/mica_rust`）演进过程中的实机回归。

## 为什么维护这份能力库

1. **把“肌肉记忆”显性化**：富文本编辑、剪贴板互操作、表格拖选、树拖拽等核心体验由数百条细微边界组成。仅靠源码记忆，一旦重构或更换 UI 技术栈，大量理所当然的细节会成片丢失。
2. **技术栈无关的验收基线**：无论未来继续深化 `nsh`，还是迁移/新增任何桌面、Web、移动端壳层，只要通过本目录所有 `P0` / `P1` 验收条款，即证明核心产品力零倒退。
3. **持续演进的唯一特性档案**：今后每一次**新增特性、交互优化、关键 Bug 根因修复**，都必须同步落入对应能力文档的条款、边界反例和演进记录中。

## 维护纪律

- **编号规则**：每条核心行为使用稳定 ID `CAP-<模块两位编号>-<序号>`（如 `CAP-01-01`），便于 Commit、Issue、测试用例和评审直接引用；编号只增不复用，废弃条款标明原因而非静默删除。
- **优先级定义**：
  - `P0`：核心不可破（阻塞正常读写、协作或导致数据/格式丢失，发版与重构前必须 100% 通过）。
  - `P1`：标准产品体验（显著影响高频编辑效率与观感）。
  - `P2`：进阶润色与平台适配（锦上添花或特定端增强）。
- **状态图例**（针对当前纯 Rust 客户端 `clients/mica_rust`）：
  - `✅`：已实现并通过自动化测试（或已实机确认）。
  - `🟡`：已实现并通过无头/单测，待真机/特定端（Web/移动）进一步复测或存在已知小边界。
  - `⬜`：规范已定义，待补齐实现。

---

## 能力总索引

### 一、文档编辑器核心交互（对照 `clients/mica_flutter/lib/editor/` 全量沉淀）

| 编号 | 能力文档 | 覆盖范围 |
| --- | --- | --- |
| 01 | [01-editor-caret-ime-typing.md](01-editor-caret-ime-typing.md) | 光标闪烁与命中、CJK IME 组合输入、打字与 Markdown 输入规则、`Enter`/`Backspace`/`Delete` 块级结构规则、首行回退标题、点击底部追加行 |
| 02 | [02-editor-selection-marquee.md](02-editor-selection-marquee.md) | 文本选区（单/双/三击）、跨块自由拖选与边缘滚屏、页边距与正文空白处框选（Marquee）、点击外区释放选区、多块批量操作 |
| 03 | [03-editor-clipboard-paste.md](03-editor-clipboard-paste.md) | 同应用无损 Markdown+HTML 复制粘贴、Windows `CF_HTML` 切片与跨应用（含 v0.14.11/浏览器/Word/Excel）粘贴、剪贴板截图/文件图片粘贴、粘贴围栏修复 |
| 04 | [04-editor-blocks-and-marks.md](04-editor-blocks-and-marks.md) | 段落/标题/嵌套列表/待办/单层引用与 5 种 GFM Callout/分割线、行内 marks、光标锚定 22 项 `/` 斜杠菜单、贴近选区的浮动气泡栏、页内查找 `Ctrl+F` |
| 05 | [05-editor-tables.md](05-editor-tables.md) | GFM 表格模型、内容自适应列宽与手动调宽、自由上下左右拖选多单元格、多格复制/剪切/清空、单格 WYSIWYG marks 编辑、键盘导航与行列增删 |
| 06 | [06-editor-code-blocks.md](06-editor-code-blocks.md) | 无框暖灰代码块与悬停工具条、顶层防遮挡语言选择器、换行/行号/标题/AI 助手、`min_rows(1)`、`(0,0)` 按 Backspace 删除代码块、大文档懒挂载 |
| 07 | [07-editor-media-math-mermaid.md](07-editor-media-math-mermaid.md) | 图片自然比例渲染/对齐/缩放/全屏灯箱/缓存、RaTeX 块级与行内公式（含原子选区保护）、Mermaid 宿主机主题渲染/全屏灯箱/源码编辑 |
| 08 | [08-editor-scroll-layout-virtualization.md](08-editor-scroll-layout-virtualization.md) | 长文档（141KB+/数千块）视口虚拟化与信号隔离红线、编辑期零跳滚策略（`Jump` vs `KeepVisible`）、窗口最右侧自适应比例滚动条、标题与正文间距、11 档对称拖拽页宽 |
| 09 | [09-editor-undo-crdt-collab.md](09-editor-undo-crdt-collab.md) | 单栈 CRDT 兼容撤销/重做、输入聚合、远端 yrs 更新合并保护本地未提交编辑、选区自动重定位（`SelectionReconcile`）、离线容错与断线重试 |

### 二、白板、应用外壳、知识组织与系统能力

| 编号 | 能力文档 | 覆盖范围 |
| --- | --- | --- |
| 10 | [10-whiteboard-and-embedded-board.md](10-whiteboard-and-embedded-board.md) | 飞书式无限白板（工具坞、图形/架构图标库、上下文检查器、思维导图自动布局、自由画笔、导入导出）与文档内嵌单层画板卡片、缩放手柄、三路合并写回 |
| 11 | [11-sidebar-tree-tabs-shell.md](11-sidebar-tree-tabs-shell.md) | 34px 无边框顶栏、应用菜单、浏览器式多标签与单标签历史、目录树 `folder` 唯一容器与防环、30/40/30 拖拽排序与跨层弹出、多选批量菜单、收藏、Emoji、子树回收站 |
| 12 | [12-page-chrome-outline-properties-backlinks.md](12-page-chrome-outline-properties-backlinks.md) | 页面顶栏与真实路径复制、页面标题联动、YAML Front-Matter 属性面板、右侧大纲/评论双标签、页底反向链接、980×640 双栏块级 Diff 版本历史与检查点 |
| 13 | [13-search-graph-and-ai.md](13-search-graph-and-ai.md) | 全局搜索（工作区/文件夹/页面 + 高亮片段 + 最近编辑）、力导向知识关系图谱、全局/行内/代码块 AI 改写与多服务商配置 |
| 14 | [14-workspace-cloud-sync-sharing.md](14-workspace-cloud-sync-sharing.md) | 本地离线 Vault 与多云端服务器共存（世界切换分屏）、工作区设置与成员邀请、公开分享 `/s/{token}`、API 令牌 |
| 15 | [15-import-export-and-vault.md](15-import-export-and-vault.md) | `.mica.zip` 无损备份、Markdown+图片 ZIP、Notion 导入、文件夹递归导入、HTML/PDF 导出、CommonMark/GFM 规范化往返不变量 |
| 16 | [16-settings-shortcuts-accessibility-mobile.md](16-settings-shortcuts-accessibility-mobile.md) | 8 标签设置中心与实时外观预览、全量快捷键体系、浅/深色主题与字号缩放、移动端触摸选区/抽屉导航/阅读优先适配 |

---

## 关联权威文档

- 架构与红线：[`AGENTS.md`](../../AGENTS.md)、[`docs/architecture/fullstack-rust-architecture.md`](../architecture/fullstack-rust-architecture.md)
- 快捷键详表：[`docs/features/shortcuts.md`](../features/shortcuts.md)
- Markdown 规范记分牌：[`docs/architecture/commonmark-scoreboard.md`](../architecture/commonmark-scoreboard.md)
- 历史开发教训：[`docs/dev/lessons.md`](../dev/lessons.md)
