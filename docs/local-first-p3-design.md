# Mica P3 设计:溶解双模式 —— 「工作区:本地 / 已连云」共存

> 状态:**已审批开工**(2026-07-11)。§9 决策已拍板:①单活跃服务器 ②**已登录则默认云**(未登录默认本地——开发者改推荐)③detach 进 P3f 可砍 ④上云后默认删除+可选保留 ⑤origin 保持 URL。
>
> 原状态:待开发者审批(2026-07-10)。调研 provenance:**AFFiNE 实证**(子代理读真实源码 toeverything/AFFiNE@a868f54:`workspace/metadata.ts`、`workspace-engine/impls/{local,cloud}.ts`、`workspace-selector/*`、`services/transform.ts`);**AppFlowy 复用 2026-06 已有实读**(AppFlowy-IO/AppFlowy@4af02cdc:`AuthType{Local|AppFlowyCloud}` 切同一服务 trait、匿名用户假 email 反模式、`AnonUserWorkspaceTableMigration` 教训);**mica-current 全部关键行号已由主代理直接核实**(main.dart:75/99-141/303/343/912/1595/1936/2785-2812/3048、store.rs:28/104-135/524-617、FRB store.rs:220-253)。
>
> 结论沿用 `local-first-plan.md` 拍板:P0-P2 已备好地基(单 store、origin+role 镜像、append-log outbox、离线读写、CloudSyncSession 对账),**P3 是纯客户端 nav/UI/身份/schema 重构,不动同步协议、不动服务端**。

---

## 0. 参照系结论(直接决定 P3 形态)

AFFiNE 是本设计的主参照(它已经活在 P3 的终态里),三条硬结论:

1. **没有全局模式**。AFFiNE 全仓找不到任何 app 级 online/local 开关;共存粒度是 per-workspace `flavour`('local' 或某 server id),`WorkspaceMetadata = {id, flavour}`,`(flavour, id)` 是全引擎的事实复合主键(storage 构造、universalId 都带)。→ **直接背书 Mica 的 `origin` 列 + P1b-2′ 复审的 `(origin,id)` 复合主键**。
2. **上云 = 复制到新工作区,不是原地翻 flavour**。`transformLocalToCloud` 建新云工作区(新 id)、重放 root doc + 全部 subdoc yrs 字节 + blobs,然后删本地;源不是 'local' 直接 throw。**全仓无 cloud→local 反向路径**——AFFiNE 的"双向"只是"云工作区的本地副本永远离线可读写",detach 是它刻意不做的 scope 决策,不是技术不可行。→ Mica 的 `_runWorkspaceMigration` 形态(复制)是对的;detach 若做,是超出参照系的自选动作(§6)。
3. **signed-out ≠ offline**。登出隐藏该服务器的云工作区(`revalidate()` 无 accountId → 清列表);但**已登录+断网**时从持久缓存(globalState `'cloud-workspace:'+accountId`)恢复工作区列表,против本地副本打开,卡片挂 "Offline" 徽标。→ 这正是 Mica 缺的"离线切工作区"的答案(§5),且 Mica 的镜像已经在 SQLite 里,比 AFFiNE 的 globalState 缓存还扎实。

AppFlowy 补一条反模式警告:匿名本地身份硬编码假 email `anon@appflowy.io` 导致到处特判。Mica 现在的 `_localSession`(main.dart:303,token `'local-offline'`、user id `'local'`)就是同款假身份,P3 要拆掉,不是换一个更像样的假身份(§2.4)。

---

## 1. 目标 UX

### 1.1 一张工作区列表,两种出身

侧栏工作区切换器(现 `WorkspaceView` 的工作区下拉)改为**分组单列表**,对齐 AFFiNE `AFFiNEWorkspaceList`:

```
┌──────────────────────────────┐
│ ☁ Mica Cloud  you@mail.com ⋮ │   ← 云区头:服务器名 + 账号;未登录时显示
│    ├ 团队笔记      [已同步]   │     "未登录" + 行内「登录」入口
│    └ 项目甲   [离线·可编辑]   │   ← per-card 徽标(见 1.2)
│ ──────────────────────────── │
│ 💻 本地                      │   ← 本地区头:无账号行
│    ├ 本地工作区               │
│    └ 草稿     (悬停:上云 ↑)  │   ← 悬停出「上云」按钮(AFFiNE 同款)
│ ──────────────────────────── │
│ ＋ 新建工作区…                │   ← 一个入口,对话框里选类型(1.4)
└──────────────────────────────┘
```

