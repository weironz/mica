# Mica P2 设计:云文档离线编辑(local-first 合并)

> 状态:**待开发者审批**(2026-07-10)。调研 provenance:AFFiNE `nbstore` 由子代理读真实源码实证(`sync/doc/peer.ts` 三时钟 remoteClock/pulledClock/pushedClock + pull-then-push);**AppFlowy 已补齐**(见 §0,源自本仓 codebase-memory obs 1012/1018/1029/1030/1031——2026-06-06 实读 `client-api/src/v2/db.rs`、`compactor.rs`、`PROTOCOL.md`、`collab.proto`);mica-current 的关键事实**已由主代理直接核实**(`appendUpdate`/`updatesAfter`/`squash` 只在生成的 FFI 绑定里、无编辑路径调用;云 outbox 是 prefs `cloudUnacked` + 400ms 整档 `_saveLocalSoon`)。
>
> 结论沿用 `docs/local-first-plan.md` 拍板:**这是接线,不是重写**。P1 已把云副本镜像进 `MicaStore`(离线读闭环),P2 只欠「离线**写** + outbox 迁到 append-log + 重连对账」。

---

## 0. AppFlowy 对照(closest analog,实证确认)

AppFlowy(Flutter 原生 + Rust + yrs,和 Mica 约束最像)的客户端存储与我们设计的 **Model B 几乎一模一样**,是最强的「路子对了」旁证:

- **存储 shape 同构**(`client-api/src/v2/db.rs`):RocksDB 键 `DOC_SPACE/object_id → doc state`(=我们 `doc_snapshot`)+ `DOC_STATE_VEC`(state vector)+ **`DOC_UPDATE/clock`(clock-keyed 增量 update 日志)**(=我们 `doc_update.clock`)。`load(flush=true)` 把增量折进快照(=我们的 compaction / `trim`/`squash`)。**`last_message_id`(Redis Rid)持久化用于断线续传**(=我们 `sync_cursor.last_synced_rid`)。
- **compaction 层**(`compactor.rs`):`ChannelReceiverCompactor` 非阻塞合并同 doc 的本地 update(`yrs::Update::merge_updates`,64KB 上限),纯性能、无正确性影响——**确认决策③**(P2 逐条推、P4 再合并)。
- **一处刻意分歧(强化排除法⑦)**:AppFlowy 的 push 走 **yrs state-vector 差量**(`SyncRequest{state_vector,lastRid}` → 服务器算出缺的字节),我们走 **raw-diff replay**(`updates_after(pushed_clock)` 逐条重放)。两者都对:raw-replay 靠 yrs update 幂等+位置无关(离线期算的 diff 联网后照样正确 apply),更简;state-vector 是省字节优化。**我们的 rid-only + 「流被裁则 re-bootstrap 合并」已覆盖正确性,state-vector 明确留 P4**(与 AppFlowy 结论一致,它也是 rid 续传 + state-vector 只做 diff 计算)。
- **⚠️ watch-item(client_id 宽度)**:AppFlowy 把 `client_id` 钉在 **32-bit**(`random u32`),注释指向一个 y-crdt(Rust)decoder bug;Mica 用 **53-bit**(`store.rs:246`「Yjs-compatible」)。Mica 的 yrs↔yjs 跨引擎互读已实测(`web_interop.rs` 绿),53-bit 当前工作正常,**非 P2 阻塞**;但记下:若 P2 更重的 update 流量哪天暴露大 client_id 的 decode 问题,32-bit 是现成兜底(32-bit ⊂ 53-bit,同时满足 yrs-Rust-bug 与 Yjs 兼容)。改宽度会废掉已持久化的 identity,故非轻改。
- **排除法再确认**:AppFlowy 同样**不做 OT / 不做手动冲突 UI / 不做字段锁**;冲突全交 yrs 合并;服务端只 append+广播 Rid 流。与 §2 一致。

关键事实校准(读代码得出,纠正一处直觉):**当前云路径和本地路径都还没用 append-log**。`CloudSyncSession._saveLocalSoon` 走的是 `CloudDocStore.save(fullState, cursor)` = **整份快照** write-through(P1),outbox 是 prefs `cloudUnacked:<docId>`;`LocalDocBackend.flush` 也是 `store.saveDoc`(整份快照)。`crates/mica-core/src/store.rs` 的 `append_update / updates_after / squash` 已在 P0 桥到 FFI(`store.dart`),但**无人调用**。P2 的核心就是让云编辑真正落到这条日志上。

