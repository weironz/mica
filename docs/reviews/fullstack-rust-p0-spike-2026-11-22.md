# Phase 0 全景实证与 L2 代码边界摸底报告（2026-11-22）

> **关联架构文档**：[fullstack-rust-architecture.md](../architecture/fullstack-rust-architecture.md) · **分支**：`feat/fullstack-rust-architecture` · **跟踪 Issue**：[#172](https://github.com/weironz/mica/issues/172)
> **实测环境**：Windows 11/10 x86_64 · `rustc 1.100.0 (0ef1bc386 2026-11-09)` · `wasm32-unknown-unknown` · `wasm-bindgen` · Node.js · Chrome / Edge（Firefox / Android 真机 / Safari 本机未安装，标注为剩余外设补验项）

---

## 1. 一句话总结（Phase 0 决策结论）

**所有核心未知数已在本机实测摸清，路线二-B（三层全栈 Rust + 自有 `mica-editor` + NSH 纯 Rust 原生壳）技术可行性全面过关，可立即进入 Phase 1 → Phase 2 连续实施：**

1. **NSH v0.6.1 纯 Rust Windows 原生后端（Win32 + Direct2D + DirectWrite）**：在 Mica 锁定的 `rustc 1.100.0` 下**零报错一次编通**（已生成可运行的 Mica 路线二-B 三合一实机探针 `mica_probe.exe`）。
2. **NSH v0.6.1 瘦身纯 Rust Web WASM（颠覆级利好）**：关闭 `browser-core`（无 TSX/JS 运行时、无 `nsh:rt` importmap）并移除与 Mica 重复的 `nsh-base` 重型可选 feature（`editor/rich-editor/highlight/math/emoji-picker/plot/term`）后，**零 C 依赖（无需 clang）一次编通**，且实测体积：
   - **Raw**：`6.18 MiB`（`6,475,819 B`，较官方带 JS 版 12.6 MB 腰斩，较 `gpui-kit` 18 MB 小 3×）；
   - **Gzip-9**：`1,935 KiB`（`1.89 MiB`）；
   - **Brotli-11**：**`1,422.7 KiB`（`1.39 MiB`）**（尚未经过 `wasm-opt -Oz` 与剔除 `naga`）；
   - 且 Web 端走浏览器原生 DOM 字体（**0 B 中文字体下发**）。加上 Mica 自有引擎（~450 KiB br）后总首屏约 **~1.85 MiB Brotli**，**比现行 Flutter Web（CanvasKit 1,505 + main.dart.js 1,098 + md/doc-wasm 380 = ~2.98 MiB Brotli + 中文字体）更轻约 35%**。
3. **路线二-B 在 NSH 原语层 100% 天然成立**：阅读 `nsh-base/src/rich_editor/view/render.rs` 与 `touch.rs` 证实，NSH 的中文 IME、预编辑与移动端编辑菜单全部基于 `nsh-core` 公开的透明悬浮 `sink` 原生 `input`（`.on_composition_change` / `.on_key_press` / `.on_paste` / `EventKind::EditAction`）+ `virtual_list`（含 `ListViewport::Parent`）+ `canvas(Painter)` + `svg(ImageSrc::svg_markup(...))`；无需依赖 `nsh::RichEditor`（彻底规避双文档模型漂移与 `#2376` 未虚拟化问题）。
4. **Mica 内部 L2 代码级边界审计（重大利好）**：
   - `EditorController`（3,561 行）**几乎零 Flutter 依赖**（仅用 `ChangeNotifier`，零 `BuildContext`/`FocusNode`/`TextEditingValue`）；
   - `RenderDocument`（`render.dart` 3,556 行）**完全不导入 `EditorController`**，且自带基于引用恒等的 O(1) 块级 `_layoutCache`——消费 `EditorPatch` **几乎零改动 `render.dart` / `block_renderers.dart`**；
   - 已定位 **44 个纯逻辑 Dart 单元测试**，可直接 1:1 翻译为 `crates/mica-editor` 的黄金对拍回归集。
5. **对照实测 `gpui-kit v0.10.0`（`--features gpui-fast`）**：官方 `examples/input/src/main.rs` 在开启 `gpui-fast 0.1.16` 开关时因 `WindowBounds::centered` 签名漂移直接编译失败（需手动补 `None` 参数），印证了 0.x 实验分支的漂移风险。

---

## 2. 本机实测数据总表

### 2.1 Web WASM 体积实测（`--profile web`, `opt-level="z"`, `strip=true`, `panic="abort"`）

| 产物 | Raw | Gzip-9 | Brotli-11 | 中文字体下发 | 备注 |
| --- | ---: | ---: | ---: | --- | --- |
| **NSH v0.6.1 纯 Rust 瘦身版**（本机实测） | **6.18 MiB** | **1,935 KiB** | **1,422.7 KiB** | **0 B（系统/浏览器字体）** | 关闭 `browser-core` 与重复 `nsh-base` feature；零 C 依赖；未跑 `wasm-opt`；内含 `naga` / `html5ever` / `markdown` 仍有 ~300–500 KiB 裁剪空间 |
| NSH 官方带 JS/TSX 完整版（官网基准） | 12.64 MiB | 4,571 KiB | 3,453.6 KiB | 0 B | 含 JS 桥、全部组件与高亮设施 |
| `gpui-kit v0.9.1` WASM 示例 | 17.24 MiB | — | — | 需自带数 MB 字体 | 缺字体呈豆腐块；默认后端候选窗停在 `(0,0)` |
| **Mica 现行 Flutter Web（CanvasKit）** | ~4.2+ MiB | — | **~2,983 KiB** | 需额外按需字体 | `canvaskit` 1,505 + `main.dart.js` 1,098 + `md/doc-wasm` ~380 KiB |

> **关键发现（依赖陷阱已提前排雷）**：`nsh-platform-web/Cargo.toml` 默认打开 `nsh-base` 的 `math` 特性，会拉入 `zstd-sys`（C 代码），在未装 `clang` 的 Windows 机器上交叉编译 `wasm32` 会报错。路线二-B 关闭 `nsh-base` 重复特性后，**100% 纯 Rust 无 C 依赖**，Windows / Linux 均开箱即编。

### 2.2 路线二-B 无头性能基准（2,000 块虚拟化文档 + 500 节点 Canvas，unoptimized debug 构建）

使用 `nsh-testing::Headless` 在本机 Windows（`rustc 1.100.0`，未开启 release 优化的 debug 构建）对路线二-B 核心模式（**`structure_rev` 结构信号与单块细粒度 `Signal<String>` 分离** + `virtual_list` + `nsh::Canvas`）实测结果：

| 指标 | NSH 路线二-B 实测（debug） | 对照：NSH `RichEditor` 在线示例 | 对照：Mica 现行 Flutter Web |
| --- | ---: | ---: | ---: |
| **2,000 块文档首帧挂载** | **`15.27 ms`** | 13,400 ms（粘贴载入） | 页面加载即显 |
| **2,000 块实际挂载文本数** | **`98` 个**（`#1999` 未挂载，真虚拟化） | ~19,900 个（全量挂载） | 画布无 DOM 元素 |
| **2,000 块单块编辑响应更新** | **`17 µs`**（`0.017 ms`，仅唤醒被编辑块） | 283 ms（v0.4 官网）/ ~1 ms（#2276） | 33 ms（含 Chromium 帧渲染） |
| **500 节点 + 460 连线 `Canvas` 首帧** | **`4.36 ms`** | — | 60 FPS（build p50 2.3~2.6 ms） |
| **500 节点 + 460 连线 `Painter` 指令录制** | **`1,119 µs`**（`1.12 ms`） | — | — |

> **关键架构规约（已固化）**：`virtual_list` 的 `.keys(...)` 与 `count` **严禁订阅正文编辑信号**，必须只订阅 `structure_rev`（增删/重排块时变更）；块内文本/样式更新走 per-block 细粒度 `Signal`，确保 2,000+ 块文档敲字时 `O(1)` 只唤醒单个可见块。

### 2.3 Windows 真人中文输入法实机验证程序（已构建就绪）

两款本机 Windows 可执行文件均已用 `rustc 1.100.0` 构建完毕（位于会话 scratch 目录，不污染仓库），供用户直接双窗对比微软拼音 / 搜狗真实中文输入法：

- **NSH v0.6.1 路线二-B 三合一探针**：`mica_probe.exe`（Tab 1: 2,000 块 `virtual_list` + 原生 IME 编辑；Tab 2: 500 节点 + 460 连线 `Canvas` 平移缩放；Tab 3: 多行原生 IME `on_composition_change` 实时监控）
- **`gpui-kit v0.10.0`（`gpui-fast`）输入法对照**：`input.exe`（手动修复 `WindowBounds::centered(None, ...)` 后编译）

### 2.4 剩余外设补验项（仅影响 Phase 3 启动闸门，不阻塞 Phase 1–2）

- Windows 本机真人用微软拼音/搜狗亲手敲击 `mica_probe.exe` 与 `input.exe` 验证候选窗位置、退格、取消组字；
- Android 真机跑 NSH 纯 Rust APK（组字、长按选词、选择把手、系统菜单）；
- Firefox / Safari 上实测纯 Rust `nsh_platform_web.wasm`（因无 `browser-core` / `nsh:rt` importmap，理论上已彻底绕过此前 Firefox 加载故障，仍需实机确认）。

---

## 3. Mica 内部 L2 代码级边界摸底（Phase 1–2 施工蓝图）

### 3.1 M1.1 `crates/mica-board` `BoardSession` 抽离边界

- **待消除重复**：`clients/mica_flutter/lib/board/board_models.dart`（1,005 行）完整镜像了 `crates/mica-board/src/model.rs` 并重复实现了 `layoutMindmapLocal` / `resolveEdgeEndpoints` / `nodeAnchorOffset`；`board_editor_screen.dart` 每次 `pointerUp` 整图 JSON 往返调 Rust 路由。
- **迁入 Rust `BoardSession`**：`BoardSnapshot` + `node_by_id`/`edges_by_node`/`routed_edges`（直调 `routing::route_edge`）、视口变换（`zoom` `0.2..3.0`/`pan`/`fit_to_content`）、全部命中测试（折叠徽章、锚点端口、8 控制点、Frame 30px 标题栏、边曲线）、拖拽与 ±6px 智能吸附、框选、连线重连、自由画笔采样、导图快捷键（Tab/Enter/折叠/自动重排）、100 步手势级撤销栈、双层脏标记（`graph_rev` / `viewport_rev` / `active_rev` / `rebuild_chrome`）。
- **留在 Flutter L3**：`StaticBoardPainter`（`ui.Picture`）+ `ActiveBoardPainter`、`TextPainter` 缓存（订阅 `PaintingBinding.instance.systemFonts`）与缩放态 `Transform.scale` `EditableText` CJK IME 浮层（文本变化时回传 `measured_height` 给 `BoardSession::set_node_text`）。
- **桥接协议**：Desktop 在 `clients/mica_flutter/rust/src/api/board.rs` 暴露 `#[frb(opaque)] BoardSessionHandle`（`#[frb(sync)]` 零等待）；Web 在 `crates/board-wasm` 暴露 `#[wasm_bindgen] WasmBoardSession`（维持按需分片，普通页 0 B）；高频 `pointer_move` 提供数值参数快路径，返回增量 `BoardPatch`。

### 3.2 M1.2 `crates/mica-sync` Sans-I/O 同步引擎与统一存储边界

- **桌面 SQLite vs Web IndexedDB 差异收口**：
  - 定义纯同步 `SyncStorage` trait（`load` 显式区分 `Ok(None)` vs `StorageError::Corrupt` vs `Io`、`save_base`、`append_outbox`、`outbox_after`、`cursor`、`advance_cursor`、`append_remote_batch` 同事务更新 `last_synced_rid`、`log_sizes`、`compact`、`is_broken`）。
  - 桌面直接由 `mica_core::LocalStore` 实现；Web 由 WASM 侧内存镜像实现，并通过 `SyncEffect::MirrorIdb(IdbSnapshotTxn)` 吐出已固化的写快照给极薄 JS/Dart FIFO 写回队列（从结构上根除 #161 延迟闭包脏读，Web Lock 生命周期留在 L3）。
- **Sans-I/O `SyncEngine`（内置 `MicaDoc`）**：收拢 `cloud_sync_session.dart`（1,532 行）+ `pending_doc_update.dart`（70 行）的全部帧状态机（bootstrap/base/pull/updates/ack/error/ping-pong、64 窗流控、8+ 积压合并与毒丸拆包、B11 `repairSvFor`、3 次自动自愈熔断、256 条自动压缩、心跳 8s/20s 与 6 档指数退避重连、presence 名单）。
- **严格落实 [异步评审清单](../dev/async-review-checklist.md)**：`SyncReconcileController` 留 L3 专管多 Tab 调度与四元组 `SessionClaim(accountGen, origin, workspaceId, docId, syncGen)`，所有 `SyncEffect::{OpenSocket, ScheduleTimer}` 绑定 `epoch`，跨 `await` 与错误路径统一校验归属，`dispose()` 走 `try/finally` 等 Web IDB `_tail` 排空后再释放 Web Lock。

### 3.3 Phase 2 `crates/mica-editor` 无头编辑器内核边界

- **极低耦合度实证**：
  - `controller.dart`（3,561 行）+ `controller_history.dart` + `controller_commits.dart` 仅依赖 `ChangeNotifier`；
  - `model.dart` / `operation_history.dart` / `selection_reconcile.dart` / `selection_marks.dart` / `paste_repair.dart` / `text_common.dart` / `ai_edits.dart` 均为 100% 纯 Dart；
  - `marks.dart` 仅 `buildMarkedSpan` 依赖 Flutter，其余 300+ 行全为纯算法；
  - `table.dart` 仅用了 `@immutable` 注解，迁入 Rust 后直接省掉 4,096 条 Dart LRU 缓存与 64 条 JSON 批编解码；
  - 全链路统一 **UTF-16 code unit 偏移**，与 `crates/mica-core/src/doc.rs`（`OffsetKind::Utf16`）和平台文本引擎零偏差。
- **零侵入 `render.dart` 切换策略**：`RenderDocument` 不导入 `EditorController`，且 `performLayout()` 内置基于 `identical(srcText, node.text) && identical(srcData, node.data)` 的 O(1) 块级布局复用。L3 薄代理收到 `EditorPatch` 后仅替换变更块对象引用即可命中复用，**无需动 `render.dart` 与 `block_renderers.dart` 的 28 处内部私有访问**。
- **平滑过渡三步走**：
  - Step A：`EditorSession` 输出 `EditorPatch` + 兼容 `doc_ops`，用 **44 个纯逻辑 Dart 单元测试**做黄金对拍验证；
  - Step B：`EditorSession` 直接持有/操作 `mica_core::MicaDoc`（直调 `text_insert/delete/format/split_block/join_into_prev` 与同步 `undo/redo`），**彻底淘汰 `DocOp` JSON 往返、`BlockOpMirror._dataById`、异步 `_historyDrafts` / `_replayHistoryDraft`**；
  - Step C：远端 CRDT update 经 `SyncEngine` → `EditorSession::reconcile_from_doc()` 直出增量 `EditorPatch`。
