# Local-first 统一改造(option C)

> 目标:把当前「online 云同步」与「local 离线」两个**隔离模式**合并为**一套本地优先存储 + 可选云同步层**——每个文档(含云端)都有完整本地 yrs 副本,离线读写原生,重连 CRDT 无冲突合并。对标 AFFiNE `nbstore` / AppFlowy `CollabKVDB`,也消掉「两个世界 + 单向迁移」的架构分裂。

## 结论先行:这是「接线」不是「重写」

同类调研(Notion/Obsidian/AppFlowy/AFFiNE/…)结论:纯在线零缓存已近绝迹;两个直接参照(AppFlowy、AFFiNE)都走**统一 local-first + 可选同步**。架构映射发现 Mica **脚手架已建约 80%**:

| 已有(可复用) | 位置 |
| --- | --- |
| 一套 yrs 核心,双模式共用 | `MicaDocument`(Rust FFI)/ `MicaYDoc`(web JS),字节兼容 |
| op→yrs 统一翻译 | `lib/local/doc_ops.dart` `DocOpMirror`(云/本地都用它) |
| **本地优先落地模板** | `lib/local/local_doc.dart` `LocalDocBackend`:镜像 op 进 yrs + 全量快照落 `MicaStore`(debounce)+ checkpoint |
| **nbstore 形态的本地存储** | Rust `crates/mica-core/src/store.rs`:`doc_snapshot`(base)+ `doc_update`(增量日志/squash)+ `sync_cursor`(last_synced_rid、pushed_clock) |
| 服务端 nbstore 形态 | `crates/app-core/src/sync.rs`:per-workspace `workspace_updates`(rid 流)+ 折叠 `document_yrs_base`(+ state_vector 已存) |
| seq/rid 重连对账 | WS:`sync.bootstrap→sync.base`、`sync.pull{since_rid}→sync.updates{head}`、`sync.push→sync.ack{rid}` |
| 图片 CAS + 待上传队列 | 单一 `blobs/`(sha256+UUID)、`PendingUploads` |

**缺口(全部集中在客户端接线)**:
1. **云端 yrs 副本从不落地** —— `CloudSyncSession._doc` 只在内存;唯一持久化的是 prefs 里的未同步 diff(`cloudUnacked:<docId>`)。重启即需 `sync.bootstrap` 重下 base → 云文档无法离线读写。
2. **Rust store 的 `append_update/updates_after/squash/sync_cursor` 没桥到 Dart FFI**(`store.dart` 只有 save/load/checkpoint/rollback)。
3. **两个 backend 未合并**:`LocalDocBackend`(落 SQLite)与 `CloudSyncSession`(内存+prefs)各走各的;`ServerMode` 是硬全局开关。
4. **id/页树未统一**:本地用时间戳 id + `local_view/local_workspace`;云端用 UUID + REST views,从不进本地库 → 云文档离线不可列。迁移只有 local→cloud「上行」,缺 cloud→local「下行镜像」。

## 目标架构

```
编辑器 op ──DocOpMirror──▶ yrs 副本(MicaDocument)
                              │  每次变更
                              ▼
                    本地存储 MicaStore(SQLite)  ← 权威源,离线读写
                    doc_snapshot(base)+ doc_update(log)+ sync_cursor
                              │  workspace 绑定了 cloud 时,附加:
                              ▼
                    同步层(WS):pull{since_rid=last_synced_rid} → 应用远端
                                push(doc_update.clock > pushed_clock) → ack 推进 cursor
                                CRDT 合并、收敛;离线则挂起,重连自动补
```

- **工作区是本地的**;可选「绑定到某云端 workspace」→ 挂同步层(替代现在的硬模式切换 + 单向迁移)。
- 文档永远**从本地库开**(离线秒开);在线时同步层在后台对账。
- **实时性不变量**:一次编辑产生的 yrs update **同时**走两条并行路 ——(a)**立即** WS push 到服务器(实时同步,毫秒级,与现状一致);(b)追加进本地 `doc_update` 日志(离线/崩溃兜底)。**本地落地不挡推送**。推送节奏 = **每次变更立即**(现有 `_enqueue` 的 `_sendPush` 就是);本地落盘节奏可**防抖/批量**(省 I/O,只影响"崩溃恢复到多新",不影响云端实时)。⇒ local-first ≠ 同步变慢,单字符照样即时上云。
- 服务端**几乎不动**(已是 nbstore 服务端形态);seq/rid 重连已覆盖双向;可选后加 state-vector 快速对账(`sync.have{sv}→sync.diff`,基元已具备)。