---

## 1. 核心对账机制(重连算法 + 无损证明)

### 存储不变量(Model B:base=合并态,log=本地 outbox)

对每个云文档 `docId`(以云 UUID 为 key):

| 表 | 语义 | P2 写入时机 |
|---|---|---|
| `doc_snapshot(state)` | **完整合并副本**(本地编辑 + 已并入的远端 update),离线读用 | 编辑/远端合并后 debounce write-through(沿用 `_saveLocalSoon`) |
| `doc_update(clock,payload)` | **仅本地编辑的 yrs diff**(outbox),`clock` 单调递增 | 每次本地编辑**同步**追加(`append_update`,编辑瞬间,不 debounce) |
| `sync_cursor.last_synced_rid` | 已 pull 并 apply 的最高服务器流 `rid` | 收 `sync.updates`/`sync.ack` 时推进 |
| `sync_cursor.pushed_clock` | 已被服务器 ack 的最高本地 `clock` | 收 `sync.ack` 时推进 |

**未推送 outbox ≡ `updates_after(pushed_clock)`**。远端 update **不进 `doc_update`**(只并入 base + 推 `last_synced_rid`),所以日志纯本地,`pushed_clock` 语义干净。`load_doc` = base + replay(log),replay 幂等(即便 base 已含该 diff)。

### 重连消息流(用我们的协议,pull-then-push)

对标 AFFiNE `sync/doc/peer.ts` 的 `jobs.connect → pullAndPush`,顺序 = **先整合远端,再推本地**:

```
(重)连接 →
  0. 先 seed 副本 = load_doc(base+log),cursor = last_synced_rid   [_seedFromLocalOnce,已有,离线秒开]
  1. 冷启:sync.bootstrap → sync.base{base,base_rid}
       applyUpdate(base)   // CRDT 合并;未推送本地编辑存活
       last_synced_rid = max(cur, base_rid)
     热重连:跳过 bootstrap,保留副本
  2. sync.pull{since_rid = last_synced_rid} → sync.updates{updates[],head}
       for u in updates: applyUpdate(u); last_synced_rid = max(cur, u.rid)
       write-through base;若批被截断(head 更高)继续 pull   [已有 B2 catch-up 循环]
  3. for e in updates_after(pushed_clock) 按 clock 升序:
       sync.push{ id: e.clock, payload: e.payload }
     server 折叠(幂等) → sync.ack{ ack_id: e.clock, rid }
       pushed_clock = max(cur, ack_id);  last_synced_rid = max(cur, rid)
```

步骤 2 在 3 之前 = 服务器离线期的推进先并进本地副本,再把本地 diff 顶上去。

### 无损 / 无分叉证明

- **服务器离线期推进**:其 update 在步骤 2 经 `applyUpdate` CRDT 合并入副本。yrs 是整合非覆盖 → 不冲掉任何本地编辑。
- **本地离线编辑**:落在 `doc_update`,`clock > pushed_clock`,跨离线期与崩溃持久。步骤 3 精确推这些;服务器整合**同一份字节兼容 yrs update**,整合可交换+幂等 → 服务器收敛到「它自己的推进 ∪ 我的离线编辑」。
- **push 后、ack 前崩溃**:`pushed_clock` 未推进 → 重启后 `updates_after(pushed_clock)` 仍含该条 → 重推 → 服务器幂等再整合,**不重复不损坏**(和 AFFiNE 的 pushedClock high-water 同理)。
- **编辑后、快照 write-through 前(<400ms debounce)崩溃**:`append_update` 是**同步**的,编辑瞬间已落盘;快照即便丢了这段尾巴,`load_doc = base + replay(log)` 会把它补回。**这一点比现有 P1 的「纯快照 write-through」更强**——现状硬崩会丢最近 <400ms 编辑,P2 反而修掉这个数据丢失点。

---

## 2. 排除法结论:P2 明确**不建**什么

两个真参照(AFFiNE 实证、AppFlowy 同族)都跳过,且 Mica「全平台一套 yrs」使其 moot:

