# 无限画板与思维导图架构设计与开发计划（方案 C）

> 状态：**设计定稿，全量实施中** · 2026-11-22
> 分支：`spike/board-canvas` · 探针基线：`6510c776` / `72851011`

---

## 1. 产品定位与最终决策

基于探针实测与需求确认，Mica 的绘图体系遵循**最小充分解 + 单一数据面**原则，收敛为以下 4 条定稿决策：

1. **以「内嵌在 Markdown 文档内」为第一主形态**：
   - 在 Markdown 文档正文通过 `/` 斜杠菜单插入 **画板**、**思维导图** 或 **流程图**；
   - 正文内展示高清矢量渲染卡片（复用 `AtomicBlockRenderer` + 异步光栅管线），点击 / 双击展开为全屏无限画布编辑器，完成编辑即时回写并刷新预览；
   - 存储形态为标准围栏块 ```` ```mica-board ````（紧凑规范化 JSON v1）：在**本地离线 SQLite、Vault 本地 `.md` 目录、云端 Postgres、版本历史快照、跨页复制粘贴、公开分享与 HTML 导出**全部链路天然零缝隙工作；HTML 导出与公开分享由 Rust 无头直接渲染为内联 `<svg>`，与现有 Mermaid 链路一致；Vault / 导出可同时输出 `.svg` + 同名 `.mica-board.json`。
2. **统一 1 套画板引擎覆盖 3 种场景，UML 继续用原生 Mermaid**：
   - **画板**：自由白板、便签、文本、Frame、自由画笔、架构图；
   - **思维导图**：同一引擎的树状自动布局模式（`Tab` 加子节点 / `Enter` 加同级 / `Shift+Tab` 升级 / 折叠 / 拖拽重挂 + Markdown 大纲双向互转）；
   - **流程图**：同一引擎内置流程图图形 + 正交折线自动避障；
   - **UML 图**：继续使用 Mica 已有的纯 Rust `merman` 引擎（[mermaid.md](mermaid.md)），不重复造拖拽式 UML 引擎。
3. **两层图形库（轻量不炸包体）**：
   - **第一层（内置参数化矢量图形，16 种）**：矩形、圆角矩形、椭圆、菱形、胶囊（起止）、平行四边形（I/O）、圆柱（数据库）、波浪文档、云朵、六边形、三角形、便签、纯文本、Frame 容器、思维导图节点、图标卡片——拉伸不失真、Flutter 画布与 Rust SVG 导出 100% 同构；
   - **第二层（高频技术架构图标库，40+ 矢量图标）**：按计算/存储/网络/客户端/云安全分组的极轻量内联矢量图标，并支持导入自定义 SVG/图片，不把数 MB 第三方云图标包打进主包。
4. **纯 Rust `draw.io` 导入兼容（零新增第三方 crate）**：
   - 支持 `.drawio` / `.xml` / `.drawio.svg` / `.drawio.png`（含内置纯 Rust RFC 1951 raw deflate / zlib 解压，复用已锁定的 `roxmltree` / `base64` / `percent-encoding`）；
   - 还原 90%+ 常见形状、容器/泳道绝对坐标、连线箭头、手动拐点、HTML 清理文本与多页签 Frame；未收录的 draw.io 专有 stencil 安全降级为保留文字与连线的圆角卡片。

---

## 2. 技术探针实测结论（已固化为架构红线）

在真实 **Flutter Web Release（CanvasKit）+ Playwright Chrome** 与 **Windows 原生桌面（Impeller）** 验证了 500 节点 + 461 连线场景：

| 维度 | 实测结果 | 固化规范 |
| --- | --- | --- |
| **Dart `deferred as` 分片** | 首屏 **0** 请求；按需拉取 `38.8 KB` Brotli | 画板 UI 必须独立于 `lib/board/` 并走 `deferred` |
| **500 节点拖拽/平移帧率** | Optimized 全程 **60.0 FPS**（build p50 **2.3~2.6 ms**），Naive 47 FPS（41 ms） | 必须采用「`TextPainter` 缓存 + 矢量 `ui.Picture`（`RepaintBoundary`）+ 动态 Overlay」双层渲染 |
| **缩放态中文 IME（45%/100%/200%）** | DOM `<textarea>` 偏移 `(5.4,4.5)` / `(12,10)` / `(24,20)`，**0.0 px 漂移**；自动撑高 + 连线跟随 | 必须用 `Positioned` + `Transform.scale(scale: zoom)` + 世界坐标 `EditableText` |
| **CanvasKit 异步 CJK 字体坑** | 首次 `ui.Picture` 未订阅字体加载事件导致 499 节点出现豆腐块 | **强制红线**：所有 `TextPainter` 与 `ui.Picture` 缓存必须订阅 `PaintingBinding.instance.systemFonts` 失效重建 |

---

## 3. 架构总览与 Crate / 模块职责

```text
                       crates/mica-board (纯 Rust，零新增外部依赖)
       ┌──────────────────────────────────────────────────────────────────┐
       │ model.rs    : BoardSnapshot v1 · 16 图形 · 40+ 技术图标 · 模板   │
       │ crdt.rs     : BoardDoc (yrs CRDT, feature="crdt") · 3-way 语义合并│
       │ layout.rs   : 思维导图 Reingold-Tilford 变高子树布局 · MD 互转   │
       │ routing.rs  : 正交折线 A* 自动避障路由 · manual_waypoints 保留   │
       │ drawio.rs   : .drawio/.xml/.drawio.svg/.drawio.png + RFC1951 解压│
       │ svg.rs      : 无头确定性 SVG 导出（浅/深色主题，无 foreignObject）│
       │ search.rs   : 全文检索纯文本抽取                                 │
       └───────────────┬──────────────────┬──────────────────┬────────────┘
                       │                  │                  │
                       ▼                  ▼                  ▼
           crates/md-core &          桌面端 (FFI)       Web 端 (按需 WASM)
           crates/app-core      clients/mica_flutter/   crates/board-wasm
           crates/api-server    rust/src/api/board.rs   /mica_board/board.{js,wasm}
           · HTML/分享内联 SVG   · 完整 FFI 绑定         · 独立构建 ≤ 256 KiB gzip
           · WS Room 双通道                              · 首屏普通文档 0 B
             (crdt_tx + presence_tx)
                                          │                  │
                                          └────────┬─────────┘
                                                   ▼
                                 clients/mica_flutter/lib/board/
                                 (独立库，`deferred as board` 按需分片)
                                 · board_models.dart / board_icons.dart
                                 · board_bridge*.dart (FFI / Web / Fallback)
                                 · board_controller.dart (交互/撤销/吸附/导图)
                                 · board_painter.dart (双层 RepaintBoundary)
                                 · board_toolbar.dart / board_inspector.dart
                                 · board_editor_screen.dart (全屏飞书风格编辑器)
                                                   │
                                                   ▼
                                 Mica 编辑器集成（严守文件行数棘轮）
                                 · `/` 斜杠菜单：画板 / 思维导图 / 流程图
                                 · BoardPreviewer + BoardRenderer (AtomicBlockRenderer)
                                 · 点击/双击卡片展开全屏画板编辑器，保存自动 3-way 合并回写
