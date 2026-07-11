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

## 方向拍板(2026-07-10,用户"要完美不要难就不做")

**完整做到统一 local-first 端态,不停在 P1b-1。** 离线读/写是 local-first 的自然结果,不单独论证。修正:**页树进统一 store(不是 prefs hack)**——`local_view`/`local_workspace` 加 origin 标记区分本地/云,store 成为导航权威源;P1b-2 的 prefs 缓存(commit e0ca19b)降级为**将被替换的临时步**(`toJson`/纯序列化函数留用)。每步仍增量+测试,但建真东西不建将来要拆的。修正路线:

| 阶段 | 内容 | 态 |
|---|---|---|
| P0 / P1a / P1b-1 | FFI + 会话持久化 + 文档内容镜像 | ✅ 真东西,保留 |
| **P1b-2′** | 页树进 store(local_view/workspace 加 origin 标记,镜像云页树)—— 替换 prefs hack,P3 地基 | ✅ 完成 |
| P1c | 离线读取回退 + doc-open chicken-and-egg(从 store 读)→ 闭环离线读 | ✅ 完成 |
| P2 | 离线编辑(append-log outbox 统一,重连 CRDT)—— 设计与逐步进展见 [`local-first-p2-design.md`](local-first-p2-design.md)(P2a 使能 → P2b outbox 切 append-log → P2c 双副本收敛 → P2d 放开离线编辑门 → P2e trim 压实,每步配对抗复审) | ✅ 完成 |
| P3 | 溶解双模式为"工作区:本地/已连云"(双向)+ UX —— 设计与逐步进展见 [`local-first-p3-design.md`](local-first-p3-design.md)(P3a 复合主键 → P3b 统一接线 → P3c-1 溶解核心 → P3d op 路由 → P3e 离线切工作区 → P3f 双向,每步配对抗复审) | ✅ 完成(含 P3c-2) |
| P4 | web IndexedDB、props CRDT、state-vector 对账、纯 append-log 落盘 | ⏭️ 已评估未动工(见下) |

### P4 评估(2026-07-11,P3 收尾时)

四项均为 P2b 级以上的独立工程,各自需要完整的「实现→验证→复审→修复」周期,不宜在单夜尾声仓促落码。按成本/收益排序的建议顺序:

1. **纯 append-log 落盘(桌面)**——收益:去掉每 400ms 整档 encode 的 I/O(大文档尤甚);成本:中(持久化节奏重写 + 崩溃安全重推演 + 复审)。**建议下一个做**,与 P2e 的 trim 已是半成品衔接。
2. **web IndexedDB nbstore**——收益:web 也 local-first(目前 web 明确在线-only);成本:大(JS bundle 侧 y-indexedb 或自研 + web CloudDocStore + playwright e2e)。做之前先确认 web 用户面是否值得。
3. **state-vector 快对账**——收益:超陈旧副本首连省流量;成本:中大(动服务端协议——P2/P3 全程守住的「服务端不动」边界要打开)。等真实遇到「重连拉全量太慢」再做。
4. **props 字段级 CRDT**——收益:并发改属性不互斥;成本:最大(Rust+JS 双引擎 + 服务端折叠 + 编辑器)。维持「明确暂缓」。

~~P3c-2 遗留打磨~~ ✅ 已完成(2e32f8d):Settings 改「云服务器」节、ServerMode/ServerConfig 退役、token per-origin(换服务器保凭证)。

### P1b-2′ 精确步骤(原子全栈迁移,一次提交)

`origin` 列区分本地/云页树条目('local' vs 云 URL),让一个 store 同装两者、按 origin 隔离(P3 统一 schema 的地基)。因改 `LocalView`/`LocalWorkspace` 结构体会连锁 FFI+Dart,**必须一起改**:

1. **mica-core `store.rs`**:`SCHEMA_VERSION 1→2`;`local_view`/`local_workspace` 的 CREATE TABLE 加 `origin TEXT NOT NULL DEFAULT 'local'`;迁移块按现有 pragma 模式 `ALTER TABLE … ADD COLUMN origin`(两表);`LocalWorkspace`/`LocalView` 加 `origin: String`;`save_view`/`save_workspace` INSERT 带 origin;`list_views`/`list_workspaces` **加 `origin: &str` 过滤参** `WHERE origin=?1` + SELECT origin(local 模式传 `"local"`,云镜像传 serverUrl)。Rust 测:迁移给旧行填 'local'、按 origin 隔离 list。
2. **FFI `rust/src/api/store.rs`**:`LocalView`/`LocalWorkspace` 加 `origin` + `From` 双向映射;`list_views`/`list_workspaces` wrapper 加 origin 参。`flutter_rust_bridge_codegen generate` 重生成。
3. **LocalOffline**(io+web):`listViews/saveView/listWorkspaces/saveWorkspace` 穿 origin;加 `mirrorCloudPageTree(serverUrl, workspaces, views)`(按 origin 写)+ `cachedCloudPageTree(serverUrl)`(按 origin 读)。web 桩 no-op。
4. **main.dart**:`_cacheCloudPageTree()` 改成写 store(origin=baseUri),替换 prefs;删 `cloudPageTreeToJson`/prefs 那套(或留 toJson)。本地模式 list 传 origin='local'(现有行为不变)。
5. 之后 **P1c** 才接离线读取回退(启动 catch 用 `cachedCloudPageTree` + doc-open chicken-and-egg)。

