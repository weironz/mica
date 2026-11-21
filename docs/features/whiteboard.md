# 无限画板与思维导图架构 RFC（方案 C）

> 状态：**架构 RFC（已完成 `spike/board-canvas` 关键风险实测，待评审与分期实施）** · 2026-11-22
> 关联探针：分支 `spike/board-canvas`（提交 `6510c776`）· [board_spike_canvas.dart](../../clients/mica_flutter/lib/board/board_spike_canvas.dart) · [board_spike_e2e.mjs](../../e2e/board_spike_e2e.mjs)

---

## 1. 背景、目标与刻意不选的方案

### 1.1 目标
在 Mica 中自研一套**飞书画板级别的无限画布 + 思维导图 + 实时多人协同**引擎，同时严格守住 Mica 的四条架构底线：
1. **零普通文档回归**：不增加普通 Markdown 页面的 Web 首帧体积、冷启动时间与输入延迟。
2. **Rust-first 单一数据面**：图元模型、CRDT 合并、布局算法、正交路由、文本抽取与 SVG 导出只有一份 Rust 实现（`crates/mica-board`），服务端、桌面 FFI、Web WASM 三端同构。
3. **离线本地优先**：桌面 SQLite 与 Web IndexedDB 原生离线可编辑，重连后经现有 `yrs` state-vector 自动合并。
4. **Markdown 开放格式不变量**：画板不成为封闭黑盒——导出为 Markdown 时自动产出独立可读的静态 SVG，并附带结构化 JSON 支持无损回导；思维导图支持与标准 Markdown 大纲双向互转。

### 1.2 刻意不选的方案与取舍

| 备选方案 | 为什么刻意不用 |
| --- | --- |
| **内嵌 Excalidraw / tldraw（WebView2 + `HtmlElementView`）** | 桌面端被迫引入 WebView2 与 React/JS 运行时，破坏轻量化安装包与离线统一存储；同步层需维护两套状态桥接，且难以向 AI / MCP 暴露干净的 Rust 原生图谱操作。 |
| **DOM + Canvas 混排（AFFiNE BlockSuite 模式）** | 强绑定浏览器 DOM 生态，桌面必须套 Electron；与 Mica 的 Flutter 桌面原生渲染路线背离。 |
| **将画板逻辑并入 `mica-doc-wasm` 或 `editor.dart`** | `mica-doc-wasm` 的 256 KiB gzip 硬预算仅剩 ~12 KB 余量；`main.dart` / `editor.dart` / `render.dart` 均受 `scripts/file-size-budget.txt` 只减不增棘轮约束。必须物理分包与按需加载。 |

---

## 2. 技术探针实测结论（`spike/board-canvas`）

在进入正式开发前，先在真实 **Flutter Web Release（CanvasKit）+ Playwright Chrome** 上对三条最高风险做了量化探针（500 节点 + 461 条连线，含中英文多行标签）：

### 2.1 实测数据汇总

| 验证维度 | 实测结果 | 架构意义 |
| --- | --- | --- |
| **Dart `deferred as` 懒加载分包** | 首屏下载 **0 个** `.part.js`；打开画板按需下载 `main.dart.js_1.part.js`，仅 **38.8 KB Brotli**（45.9 KB gzip / 131.1 KB raw） | 普通文档首帧包体积零污染 |
| **500 节点 100% 拖拽（含 5 条关联边）** | Optimized：**60.0 FPS**，build p50 **2.6 ms**，total p95 **9.2 ms**，静态层仅起止重绘 2 次；Naive：46.9 FPS，build p50 41.8 ms，total p95 49.9 ms | `ui.Picture` + `RepaintBoundary` 提速 **16×** |
| **500 节点 35% 全图同屏拖拽** | Optimized：**60.0 FPS**，build p50 **2.3 ms**，total p95 **7.4 ms**；Naive：47.1 FPS，build p50 40.3 ms | 即使全画板同屏依然满帧 |
| **500 节点 50% 连续平移** | Optimized：**60.0 FPS**，build p50 **2.4 ms**，**`ui.Picture` 0 次重建**；Naive：48.1 FPS，build p50 39.2 ms | 矢量 `ui.Picture` 缩放平移无须逐节点重绘 |
| **缩放态中文 IME（CDP `Input.imeSetComposition`）** | 45% / 100% / 200% 下，Chrome DOM `<textarea>` 坐标与节点内边距完全一致：**`(5.4, 4.5)` / `(12.0, 10.0)` / `(24.0, 20.0)`**，**0.0 px 漂移**；多行中文输入自动撑高节点（`68 → 106`），连线实时跟随，`Escape` 正常提交退出 | `Transform.scale` 方案可直接用于生产，原生 IME 候选框定位精准 |