```

---

## 4. 核心子系统规格

### 4.1 规范化数据模型（`BoardSnapshot` v1）
```json
{
  "version": 1,
  "title": "未命名画板",
  "mode": "whiteboard",
  "nodes": [
    {
      "id": "n1",
      "kind": "mind_root",
      "x": 120, "y": 180, "w": 200, "h": 56, "rot": 0, "z": 1,
      "parentId": null, "rank": 0, "collapsed": false,
      "text": "中心主题",
      "icon": null,
      "style": {
        "fill": "#E8F3FF", "stroke": "#3370FF", "strokeWidth": 1.6,
        "dashed": false, "fontSize": 14, "textColor": "#1F2329", "align": "center"
      }
    }
  ],
  "edges": [
    {
      "id": "e1", "fromId": "n1", "toId": "n2",
      "fromAnchor": "auto", "toAnchor": "auto",
      "routing": "curved", "arrowStart": "none", "arrowEnd": "arrow",
      "waypoints": [], "label": "",
      "style": { "stroke": "#8C959F", "strokeWidth": 1.6, "dashed": false }
    }
  ],
  "strokes": [
    { "id": "s1", "color": "#3370FF", "width": 2.4, "points": [[10, 20], [30, 45]] }
  ]
}
```
- **并发编辑防 JSON 破坏**：多端同时编辑同一篇文档里的 `mica-board` 块时，通过 `mica_board::merge_board_json(base, local, remote)` 按 `node.id` / `edge.id` / `stroke.id` 做**字段级三路语义合并**（同时 `BoardDoc` 提供完整 `yrs::Doc` 二进制 CRDT 支持后续独立画板协同）。

### 4.2 WebSocket 房间双通道（`crates/app-core/src/rooms.rs` + `crates/api-server/src/routes/ws.rs`）
- 保持 `WS_PROTOCOL_VERSION = 1` 完全向后兼容；
- 将 `Room` 单一 `broadcast(256)` 拆为：
  - `crdt_tx`（容量 256，可靠通道）：只传 `sync.update` 等持久化变更，滞后仍触发 `client_out_of_date`；
  - `presence_tx`（容量 128，有损通道）：传 `presence.update` / `presence.leave`，滞后时静默丢弃旧帧，**绝不挤占 CRDT 队列、绝不踢出慢客户端**。

### 4.3 编辑器集成与结构门禁约束
- `clients/mica_flutter/lib/editor/editor.dart` 与 `render.dart` 受 [file-size-budget.txt](../../scripts/file-size-budget.txt) 只减不增棘轮保护；
- 将 `_ImageEditDialog` 迁出为独立库 `lib/editor/image_edit_dialog.dart`，将画板预览/渲染逻辑放进独立库 `lib/editor/board_preview.dart` 与 `lib/board/`，确保 `main.dart` / `editor.dart` / `render.dart` 三库行数均严格 ≤ 预算基线。

---

## 5. 全量实施计划（一次连续交付清单）

1. **阶段 A · Rust 核心引擎与后端协同**
   - [x] `crates/mica-board`：模型、16 图形、40+ 技术图标、模板、思维导图自动布局、Markdown 大纲互转、正交避障路由、纯 Rust RFC 1951 解压 + `draw.io` 导入、无头确定性 SVG 渲染、3-way 语义合并、`yrs` `BoardDoc`、全文检索抽取与完整单测；
   - [x] `crates/md-core`：`render-board` 特性集成，HTML 导出与公开分享渲染 ```` ```mica-board ```` 为内联 `<svg>`；
   - [x] `crates/app-core` + `crates/api-server`：WS `Room` 双通道拆分 + 抗 presence 洪泛单测；
   - [x] `crates/board-wasm` + `scripts/build-board-wasm.sh`：Web 按需 WASM 构建与体积门禁（≤ 256 KiB gzip-9）。