1. **不做 OT / 操作变换**。yrs 整合可交换幂等,双端离线并发编辑各自补 diff 即收敛,无「变换」可言。
2. **不做手动冲突解决 UI**(「你有冲突版本,请选一个」)。pull 把远端并入、push 把本地顶上,合并后的 CRDT 态就是唯一答案。
3. **不做服务端 transform / rebase**。`crates/app-core/src/sync.rs` 已是「哑 append+broadcast」流存储,只发 `rid`,不重排不改写客户端内容——保持不动。
4. **不做文档编辑锁 / 字段锁**。离线自由编辑,正确性来自 CRDT 而非互斥。
5. **不建独立 outbox 表**。未推送工作 = append-log 中 `clock > pushed_clock` 的行(推导,非显式队列),正是 AFFiNE 的做法。删掉 prefs `cloudUnacked` 这套「第二真相源」。
6. **不改用 wall-clock keying**。AFFiNE 用 `Date` 做 log/clock key 是它的**缺点**(时钟漂移风险);Mica 保留单调 `clock` + 服务器 `rid`,更稳,**不学这条**。
7. **不引入 state-vector 差量作为主路径**。我们有服务器 `rid`,`pull{since_rid}` 足够且更简。`diffUpdate`/state-vector 仅作 P4 的「超陈旧副本首连」兜底优化,不进 P2。

---

## 3. Backend 合并:让云文档走 append-log 编辑

现有两条:`LocalDocBackend`(`lib/local/local_doc.dart`,纯本地、无 WS、写快照)、`CloudSyncSession`(`lib/cloud/cloud_sync_io.dart`,WS 同步、内存副本、prefs outbox)。**共同底座**已存在:两者都用 `DocOpMirror`(`lib/local/doc_ops.dart`)把编辑器 op 翻成 yrs,都持 `MicaDocument`。差异只在「outbox 存哪」和「有没有挂 WS」。

**合并策略(分两刀,先低风险)**:把 outbox 存储抽象成接口,让 `CloudSyncSession` 的同步状态机(bootstrap/pull/push/ack/重连回退/fault 自愈——这些都好,保留)**只换 outbox 后端**;`LocalDocBackend` = 「没挂 WS 的同一后端」。

### 精确接缝

**(a) 扩接口 `CloudDocStore`**(`lib/cloud/cloud_doc_store.dart`)加 outbox 三件套:
```dart
int  appendOutbox(Uint8List diff);                 // → store.appendUpdate(docId,diff) 返回 clock
List<({int clock, Uint8List bytes})> outboxAfter(int pushedClock); // → store.updatesAfter
({int lastSyncedRid, int pushedClock}) cursor();   // → store.syncCursor
void advance({int? lastSyncedRid, int? pushedClock}); // → store.setSyncCursor
```
`load()` 语义改为 base+log replay(桌面 `StoreCloudDocStore` 内部改用 `store.loadDoc(docId)`,cursor 取 `syncCursor.lastSyncedRid`),使离线开文档能看到未落快照的尾巴编辑。实现在 `lib/local/local_offline_io.dart`(`StoreCloudDocStore`);web 变体 `local_offline_web.dart` 全部返回 null/no-op。

**(b) `CloudSyncSession` 内部按 `persistence` 是否存在二选一**:
- `persistence != null`(桌面):删 `_unacked`/`_Pending`/`_persistSoon`/`restoreUnacked`/`onPersistUnacked` 这套。
  - `_enqueue(diff)` → `final clock = persistence.appendOutbox(diff); _sendPush(clock, diff)`(`id = clock.toString()`)。
  - `sync.ack` 分支 → `persistence.advance(pushedClock: int(ackId), lastSyncedRid: rid)`。
  - `_flushUnacked(resendAll)` → `for e in persistence.outboxAfter(persistence.cursor().pushedClock) _sendPush(e.clock, e.bytes)`。
  - `_saveLocalSoon/_saveLocalNow` 保留(base 快照 write-through,离线读)。
- `persistence == null`(**web**):**逐字保留现状**(内存 `_unacked` + prefs)。这满足 kIsWeb「web 留在线」约束,且 web 无 SQLite。