- **web**:没有本地 store(`local_offline_web.dart` 全 no-op)→ 本地区整段不渲染,列表 = 纯云区;未登录时云区就是登录面板(等效今天的整屏门,只是换了框)。不学 AFFiNE 的 "local demo workspace" 红警告——我们干脆不在 web 提供本地工作区,更诚实。
- **服务器数量**:P3 保持**同时只有一个活跃云服务器**(Mica Cloud 或一个自建 URL),即云区最多一段(推荐理由见 §9-1)。数据模型(origin=URL)天然支持未来多服务器,UI 先不开。

### 1.2 Per-workspace 出身与同步徽标

每张工作区卡片一行状态(对齐 AFFiNE `WorkspaceCard`):

| 状态 | 显示 | 数据源 |
|---|---|---|
| 本地 | 💻 本地 | `origin == 'local'` |
| 云·已同步 | ☁ 已同步 | 选中且 `CloudSyncSession` outbox 空(`persistence.outboxAfter(pushedClock).isEmpty`) |
| 云·待上传 | ☁ 待同步 n 条 | outbox 非空(P2b 的 `_pushStalled` 熔断也在这里 surface) |
| 云·离线 | ☁ 离线(仍可编辑) | `_offlineNav`(P3e 改 per-origin)为真 |
| 云·只读 | ☁ 只读 | 镜像 `role`(P2d 已持久化)不满足 `matchesEditRole` |

非选中工作区不建会话,徽标退化为静态出身图标——够用,不为徽标开 N 条 WS。

### 1.3 登录住在哪

**登录从「app 的门」降级为「连接一个云服务器」的动作**:

- `build()` 的整屏登录 Row(main.dart:2793-2824)在桌面上**删除**:首启直接进默认本地工作区(`resolve()` 的桌面 local-first 默认已是如此,只是不再需要 `onUseLocal` 按钮 main.dart:2805——它连同 `SidePanel.onUseLocal` prop 一起退役)。
- 登录入口两处:切换器云区头的「登录」行(未登录时)、设置页「云服务器」节。都弹现有 `_promptCloudAuth`(main.dart:2093)风格的对话框(email/密码 + 可选自建 URL),成功后云区出现该账号的工作区。
- **登出 = 收起云区**(隐藏该 origin 的工作区,镜像行保留在 store 里不删),本地区不受影响。`_signOut` 不再是"清世界"。
- **断网 ≠ 登出**:有持久 session + 断网 → 云区照常列出(来自 store 镜像,§5),卡片挂「离线」。

### 1.4 新建工作区:一个对话框,类型二选一

现在本地/云各有 `_createWorkspace`/`_localCreateWorkspace`。P3 合并为一个对话框:名字 + 类型选择(本地 / 云)。默认值(拍板):**未登录默认「本地」,已登录默认「云」**(登录这个动作本身表达了协作意图;未登录仍是 local-first 哲学),web 恒「云」且无选择器。选云但未登录 → 先走登录(AFFiNE 同款 reroute)。

### 1.5 双向入口

- **本地 → 云**:本地卡片悬停「上云」/ 右键菜单,走现有 `_migrateLocalWorkspaceToCloud`(main.dart:1936)——语义仍是**复制到新云工作区**(AFFiNE 实证这是正确形态,不做原地翻 origin),完成后落点见 §6.1。
- **云 → 本地(detach)**:云卡片菜单「断开服务器,转为本地工作区」——P3 最小版,机制与 scope 见 §6.2 和 §9-3。

---

## 2. 架构收敛

### 2.1 统一工作区模型与选中态

```dart
/// 工作区的全局唯一引用:origin 是 'local' 或服务器 URL(store 的 origin 语义,P1b-2′ 已定)。
typedef WorkspaceRef = ({String origin, String id});

class WorkspaceEntry {
  final String origin;        // 'local' | serverUrl
  final Workspace workspace;  // 现有模型复用
  final String role;          // 本地恒 'owner';云来自 API 或镜像(P2d)
  bool get isLocal => origin == 'local';
}
```

状态收敛(main.dart `_MicaAppState`):

| 现状(两套) | P3(一套) |
|---|---|
| `_workspaces` / `_localWorkspaces` | `List<WorkspaceEntry> _workspaceEntries`(build 时按 origin 分组渲染) |
| `_selectedWorkspace` / `_localSelectedWorkspace` | `WorkspaceRef? _selected` + 由它索引 entry |
| `_viewsByWorkspace`(裸 wsId key)/ `_localViews` | `Map<WorkspaceRef, List<DocumentView>> _viewsByWorkspace`(key 带 origin,消掉裸 id 撞键) |
| `_selectedView/_selectedBootstrap/_selectedMarkdown` / `_localSelectedView/_localBootstrap` | 各一份(选中态本来就该只有一份) |
| `_localEditorEpoch` | `_editorEpoch`(云路径恒 0,回滚是 local-only 能力) |
| `_session` / 静态假 `_localSession` | `_session`(可空,只表示云身份);**`_localSession` 删除**(§2.4) |
| `_membersByWorkspace`、`_presence` | 保留,只对云 origin 有值;本地 entry 查不到 → UI 隐藏 |