### 2.2 探针挖出的隐藏深坑（已升级为架构红线）
- **CanvasKit 异步 CJK 字体切片导致缓存豆腐块**：Web 端中文 fallback 字体切片在首次 `TextPainter.layout()` 后异步下载；首版静态 `TextPainter` 缓存与 `ui.Picture` 未监听字体事件，导致 499 个未编辑节点永久显示 `□□□`，只有 `EditableText` 正常。
- **强制规范**：所有画板自绘文本缓存与 `ui.Picture` 必须订阅 `PaintingBinding.instance.systemFonts`，字体切片到达时清空文本布局缓存并重建静态 `ui.Picture`（复测已确认 500 节点全量中文矢量锐利）。

### 2.3 探针尚未覆盖、须在 Phase 1 前置补验的项
1. Windows 原生桌面端 + 搜狗/微软拼音在 45%/200% 缩放下的实机候选框位置与按键手感；
2. `crates/board-wasm` 带 `yrs` + 布局/路由后的实际 gzip-9 体积；
3. 超过 2,000 节点、含高分辨率贴图时的 Web CanvasKit 显存与分块策略。

---

## 3. 整体架构与 Crate / 模块边界

```text
                       crates/mica-board (纯 Rust，零 UI / 零网络)
       ┌──────────────────────────────────────────────────────────────────┐
       │ BoardDoc (yrs CRDT) · UndoManager · 空间索引 (R-Tree/Grid)       │
       │ 思维导图树排版 · 正交折线避障路由 · 纯文本抽取 · 无头 SVG 导出   │
       │ Markdown 大纲双向转换 · .mica-board.json 无损编解码              │
       └───────────────┬──────────────────┬──────────────────┬────────────┘
                       │                  │                  │
                       ▼                  ▼                  ▼
             服务端 / CLI / MCP      桌面端 (FFI)       Web 端 (按需 WASM)
             api-server/app-core  mica_flutter/rust     crates/board-wasm
             cli export/backup    (与 md/doc 同静态库)  /mica_board/board.{js,wasm}
             mcp-server 图谱工具                         (独立懒加载，0 B 首屏)
                       │                  │                  │
                       │                  └────────┬─────────┘
                       │                           ▼
                       │         clients/mica_flutter/lib/board/
                       │         (独立 Dart 库，经 `deferred as board` 按需分片)
                       │         · BoardController / SyncBoardReplica
                       │         · 双层 RepaintBoundary (ui.Picture + Overlay)
                       │         · 缩放态 EditableText 浮层 + systemFonts 监听
                       │         · 手势/工具栏/多选/对齐参考线/协同光标层
                       │                           │
                       ▼                           ▼
               WS Room 双通道              文档内嵌薄胶水 (block_renderers.dart)
          crdt_tx (可靠) + presence_tx    BoardEmbedRenderer (AtomicBlockRenderer)
            (有损高频，不踢慢客户端)       复用 RasterPreviewPipeline 异步光栅预览
```

### 物理隔离原则
- **严禁塞进现有启动包**：`crates/board-wasm` 独立构建（新增 `scripts/build-board-wasm.sh`），仅在首次打开画板或渲染画板内嵌预览时动态拉取，不碰 `mica-doc-wasm`（256 KiB）与 `mica-md-wasm`（288 KiB）预算。
- **严禁向三大棘轮库堆代码**：`clients/mica_flutter/lib/board/` 为独立 library，按子域拆文件（`model/`、`render/`、`interaction/`、`mindmap/`、`widgets/`）；`main.dart` / `editor.dart` / `render.dart` 只留最小路由入口与 `BoardEmbedRenderer` 注册。

---

## 4. 数据模型与 CRDT Schema（`BoardDoc`）

### 4.1 对象实体与树规则
- **Postgres 迁移**（如 `migrations/0040_view_board.sql`）：
  ```sql
  ALTER TYPE object_type ADD VALUE IF NOT EXISTS 'board';
  ```