**(c) 编辑 op 汇聚点**(`main.dart:_applyEditorOperations` ~1577):现在三分支(cloud ready→`applyLocalOps` / REST / localOffline→`_local.applyOps`)。P2 内保持这个分支(cloud 分支现在自动走 append-log);**P2d 再收敛**成「统一 local-first backend 的单一 `applyOps`」,把 REST 兜底和 `ServerMode` 硬分支溶掉(与 P3 的模式统一同批做,避免半路重构)。

**保留不动**:`DocOpMirror`、`MicaDocument` FFI、服务端 `sync.rs`/`ws.rs`、`CloudSyncSession` 的重连 backoff / B2 catch-up / `onFault` 自愈 / `onServerConnected`→`_recoverOnlineNav`(P1c)。

---

## 4. Outbox 迁移:prefs `cloudUnacked` → `updates_after(pushed_clock)`

**迁移(桌面,P2b 首次运行,一次性)**:打开文档时,若存在 prefs `cloudUnacked:<docId>`:把其中每条 base64 diff `appendOutbox` 进 `doc_update`(进入 outbox),然后删该 pref。幂等——这些 diff 若服务器早已整合,重推是 no-op;若没整合,现在被日志接管照推。放在 `_setupCloudYrs` 建会话前,或 `StoreCloudDocStore` 首次 attach 时。

**幂等 / 崩溃安全**:
- 追加即持久(同步 `append_update`);`pushed_clock` 只在 ack 后推进 → 任何时刻崩溃,`updates_after(pushed_clock)` 都是精确的待推集,重推幂等。
- ack 用 `ack_id = clock` 精确匹配到具体条目(替代旧 `_Pending.id`),跨重连重发不错配。
- `drainOutbox`(切文档/登出前排空,`_closeDocumentSync`→`drainAndDispose`)改判据:`persistence.outboxAfter(pushedClock).isEmpty`。

**压实(log 不能无界长)**:acked 条目(`clock ≤ pushed_clock`)在服务器和 base 里都有,可删。~~危险点:squash 无条件清空整条 log~~ **(已在 P2a 加固修掉——见下)**。压实用 P2a 新增的有界 `trim_updates_through(docId, pushed_clock)`(`DELETE ... WHERE clock ≤ pushed_clock`,ack 后调用,不碰未推送 outbox、不重编码整档),这是 §7 决策① 拍板的方案。

> **P2a 加固记(2026-07-10,对抗复审后)**:对 append-log 三个删除/写入原语焊死了 `pushed_clock` 下界,**safe-by-construction**,以防 P2b/P2e 接线时误伤未推送 outbox:
> - `append_update` clock = `max(MAX(clock), pushed_clock)+1`——跨 trim/squash 严格单调,不会 reset 到 1 撞已删 clock。
> - `squash` 从「无条件删全表」改为只删 `clock ≤ pushed_clock`(保留未推送尾巴,base 已折叠故 load 幂等不变;pure-local pushed=0 且日志本就没用 = 无害 re-baseline)。原来在有未推送尾巴时 squash 会永久丢那些编辑(只活在本机 base、永不同步出去)——真地雷,零调用方故休眠,已消。
> - `trim_updates_through` 内部 `clamp(up_to, pushed_clock)`,不寄托调用纪律。
> - `appendOutbox`(Dart)对 FFI `append_update` 的 `0`(被 `.unwrap_or(0)` 吞掉的 store 错误)抛异常,不静默丢。
> 回归:`trim_bounds_log_and_clock_stays_monotonic`、`squash_keeps_unpushed_tail_and_clock_monotonic`(mica-core)。

---

## 5. 分步实施(每步独立可测可提交,标注数据丢失点)

**P2a — Rust/FFI 使能(零行为变化)** ✅ **完成(commit 5e6197a + 加固)**
- 加 `trim_updates_through(doc_id, up_to_clock)`(mica-core `store.rs`)+ 桥到 FFI `store.dart`;扩 `CloudDocStore` 加 outbox 五方法(append/outboxAfter/cursor/advance/trim)+ `StoreCloudDocStore` 实现(io),web 无 impl(`persistence==null`)。附:修 append clock 单调 + squash/trim 焊死 pushed_clock 下界(见上「P2a 加固记」)。mica-core 18 测、FFI 集成测(-d windows)、analyze、web build 全绿。
- 验证:Rust 单测(追加→`updates_after`→`trim` 保留 `> pushed_clock`、幂等 replay);FFI 集成测(`frb_store_test.dart` 风格,-d windows)往返。
- 丢失点:无(纯新增,无人调用)。