### 2.2 Handler 收敛:19 对 → 一套 + origin 分派

每对 handler 合并成一个,开头按 `_selectedEntry.isLocal` 分派,两个现有函数体成为私有分支(先机械合并,不重写逻辑):

```dart
Future<void> _createDocument(String name, {String? parentViewId}) =>
    _selectedEntry!.isLocal
        ? _localCreateDocument(name, parentViewId: parentViewId)
        : _cloudCreateDocument(name, parentViewId: parentViewId);
```

覆盖清单(main.dart 1638-2336 的本地全集 ↔ 822-1636 的云侧):selectWorkspace、create/rename/deleteWorkspace、createDocument(+Child)、selectView、rename/delete/reorder/restore/purgeView、loadTrash、updateRootBlockText、applyEditorOperations(§4 单独处理)、图片四件套(云 REST vs 本地 CAS)、importTree、onRefresh。**云独有**(搜索/导出/成员/AI/token/profile)与**本地独有**(rollbackDoc)不合并,变成 capability 可空 prop(§2.3)。

### 2.3 一次 WorkspaceView 实例化

`build()`(main.dart:2785)的三岔(localShell / 登录 Row / 云 WorkspaceView)收敛为**一次** `WorkspaceView` 实例化:

- `_buildLocalShell`(main.dart:2937-3053)**整体删除**。
- 现在本地壳喂的 no-op stub 闭包(搜索/导出 zip/AI/成员/presence/token/改密)改为 **prop 可空**:`WorkspaceView` 把这些回调声明为 nullable,null → 对应菜单项/面板不渲染。这是「capability = prop 是否为 null」的最小模型,不建 capability 框架(§7-4)。
- 本地独有 prop(`onRestoreCheckpoint`/`editorEpoch`)与云独有(`presence`/`members`)同理:按选中 entry 的 origin 传值或 null。
- `onMigrateToCloud`(main.dart:3048)从壳级 prop 移到工作区卡片菜单(§1.5)。

### 2.4 身份:删掉 `_localSession` 假身份

AppFlowy 的教训是假 email 到处特判。P3:

- `WorkspaceView.session` 改**可空**:null = 无云身份(纯本地使用)。账号 UI(头像/登出/改密)只在非空时渲染。
- 本地工作区的编辑不需要 AuthSession——编辑器权限门 `matchesEditRole` 对 local origin 恒 owner(P2d 已如此),`ownerId` 之类展示字段用空串或本地占位。
- CRDT 身份与登录无关:`LocalStore` 的 53-bit `client_id`(P0 已有)就是本地第一性身份,对齐 AppFlowy 的 anon uid 设计。

### 2.5 ServerMode / ServerConfig / prefs 退役与迁移

- `enum ServerMode`、`ServerConfig`(main.dart:75-150)、`_saveServerConfig`(343,"换世界"语义)全部删除。替代物:`String? _activeCloudOrigin`(prefs 键 `cloudOrigin`,null = 未配置云)。
- **认证改 per-origin 键**:`authToken:<origin>` / `authUser:<origin>`(origin 规范化后的 URL)。单活跃服务器下同时只有一份在用,但换服务器不再销毁旧服务器的 token(回切免重登)。
- **读侧迁移**(一次性,启动时):

| 旧 prefs | 迁移动作 |
|---|---|
| `serverMode=='local'` | 无(本地工作区自然出现在列表);删旧键 |
| `serverMode=='online'` + `authToken` | `cloudOrigin = serverUrl(空则 kMicaCloudUrl)`;token/user 改写为 `authToken:<origin>`;启动自动连接该 origin(等效今天的 `_restoreSession`) |
| `serverMode=='online'` 无 token | `cloudOrigin = serverUrl`,云区显示"未登录" |
| legacy `'cloud'`/`'self'` | 沿用现有 resolve() 的映射后再走上行 |
| `migrated:<wsId>` | 语义废弃(§6.1),键留着不读(无害) |

- **`kDevAutoLogin`**(main.dart:31,现只在 online+localhost 触发,325):改为"启动时若 `cloudOrigin` 未设且 dev define 打开 → 自动把 dev origin 设为活跃云服务器并登录"。放进新的 connect 流程,不再依赖 mode。

