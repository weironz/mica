# Phase 2:离线优先 + CRDT(双路线)实施方案

> 2026-06-06 定稿。承接 `docs/desktop-plan.md` 的 Phase 2。**新会话先读 `desktop-plan.md` + 本文 + CLAUDE.md。**
> 本文是「深入调研 AppFlowy + AFFiNE 后定的方案」,**写完即开始动代码的依据**。

## 0. 战略 pivot 与定位

- **从「云/web 优先」→ 双路线(云端 ‖ 桌面)**,桌面支持**纯离线**:不登录也能用,数据全在本地,联网时与云双向同步。
- **离线野心 = C 档**(多设备 / 离线↔云**自动无冲突合并**)→ 必须上 CRDT。
- **节奏 = 直奔 yrs**(不走老 op 模型弯路,编辑器↔核心绑定一开始就按 CRDT 设计)。
- **定位结论**:Mica 的栈(Rust 核心 + Flutter + 自绘编辑器 + Rust markdown 权威)在每条主轴上 **≈ AppFlowy**,几乎不贴 AFFiNE(后者 JS Yjs 在前端当数据面 + 随机不持久 clientID,与我们相反)。
  → **架构骨架跟 AppFlowy;借 AFFiNE 的「机制」不借「栈」;把 AFFiNE 踩过的坑设成设计红线(§10)。**

## 1. 已定技术选型(汇总)

| 项 | 选型 | 备注 |
|---|---|---|
| CRDT 内核 | **`yrs`(y-crdt)** | 非 Yjs(JS)、非 automerge;AppFlowy 同款 |
| 文档模型 | 扁平 `Map<id, block>` + `YArray` children + **块内 `TextRef`(Y.Text)逐字 CRDT** | §3 |
| 块属性 | `prop:` 命名空间 + 递归 native↔Y 映射(字段级 CRDT)+ `Boxed` 不透明逃生舱 | 借 BlockSuite |
| 本地数据库引擎 | **官方 SQLite** | Rust 接入大概率 `sqlx`(后端 PG 同库、async);`rusqlite` 备选。**Turso 记 watch(§11)** |
| 云端数据库 | Postgres(现有) | yrs update 存 `bytea` |
| 同步单调 ID | **Postgres `bigserial`**(per-workspace 单流)+ **yrs state-vector diff 兜底** | §5 |
| FFI | **flutter_rust_bridge v2** | 非 AppFlowy 自研 protobuf dispatch |
| 本地身份 | **无账号,但持久 `device_id` → 稳定 yrs `client_id`** | §6 |
| 对象存储 | 沿用 `file_id` 内容寻址;本地 CAS + 云 S3,blob 同步独立于 doc 同步 | §7 |

## 2. 三层架构与 trait 边界

核心 crate(`crates/` 内,暂名 `mica-collab`/`mica-core`)**storage- 与 transport-agnostic**,编译成库 → 云端服务进程 + 桌面 FFI 共用同一份。

```
            ┌─────────────── 同一个 Rust 核心 crate ───────────────┐
 Flutter ─FFI(frb v2)→  yrs 文档模型 + 同步引擎 + markdown 权威
            │   trait Store        trait ObjectStore     trait SyncTransport
            └──────┬───────────────────┬────────────────────┬──────┘
        云端 ┌─────┴─────┐       ┌──────┴──────┐      ┌──────┴──────┐
             │ PgStore   │       │ S3Store     │      │ WsServer    │
        桌面 │ SqliteStore│       │ LocalCasStore│     │ WsClient/None│
             └───────────┘       └─────────────┘      └─────────────┘
```

- **`trait Store`**:doc snapshot + update 日志的读写(借 AFFiNE 的 `DocStorage`/`SyncStorage` 正交抽象,本地/云同接口)。
- **`trait ObjectStore`**:blob presign/put/get,云=现有 `S3Config`(`crates/infra/src/storage.rs:14-71`,自研 SigV4),桌面=`LocalCasStore`。
- **`trait SyncTransport`**:交换 yrs update + 单调 Rid;桌面纯本地时为 `None`(同步层关闭)。
- 「纯本地」= 同步层关闭、只剩本地 Store/ObjectStore;云同步是可选挂载。