**P2b — 桌面云 outbox 切 append-log(仍在线编辑)** ✅ **完成(commit 见 git log)**
- `CloudSyncSession` persistence 分支:编辑→`appendOutbox`(同步)+push;ack→`advance(pushedClock=max, lastSyncedRid=max)`(单调);重连→`_flushUnacked` 重发 `outboxAfter(pushed_clock)`(连内 `_sentThroughClock` 跳过在途);`drainOutbox` 判据 `_outboxEmpty`;`_restoreUnackedOnce` no-op。prefs `cloudUnacked` 在 `_setupCloudYrs` 一次性迁移(先追加后删)。web(`persistence==null`)**逐字保留** prefs 路径,`cloud_sync_web.dart` 未动。
- 验证:FFI 集成(-d windows,复用 `_FakeSyncServer`)—— ①编辑落 outbox→push→ack→`pushed_clock` 推进、outbox 排空;②未 ack 编辑跨会话重启仍在 outbox、重连按同 clock 重推。既有 B1/C1/B2/reconnect 无回归;单元 232 绿(web 不变)。
- 丢失点(均已处理):①迁移**先追加后删 pref**(反了=丢在途)。②`appendOutbox` 在 push 前**同步**完成。
- **对抗复审抓到高危丢数据(已修 e6a0ce9)**:原 ack 用 `max(cur, ackId)` 推 `pushed_clock`,但一条 push 可能被服务端回 `error`(非 ack,如 `push_update` 并发争用瞬时失败)→ clock 3 error、clock 4 ack → `pushed_clock` 跳到 4、outboxAfter(4) 永久漏 clock 3 = 静默分叉。「ack 顺序到→连续高水位」的假设漏了 push 可能回 error 打破连续。修:ack **只连续前缀推进**(`_ackedAhead` 存乱序 ack、只 `while remove(pushed+1)` 抬水位);新增 `case 'error'` 重推被拒 clock,`_pushRejects` 有界(仅连续进展清零,防高 clock 的 ack 让永久失败低 clock 死循环)。再复审(e3644a6):budget 耗尽 `_pushStalled` 熔断主动推送,止住永久拒推下 `_ackedAhead` 增长/重发风暴(编辑仍持久落 outbox、reconnect 重试)。测:`_FakeSyncServer` 加 rejectPushOnce/Always,瞬时拒推不丢 + 永久拒推有界+surfaced;9 集成测绿。

**P2c — 重连对账 pull-then-push 走通(无损收敛)** ✅ **完成(commit 见 git log)**
- §1 顺序在 P2b 已落地(`connect` 热重连先 `pull{since_rid}` → 再推 `outboxAfter`;远端 update 只进 base+`last_synced_rid`、不进 log 是结构性保证——`appendOutbox` 只被本地编辑调用)。P2c 是**验证**。
- 验证:`cloud_sync_converge_test.dart` 自建 fold+relay 假服务器(真 FFI 折叠 push、分 rid、广播给其它副本)—— A、B 各断网编辑不同块 → 依次重连 → 断言两端收敛到 `{a:hi A, b:yo B}`(各自离线编辑都在,B 那侧即「服务器离线期推进(A)+ 本地离线编辑(B)」的单副本场景),且**各 store 的 `doc_update` 只含自己的编辑**(远端 update 经 `_applyRemote` 合并、绝不 `appendOutbox`)。-d windows 绿。
- **测试教训**(纸面推不出,建两副本测才现形):①三个副本必须**共享同一份 base 字节**——每副本各自 `fromBlocksJson` 会给相同文本铸不同 yrs item id,diff 引用对方没有的 item → 永久 pending 不合并;②两「设备」必须 **client_id 相异**——同 id 下 A、B 的并发编辑撞同一 `(client_id, clock)`,yrs 当重复跳过。生产天然满足(单一 server base + 各设备独立 client_id)。
- **顺带修的真产品缺口**:`_send` 原来 `sink.add` 无 try/catch —— 离线(socket refused/unreachable)时编辑→`_enqueue` 因 `_ready`(本地 seed 置的、非真连接)为真而尝试推送→`sink.add` 抛未捕获异常崩会话。加 try/catch 容错(帧丢弃安全,durable outbox 重连重推)。