2. **阶段 B · 桌面 FFI 绑定**
   - [x] `clients/mica_flutter/rust/src/api/board.rs` 暴露全量 `mica-board` 能力并运行 `flutter_rust_bridge_codegen` 生成 Dart 绑定。
3. **阶段 C · 完整 Flutter 飞书风格画板编辑器（`lib/board/`）**
   - [x] 工具栏（选择/抓手/16 图形库/40+ 架构图标库/便签/文本/连线/自由画笔/Frame/思维导图）；
   - [x] 交互引擎（框选多选、拖拽+智能对齐参考线、8 向缩放句柄、四向锚点连线、思维导图快捷键流与折叠、撤销/重做、复制/粘贴/克隆、缩放/自适应视图）；
   - [x] 浮动属性检查器（填充/边框/虚线/字号/图形切换/路由切换/箭头/层级）；
   - [x] 导入导出弹窗（`.drawio`/`.xml`/`.drawio.svg`/`.drawio.png` 文件导入、Markdown 大纲 ↔ 思维导图互转、导出 SVG / JSON / Markdown）；
   - [x] 探针验证的 `ui.Picture` + `RepaintBoundary` + `systemFonts` + 缩放态中文 IME 编辑器。
4. **阶段 D · Mica 编辑器深度集成**
   - [x] `/` 斜杠菜单新增「画板」「思维导图」「流程图」+ 中英文 l10n；
   - [x] `BoardPreviewer` + `BoardRenderer`（正文内高清矢量卡片预览 + 点击/双击全屏编辑 + 源码切换）；
   - [x] 保持 `just check-structure` 全绿。
5. **阶段 E · 全面测试、门禁与 Windows 桌面程序构建**
   - [x] `cargo test`、`cargo clippy`、`cargo fmt --check`、`flutter analyze`、`flutter test`、`scripts/build-board-wasm.sh`、`check-file-size.sh`、`check-dart-markdown.sh`、`check-ignored-sources.sh`；
   - [x] 构建完整 Windows 桌面 Release 程序（`clients/mica_flutter/build/windows/x64/runner/Release/mica_flutter.exe`）并完成启动冒烟验证。