## 3. 文档 / CRDT 模型(yrs)

借鉴 BlockSuite(比 AppFlowy 更成熟的三处)+ AppFlowy 同构骨架:

- **一篇文档 = 一个 yrs `Doc`**;根 `MapRef` 挂:
  - `blocks: Map<block_id, Block(MapRef)>` —— **所有块扁平平铺**,非物理嵌套。
  - 每个 `Block` 是 `MapRef`,含 `id / ty(flavour) / version / parent / children(ArrayRef<String>) / prop:*`。
  - **块内富文本 = `TextRef`(Y.Text)直接作为块的 prop 挂上**(放弃 AppFlowy 的 `text_map`+`external_id` 间接层 → 块删文本删、无 GC、无二级查找)。
- **props 字段级 CRDT**:嵌套 object→`MapRef`、list→`ArrayRef`;大二进制/不想合并的值用 `Boxed` 式不透明包装(并发改不同字段不冲突)。
- **父子**:权威是 `children: ArrayRef`(顺序由 CRDT 收敛);**额外维护 `parent` 反向索引**(本地缓存,避免 BlockSuite `getParent` 全树 DFS 的 O(n))。
- **schema + version**:每块类型一个 `flavour` 字符串 + `version`;add/move 时校验父子合法性(role 约束)。`version` 为迁移留钩子,Rust 端做权威迁移。
- **拆块/并块**:`Text.split`/`Text.join` + addBlock/deleteBlock,**单 yrs transaction 内组合**;deleteBlock 带 **`bringChildrenTo`**(删块时子块原位提升,免孤儿)。
- **选区/光标不入 CRDT** —— 是自绘编辑器(`editor.dart` TextInputClient 层)的本地 UI 态;只有文档结构 + 文本进 yrs。
- **delta ↔ marks 双向映射层(关键工作量)**:yrs Y.Text 的 delta(insert/retain/delete + attributes)↔ Mica 的 marks-over-plaintext 模型。
  - 硬换行 `\`+换行 约定:**块内是普通字符,跨块换行才是结构操作**,两套必须分清。
  - inline 对象/mention = delta 里带 attributes 的 embed(零宽占位符,两端长度一致防 offset 错位)。
  - Markdown 语义仍以 Rust `crates/markdown` 为权威;round-trip 是不变量,延伸到 delta↔marks。

## 4. 本地持久化(SQLite via sqlx)

落盘模型(AppFlowy / AFFiNE 高度一致,采用):**base snapshot + (clock, update) 增量队列 + squash 折叠**。per-doc 粒度。

建议表(本地 SQLite):
```sql
-- 每个文档一份基线 + 一串增量 update
doc_snapshot(doc_id TEXT PRIMARY KEY, state BLOB, state_vector BLOB, updated_at INTEGER)
doc_update(doc_id TEXT, clock INTEGER, update BLOB, PRIMARY KEY(doc_id, clock))  -- WHERE doc_id=? ORDER BY clock
-- 同步游标(每 doc 相对云的高水位)
sync_cursor(doc_id TEXT PRIMARY KEY, last_synced_rid INTEGER, pushed_rid INTEGER)
-- 本地身份 / 元数据
local_meta(key TEXT PRIMARY KEY, value BLOB)   -- device_id, yrs client_id, workspace 列表…
-- 文件元数据(镜像云 FileRecord)
file(file_id TEXT PRIMARY KEY, object_key TEXT, name TEXT, mime TEXT, size INTEGER, sha256 TEXT, sync_state INTEGER)
```
- **squash**:把 `[snapshot, ...updates]` 用 yrs merge 折叠回单一 base + SV,删旧 update。可读触发(AFFiNE)或周期(AppFlowy);**覆盖加锁、只在更新时覆盖**(防并发覆盖,AFFiNE 真坑)。
- 加载:apply base + 按 clock 顺序 apply updates;遇坏 update 截断自愈。
- 数据目录**按 `user_id` 分目录**:`{appdata}/mica/{user_id}/`(哪怕当前只一个本地用户)。

## 5. 同步协议(bigserial 单调流 + SV 兜底)

走 AppFlowy 路线(你已选),**不学 AFFiNE 的纯 SV 协商**(它主动删了 seq):

- **云端 per-workspace 单流**:一个 workspace 一条 `bigserial` 序列,update 行带 `doc_id`。client 只需 **一个续传游标 `last_synced_rid`**;服务端按 Rid 范围分页拉(比 AFFiNE per-doc clock 表遍历高效)。
- **在线实时 与 离线重连 = 同一套**:服务端把 update 顺序写入流并分配 Rid;client (重)连时带 `last_synced_rid` → 服务端续传其后所有 update。无第二套合并逻辑。
- **SV diff 作兜底**:首次同步 / 流断裂 / 缺口检测时,降级用 yrs state-vector diff 对账纠偏。
- **⚠️ 本地序 → 入云 Rid 映射(必须正面处理)**:`bigserial` 是服务端单调,本地离线产生的 update 入云前没有 Rid。client 本地用临时序累积,推送上云时由服务端分配 Rid 并回执,client 更新游标(AppFlowy 的 client `last_message_id` 正是解此)。
- 传输:WebSocket(复用现有 `crates/api-server/src/routes/ws.rs` 的房间/广播骨架,payload 换成 yrs update + Rid)。

## 6. 本地身份与账号

- **纯离线 = 不登录任何远端账号,但首启即生成稳定本地身份**:`device_id`(随机,持久化)→ 派生稳定的 yrs `client_id`(u64,持久复用,**别每次启动随机** —— AFFiNE 随机不持久是其弱点)。
- **「本地」做成一种身份类型**(`AuthType::Local`),不是散落 if 分支;**不学 AppFlowy 的假 email anon hack** —— LocalIdentity 一等公民(无 email/token),登录云时再 attach cloud account。
- **本地 → 云迁移 = 原地挂载云 remote、不换 workspace ID、不删本地**(可逆、失败可回退;优于 AFFiNE「建新 ID + 复制 + 删旧」的不可逆脆弱迁移)。接云那一刻才要真实账号。

## 7. 对象存储(沿用 file_id,本地 CAS + 云 S3)

- 文档内只存 **`file_id`(内容寻址 sha256)**,不存 URL —— Mica 现状已如此,继续。
- `trait ObjectStore`:云 = 现有 `S3Config`(`crates/infra/src/storage.rs`);桌面 = `LocalCasStore { root }`,文件落 `{appdata}/mica/{user_id}/blobs/{sha256}.{ext}`,`download_url` 返回 `file://`。
- **blob 同步独立于 doc 同步**(借 AFFiNE):`list()` 求差集决定 up/download + `uploaded_at` 游标去重;云端 S3 presigned/multipart 续传。
- 前端 Dart 加载时 `resolve(file_id)`:在线查云、离线查本地 → 返回对应 URL。逻辑层几乎不改(已是 file_id→resolve)。