### 2.6 Web 退化

web 无本地 store → 溶解后的 UI 在 web 上自然退化为"只有云区":

- 本地区、「新建本地工作区」、「上云/detach」、离线徽标全部由 `LocalOffline.supported`(io=true / web stub=false)一个开关隐藏——把散点 `kIsWeb`(2691/2726/1937/2805 等)收敛到这一个 capability 位。
- 未登录的 web = 云区空 → 渲染登录面板占据列表区,行为与今天的整屏门等效。
- web 的 `CloudSyncSession`(yjs 变体)与 REST 路径**逐字不变**(§4 的 REST 兜底在 web 保留)。

---

## 3. Schema:`(origin,id)` 复合主键(SCHEMA_VERSION 3→4)

### 3.1 为什么是现在

P1b-2′ 复审已标:`local_view`/`local_workspace` 主键是裸 `id`(store.rs:105/114),origin 只在 `WHERE` 过滤;`save_*` 的 `ON CONFLICT(id)`(store.rs:552/603)、`purge_view(id)`(572)、`delete_workspace(id)`(611)都按裸 id 操作。P2 之前这是理论风险(两侧都 UUID,碰撞概率忽略);**P3 把它变成必然事故**:cloud→local detach(§6.2)的最自然实现就是"同一 id 在 'local' 和 serverUrl 两个 origin 下各有一行"(detach 后云端还在、下次登录又镜像回来)——裸 id PK 下第二行会 upsert 覆盖第一行,`purge_view` 会跨 origin 误删。**隔离必须从约定升级为约束,且必须在任何双向流程落地之前。**

### 3.2 迁移(SQLite 不能 ALTER PK → 重建表)

`store.rs:28` `SCHEMA_VERSION: i64 = 3 → 4`。沿用现有 pragma 探测模式,判据改查主键构成(`pragma_table_info` 的 `pk` 列):

```sql
-- local_view(local_workspace 同理)
CREATE TABLE local_view_v4(
    origin    TEXT NOT NULL DEFAULT 'local',
    id        TEXT NOT NULL,
    workspace_id TEXT NOT NULL DEFAULT 'local',
    parent_id TEXT,
    object_id TEXT NOT NULL,
    name      TEXT NOT NULL,
    position  TEXT NOT NULL,
    trashed   INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(origin, id)
);
INSERT INTO local_view_v4 SELECT origin,id,workspace_id,parent_id,object_id,name,position,trashed FROM local_view;
DROP TABLE local_view;
ALTER TABLE local_view_v4 RENAME TO local_view;
```

整段包在一个事务里。**降级门必须在任何迁移之前跑**(P3a 复审 Attack-7 抓到:原门在 rebuild 之后,probe-driven 的破坏性重建会先把未来 schema 的表按 v4 列表阉割再被拒——已修:门挪到 CREATE IF NOT EXISTS 批之后、一切 ALTER/rebuild 之前,配伪 v5 库回归测钉死「拒绝时结构未动」)。`doc_snapshot`/`doc_update`/`sync_cursor` **不动**——doc 以云 UUID/本地 id 直接 key,两个空间都是自生成 UUID 且 detach 时 doc 行是**同一份内容**(刻意共享,见 §6.2),不需要 origin 维度。

### 3.3 SQL / FFI / facade 波及面

| 层 | 改动 |
|---|---|
| mica-core `store.rs` | `save_view`/`save_workspace` → `ON CONFLICT(origin,id)`;`purge_view(origin,id)`、`delete_workspace(origin,id)`(其内部 `DELETE FROM local_view WHERE workspace_id=?` 也加 `AND origin=?`);`list_*` 不变(已带 origin) |
| FRB `rust/src/api/store.rs:226/251` | `purge_view`/`delete_workspace` 加 `origin: String` 参;`flutter_rust_bridge_codegen generate` 重生成 `frb_generated.dart`/`store.dart` |
| facade `local_offline_io.dart` | `purgeView`/`deleteWorkspace` 穿 origin(默认 `'local'`,现有本地调用点零改动);`mirrorCloudPageTree` 的清换 purge 循环显式带 serverUrl origin(从此**结构性不可能**误删他 origin 行);web stub 同签名 no-op |
| main.dart | `_localPurgeView`/`_localDeleteWorkspace` 传 `'local'`(默认参可不动);P3b 后统一 handler 按选中 entry 的 origin 传 |

### 3.4 验证