## 决策(2026-07-10 已定)

1. **Web 范围** —— ✅ **桌面/移动先行 local-first,web 暂留在线**。`local_offline_web` 是空桩、web 云走内存 yjs;Phase 0-2 只碰桌面/移动(它们有 SQLite 本地库)。web 的 IndexedDB nbstore 后端(可复用 yjs `y-indexeddb`)后续再评估,**不在本轮范围**。含义:Phase 1/2 的 write-through、backend 合并都要 `kIsWeb` gate,web 保持现有在线路径。
2. **模式 UX**(Phase 3)—— 会把刚合并的「远程服务器/本地」设置改成「local-first + 连接云工作区」。方向一致(更 AFFiNE)。待 Phase 3 时再细化。
3. **props CRDT** —— 延后到 Phase 4(核心离线读写不依赖)。

## 进展

- ✅ **Phase 0 完成**(2026-07-10,commit a77e8ab):`rust/src/api/store.rs` 桥出 `append_update / updates_after / squash / sync_cursor / set_sync_cursor` + `SyncCursor`/`DocUpdate` 类型;frb 重生成;集成测试 `frb_store_test.dart`(-d windows 全过)验证 append-log 往返重建文档 + sync cursor 跨重开持久化。零行为变化。
- ✅ **Phase 1a 完成**(2026-07-10,commit 0840152):`CloudSyncSession` 加可选 `persistence`(`CloudDocStore` 字节接口)——connect 时先 seed 本地副本立即 onReady(离线读)、编辑/远端/ack 后 debounced write-through、dispose 同步 flush;`StoreCloudDocStore`(MicaStore 按云 UUID）。web 变体接受参数但忽略。`null`=零变化(单元 230 绿)。集成测(无服务器)验证离线 seed 渲染 + 适配器往返。**尚未接线 main.dart**。〔注:`cloud_sync_test` 需真服务器,本机 8090 有杂散 HTTP 服务骗过其 health 检查导致 load 失败,与本改动无关。〕
- ✅ **Phase 1b-1 完成**(2026-07-10,commit 829a80d):`LocalOffline.cloudDocStore(docId)`(io→`StoreCloudDocStore`、web→null,封装 `MicaStore`);`_setupCloudYrs` 传 `persistence: _local.cloudDocStore(documentId)`(`deviceClientId()` 已在建会话前开 store)。每个打开过的云文档现镜像到本地库、再打开先 seed 渲染。web 构建过(FFI 不进 bundle)、单元 230 + 离线读集成测试绿。
- ✅ **Phase 1b-2 地基完成**(2026-07-10,commit e0ca19b):`Workspace`/`DocumentView` 加 `toJson`;纯函数 `cloudPageTreeToJson`/`cloudPageTreeFromJson`(页树↔JSON,可测);`_cacheCloudPageTree()` 视图加载成功后按服务器 URL 写 prefs(避开 `local_view` 表的 id-space 混淆,改用 prefs 缓存;桌面 only)。单测 2 例、套件 232 绿。
- ✅ **Phase 1b-2′ 完成(原子全栈迁移,2026-07-10)**:`local_view`/`local_workspace` 加 `origin` 列(`SCHEMA_VERSION 1→2`,旧行 `ALTER TABLE … ADD COLUMN` 回填 `'local'`),一个 store 按 origin 同装本地页树 + 云镜像。全栈打通:(1) mica-core `store.rs` `list_views`/`list_workspaces` 加 `origin: &str` 过滤参、`save_*` 带 origin、结构体加字段 —— Rust 测新增 origin 隔离用例、迁移回填断言(16 绿);(2) FFI wrapper 加 `origin` + `From` 双向 + list 参,`flutter_rust_bridge_codegen generate` 重生成;(3) `LocalOffline` io+web:`listViews/saveView/listWorkspaces/saveWorkspace` 穿 origin(默认 `'local'`,本地调用点零改动),加 `mirrorCloudPageTree(serverUrl,ws,views)`(origin 作用域**干净替换**:先 purge 旧镜像再重写)+ `cachedCloudPageTree(serverUrl)`;(4) main.dart `_cacheCloudPageTree()` 改成写 store(origin=baseUri),**删** `cloudPageTreeToJson`/`FromJson` + 两个模型 `toJson`(prefs 那套已被 store 取代,连同其单测一并删)。`origin` 作为 store 方法的作用域参而非 `ViewData`/`WorkspaceData` 字段,保持这两个 record 为纯内容。`flutter analyze` 净(仅存量 info),套件 230 绿(-2 = 删掉的 prefs 缓存单测)。**镜像写入已通,尚无读取方**——P1c 才接离线读回退。
- ✅ **Phase 1c 完成(离线读取回退,2026-07-10)**:镜像终于有了读取方——server 模式断网/重启还能进工作区、列页树、开已缓存的云笔记。三处接线(全 `kIsWeb` gate,web 保持在线):
  1. **启动恢复回退**(`_restoreSession` catch):网络错(非 `unauthorized`)→ `_applyOfflineCloudNav(session)`:`_local.cachedCloudPageTree(baseUri)` 读 store 镜像,纯函数 `rebuildCloudNavFromCache(cache, userId)` 重建 `_workspaces`+`_viewsByWorkspace`,set `_session`,并把选中工作区的首个视图离线打开。`unauthorized` 仍清 session 回登录页。
  2. **doc-open(chicken-and-egg 用同步 load 化解,比原 async 草案更干净)**:新增 `LocalOffline.openCloudDocMirror(docId)`——直接 `store.loadDoc(docId)` 取 `rootBlockId`+blocks(镜像副本已含 root 块),`_offlineCloudBootstrap(view)` **同步**构一个完整 `_localBootstrapFrom` bootstrap(正确 rootBlockId,不是空 placeholder),秒开无需等 `onReady`。随后 `_reconcileSync` 起 `CloudSyncSession`(同 docId)本地 seed + 联网时对账。`rootBlockId` 是渲染承重件(`DocumentBootstrap.childBlocks` 靠它在 payload 里找 root),同步 load 保证它一开始就对,故**不必**改 `_applyCloudBlocks`/onReady。
  3. **tap 已缓存视图离线打开**(`_selectView` 的 `_run` 内):`bootstrapDocument` 抛错(非 auth)→ `_offlineCloudBootstrap(view)` 兜底;未缓存的 doc → bootstrap=null → 空编辑区(诚实,不伪造空文档)。
  - **镜像有损**:只存 id/name/position/tree,`role` 离线强制 `'viewer'`(读-only,`matchesEditRole` 门自动挡编辑,离线编辑是 P2)、`objectType='document'`、`ownerId=当前用户`;联网下次成功加载即恢复真值。要精确需给 `local_view`/`local_workspace` 加列(连同 P3 复合主键一起做)。
  - 测:纯函数 `rebuildCloudNavFromCache` 单测 2 例(分组/顺序/只读角色 + 空视图);FFI 集成测 `origin scopes ... through FFI`(-d windows 绿)。`flutter analyze` 净、套件 232 绿。**在线路径逐字未变**(仅异常分支新增兜底)。
  - **对抗复审(commit b5b9e54)抓到 3 个真问题,已修**:①【高】`_applyOfflineCloudNav` 读镜像前**从没开 store**(云冷启动路径只有 `_initLocalOffline`/`_setupCloudYrs` 开 store,而它们此时都没跑)→ 断网冷重启时 `cachedCloudPageTree` 恒 null、功能静默空转。修:改 async,先 `await _local.deviceClientId()`(开 store,幂等)再读,`if(!mounted)` 守卫。②【中】`_selectView` 把**所有非 `unauthorized` 错误**都当离线兜底,把服务端删/撤权的 doc(403/404/500)渲染成活的可编辑幽灵。修:`on ApiException { rethrow }`(服务端应答的错误上抛),只对真连接失败(非 ApiException = SocketException/ClientException)兜底。③【中】离线回退后**联网不刷新**,`role='viewer'` 一直卡着、owner 重连也不能编辑自己的 doc。修:加 `_offlineNav` 标志 + `_recoverOnlineNav()`;`CloudSyncSession` 加 `onServerConnected`(收到首个服务端帧即"上线"信号)接 `_recoverOnlineNav` → `_refreshWorkspaces` 拉回真角色;`_selectView` 联网成功也触发。新增 FFI 集成测 `cloud page tree + doc mirror survive a fresh LocalOffline`(-d windows 绿)端到端验证镜像→"重启"→读回。web 端 `onServerConnected` 收参但忽略(离线 nav 桌面-only)。
  - 未做(留 P2):离线**编辑**(outbox 从 prefs `cloudUnacked` 迁到 `updates_after(pushed_clock)`、重连 CRDT 对账);通用云图片下载 `putBlobAs` 落 CAS;工作区切换的离线兜底(现仅启动 + tap 已接,切换工作区离线仍走在线报错)。

## 复用率 / 工作量速估

- 服务端:~0(已是 nbstore 服务端形态)。
- Phase 0:小(纯 FFI 桥接 + 测)。
- Phase 1:中(云会话 write-through + 页树缓存 + 图片镜像)。
- Phase 2:中大(合并 backend + outbox 迁移 + 重连对账走通)。
- Phase 3:中(schema 统一 + 迁移 + UX)。
- 大部分是**把已有 Rust 能力桥出来 + 让云路径复用本地路径**,不是新造。