### 7.1 实施决策(2026-06-08,调研 AppFlowy + AFFiNE 源码定案)

> 背景:本地 CAS 用 `file_id = sha256`(M5),云端 `files.id = UUID`(`object_key` 内嵌 sha256)。「sha256↔UUID 两套 id 怎么对账」一度被当成 §6 迁移的核心难点。派子代理读两家真实源码后,**核心前提被证伪**——这难点是我们自找的。

- **两家都没有「两套 id 对账」问题,因为它们让内容哈希在本地和云端都直接当 blob id**,从根上消除了 UUID↔sha256 的二义。证据:
  - **AppFlowy**:云端 `FileId::from_bytes = base64url(sha256)+"."+ext`(`AppFlowy-Collab/.../importer/util.rs`),客户端上传前用 `FileId::from_path` 算出**同一个** id(`flowy-storage/src/manager.rs`)。S3 key = `{ws}/{parent}/{file_id}`。blob 路径里**没有 UUID**。
  - **AFFiNE**:blob id = `sha256(buffer).digest('base64url')`,`BlobStorage` 抽象(`nbstore/src/storage/blob.ts`)本地/云端**都只按 `key`(=哈希)寻址**;云端 REST/GraphQL 直接收 key。
- **blob 同步差集机制(抄 AFFiNE 三层,全按哈希)**:① 本地 per-remote `uploaded_at` 表当「待上传」首要信号(免网络);② 批量较大时拉一次 `remote.list()` 做集合差集跳过已存在;③ 服务端上传幂等(`alreadyUploaded`)。**下载 = `difference(remoteList, localList)`**。差集器**永不删 blob**——删除走显式 `delete(key)` + 独立 `release()`/GC,避免「一端删→同步把删除传播→误删他处仍引用的 blob」。blob 同步与 CRDT 同步**完全独立的任务/队列**。
- **§6 迁移 = 原地挂载,不是「建新 ID+复制+删旧」**。AFFiNE 正是后者(`transform.ts`:`factory.create` 建新云 ws → 拷 doc/blob → `deleteWorkspace` 删旧),其 data-loss 报告(#12155/#13941/#4694)直接来自这条非原子的「拷完删旧」。内容寻址让原地挂载变便宜:「推云」= 枚举本地 blob、把云端没有的上传(同一套差集)、开始同步 doc——**无需重生 ID、无需改写 doc 里的 blob 引用**。
- **AppFlowy 的反面教训**:它 doc 里存的是**内嵌 host+workspace_id 的完整 URL**(`get_object_url_v1`),迁移重生 ID 时**没改写图片 URL** → 图片全断(AppFlowy-Cloud #1307)。**存 URL 是迁移变难的根因;存裸 id 则安全**。

**Mica 的取舍(已落地方向)**:

- **不动稳定线上云**:线上 web/桌面云端正常,既有云 doc 的图片块已存 UUID `file_id`。把云端整体改成内容寻址(理想终态)会动到稳定产品 + 既有数据迁移,**风险高,暂不做**。
- **§7 读侧镜像(已实现并测,见 `local_offline_io.dart::putBlobAs` + `main.dart::_loadEditorImageBytes`)**:云端模式下加载图片先查本地 CAS(离线可用、省往返),miss 才 resolve+下载并按云 `file_id`(UUID)缓存进**同一个** `blobs/` 目录(UUID 与 sha256 文件名不冲突)。这就是 §7「在线查云、离线查本地」的下行半。
- **§6 迁移的 id 对账 = 迁移时一次性改写(已落地)**:因为 **Mica 的块存的是裸 `file_id`(不是 URL)**,把本地 blob 上云拿到 UUID 后改写图片块 `file_id`(sha256→UUID)是**有界、安全**的操作——正是 AppFlowy 用 URL 才会踩的坑,我们存裸 id 不会踩。**理想是内容寻址消除改写;在「不动线上云」约束下,有界改写是务实解**。
- **§6 全量迁移(已实现并端到端实测)**:`cloud/workspace_migration.dart`(纯核心,headless 可测)+ `main.dart::_runWorkspaceMigration`(编排)。**原地挂载**(本地数据只读、不删):建云工作区 → 逐页(父先于子)上传 blob(`uploadImage`,sha256→UUID 映射 + `putBlobAs` 本地镜像)→ 建云 doc → headless `CloudSyncSession` 把本地块树**重放为 ops 挂到云端 doc 的 root**(策略c:跳过本地 root、内容落云 root、子树重挂、图片 file_id 改写;**不写 meta** → 规避 `meta.root` LWW 冲突+孤儿块,非 AFFiNE「建新ID+拷+删旧」的 data-loss 路径)→ `drainOutbox` 等 ack 后 dispose。触发:本地页菜单「连接云端并迁移此工作区」+ 登录/注册对话框;`migrated:<wsId>` pref 防重。测:`test/workspace_migration_test.dart` 9 项 + `crates/mica-core/tests/ops.rs::migration_replays_onto_cloud_root_without_meta_collision` + `integration_test/migration_sync_test.dart`(起全栈:重放→服务端 fold→第二客户端读回,验证 file_id 已对账、子树完整)。
- **待建(§7 上行 differ,非阻塞增强)**:云端**离线**态下新插图片目前仍走云上传(离线会失败)。完整 §7 三层差集(本地 `uploaded_at` 标记 → `remote.list()` 集合差 → 服务端幂等)+ 离线插图落 CAS、重连后上传改写,属窄边角增强(主离线面是本地模式,已全覆盖;迁移期的 blob 上行已实现)。设计见上「blob 同步差集机制」,留作后续。

## 8. FFI 边界(flutter_rust_bridge v2)

- 核心 crate 编译成动态/静态库,**frb v2** 生成 Dart↔Rust 桥(省掉 AppFlowy 自研 .proto + codegen 一大摊)。
- 暴露给 Dart 的 API 按「编辑器意图」设计:`openDoc / applyEditorOp(insert/update/delete/move block, text delta) / subscribeDocChanges(stream) / resolveFile…`,**底层是 yrs transaction**。
- 编辑器(`render.dart`/`controller.dart`/`editor.dart`)从核心读文档、把编辑写成核心 API 调用;yrs 变更事件 → 推回 Dart 驱动重绘。
- web 端:核心 crate **不编译进 web**(web 仍走现有云端 API 路径);用条件导入/平台分支隔离 FFI。

## 9. 里程碑拆解(Phase 2)

| 里程碑 | 内容 | 价值 |
|---|---|---|
| **P2-M0 骨架** ✅ | `crates/mica-core`(共享数据面)+ `clients/mica_flutter/rust`(薄 frb 包装,独立 `[workspace]`)+ frb v2 2.12.0 + cargokit;Windows round-trip 实测全绿(`integration_test/frb_roundtrip_test.dart`)。`trait Store/ObjectStore/SyncTransport` 留到 M1/M2 随模型一起定。 | 管线已验证 ✅ |
| **P2-M1 yrs 文档模型** ✅ | 块结构(`block.rs`)+ 块内 Y.Text + **marks↔delta 映射**(`marks.rs`)+ `MicaDoc`(`doc.rs`):from/to_blocks、encode/decode、**编辑操作**(insert/update/delete〔可 bring_children〕/move/text_insert/delete/format/set_block_text/split/join,各一个 yrs txn)。**FFI**(`rust/src/api/document.rs` 的 `MicaDocument` opaque,块走 JSON)暴露给 Dart。**30 个 Rust 测试**(8 round-trip + 13 ops + 7 markdown 不变量 + 2 lib)+ **3 个 Windows FFI 集成测试**全绿。 | 核心数据面 ✅ |
| **P2-M2 本地存储 + 身份** ✅ | `crates/mica-core` `store` feature:`LocalStore`(rusqlite bundled SQLite,`doc_snapshot` 全量快照 + `local_meta`)+ 稳定 `Identity`(`device_id` uuid → 53-bit `client_id`,持久复用)。**FFI**(`rust/src/api/store.rs` 的 `MicaStore` opaque:open/client_id/device_id/list/delete/save/load)。**4 个 Rust store 测试**(round-trip / 改后重存 / list+delete / 身份跨重开稳定)+ **2 个 Windows FFI 集成测试**(`integration_test/frb_store_test.dart`:存→重开→载回字符级一致、身份稳定;缺失 doc → null)全绿。〔注:落盘是全量 snapshot,**update 增量队列 + squash 折叠**(§4)推迟到 M4 上云同步时一起做——单设备纯离线全量覆盖已够。rusqlite 钉 0.37 与后端 sqlx-sqlite 共用 libsqlite3-sys <0.38;FFI crate 锁里 `cargo update -p cc` 解 dart-sys 旧 cc 与 libsqlite3-sys 的版本冲突。〕 | **单设备纯离线已可用** ✅ |
| **P2-M3 编辑器绑定 + 本地工作区** ✅ | **① 引擎闭环**:自绘编辑器的单一 op 出口(`controller._send`→`onOps`)接 `LocalDocBackend`(`lib/local/local_doc.dart`):每个 `DocOp` 镜像进 `MicaDocument`(yrs),快照防抖落 `MicaStore`(SQLite)。core 粗粒度 `MicaDocument.update_block(kind?/text?/data?)` 对齐 update_block(有 text→text+marks 一起;仅 data→`set_block_marks` 调和,无 marks 即清,对应 turn-into 语义);`marks_from_runs` 跳过 `key:Null`。**② 本地工作区+页树**:`LocalStore` 加 `local_view` 表 + FFI;`lib/local/local_offline.dart`(io/web 条件导出,FFI 不入 web bundle)的 `LocalOffline` 门面封装 store+页树+活动文档;`main.dart` 合成本地身份 + `_local*` 回调,**复用 `WorkspaceView`**(纯 presenter,零云依赖)渲染本地页树+编辑器。Settings→Server 选 Local 即进入。图片/AI/协作在本地禁用(M5+)。**测试**:`set_block_marks` + `views_crud`/`views_survive_reopen` Rust 单测;`frb_editor_binding_test.dart`(真实 `EditorController` 跑打字/加粗/分裂/改 heading/合并→重开 store 一致,Windows 3 项)。 | 桌面纯离线可建页/编辑/持久化 ✅ |
| **P2-M4 云同步** | per-workspace bigserial 流 + 续传 + SV 兜底 + 本地序→Rid;**现有云端文档数据迁移**(snapshot→yrs base) | **双路线打通** |
| **P2-M5 对象存储双路** | 本地 CAS + 云 S3 复用 + blob 独立同步 | 图片离线可用 |
| **P2-M6 红线加固** | §10 全部红线 + 迁移路径(本地→云原地挂载)+ 跨版本迁移测试 | 生产可靠 |

> 注:P2-M4 含**存量云端数据迁移**——现有 `document_snapshots`(blocks JSON)→ 构造初始 yrs doc → 存为 base state。机械但需测试。现有 op 模型(`documents.rs`/`store.rs`/`ws.rs`)随之退役/重写。

## 10. 设计红线(AFFiNE 教训,违反即返工)

1. **同步前完整性校验,坏状态熔断停同步、绝不静默上传**(AFFiNE #7108:本地损坏→静默同步到云→无恢复)。
2. **本地快照 / 版本回滚兜底**(AFFiNE 缺此吃大亏);yrs SnapshotHistory + 回滚 update。
3. **存储路径 / schema 显式版本号 + 跨版本迁移测试**(AFFiNE #12155/#13941:升级丢 workspace、跨版本导入失败)。延伸 round-trip 不变量纪律。
4. **数据面权威单点留在 Rust** —— 别被「AFFiNE 用 Yjs 也行」诱回 JS;这正是规避 AFFiNE「编辑器↔Yjs 强耦合 + JS 热路径」两个结构性问题的优势。
5. **大文档**:per-doc 懒加载 + squash 控历史膨胀;自绘编辑器注意视口裁剪(AFFiNE #14333 edgeless 无裁剪卡顿)。

## 11. Watch list / 待决

- **Turso(纯 Rust SQLite 重写)**:作者明示 BETA / not production ready;成熟到「SQLite-level 可靠」后可低成本迁移(SQL + 文件格式兼容,且本地存储已隔离在 `trait Store` 后)。纯 Rust + async + MVCC 长期更贴核心。
- **强协同体验**:多人**同一块**实时字符级协同光标/awareness —— 基础 Y.Text 已支持字符级合并,协同光标是后续 UI。
- **`window_manager → nativeapi-flutter`** 迁移(见 desktop-plan.md,与 Phase 2 无关但同类 watch)。

## 12. 风险

- **最大风险 = delta↔marks 映射 + 跨块操作合并语义**(§3)。Y.Text 字符级只在单块内成立;跨块拆/并/选区删是结构操作组合,需专门设计 + 充分测试。
- 存量云端数据迁移到 yrs 的正确性(P2-M4)。
- frb v2 对自绘编辑器热路径的调用频率/延迟(IME、逐字输入);需 benchmark,必要时把热路径留在 Dart、只把结构/持久化过 FFI。