- **树不变量不变**：`views.object_type = 'board'` 与 `'document'` 同为**叶子节点**，`folder` 仍是唯一容器。`mica_core::tree_rules`、`documents::ensure_parent_accepts_children`、DB 触发器 `views_parent_must_be_folder` 以及 `contracts/tree-rules.json` 两侧回放继续完全适用（仅在测试夹具中补充 `'board'` 作为叶子样本）。
- **零新增同步表与离线引擎**：画板复用现有 `documents`、`document_yrs_base`、`document_yrs_updates`（服务端）以及 `local_doc` / `doc_snapshot`（本地 SQLite / IndexedDB），天然获得增量同步、版本历史快照、权限控制、回收站与分享链接能力。
- **Yrs 根命名空间隔离**：`MicaDoc` 使用根键 `blocks` + `meta`；`BoardDoc` 使用根键 `board_meta` + `board_nodes` + `board_edges`，两类文档在 CRDT 字节层即可明确识别，永不误解析。

### 4.2 字段级 CRDT Schema（`yrs::Doc`，`OffsetKind::Utf16`）

```text
Doc (client_id = 设备持久化 53-bit ID)
├── board_meta: YMap
│   ├── schema_version: u32 = 1
│   ├── mode: "whiteboard" | "mindmap"        // 默认交互模式（两者可自由混排）
│   └── title: YText                          // 与 views.name 同步
│
├── board_nodes: YMap<node_id, YMap>          // 扁平存储，避免深层嵌套移动冲突
│   ├── kind: "rect" | "rounded" | "diamond" | "ellipse" | "sticky"
│   │         | "text" | "frame" | "group" | "image" | "mind_node" | "doc_card"
│   ├── x, y, w, h, rot: f64                  // 独立键：并发改色与拖拽互不覆盖
│   ├── z: String                             // Fractional Indexing 字符串（O(1) 调层且无数组冲突）
│   ├── parent_id: Option<String>             // 所属 Frame / Group / 思维导图父节点
│   ├── rank: Option<String>                  // 同父下子节点的分数索引顺序（思维导图/分组）
│   ├── collapsed: bool                       // 思维导图分支折叠态
│   ├── text: YText                           // 字符级 CRDT 文本（UTF-16，与 Dart 索引一致）
│   ├── style: YMap                           // fill / stroke / stroke_w / font_size / align / radius
│   └── ref: YMap                             // image -> {asset_key, nat_w, nat_h}; doc_card -> {view_id}
│
└── board_edges: YMap<edge_id, YMap>
    ├── from_id: String, from_anchor: "auto" | "t" | "r" | "b" | "l" | [fx, fy]
    ├── to_id: String,   to_anchor:   "auto" | "t" | "r" | "b" | "l" | [fx, fy]
    ├── routing: "curved" | "orthogonal" | "straight"
    ├── arrow_start / arrow_end: "none" | "arrow" | "diamond" | "dot"
    ├── manual_waypoints: YArray<[f64, f64]>  // 仅存用户手动拖过的拐点；自动避障路径运行时实时算，不落库
    ├── label: YText
    └── style: YMap
```

### 4.3 并发冲突与局部 Undo/Redo 语义
- **确定性冲突兜底**：Rust `BoardDoc::snapshot()` 读取视图时，如因并发编辑出现悬空 `parent_id`（父节点已被他人删除）或 `parent_id` 成环，**确定性提升为顶层节点**；悬空连线端点回退为最后一次已知锚点坐标或隐藏不崩溃。
- **局部撤销栈**：基于 `yrs::UndoManager` 仅追踪本机 `origin` 产生的事务（`capture_timeout_millis = 400`），避免 `Ctrl+Z` 撤掉协作者刚画的元素；一次拖拽操作在 `pointerUp` 时合并为单笔 CRDT 事务，构成 1 个原子 Undo 步。
- **文档内嵌卡片**：文本文档中新增原子块 `board_embed`（`props.board_id`、`props.frame_id?`、`props.height`）。普通文本文档自身 `Y.Doc` 不携带任何画板图元，保持轻量。

---

## 5. 实时协同协议与 WebSocket 双通道改造