- Rust 单测:v3 库(裸 id PK + 数据)开库 → 迁移后数据完好、PK 复合(`pragma_table_info` 断言);**同 id 双 origin 共存**:`save_view(origin='local',id=X)` + `save_view(origin=url,id=X)` → 两行都在、`purge_view('local',X)` 只删一行。
- FFI 集成测(`frb_store_test.dart` 风格,-d windows):同 id 跨 origin 经真实 Dart→FFI 往返隔离。
- 现有 16+ mica-core 测、232 单元套件无回归。

---

## 4. op 路由单一化

现状三岔(main.dart:1595-1636 + 本地壳的 `_localApplyEditorOperations`):① `_cloudSession.isReady` → `applyLocalOps`(P2b 起即 append-log outbox);② 否则 REST `applyDocumentUpdate`(旧服务器兼容兜底);③ 本地壳 → `_local.applyOps`。

**P3 终态(桌面)**:

```dart
Future<void> _applyEditorOperations(List<Map<String, dynamic>> operations) async {
  if (_selectedEntry!.isLocal) { await _local.applyOps(...); return; }   // 原 _localApplyEditorOperations 体
  _cloudSession?.applyLocalOps(operations);                              // 永远走 CRDT
}
```

- **REST 兜底在桌面删除**。P2 后桌面云文档的 `isReady` 由本地 seed 置真(离线/冷启也 ready),ready==false 的窗口实际不存在;真正连不上服务器时编辑落 outbox,这正是 local-first 语义。删掉它同时消灭"REST 分支绕过 outbox 导致离线丢编辑"的暗道。
- **web 保留 REST 兜底**(`kIsWeb` 收在这一处):web 无 persistence,WS 未就绪的窗口真实存在,REST 是唯一网。web 在线路径逐字不变,P4 web local-first 时再撤。
- `_reconcileSync`(main.dart:509-550)删 `mode != localOffline` 守卫,改判据:选中 entry 是云 origin 且有该 origin 的 session → `_setupCloudYrs`;本地 origin → 拆云会话。
- **仍走 REST 的(明确不碰)**:auth、workspace/view CRUD(create/rename/delete/reorder/trash)、成员/邀请、搜索、导出、AI 流、token、blob 上传/下载。单一化只针对**文档内容 op**;页树操作的离线化(云工作区离线建页等)超出 P3(§7-2)。

---

## 5. 离线切工作区

现状:P1c 只覆盖启动回退(`_applyOfflineCloudNav`,main.dart:2725)和 tap 开页(`_selectView`:1385 的连接失败 catch → `_offlineCloudBootstrap`);`_selectWorkspace`(912)只走 REST(`_loadSelectedWorkspaceMembers`/`_loadSelectedWorkspaceViews`),断网即报错。

**P3e**,复用 P1c 的既有件,对齐 AFFiNE"signed-in-offline 从缓存开工作区":

1. `_selectWorkspace` 的云分支包 try/catch,**判据与 P1c 完全一致**:`on ApiException { rethrow }`(服务端应答的 403/404/500 上抛,不假装离线),真连接失败(SocketException/ClientException)→ `_local.cachedCloudPageTree(origin)` 取该工作区的视图子集填 `_viewsByWorkspace[ref]`,members 置空,置降级标志,并用 `_offlineCloudBootstrap` 开首个已缓存视图。
2. 若已处于降级态(标志为真),`_selectWorkspace` **直接走镜像**不再空试 REST(避免每次切换都等超时)。
3. `_offlineNav` 标志改 **per-origin**:`Set<String> _offlineOrigins`。单活跃服务器下退化为一个元素,但语义从此正确(本地工作区永不"离线")。`_recoverOnlineNav`(P1c 的 `onServerConnected` 钩子)恢复时按 origin 清标志 + `_refreshWorkspaces` 拉真数据。
4. 镜像已含 `role`(P2d)→ 离线切过去的工作区可编辑性正确,编辑落 outbox。

验证:在线登录多工作区 → 断网(或杀服务器)→ 在切换器里来回切两个云工作区:页树列出、页可开可编辑、徽标显示「离线」;回线 → 自动恢复真角色/成员,outbox 推空。配 `rebuildCloudNavFromCache` 风格纯函数单测(按工作区过滤)+ 实机断网截图(修复纪律)。

---

## 6. 双向 local↔cloud

### 6.1 本地 → 云(已有,P3 只搬家 + 语义微调)

`_migrateLocalWorkspaceToCloud`(main.dart:1936-2089)的机制保留(创建云工作区 → 视图先序重放 → blob 反向镜像 → headless CloudSyncSession 重放块树 → drainOutbox),AFFiNE 实证"复制到新 id"是正确形态。P3 改三点:

