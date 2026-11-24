# Mica 全栈 Rust 三层标杆架构设计方案与分阶段开发计划

> **状态**：架构路线定稿，分阶段实施启动（Phase 0 就绪）· 2026-11-22
> **开发分支**：`feat/fullstack-rust-architecture`（基于 `main @ 8b07d76d`）
> **跟踪 Issue**：[#172](https://github.com/weironz/mica/issues/172) · **前序调研**：[ui-gpui-evaluation.md](ui-gpui-evaluation.md)

---

## 1. 架构愿景与不可退让红线

### 1.1 愿景定位

将 Mica 打造为开源界首个 **全栈 Rust、跨平台（正式切换交付 Windows 桌面、Web 与可选 Linux `.deb`）、Local-First CRDT、富文本与无限画板一体化** 的标杆知识库架构。

> **平台范围决定（2026-11-24）**：正式切换至全栈纯 Rust 客户端时，提供 **Windows 桌面**、**Web（WebAssembly）** 以及现有的可选 **Linux `.deb`** 构建；**不再提供 Android 与 macOS 桌面安装包**。原有 0.14.x Android 安装包仍可继续使用，但不接收新功能或更新。

核心工程哲学：**「三层全栈 Rust 分层 + 路线二-B（主权在我）」**——

- 不把业务、编辑器大脑或文档模型写死在任何 0.x UI 框架里；
- 让 **85%+ 的核心价值沉淀在零 UI 依赖的纯 Rust 内核（L1 + L2）** 中；
- 让 UI 层（L3）薄到极致、可插拔、可灰度对拍替换，最终在全端验证达标后整体退役 Flutter 与跨语言桥接层。

### 1.2 四条不可退让红线

1. **绝不再造第二套文档表示**（见 [开发教训 §1](../dev/lessons.md)）：客户端编辑态直接操作 `mica-core` 的 `MicaDoc`（`yrs` 扁平块模型），拒绝引入 ProseMirror 树等第二套文档表示做双向转译。
2. **Markdown Round-Trip 与单源引擎不变量不退化**：解析、序列化、HTML、代码高亮继续严格单源归属 `crates/md-core`（[规范记分牌](commonmark-scoreboard.md)）。
3. **中文 IME、大文档性能与无限画板体验零倒退（2026-11-24 范围修订）**：Windows 桌面（微软拼音 / 搜狗）与 Web（Chrome / Firefox / Safari）中文输入法必须达标；2,000 块含表格/公式文档与 500 节点画板维持 60 FPS；正式切换范围定为 Windows 桌面、Web 与可选 Linux `.deb`（不提供 Android 与 macOS 桌面，不再强制要求移动端触屏编辑或 Android 真机验收）。
4. **渐进演进、拒绝大爆炸重写**：每个 Phase 均可独立测试、合入并产生复利收益，不存在“停工半年赌一个 0.x 框架”的死谷。

---

## 2. 代码库现状解剖（2026-11-22 `main @ 8b07d76d` 实测）

| 层级 / 目录 | 文件数 | 行数 | 当前状态与归属判定 |
| --- | ---: | ---: | --- |
| **`crates/`（L1 纯 Rust 数据与引擎）** | 260 | 102,244 | 已建成核心壁垒：`api-server` (34.9k)、`md-core` (15.6k)、`mica-core` (12.4k)、`app-core` (9.7k)、`interchange` (6.3k)、`cli` (5.9k)、`mcp-server` (5.3k)、`markdown` (4.9k)、`mica-board` (3.8k)、`infra` (2.7k)、`md/doc/board-wasm` + `api-types` (0.8k) |
| **FFI 跨语言桥接层（冗余负债）** | 24 | ~21,936 | `clients/mica_flutter/rust/src/` (10,763 行，含 `frb_generated.rs` 7,085 行) + `lib/src/` (11,173 行 FRB 生成代码)。**全栈 Rust 落地后可 100% 删除** |
| **被困在 Dart 的 L2 大脑（迁移主战场）** | ~125 | ~19,500 | 编辑器状态机 (`controller.dart` 3,561 + `marks/model/table/history/paste_repair/selection_reconcile` ~4,000)、画板状态 (`board_controller.dart` 1,330 + `board_models.dart` 1,005)、离线同步与本地存储 (`cloud/` 3,384 + `sync/` 858 + `world/` 1,611 + `api/` 3,694) |
| **Flutter L3 视图与平台壳** | ~150 | ~61,000 | `editor/render.dart` + `block_renderers.dart` + `editor.dart` Widget/IME 部分、`board_painter/chrome/screen`、`ui/` (21.3k)、`main.dart` 外壳 (12.8k) |

**关键结论**：

1. L1 已经完全在 Rust 中成型，无需重做。
2. 阻碍 Mica 成为全栈 Rust 标杆的真正瓶颈，不是“缺一个 UI 框架”，而是 **~1.95 万行 L2 客户端交互大脑与 ~6.1 万行 L3 绘制代码在 Dart 中深度耦合**，并拖带了 **~2.19 万行跨语言 FFI 桥接胶水**。

---

## 3. 教科书级三层全栈 Rust 架构（L1 / L2 / L3）

```text
┌──────────────────────────────────────────────────────────────────────────────┐
│ L3 · 薄呈现壳（Presentation Shell，唯一绑定 UI 框架的层，目标 ≤20% 代码量）     │
│  职责：窗口/导航布局 · 主题样式 · OS 级中文 IME 与选区把手 · 原子块/Canvas回放  │
│  · 过渡壳（Phase 1–2）：clients/mica_flutter（逐步瘦身为纯视图代理，保四端可用）│
│  · 目标纯 Rust 壳（Phase 3）：clients/mica_rust（路线二-B：基于 NSH 原生壳）    │
│  · 可选扩展壳：桌面旗舰纯 GPUI 壳 / 终端 TUI（复用同一 L2，架构决策可逆）       │
├──────────────────────────────────────────────────────────────────────────────┤
│         ↑ EditorPatch / BoardFrame / SyncEvent   │ Dispatch(Command) ↓       │
├──────────────────────────────────────────────────────────────────────────────┤
│ L2 · 无头客户端大脑（Headless Client Core — 100% 纯 Rust，零 UI/平台依赖）     │
│  · crates/mica-editor   : 无头块编辑器内核（直连 MicaDoc · 选区代数 · 列表/表格 │
│                           块级撤销栈 · 输入规则 · 粘贴修复 · 脏块 PatchDiff）  │
│  · crates/mica-board    : 扩展 BoardSession 交互态（拖拽/吸附/框选/缩放/撤销）  │
│  · crates/mica-sync     : 统一离线存储与同步引擎（DocKvStore trait：桌面SQLite │
│                           / Web 极薄 OPFS-IDB，消灭 web_idb_doc_store 双实现） │
│  · crates/mica-client   : 工作区/页树 ViewModel · HTTP/WS 传输 · AI/搜索编排   │
├──────────────────────────────────────────────────────────────────────────────┤
│ L1 · 纯数据面与无头引擎（Domain & Engines — 已建成 10.2 万行纯 Rust）           │
│  · mica-core (yrs CRDT, SQLite store, tree_rules)                            │
│  · md-core / markdown (CommonMark/GFM/HTML/高亮)                              │
│  · mica-board (模型/导图布局/A* 路由/draw.io/无头 SVG)                         │
│  · merman (Mermaid) · ratex-* (公式) · mica-api-types · interchange          │
│  · api-server · app-core · infra · mica-cli · mica-mcp                       │
└──────────────────────────────────────────────────────────────────────────────┘
```

### 3.1 为什么选择「路线二-B：主权在我」作为 L3 目标方案？

截至 2026-11-22，对比 Rust UI 生态的三种路线：

| 方案 | 渲染与输入机制 | 对 Mica 约束的适配结论 |
| --- | --- | --- |
| **纯 GPUI / `gpui-kit`** | 全平台 GPU 自绘，框架自管排版与 IME | **否定作为全平台唯一壳**：无官方移动端；Web 产物 ~18 MB + 缺中文字体 + 隐藏 textarea 候选窗问题尚未在默认后端收口。可作为未来 L2 之上的可选桌面旗舰壳，不能当全端唯一底座 |
| **路线二-A：直接用 `nsh::RichEditor`** | `longbridge/nsh` 自带的 ProseMirror Rust 移植编辑器 | **不作为主编辑器**：ProseMirror 树（UTF-16 偏移）与 Mica `yrs` 扁平块 `MicaDoc` 存在双表示阻抗失配（违背红线 1）；且上游尚未实现视口虚拟化（`nsh #2376` 仍 open） |
| **路线二-B（选定）：自有 `mica-editor` + NSH 原生虚拟 `List` + 原生文本输入代理 + `nsh::Canvas`** | **L2 `mica-editor` 掌控文档与编辑主权；L3 由 `longbridge/nsh` 纯 Rust (`rust_app!`) 提供全平台原生控件、OS 级中文 IME 与 `Canvas` 2D 加速** | **唯一同时满足全部约束的路线**：① 单一文档模型（直连 `MicaDoc`）；② 天然视口虚拟化（复用 NSH 已验证 50,000 行的变高虚拟 `List`）；③ 桌面（Win32/AppKit/GTK4）、移动（UIKit/Android View）、Web（原生 HTML DOM）全部吃透平台原生中文 IME 与手势菜单；④ `nsh::Canvas` 原生承接 `mica-board` |

### 3.2 关键边界切分：L2 与 L3 各自拥有什么？

#### 编辑器边界（`mica-editor` ↔ L3 视图）

| 能力域 | 归属层 | 设计理由 |
| --- | --- | --- |
| 文档块数组、`MicaDoc` CRDT 读写、脏块 Revision | **L2** | 单一权威态，杜绝跨语言复制与双表示漂移 |
| 选区表示（`BlockId + utf16_offset`）、折叠/范围/跨块/单元格选区代数 | **L2** | 纯确定性算法，微秒级无头回归测试覆盖 |
| 全部编辑命令（插入、删除、Enter 拆分、Backspace 合并、Tab 缩进、ToggleMark、表格行列增删、移动块、粘贴/复制、撤销/重做） | **L2** | 规则集中，零 UI 耦合；直接调用 `md-core` 做粘贴/复制编解码 |
| 键盘输入规则（`# ` 变标题、`- ` 变列表、`**x**` 加粗等） | **L2** | 规则进入 Rust 后，彻底退役 Dart 侧 `md-syntax-ok` 豁免与 `check-dart-markdown.sh` |
| 远端 CRDT 合并后的选区重映射（`selection_reconcile`） | **L2** | 依赖 `yrs` 差量与 `utf16_text_diff`，纯 Rust 更准确 |
| IME 组合态（`composing_range` / `preedit`）、系统候选窗位置、移动端长按选词菜单 | **L3** | 必须紧贴操作系统 / 浏览器输入协议；L3 在预编辑期间仅渲染局部叠加态，提交（commit）时一次性发 `EditorCommand::CommitComposition` 进 L2 |
| 字体字形排版、断行坐标测量、像素级命中测试（`offset_for_point` / `rect_for_offset`） | **L3** | 依赖系统文本引擎（DirectWrite / CoreText / Pango / StaticLayout / DOM / Flutter TextPainter） |
| 原子块渲染（表格外观、公式光栅、Mermaid SVG、`mica-board` 卡片、图片/附件） | **L3** | 通过统一渲染器注册表消费 L1/L2 产出的矢量/位图/状态快照 |

#### 画板边界（`mica-board` `BoardSession` ↔ L3 Canvas）

- **L2（`crates/mica-board`）**：持有权威 `BoardSnapshot`（及可选 `BoardDoc` CRDT）、选中集合、视口变换（pan/zoom）、交互状态机（`Idle` / `DraggingNodes` / `Marquee` / `Resizing` / `ReconnectingEdge` / `FreehandStroke`）、对齐吸附线计算、A* 路由缓存、导图快捷键布局、命令撤销栈；每次指针/键盘事件返回极简 `BoardPatch`（更新的节点/边、参考线、是否需要重绘静态底图 vs 仅重绘交互 Overlay）。
- **L3（Flutter `CustomPainter` / 未来 `nsh::Canvas`）**：仅做双层绘制（静态图元缓存 + 动态 Overlay）与缩放态文本编辑叠层。

#### 离线同步与本地存储边界（`mica-sync` ↔ 平台存储/网络）

- **L2（`crates/mica-sync`）**：定义极简异步/同步存储 trait `DocStoreBackend`（快照、待发 outbox、远端日志、游标、附件队列）与同步状态机（去重、退避重试、冲突 3-way/CRDT 合并、账号/会话代际校验——严格落实 [异步评审清单](../dev/async-review-checklist.md)）。
- **平台适配（极薄 ≤200 行）**：
  - 桌面 / 移动原生：直接实现于 `mica-core::LocalStore`（SQLite）；
  - Web：实现极薄的 IndexedDB / OPFS KV 适配器，彻底删除 `web_idb_doc_store.dart`（632 行）的业务逻辑复制。

---

## 4. 核心接口契约与工程门禁设计

### 4.1 `crates/mica-editor` 核心 API 草案

```rust
pub struct EditorSession {
    doc: MicaDoc,              // 唯一权威 CRDT 文档（无第二套表示）
    view_cache: Vec<BlockView>,// 扁平只读视图缓存，按 block_id 跟踪 revision
    selection: Option<DocSelection>,
    history: HistoryStack,     // 块级/文本合并撤销栈（与 CRDT 前向操作兼容）
    generation: u64,           // 会话/文档代际，防异步竞态回写
}

pub enum EditorCommand {
    InsertText { text: String },
    CommitComposition { replace_range: TextRange, text: String },
    DeleteBackward { granularity: DeleteGranularity },
    DeleteForward { granularity: DeleteGranularity },
    SplitBlock,
    SetSelection(Option<DocSelection>),
    ToggleMark(MarkKind),
    Indent { delta: i8 },
    TransformBlock { block_id: BlockId, target: BlockKind },
    TableOp(TableCommand),
    Paste { plain: String, html: Option<String> },
    CopySelection,
    Undo,
    Redo,
    ApplyRemoteUpdate(Vec<u8>),
    FlushPendingCommit,
}

pub struct EditorPatch {
    pub content_revision: u64,
    pub dirty_blocks: Vec<BlockPatch>, // Insert / Update / Remove / Reorder
    pub selection: Option<DocSelection>,
    pub crdt_update: Option<Vec<u8>>,  // 待落盘/广播的 yrs 增量
    pub clipboard_out: Option<ClipboardPayload>,
    pub effects: Vec<EditorEffect>,    // ScrollIntoView / OpenSlashMenu / Fault 等
}
```

**关键性能约定**：

- 过渡期（Flutter 壳仍在使用时）：`dispatch` 在桌面走单次 FFI 调用，在 Web 走 `editor-wasm` 单次扁平字节/紧凑结构返回，**禁止每敲一个字做多次跨界 JSON 全量序列化**。
- 终局期（`clients/mica_rust`）：进程内零拷贝 Rust 直接调用，单次敲字 L2 耗时目标 `< 0.5 ms`。

### 4.2 过渡期文件大小与双实现门禁治理

1. **巨石文件棘轮自然收敛**：随 Phase 1–2 推进，[scripts/file-size-budget.txt](../../scripts/file-size-budget.txt) 中的 `main.dart`（11,675 行）、`editor.dart`（6,594 行）、`render.dart`（5,501 行）只减不增，每完成一个里程碑同步收紧预算基线。
2. **新增纯 Rust crate 遵守既有规则**：单文件硬限 8,000 行、软限 4,000 行；严格遵循 [AGENTS.md](../../AGENTS.md) 依赖边界，L2 新增 crate 零第三方重依赖（复用 workspace 既有 `yrs`、`serde`、`uuid`、`thiserror` 等）。
3. **测试回执不放松**：所有新 crate 纳入根 `Cargo.toml` workspace 与 `just ci rust`；过渡期每次合并均须通过完整 Rust + Flutter + 浏览器 E2E 门禁。

---

## 5. 分阶段开发计划（Phase 0 → Phase 3）

### 阶段总览

| 阶段 | 核心目标 | 预计周期 | 依赖外部条件 | 独立收益 |
| --- | --- | --- | --- | --- |
| **Phase 0** | 本机实证与 L3 底座定标（Windows IME + 纯 Rust Web + 路线二-B 探针） | 1~2 周 | 无（本机立即可做） | 消除 L3 全部关键未知数，拿到实测数字 |
| **Phase 1** | 统一离线同步 `mica-sync` + 画板交互态 `BoardSession` 收口 | 4~6 周 | 无 | 消灭最后双存储实现 + 删除 `board_models.dart` 镜像 |
| **Phase 2** | 建成无头编辑器内核 `mica-editor` + 客户端内核 `mica-client` | 8~12 周 | 无 | **达成 85%+ 全栈 Rust 标杆内核**，Flutter 彻底沦为薄壳 |
| **Phase 3** | 落地纯 Rust 全平台 UI 壳 `clients/mica_rust` 并退役 Flutter | 8~16 周 | `nsh-compat` 冻结 + Phase 0 四端全过 + Phase 2 完成 | **100% 全栈 Rust 全平台**，删除 Flutter 与 2.2 万行 FFI 胶水 |

> **关键设计**：Phase 1 与 Phase 2 **完全不依赖 `nsh` 是否发布 1.0**。即使上游冻结时间延后，Mica 依然按节奏把整个客户端大脑 100% Rust 化，随时处于“可换壳”的临战状态。

---

### Phase 0：本机实证与底座定标（1~2 周）

**目标**：在当前 Windows 真机上补齐 2026-11-10 云端 Linux 未能完成的关键验证，用一手实证钉死 L3 底座可行性。不改动主线业务代码（探针放独立隔离目录 `spikes/` 或临时分支，不污染生产构建）。

- **P0.1 Windows 真人中文 IME 实测**
  - 对象：`longbridge/nsh v0.6.1`（Win32 + Direct2D 后端）与 `longbridge/gpui-kit v0.10.0`（含 `gpui-fast` 开关）。
  - 验收项（真人使用微软拼音 / 搜狗输入法）：拼音连续组字、候选窗严格跟随光标、Esc 取消不留残字、退格删拼音不误删正文、快速中英混打无重复上屏、Ctrl+Z/C/V 富文本与纯文本剪贴板。
- **P0.2 NSH 纯 Rust Web 探针（不启 `browser-core`）**
  - 编译最小 `rust_app!` 到 `wasm32-unknown-unknown`（`opt-level=z` + Brotli 11），实测真实传输体积（目标验证能否压进合理预算，对比现行 Flutter CanvasKit 2.6 MB Brotli）；
  - 在 Chrome 与 Firefox（复验 `#582` `nsh:rt` importmap 修复）实测加载、中文 IME 候选窗与剪贴板。
- **P0.3 路线二-B 核心机制探针**
  - 在 NSH 变高虚拟 `List` 中挂载 2,000 个扁平块 + 1 个 `nsh::Canvas` 500 节点画板卡片；
  - 实测：① 首屏挂载耗时与 2,000 块打字 p50/p95 延迟（对比 Flutter 33 ms 基线）；② 聚焦块编辑与跨块键盘/鼠标选区原型；③ 画板平移/缩放 60 FPS 与缩放态中文输入；④ 若具备设备条件，补充 Android 真机与 Safari 实测。
- **P0 退出交付物**：实测报告归档至 `docs/reviews/fullstack-rust-p0-spike-<date>.md`，同步更新 [ui-gpui-evaluation.md](ui-gpui-evaluation.md) 第 8 节与 Issue #172。

---

### Phase 1：统一本地存储与画板交互收口（4~6 周）

**目标**：先挑边界最清晰、风险最低的两块 L2 大脑开刀，跑通“Rust 状态机 + Flutter 薄视图”的工程范式。

#### M1.1 `crates/mica-board` 交互态 `BoardSession` 收口（2~3 周）

- 在 `crates/mica-board` 新增 `session` 模块（`BoardSession`、`BoardPointerEvent`、`BoardKeyCommand`、`BoardPatch`）：
  - 迁入选择集管理、多选拖拽 + 对齐参考线吸附（保持 `29b13211` 无粘滞优化）、8 控制点缩放、框选（Marquee）、连线端点吸附重连、自由画笔采样简化、视口 pan/zoom 坐标换算、命令撤销重做栈、思维导图快捷键（`Tab` / `Enter` / `Shift+Tab` / 折叠 / 拖拽重挂）。
- 更新 `clients/mica_flutter/rust/src/api/board.rs` 与 `crates/board-wasm` 暴露会话接口。
- 瘦身 `clients/mica_flutter/lib/board/`：
  - 删除 `board_models.dart`（1,005 行）的大部分业务模型复制，保留极薄渲染 DTO；
  - `board_controller.dart`（1,330 行）收缩为事件转发 + 重绘通知薄代理（目标 ≤350 行）。
- **验收门禁**：Rust 新增 ≥60 条交互状态机单测；现有画板 Dart 单测与 Playwright 画板 E2E 零回归；500 节点拖拽保持 60 FPS。

#### M1.2 `crates/mica-sync` 统一离线存储与同步状态机（2~3 周）

- 新建 `crates/mica-sync`，定义存储后端 trait `SyncStorage` 与无头同步状态机 `SyncEngine`：
  - 统一“本地快照 + outbox 待发队列 + 远端游标 + 冲突合并 + 账号/工作区代际隔离”规则；
  - 桌面端对接 `mica-core::LocalStore`（SQLite）；
  - Web 端经 WASM 对接极薄的 IndexedDB / OPFS KV 桥接层（≤200 行）。
- 逐步退役 `lib/cloud/web_idb_doc_store.dart`（632 行）、`lib/cloud/cloud_sync_session.dart`（1,532 行）、`lib/sync/sync_reconcile_controller.dart`（788 行）中的核心状态逻辑。
- **验收门禁**：Rust 离线/断网/重连/多端并发冲突状态机确定性测试套件全绿；通过 [异步评审清单](../dev/async-review-checklist.md) 逐条审核；桌面 + Web 真实同步集成测试全绿。

---

### Phase 2：`crates/mica-editor` 无头编辑器内核与 `crates/mica-client`（8~12 周）

**目标**：攻克最具含金量的核心战役——将 2.6 万行编辑器中的全部语义大脑沉淀为纯 Rust 无头内核，立住“全栈 Rust 标杆架构”的底座。

#### M2.1 编辑器数据视图、选区代数与 Marks（2 周）

- 新建 `crates/mica-editor`：
  - `BlockView` 扁平模型与 `MicaDoc` 零拷贝/差量物化；
  - `DocSelection`（文本选区、跨块选区、原子块选区、表格单元格选区）与合法性归一化；
  - 选区 Marks 计算（`active_marks`、`toggle_mark`、跨块格式应用）与远端更新后的 `reconcile_selection`。
- 验收：迁移现有 `marks_test`、`selection_reconcile_test`、`editor_formatting` 回归用例到 Rust `cargo test` 对拍 100% 一致。

#### M2.2 块级结构操作、列表/引用、表格与撤销栈（3~4 周）

- 实现核心命令处理引擎：
  - 文本增量编辑（UTF-16 偏移严格对齐，复用 `mica-core::utf16_text_diff`）、Enter 拆分、Backspace/Delete 跨块合并；
  - 扁平列表（`indent`、有序列表重编号、任务列表切换）与引用嵌套（`quote` / `qbreak` / `li`）；
  - 表格行列增删、对齐调整、单元格焦点导航（替代 `table.dart` / `cell_edit_controller.dart`）；
  - 块级与字符级合并撤销/重做栈（替代 `operation_history.dart` / `controller_history.dart`，确保 CRDT 前向操作兼容 #79）。
- 验收：纯 Rust 确定性测试 ≥200 条（含属性测试 proptest 验证任意编辑序列后 `MicaDoc` 结构合法性与撤销闭环）。

#### M2.3 输入规则、粘贴修复与剪贴板编排（1~2 周）

- 将 `controller.dart` 中的 `applyInputRules`、`paste_repair.dart` 启发式、结构化 HTML/Markdown/纯文本复制粘贴编排收进 `crates/mica-editor`，直接调用 `crates/md-core`；
- 消除编辑器编辑热路径中的 JSON 跨语言编解码往返。

#### M2.4 Flutter 编辑器接入 `mica-editor` 并收紧棘轮（2 周）

- 将 `clients/mica_flutter/lib/editor/controller.dart`（3,561 行）切换为 `mica-editor` 的薄代理层；
- 经由单一 `EditorPatch` 驱动 `render.dart` 局部块重排与重绘；
- 同步下调 [scripts/file-size-budget.txt](../../scripts/file-size-budget.txt) 中 `editor.dart` 的棘轮基线。
- 验收：Flutter 315 个测试文件 + 全部 4 个浏览器 E2E 套件 + 2,000 块性能基准（≤33 ms）全绿。

#### M2.5 `crates/mica-client` 应用核心（2 周）

- 新建 `crates/mica-client`，收口工作区管理、页树 ViewModel（复用 `mica_core::tree_rules`）、本地/云端双态统一（替代 `world/` 1,611 行）、HTTP/WS 传输客户端（复用 `mica-api-types`，替代 `lib/api/` 3,694 行）、搜索与 AI 定向编辑编排。
- **Phase 2 里程碑**：全栈核心 85%+ 收归纯 Rust 单一 Cargo Workspace；Flutter 侧彻底瘦身为纯 L3 绘制壳。

---

### Phase 3：L3 纯 Rust UI 壳（`clients/mica_rust`）落地与 Flutter 退役（8~16 周）

**硬启动闸门（缺一不可）**：

1. `longbridge/nsh` 记录首个 `nsh-compat` 稳定兼容基线（或经评审确认的等价稳定纯 Rust 壳底座）；
2. Phase 0 四端真机关卡全部通过（Windows 真人 IME、Android 真机、Chrome/Firefox/Safari、Web 体积在预算内）；
3. Phase 2 全部里程碑完成且稳定运行。

**实施步骤**：

- **M3.1 应用主壳（3~4 周）**：在根 workspace 新增 `clients/mica_rust`，基于 `nsh` 纯 Rust（`rust_app!`）搭建主窗口、侧栏虚拟化页树、路由、浅/深色主题、多语言（`nsh-i18n`）、设置面板、快捷键分发、桌面托盘与系统对话框，直连 `mica-client`。
- **M3.2 路线二-B 编辑器视图（4~6 周）**：
  - NSH 变高虚拟 `List` 绑定 `mica-editor` 扁平 `BlockView` 流；
  - 聚焦块挂载 OS 原生文本输入代理（承接五端中文 IME 预编辑与候选窗）；
  - 实现跨块鼠标/触屏选区与移动端选择把手、浮动格式工具栏、`/` 斜杠菜单、表格视图与原子块注册表（公式 `ratex`、Mermaid `merman`、图片/附件、代码块高亮）。
- **M3.3 无限画板与导图视图（2~3 周）**：基于 `nsh::Canvas` 对接 `mica-board::BoardSession`，实现双层缓存绘制、缩放态叠层文本输入、导出/导入与全屏/内嵌卡片切换。
- **M3.4 全端对拍与 Flutter 退役（2~3 周）**：
  - 桌面（Windows / Linux / macOS）、Web（Chrome / Firefox / Safari）、Android APK 全量运行现有功能与性能门禁，实测全面持平或超越 Flutter 版；
  - 切主发布链路至 `clients/mica_rust`，执行体面退役：删除 `clients/mica_flutter/`、`flutter_rust_bridge`、Cargokit、`.fvmrc`、FFI 源码指纹校验与 `scripts/check-dart-markdown.sh`；
  - （可选后续）在已有的 L2 之上探索纯 GPUI 桌面旗舰模式或 `ratatui` 终端 Vault 客户端。

---

## 6. 风险矩阵与治理原则

| 风险 | 影响 | 缓解与兜底策略 |
| --- | --- | --- |
| **`nsh` API 持续大幅变动或冻结推迟** | 延后 Phase 3 启动 | Phase 1–2 零依赖 `nsh`，所有 L2 投入立刻服务于现役 Flutter 端并降低维护成本；必要时 Phase 3 可无缝切换至其他满足门禁的 Rust UI 底座 |
| **路线二-B 跨块选区/移动端把手在 NSH 虚拟 `List` 上受限** | 影响编辑器交互一致性 | Phase 0.3 提前做最小探针验证；若虚拟 `List` 跨项选区存在原生框架限制，可向 `nsh` 上游提 PR 或在编辑区采用自管虚拟化 `ScrollView` + 统一手势层（借鉴 `nsh::RichEditor` 的 `view/touch.rs`） |
| **NSH 纯 Rust Web WASM 包体偏大** | 影响 Web 首屏体验 | Phase 0.2 优先实测裁剪 `browser-core` 与语法高亮表后的真实 Brotli 体积；配合按需分包（画板/导出按需加载）与 Brotli 预压缩，设定严格体积红线 |
| **Phase 1–2 过渡期 FFI/WASM 桥接开销反弹** | 敲字延迟劣化 | `mica-editor` 采用单次粗粒度 `dispatch(Command) -> Patch` 接口（禁止逐属性跨界往返），每阶段必跑 2,000 块性能基准与 Flutter 现有 33 ms 基线对拍 |
| **长周期重构分支与 `main` 严重分叉** | 合并冲突、主线停滞 | 坚持**小步合入原则**：每个子里程碑（P0、M1.1、M1.2、M2.1…）独立成 PR 过全套 CI 后合入 `main`，重构分支作为架构主线演进，绝不积压数月不合 |

---

## 7. 最终交付状态（2026-11-22，`feat/fullstack-rust-architecture` 分支）

| 阶段 | 状态 | 实证与交付摘要 |
| --- | --- | --- |
| **Phase 0（本机实证探针）** | ✅ 已完成 | 详录于 [P0 实证报告](../reviews/fullstack-rust-p0-spike-2026-11-22.md)：精简纯 Rust `nsh-platform-web` WASM **6.18 MiB raw / 1,935 KiB gzip-9 / 1,422.7 KiB (1.39 MiB) Brotli-11**；2,000 块路线二-B `virtual_list` 挂载 **15.27 ms**（98 可视节点）、单块更新 **17 µs**；500 节点 Canvas 录制 **1.12 ms**。 |
| **Phase 1（L2 `BoardSession` + `mica-sync`）** | ✅ 已完成 | `crates/mica-board::session`（100 单测全绿）与 `crates/mica-sync`（66 测试全绿，Sans-I/O `SyncEngine` + `SqliteSyncStorage` + `MemoryMirrorStorage`）。 |
| **Phase 2（L2 `mica-editor` + `mica-client`）** | ✅ 已完成 | `crates/mica-editor`（26 套件全绿含 proptest）与 `crates/mica-client`（`PageTreeModel` / `ClaimTracker` / `MicaClientRuntime` / `search`，7 测试全绿）。 |
| **Phase 3（L3 `clients/mica_rust` 全面切换与 Flutter 彻底退役）** | ✅ 已完成 | `clients/mica_rust` 已建成全功能纯 Rust 客户端（`VaultStore` SQLite 重启持久化、`CloudConnector` HTTP+`SyncEngine`、虚拟化页树含文件夹/重命名/回收站、路线二-B 编辑器含中文 IME / 富文本 marks / `/` 斜杠菜单 / 交互表格 / `AtomicBlockRenderer` RaTeX+Mermaid+内嵌白板 SVG、无限画板手势+导图+原地编辑+SVG 导出、全文搜索、深浅色主题与设置、桌面 Windows 原生 `mica-rust.exe` + Inno Setup 打包 + WebAssembly `nsh-platform-web` 入口）。`clients/mica_flutter/`、`crates/doc-wasm/`、`crates/board-wasm/`、`.fvmrc`、FRB/Cargokit 与全部 Dart/Flutter 脚本、CI/Release/Docker/nginx 配置已彻底删除并切换为 100% 全栈纯 Rust（保留对外 `mica-md` / `mica-md-js` SDK）。 |
| **Phase 3.1（编辑器彻底落定路线二-B + L3 选型复审，2026-11-23）** | ✅ 已完成 | ① 移除 `nsh-base::RichEditor` 依赖，全面切换为 `crates/mica-editor` + NSH 原生全宽 `virtual_list`（窗口右缘滚动条、单块信号隔离、顶层 `overlay()` 代码菜单、Mermaid/图片全屏缩放 Lightbox、141 KB 真实长文档桌面实测与 Chrome WASM 0 报错验证，提交 `3f033c30`）。② 在“桌面 = Web 同等重要、移动端全功能偏阅读”前提下完成 GPUI / Dioxus / Blitz / Makepad / Slint / Xilem 横向复审，**拍板方案 B：继续留在 `nsh`**，后续聚焦 `wasm32` 异步云同步、Web WASM 体积优化与移动端阅读/触控适配，详录见 [ui-gpui-evaluation.md §10](ui-gpui-evaluation.md#10-2026-11-23-复审rust-全栈-l3-ui-备选横向评估与留在-nsh-的决定)。 |