### 5.1 现有瓶颈（必须先改）
当前 [rooms.rs](../../crates/app-core/src/rooms.rs) 中 `Room` 只有一条 `broadcast::channel(256)`，可靠的 `sync.push` 与高频 `presence.update`（单人 30 msg/s）共用该通道。画板场景下 10 人同时拖拽（250 msg/s）极易让慢客户端触发 `RecvError::Lagged`，继而在 [ws.rs](../../crates/api-server/src/routes/ws.rs) 收到 `client_out_of_date` 强制整页重拉，酿成重连雪崩。

### 5.2 改造方案（保持 `WS_PROTOCOL_VERSION = 1` 零破坏兼容）
1. **`Room` 双通道拆分**（`crates/app-core/src/rooms.rs` + `crates/api-server/src/routes/ws.rs`）：
   - `crdt_tx: broadcast::Sender<RoomMessage>`（容量 256）：仅传输可靠 `sync.update`；滞后时照旧发 `client_out_of_date` 触发增量补齐。
   - `presence_tx: broadcast::Sender<RoomMessage>`（容量 128）：传输 `presence.update` / `presence.leave`；`RecvError::Lagged(_)` 时**静默丢弃旧帧继续消费最新帧**，绝不踢连接、绝不影响 CRDT 同步。
2. **高频交互与持久化分层**：

| 阶段 | 传输通道 | 频率上限 | Postgres 写入 | 说明 |
| --- | --- | --- | --- | --- |
| 光标移动 / 框选 / 拖拽中预览 | `presence.update`（`presence_tx`） | 客户端节流 **20~30 Hz**，载荷 ≤ 4 KiB | **0 次** | 仅传 `{cursor:[x,y], selected:[ids], drag:{ids, dx, dy}}`，纯内存扇出 |
| 松手 `pointerUp` / 样式修改 / 增删图元 | `sync.push`（`crdt_tx`） | 每操作 **1 次** | **1 行** `document_yrs_updates` | 经现有 `current_write_permissions` 实时校验角色 |
| 节点内连续打字 | `sync.push`（`crdt_tx`） | 沿用 **400 ms** 客户端防抖 | 低频增量 | 与文本文档一致 |

3. **异步安全**：严格按 [异步评审清单](../dev/async-review-checklist.md) 校验——所有异步回写绑定 `(account, server, board_id, doc_version)` 代际，切页或切账号后丢弃过期回调；离线编辑先落本地 `doc_snapshot`，重连后通过现有 `sync.bootstrap` state-vector 自动合入。

---

## 6. 渲染管线、排版算法与交互红线

### 6.1 Flutter 自绘渲染架构（固化探针方案）
1. **双层 `RepaintBoundary` 隔离**：
   - **静态层**：所有非拖拽/非编辑中的图元与连线录制为矢量 `ui.Picture`，平移/缩放仅做一次 `canvas.translate/scale + drawPicture`，0 次 Dart 逐节点重绘。
   - **动态 Overlay 层**：仅绘制当前拖拽节点、选中句柄、对齐吸附参考线（Smart Guides）、关联连线、远程协作者光标与拖拽虚影。
2. **三项强制红线**：
   - **文本布局缓存 + `systemFonts` 失效**：节点 `TextPainter` 按 `(node_id, text, maxWidth, fontSize, fontFamily)` 缓存，必须订阅 `PaintingBinding.instance.systemFonts`，收到异步字体切片通知即失效并重建 `ui.Picture`。
   - **缩放态行内编辑**：使用 `Positioned(left: x*z + panX, top: y*z + panY)` + `Transform.scale(scale: z, alignment: Alignment.topLeft)` + 世界坐标系 `EditableText`，保持换行宽度与静态画布 1:1 一致、IME 候选框零漂移；文本增行实时撑高节点并驱动动态层连线重绘。
   - **超大图分级（> 2,000 节点）**：缩放倍率 `< 0.25x` 时自动跳过正文文本绘制（LOD 降级为色块骨架）；超大画板按世界坐标分块（Tile Picture）做视口裁剪。

### 6.2 Rust 侧核心算法（`crates/mica-board`）
- **思维导图自动布局**：支持变高子树的 Reingold-Tilford 树排版（向右展开 / 左右平衡 / 向下组织架构），`Tab` 加子节点、`Enter` 加同级、`Shift+Tab` 升级、拖拽任意分支实时预览目标父节点插槽；节点折叠后子树收起并显示计数徽章。
- **正交折线避障路由**：基于障碍物扩展包围盒的可视图 / A* 正交寻路，支持自动选择最近锚点，且**保留 `manual_waypoints`**——当节点拖动仅影响首尾段时，只重算首尾连接段，不粗暴覆盖用户手工调整的中间拐点。
- **空间索引**：统一在 Rust 维护 R-Tree / 均匀空间网格，提供 O(log N) 点击命中、框选查询、对齐吸附候选检索（±6 px 吸附到邻居边缘/中线）。