1. **入口搬家**:从本地壳 prop(3048)移到统一列表的本地卡片菜单/悬停按钮。未登录时先走登录(AFFiNE 的 "Sign in and Enable" 同款,`_promptCloudAuth` 已支持)。
2. **完成后的落点**:迁移成功 → 弹一次性选择「删除本地原件 / 保留」(AFFiNE 直接删;我们默认**删除**但给逃生口,见 §9-4)。选保留则本地件继续独立存在(两个不同工作区,不是同一工作区的两个 origin)。
3. **`migrated:<wsId>` 门废弃**:选删除则无二次迁移问题;选保留的用户再迁移就再复制一份——责任交给显式选择,不靠隐藏 pref 挡。

### 6.2 云 → 本地(detach):P3 最小版

AFFiNE 没有这条路(scope 决策非技术障碍)。Mica 做,因为 P2 之后它几乎是免费的——镜像已是全量副本:

**机制**(全部在客户端,服务端零改动):

1. 前置:复合主键已落(P3a),否则跨 origin 拷贝行会撞键。
2. 「断开服务器,转为本地」= 事务内:
   - `local_workspace`/`local_view` 该工作区的行**复制**一份 origin='local'(不是 UPDATE 翻转——云端工作区还在,下次在线镜像会重建 serverUrl origin 的行,复制才无幻灭/复活纠缠);`role` 置 'owner'。
   - 文档内容零拷贝:`doc_snapshot` 按 docId 直接 key,本地视图行的 `object_id` 指向同一份 doc。**但**先 `drainOutbox` 排空未推送编辑(或明确警告),然后**删除该 doc 的 `sync_cursor` + 剩余 `doc_update`**——转本地后这些 doc 不再对账,残留 cursor 会污染将来重新连云。
   - blob 已在统一 CAS(sha256),零拷贝。
3. detach 后云端原工作区照常在云区列出(用户可另行离开/删除,走服务端 API)——两者从此是独立分叉,**不承诺再合并**(CRDT 上可行,产品上不承诺,和 AFFiNE 的"复制不合并"一致)。
4. **同 docId 双工作区打开的会话隔离**:detach 副本与云原件共享 doc 行 → 若两边都打开会互相写。最小版规避:detach 时对副本的 doc **换新 id**(`store.loadDoc` → 存为新 UUID,视图行 `object_id` 跟改)——多一次全量拷贝但完全解耦。**推荐换 id 版**,零共享零惊喜。

**scope 建议**:P3f 收尾做,且是 P3 里**唯一可砍**的步(§9-3)。

---

## 7. 排除法:P3 明确不建什么

1. **不建多服务器并发连接**。AFFiNE 有 server registry + per-server provider 池;Mica 单 `ApiClient` + 单 WS 会话,P3 只做"一个活跃云 origin"。数据模型(origin=URL)已为多服务器留位,UI/连接池留 P4+。
2. **不做云工作区的离线页树写操作**(离线在云工作区里建页/改名/挪树)。P3 的离线覆盖 = 读导航 + 文档内容编辑(outbox);页树 CRUD 仍需在线(REST)。页树本身 CRDT 化是另一个工程。
3. **不做原地 origin 翻转迁移**。上云永远是复制(AFFiNE 实证);detach 也是复制+换 doc id。省掉 CRDT 身份重绑/`sync_cursor` 语义换血这类深水区。
4. **不建 capability 框架**。"本地没有 AI/成员/搜索" = 对应回调 prop 传 null,`WorkspaceView` 见 null 藏 UI。一个 enum/注册表都不加。
5. **不建 server registry 表**。origin 继续用规范化 URL 字符串(P1b-2′ 已铺开);AFFiNE 的 registry-id 间接层在"URL 会换的多服务器"场景才回本(§9-5)。
6. **不动服务端、不动同步协议、不动 web 在线路径**。

---

## 8. 分步实施(每步独立可测可提交)

**P3a — Schema (origin,id) 复合主键(纯数据层,零 UX)** ✅ **完成(63d13b9 + 门修复 28841d8)**
- 实际交付补记:真库实测(dev 机 store 竟是 v1 时代,一次 open 走完 origin→role→PK 重建全链无损)+ 永久 `#[ignore]` 冒烟测 `upgrade_real_store_smoke`(MICA_REAL_STORE 指真库副本);复审 6/7 攻面 airtight,Attack-7 版本门顺序缺口已修(先拒后迁 + 伪 v5 库回归测)。
- §3 全部:v3→4 重建表迁移、SQL 改 `ON CONFLICT(origin,id)`、`purge_view`/`delete_workspace` 加 origin、FRB 重生成、facade 默认参穿透。
- 验证:§3.4(Rust 迁移+隔离测、FFI 集成测、232 套件)。
- 风险:迁移事务写错 = 用户页树损毁 → 迁移前 `doc_snapshot_backup` 同款思路对两表做一次性 `*_v3_backup` 快照(便宜,几十行);实测拿一个 v3 老库(现有 dev 机器就有)升级验证。