## 分阶段(每阶段独立可发、可验证)

### Phase 0 — FFI 桥接(使能项,零行为变化)
把 Rust 已有的 `append_update / updates_after / squash / sync_cursor / set_sync_cursor` 暴露到 FFI `MicaStore`(`store.dart`)。**风险极低**,无用户可见变化。配 Rust+Dart 单测。

### Phase 1 — 云文档离线**只读**(先前评估里的「B」,但建在 C 地基上)
- `CloudSyncSession` 把 bootstrap 的 base + 合并的远端 update **write-through 落 `MicaStore`**(按云 UUID keying),开文档**先从本地库读**再联网对账。
- 云端页树(views/workspaces)缓存进本地库 → 云文档离线可列。
- 下载的图片经 `putBlobAs` 落 CAS(现只有迁移路径做)→ 云图离线可见。
- **成果**:云内容**重启后仍可离线读**。编辑仍走在线路径(未同步项改存 SQLite,不再 prefs)。
- 验证:在线开云文档 → 断网重启 → 仍能读。

### Phase 2 — 合并 backend → 云文档离线**可编辑**(真正的 C)
- 把 `LocalDocBackend` + `CloudSyncSession` 合成**单一 local-first backend**:永远读写 SQLite(base+append-log);同步层(WS)在「workspace 绑云 + 在线」时可选挂载。
- 未同步 outbox 从 prefs 迁到 `doc_update.clock > sync_cursor.pushed_clock`(单一真相源)。
- 重连对账用现有 seq/rid;CRDT 合并收敛。
- `ServerMode` 的「online vs local」硬开关**溶解**为「workspace:本地 / 已绑云」。
- **成果**:云内容**离线可读可写,重连自动同步、无冲突**。
- 验证:离线编辑云文档 → 重连同步;两设备离线各改 → CRDT 合并不丢。

### Phase 3 — 统一模型 / 消掉模式分裂(UX + 迁移)
- 统一 workspace/view/doc schema:一棵页树同时装「本地」与「已绑云」工作区,per-workspace 云绑定(workspace_id + sync_cursor 存在与否)标识。SCHEMA_VERSION bump + 迁移。
- 把现在「Local(离线)/ 远程服务器」模式切换 + 单向迁移,换成:**local-first + 可把某工作区「连接到云服务器」**(下行镜像 + 上行同步,双向)。
- 老用户迁移:现有本地工作区保留;现有云用户首次打开时把云文档**镜像进本地库**。

### Phase 4(可后置)— 硬化
- **props 字段级 CRDT**:`props` 现为 per-block JSON 字符串 LWW → 改 yrs `MapRef`,并发改同块属性才真收敛(非核心离线故事,可延后)。
- 可选 state-vector 快速对账(超陈旧副本首连更省)。
- **Web**:见下方决策。

## 决策(2026-07-10 已定)

1. **Web 范围** —— ✅ **桌面/移动先行 local-first,web 暂留在线**。`local_offline_web` 是空桩、web 云走内存 yjs;Phase 0-2 只碰桌面/移动(它们有 SQLite 本地库)。web 的 IndexedDB nbstore 后端(可复用 yjs `y-indexeddb`)后续再评估,**不在本轮范围**。含义:Phase 1/2 的 write-through、backend 合并都要 `kIsWeb` gate,web 保持现有在线路径。
2. **模式 UX**(Phase 3)—— 会把刚合并的「远程服务器/本地」设置改成「local-first + 连接云工作区」。方向一致(更 AFFiNE)。待 Phase 3 时再细化。
3. **props CRDT** —— 延后到 Phase 4(核心离线读写不依赖)。

## 状态

**方案已定,暂不动手**(用户 2026-07-10)。开工时从 **Phase 0(FFI 桥接,零风险使能)** 起,阶段性提交验证。

## 复用率 / 工作量速估

- 服务端:~0(已是 nbstore 服务端形态)。
- Phase 0:小(纯 FFI 桥接 + 测)。
- Phase 1:中(云会话 write-through + 页树缓存 + 图片镜像)。
- Phase 2:中大(合并 backend + outbox 迁移 + 重连对账走通)。
- Phase 3:中(schema 统一 + 迁移 + UX)。
- 大部分是**把已有 Rust 能力桥出来 + 让云路径复用本地路径**,不是新造。