---

## 7. Markdown 开放格式、导出/备份、搜索与 AI/MCP

### 7.1 Markdown 兼容与无损导入导出
1. **独立画板（`views.object_type = 'board'`）**：
   - **导出**：Rust `crates/mica-board::export_svg` 无头确定性生成不依赖 `<foreignObject>` 的纯矢量 SVG（沿用 `merman` 思路，服务端/CLI 无需浏览器），输出：
     - `<画板名>.svg`（任何浏览器、GitHub、Obsidian 离线直接查看）；
     - `<画板名>.mica-board.json`（完整结构化数据，导回 Mica 100% 无损恢复可协同画板）；
     - 若 `mode == "mindmap"`，额外支持一键导出/复制为标准 Markdown 标题/无序列表大纲（反之亦然：把 Markdown 列表粘贴进画板直接生成思维导图）。
   - **备份**：现有 `mica-cli export` 周期导出 + Zerobyte 增量备份自动覆盖 `.svg` 与 `.mica-board.json`，无需改动备份链路。
2. **文档内嵌画板卡片（`board_embed` 块）**：
   - 导出为标准 CommonMark 图片 + 回导注释锚点：
     ```markdown
     ![系统架构图](assets/board-a1b2c3d4.svg)<!-- mica:board=550e8400-e29b-41d4-a716-446655440000 -->
     ```
   - 脱离 Mica 时就是一张普通 Markdown 图片；导回 Mica 时解析锚点并与包内 `assets/board-a1b2c3d4.mica-board.json` 重新绑定为可编辑画板。
3. **HTML 导出与公开分享页（Publish-to-web）**：
   - 服务端直接用 `crates/mica-board::export_svg` 输出内联 SVG（支持只读平移/缩放预览），无需后端启动浏览器。

### 7.2 全文搜索
- 服务端 `BodyIndex`（`crates/app-core/src/search.rs`）与本地 SQLite 索引通过 `crates/mica-board::extract_plain_text(&BoardDoc)` 提取所有节点正文与连线标签写入现有 `content_text`。用户在全局搜索框输入关键词，可直接搜到架构图与思维导图里的文字并高亮定位到对应节点。

### 7.3 AI / MCP 原生图谱读写（Mica 核心差异化）
因为画板底层在 Rust 是干净的结构化图模型（而非黑盒像素），`crates/mcp-server` 与内置 AI 可直接提供语义级工具：
- `board_read`：返回精简 JSON 拓扑（节点列表、层级、连线关系、Frame 分组），极省 Token；
- `board_apply_ops`：按批量原子指令增删改节点/连线/样式并自动触发思维导图或正交重排；
- `markdown_to_mindmap` / `mindmap_to_markdown`：一键把一篇长文档转为可协同编辑的思维导图，或把头脑风暴后的思维导图沉淀为结构化文档大纲。

---

## 8. 性能、体积与质量门禁预算

以下指标纳入 CI 与发布门禁，任一项退化即阻断合并：

| 门禁项 | 硬指标 | 验证手段 |
| --- | --- | --- |
| `mica-doc-wasm` 体积 | 维持 **≤ 256 KiB** gzip-9（+0 B） | 现有 `scripts/build-doc-wasm.sh` |
| `mica-md-wasm` 体积 | 维持 **≤ 288 KiB** gzip-9（+0 B） | 现有 `scripts/build-md-wasm.sh` |
| `board-wasm` 独立体积 | **≤ 256 KiB** gzip-9（普通页面 0 B） | 新增 `scripts/build-board-wasm.sh` |
| Dart `board` deferred 分片 | **≤ 250 KB** Brotli（首屏 0 请求） | 扩展 `e2e/deferred_modules_e2e.mjs` |
| 500 节点 Web CanvasKit 帧率 | 拖拽/平移 **≥ 55 FPS**，build p50 **≤ 5 ms**，拖拽中静态层 **0 重绘** | Playwright 性能门禁 + Widget 测试 |
| 缩放态 IME 对齐 | 45% / 100% / 200% DOM 输入框漂移 **≤ 1 px**，组合态提交无丢失、无游离换行 | Playwright CDP IME 回归 |
| 代码结构门禁 | `main.dart` / `editor.dart` / `render.dart` 不涨破棘轮；无 Dart 侧 Markdown 语法泄漏 | `just check-structure` |