**P3b — 状态/handler 内部统一(行为保持,UI 不变)** ✅ **完成(b1910e4)**
- 实际形态:双实例化收敛为 _unifiedWorkspaceView(每个分歧 prop 按 _activeIsLocal 三元分派,函数体零改动);WorkspaceEntry/WorkspaceRef/_workspaceEntries/_selectedEntry 作 derived getter 落地。复审:prop-parity 逐项成立,零分歧。
- §2.1 `WorkspaceRef`/`WorkspaceEntry` + 字段合并;§2.2 十九对 handler 机械合并为 origin 分派(函数体不重写);`_viewsByWorkspace` 换复合 key。`build()` 仍按旧逻辑挑内容(mode 还在),但两个世界已喂同一套 state。
- 验证:现有单元 232 + FFI 集成全绿(这步的定义就是无行为变化);双模式各手测一轮基本流。
- 风险:纯重构量大 → 靠"函数体零改动、只动接线"纪律 + 套件回归压住。

**P3c — 溶解 ServerMode(UI 主刀)** ✅ **P3c-1 完成(8a81c70 + 修复入 80bd437);P3c-2(Settings 改版 + ServerMode 类型退役 + per-origin token)遗留为后续**
- P3c-1 交付:双世界并存(启动总开本地 store + 恢复云 session)、activeOrigin 持久化选世界、分组切换器(云区账号头/登录行 + 本地区 + 出身图标 + per-entry 行操作)、整屏登录门删除(桌面登录=对话框;web 保留门)、登出=收起云区回落本地、统一新建对话框(已登录默认云)、假身份 _localSession 删除(session 真可空)。复审抓到 4 处已修:①onSignOut 死按钮(按 session 不按世界分派)②LocalOffline.open() 单飞锁(并发初始化竞态)③token 过期回落本地世界 ④新建后 activeOrigin 跟随。
- **遗留(P3c-2)**:Settings 服务器节仍是旧 radio tiles(经 _saveServerConfig 的 activeOrigin 同步垫片工作正常);ServerMode/ServerConfig 类型仍在(作 Settings 存储 + 启动迁移输入);auth token 仍单键。均为打磨非破损。
- **桌面实机截图矩阵未做**(夜间无 computer-use 批准)——留晨间:{纯本地、登录在线、断网、登出} × {切换器、新建对话框、账号菜单}。
- §1 全部 UX:分组切换器、per-card 徽标、登录降级(删整屏门 + `onUseLocal`)、统一新建对话框、Settings「云服务器」节替换 radio tiles(main.dart:6800-6990);§2.3 单次 WorkspaceView 实例化 + `_buildLocalShell` 删除 + nullable prop;§2.4 删 `_localSession`;§2.5 prefs 迁移 + `kDevAutoLogin` 搬家;§2.6 web 用 `LocalOffline.supported` 收敛 kIsWeb。
- 验证:**UI-heavy,按修复纪律实机验证**——桌面截图矩阵:{纯本地、登录+在线、登录+断网、登出} × {切换器、卡片徽标、设置页};web 用 playwright-cli 截图(登录门等效、无本地区)。prefs 迁移单测(resolve() 风格纯函数):四种旧态 × 新态断言。老用户升级路径实测:一台 online 老配置、一台 local 老配置。
- 风险:P3 最大的一步,建议内部再切两个 commit(先单壳化、后登录/Settings 改版);`_signOut` 语义变化(不清世界)要重点回归"登出后本地工作区还在、镜像还在"。

**P3d — op 路由单一化** ✅ **完成(9edb021)**
- 单一 _applyEditorOperations 自分派;_reconcileSync 删 mode 守卫。设计偏差(有意):REST 兜底两端保留——其真实覆盖面是「未镜像 doc 冷启动 WS 未就绪窗口(在线)」,该窗口 applyLocalOps 会静默丢编辑;而「REST 旁路 outbox」不可达(论证入注释)。
- §4:桌面删 REST 兜底,web 保留;`_reconcileSync` 删 mode 守卫改 origin 判据;`_localApplyEditorOperations` 并入分派。
- 验证:桌面在线编辑(WS 帧可见)、离线编辑(outbox 增长、回线推空,复用 `cloud_sync_converge_test` 基建);web 在线编辑无回归(playwright)。
- 风险:低;若有旧服务器不支持 WS 同步的存量用户,REST 删除会断其编辑 → 确认无此类部署(自家 v0.3 服务器已带 WS)后再删。