**P2d — 溶解 op 路由 + 放开离线编辑门**
- `_applyEditorOperations` 统一走一个 local-first backend 的 `applyOps`(cloud 分支即 append-log,localOffline 分支即无 WS 配置);删 REST 兜底热路径依赖。
- 放开离线编辑角色门:P1c 离线 nav 强制 `role='viewer'`(`_applyOfflineCloudNav`),P2 对**已缓存且本人有权**的 doc 允许离线编辑(编辑落 outbox,`_recoverOnlineNav` 联网后推)。
- 验证:widget/集成——离线态编辑云文档→ `outboxAfter` 增长→模拟联网→推送清空。
- 丢失点:门开得太宽会让「服务端已撤权」的 doc 产生永不被接受的离线编辑;沿用 P1c 的 `ApiException rethrow` 边界,只对真连接失败放行。

**P2e — 压实**
- ack 后 `trim_updates_through(pushed_clock)`(或有界 squash),log 有界。
- 验证:长会话后 `doc_updates(docId).len` 随 ack 回落;`load_doc` 内容不变。
- 丢失点:trim 越过 `pushed_clock` = 分叉;单测钉死「只删 ≤ pushed_clock」。

---

## 6. kIsWeb gating 与 P3/P4 延后项

**kIsWeb 门**:所有 append-log/outbox/base-write-through/离线编辑均桌面/移动限定(有 SQLite `MicaStore`)。web:`persistence == null` → `CloudSyncSession` 走**现状**(内存副本 + prefs `cloudUnacked` + 冷 `sync.bootstrap`),在线路径逐字不变。`local_offline_web.dart` 全 no-op。接缝:persistence 是否存在,天然二分,无需散落 `if(kIsWeb)`。

**明确延后**:
- **P3**:溶解 `ServerMode`(online/localOffline 硬开关)为「工作区:本地 / 已连云」;统一 view/workspace schema、`(origin,id)` 复合主键(P1b-2′ 已留提醒);离线**切工作区**兜底(现仅启动+tap 已接);op 路由最终单一化随此批收尾。
- **P4**:web IndexedDB nbstore(`y-indexeddb`,让 web 也 local-first)—**唯一明确暂缓**;`props` 字段级 CRDT(`MapRef`);state-vector 快速对账;纯 append-log 落盘(去掉整档快照 write-through,改周期 squash,省 I/O)。

---

## 7. 开放决策(P2 早期需拍板)

1. **压实用有界 trim 还是有界 squash?** —— 推荐 **`trim_updates_through(pushed_clock)`**(§4):只删已 ack 条目,不重编码整档,不碰 outbox,语义最清。squash 留给 P4 的日志/base 合并优化。
2. **base 快照 write-through 保留还是纯 append-log?** —— 推荐 **保留快照(读)+ append-log(outbox)双写(Model B)**:base=合并态、log=纯本地。改动最小、复用 P1 的 `_saveLocalSoon`,`pushed_clock` 语义干净。纯 append-log 是 P4 优化。
3. **outbox 推送:逐条(id=clock)还是合并成一 diff 再推?** —— 推荐 **逐条 push、`id=clock`**:ack↔clock 精确映射,契合现有 `_Pending.id` 心智,服务器照样折叠。AFFiNE 的 `mergeUpdates` 合并推送留 P4(大 outbox 才有收益)。
4. **web 何时也 local-first?** —— 推荐 **P2 完全不碰 web,留 P4**(IndexedDB nbstore)。P2 只保证桌面/移动;web 维持在线,靠 persistence==null 分支零风险隔离。

---

**落点文件速查**:`crates/mica-core/src/store.rs`(+`trim_updates_through`)、`clients/mica_flutter/rust/src/api/store.rs`+`lib/src/rust/api/store.dart`(FFI 重生成)、`lib/cloud/cloud_doc_store.dart`(outbox 接口)、`lib/local/local_offline_io.dart`(`StoreCloudDocStore` 实现)、`lib/cloud/cloud_sync_io.dart`(`CloudSyncSession` persistence 分支)、`lib/main.dart`(`_setupCloudYrs` 迁移 + `_applyEditorOperations` 收敛 + `_applyOfflineCloudNav` 编辑门)。服务端 `crates/app-core/src/sync.rs`/`ws.rs`:**不动**。