---

## 9. 分期实施路线图

严格按 **3 个独立可用里程碑**推进，每期交付即可上线使用，避免长周期大爆炸集成：

### Phase 1：MVP 闭环（思维导图 + 基础白板 + 同步 + Markdown/SVG 导出）
- **Rust 核心**：新建 `crates/mica-board`（`BoardDoc` CRDT Schema、局部 `UndoManager`、空间网格、思维导图树排版、纯文本抽取、确定性无头 SVG 导出、`.mica-board.json` 与 Markdown 大纲互转）+ `crates/board-wasm` + 桌面 FFI 绑定。
- **后端与存储**：新增 `migrations/0040_view_board.sql`（`object_type` 加 `'board'`）、更新 `contracts/tree-rules.json`、接入 `BodyIndex` 搜索与 `mica-cli` 导出/备份。
- **客户端（`lib/board/`，经 `deferred` 懒加载）**：
  - 侧栏新建画板 / 思维导图入口；
  - 无限画布平移缩放 + 基础图元（矩形/圆角/便签/文本/直线与贝塞尔连线）+ 思维导图键盘流（`Tab` / `Enter` / 折叠 / 拖拽重挂）；
  - 探针已验证的双层 `RepaintBoundary` + `systemFonts` 刷新 + 缩放态中文 IME 编辑器；
  - 文档内嵌 `BoardEmbedRenderer`（卡片预览 + 点击进入全屏画板）。
- **退出条件**：桌面端（含 Windows 真实输入法）+ Web 端离线/在线编辑、同步、Markdown+SVG 无损导出回导全绿，所有体积与帧率门禁通过。

### Phase 2：飞书画板核心体验 + 实时多人协同
- **WS 双通道改造**：拆分 `Room` 的 `crdt_tx` / `presence_tx`，上线多人实时光标、选区框、拖拽虚影预览（20~30 Hz，0 DB 写入）。
- **高阶画布能力**：正交折线自动避障（含 `manual_waypoints` 保留）、多选框选、旋转与等比缩放、对齐吸附参考线（Smart Guides）、分组（Group）、Frame 容器（支持仅导出单个 Frame）、图片粘贴上传、自由手绘（贝塞尔平滑笔迹）。
- **退出条件**：10 并发模拟协作者压测下无 `client_out_of_date` 雪崩、无 Postgres 写入风暴；协同 Undo/Redo 隔离回归全绿。

### Phase 3：Mica 差异化杀手锏（AI / MCP + 文档画板互嵌）
- **MCP 与内置 AI 图谱能力**：上线 `board_read`、`board_apply_ops`、`markdown_to_mindmap`、`mindmap_to_markdown`，支持 AI 直接根据文档生成/修改架构图与思维导图。
- **双向富卡片**：画板内插入 `doc_card` 实时预览 Mica 文档；文档内按 `frame_id` 精准嵌入画板的某个局部区域；公开分享页支持只读画板交互缩放浏览。

---

## 10. 开放决策（开干 Phase 1 前待拍板）

1. **思维导图与自由白板在侧栏的入口形态**
   - **推荐**：底层统一为 `views.object_type = 'board'`（同一份 `BoardDoc`，自由画布里随时可以插入思维导图，思维导图外围也可自由贴便签），仅在侧栏“新建”菜单提供「新建画板」和「新建思维导图」两个预设模板入口（设置 `board_meta.mode` 与初始根节点）。
2. **本地 Vault 模式（纯本地 Markdown 目录）下画板的存放位置**
   - **推荐**：与图片附件同级存放，即 `.svg`（供 Markdown 渲染）+ 同名 `.mica-board.json`（供 Mica 打开编辑），保持目录干净且对 Git 友好。
3. **Phase 1 启动前是否先补一轮 Windows 原生桌面端中文输入法实机手测**
   - **推荐**：是——在 `spike/board-canvas` 直接起一次 `flutter run -d windows` 验证搜狗/微软拼音候选框跟随，确认桌面与 Web 两端均无坑后再正式落 Rust crate。