**P3e — 离线切工作区** ✅ **完成(9edb021,与 P3d 同批)**
- _selectWorkspace 云分支:降级态直读镜像;真连接失败(非 ApiException)回退 _openWorkspaceFromMirror;_offlineNav 保持 bool(单活跃服务器)。
- §5:`_selectWorkspace` 镜像回退 + 降级态直读 + per-origin `_offlineOrigins`。
- 验证:§5 末尾实测脚本 + 纯函数单测;断网切换截图。
- 风险:低(P1c 模式复刻);注意 ApiException rethrow 纪律不复发 P1c 复审②。

**P3f — 双向收尾** ✅ **完成(80bd437,未砍)**
- 上云:入口搬本地行菜单、复用已登录 session、完成后删除/保留选择(默认删)、migrated: 门废弃;detach:云行菜单「转为本地副本」,换全新 doc id 零共享,FFI 集成测端到端绿。
- §6.1 迁移入口搬家 + 完成后删除/保留选择 + 废 `migrated:` 门;§6.2 detach 最小版(换 doc id 版)。
- 验证:迁移端到端(本地建→上云→云端可开、blob 可渲、选删除后本地消失);detach 端到端(云工作区→detach→断网重启本地副本完好可编辑→重新登录云原件照常镜像、两者互不影响);outbox 非空时 detach 的警告路径。
- 风险:detach 的 doc id 换血要覆盖视图 `object_id` 全部引用;`sync_cursor`/`doc_update` 清理漏了会污染重连——各配 Rust 断言测。

依赖链:P3a → P3b → P3c → (P3d、P3e 可并行)→ P3f。P3a/P3d/P3e 小而独立,P3b 是纯重构,P3c 是 UX 主刀。

---

## 9. 开放决策(附建议)

1. **单活跃服务器 vs 多服务器并存?** —— 建议 **P3 单活跃服务器**:Mica 是单 `ApiClient`/单 WS 会话架构,多服务器要连接池 + per-origin 会话表,收益(同时挂 Mica Cloud 和自建)对当前用户面为零。但 **token 存储按 per-origin 键落地**(§2.5),origin=URL 的数据模型不变——升多服务器时纯 UI/连接层扩展,无迁移。
2. **新建工作区默认类型?** —— 建议 **桌面默认本地**(web 恒云)。与 fresh-install local-first 默认(main.dart:129-139)一个哲学;AFFiNE 默认 cloud 是拉新付费的商业选择,Mica 无此诉求。已登录用户仍一键切云。
3. **cloud→local detach 进 P3 吗?** —— 建议 **进,作为 P3f 且标记可砍**。P2 镜像使其成本极低,而它是"双向"目标里唯一超出 AFFiNE 参照系的部分——若排期紧,砍它不伤 P3 主体(溶解模式);砍了则 P3 验收口径改为"双向 = 云工作区离线全能力 + 本地可上云"(即 AFFiNE 的双向定义)。
4. **上云后本地原件:删除还是保留?** —— 建议 **默认删除 + 完成对话框里可选保留**(§6.1)。AFFiNE 无条件删;保留会造出"看似同一份实为分叉"的双胞胎,长期比丢失更伤信任。选保留的明确告知"这是独立副本,不再同步"。
5. **origin 表示:URL 字符串 vs server-registry id?** —— 建议 **保持 URL**(P1b-2′/P2 已全线铺开,store/镜像/outbox 都按它 key)。registry-id 的收益(改 URL 不迁数据)只在多服务器+URL 可变场景成立;真遇到自建迁址,写一个 `UPDATE ... SET origin=new WHERE origin=old` 的一次性工具比现在引入间接层便宜得多。

---

**落点文件速查**:`crates/mica-core/src/store.rs`(v4 重建表迁移 + SQL 改复合键,:28/:104-135/:524-617)、`clients/mica_flutter/rust/src/api/store.rs`(:226/:251 加 origin 参 + FRB 重生成)、`lib/local/local_offline_io.dart` + `local_offline_web.dart`(facade 穿 origin、`supported` 位)、`lib/main.dart`(主刀:75-150 ServerMode 删、291-355 state 合并、912 离线切、1595 op 路由、1638-2336 handler 合并、1936-2089 迁移搬家、2639-2781 signOut/离线导航、2783-3053 单壳化、6800-6990 Settings)、`lib/…/WorkspaceView`(nullable capability prop、分组切换器、卡片徽标)。服务端 `crates/app-core`:**不动**。