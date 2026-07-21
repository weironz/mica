# 全项目代码审查 — 2026-07-20

只读审查，**未改动任何代码**。方法：确定性工具（clippy / analyze / git churn / 密钥扫描）+ 四个按「不变量」而非按目录切片的只读子代理。

## 证据等级说明（重要）

本文每条结论标注了来源，请按此决定信任度：

- **【实测】** — 我亲自跑了命令或读了两侧源码确认。可直接采信。
- **【子代理】** — 子代理报告，我**未**逐条复核。方向可信（都带了 file:line），但**动手前请先自行验证该条**。
- **【未确认】** — 报告方自己标注了不确定，或需要运行时验证。

本报告**没有**做的事：未跑依赖漏洞扫描（`cargo-audit`/`deny`/`machete` 均未安装）、未做任何性能测量、未验证 12 处 Markdown 漂移中的 11 处。这些是本报告自身的空白，**不是「查过没问题」**。

> **更新（2026-07-21）**：本文 07-20 以只读方式产出。此后进入修复阶段，已落地的改动见文末「修复进度」。其中排查一起线上崩溃（用户粘贴内容后页面空白 + `PoisonError`）时，定位到一条**此前审查未覆盖**的严重问题，补记为下方 **P0-0**——它是本次会话发现的最严重一条，且与 P2-4「依赖零漏洞扫描」直接相关：正是没有依赖审计才会漏掉的东西。

---

## 摘要：三个结构性结论

**① bug 不在你以为的地方。** 变更热点显示 bug 压倒性聚集在 Dart 客户端的四个巨型文件，而非 Rust 数据面。刚出事故的是 Rust，但历史证据指向 Dart。

**② 好几条红线是「写了、也实现了、但被从侧门绕过」。** 不是没做，是机制留了洞，新需求从洞里流回了老形状。这比「没做」更难发现——主路径看起来是对的。

**③ 最危险的模式在重复。** 「空值被当成有效值写入」这个刚造成生产事故的根因，在**块级**这一层原样存在且无任何防线。

---

## P0 — 会造成数据损坏或服务不可用

### P0-0 yrs 对畸形输入 panic / UB，一条能毒化锁、一条远程可达且不可拦截 【实测】

**来历**：用户报「粘贴内容后页面空白 + `PanicException(PoisonError)`」。debug 构建抓到第一现场：

```
panicked at yrs-0.27.0/src/block_store.rs:60:46:
index out of bounds: the len is 732 but the index is 715827364
```

Dart 栈：`crateApiStoreMicaStoreLoadDoc` → `StoreCloudDocStore.load` → `CloudSyncSession._seedFromLocalOnce` → `connect` → `_setupCloudYrs`。

**根因链**（两层，务必分清）：

1. **能毒化锁的那条（已修）**：`load_doc`（`crates/mica-core/src/store.rs`）从本地 SQLite 读出**已损坏的 yrs 状态**，yrs 整合区块时索引越界 panic。注意 `decode_v1` **已经成功**，炸在之后的整合阶段——所以那几行的 `?` 从来接不住。FFI 持 `Mutex<LocalStore>` 跨这次调用 → 锁毒化 → 之后每次 `list_views` 都失败 → 页面树空白且重启前不恢复。云同步每次启动都走 `load_doc`，所以稳定复现。Dart 侧 [`cloud_sync_io.dart:276`](../clients/mica_flutter/lib/cloud/cloud_sync_io.dart) 其实**早已写好降级逻辑**（注释：损坏副本 → 冷启动从服务端 bootstrap），但 Rust 直接 panic，那段代码从没机会跑——**一个被预见的可恢复情况，被下层变成了进程级故障**。

2. **不可拦截、且远程可达的那条（无法自修）**：给 yrs 喂任意变异字节，会走到

   ```
   core/src/str/validations.rs: unsafe precondition(s) violated:
     hint::unreachable_unchecked must never be reached
   thread caused non-unwinding panic. aborting.   (exit 0xc0000409)
   ```

   yrs 把**未校验的字节当合法 UTF-8 读**——这是上游 soundness bug，**升级到 0.27.3 仍在**（2026-07-21 实测）。它**不可拦截**：`catch_unwind` 接不住 non-unwinding abort，release 构建里该检查被编掉，同样输入就是纯 UB。
   **影响面不止本地缓存**：`crates/app-core/src/sync.rs:171` 的 `apply_update` 会应用**客户端推来的 update**——即**远程可达**。**但核实后的真实暴露面比初判小得多**（我最初说「打挂 API 进程、所有连接掉线」是错的）：① 绝大多数畸形输入走 yrs 的 `Err` 分支 → 干净 400；② 触发 unwinding panic 的输入 → 无 `panic=abort`，服务端 unwind，axum handler 跑在每连接一个的 tokio task 里，**tokio 兜住 task panic → 只崩该连接，进程存活**；③ 只有 **UTF-8 UB 类**（release 检查被编掉）是真危险，可能段错误崩进程或静默读错。三类**全都需要已登录 + 有编辑权**（非匿名）。所以 panic 类现已用服务端 `contain_yrs_panic` 收成 400（见修复进度 `0723857`），**唯一剩下的 residual 是 UB 类**——`catch_unwind` 结构上接不住，只能上游修。**该 UB 除崩溃外是否可被利用，未做分析。**

**已做的防线**（见文末修复进度）：
- `contain_yrs_panic` 把 `load_doc` 的三处 yrs 调用（base + 两条 update 日志）包进 `catch_unwind` → 收敛成 `StoreError::CorruptDoc`，本地库是缓存、云端有正本，损坏降级为可重新播种。
- FFI 45 处 `.lock().unwrap()` 改成毒化恢复式取锁——即使别处再 panic，文档也不再永久不可读。
- **本地校验和**（✅ 已落地 `9b117c0`）：5 张 blob 表各加 sidecar CRC32 列，读时先验再喂 yrs——这是**唯一能在本地挡住上游 UB 的手段**（在字节到达 yrs 之前拦下）。只覆盖本地存储；服务端远程路径不适用（攻击者同时控制字节与校验和）。同时补上了此前**完全无防护**的两条 recovery 解码路径（`restore_local_version`、`rollback_doc`）。**只保护本 feature 上线后写入的数据**——老 NULL 行仍保留今天的暴露（诚实边界，不做 read-time backfill）。
- **服务端 panic 兜底**（✅ 已落地 `0723857`）：`sync.rs` 5 处 yrs 调用包进 `contain_yrs_panic` → unwinding-panic 类从「掉连接」变成干净 400。**故意没做进程隔离**：对这个认证后、panic 类已被 tokio 隔离的窄暴露面严重不成比例，还会拖垮同步热路径。**不覆盖 UB 类**（接不住）。

**仍未解决**：① 本地快照**当初为何损坏**未查清——修复只让它可生还，未让它不发生；② **服务端远程 UB 类**的根治在上游，本地无解——**不建议进程隔离**（对认证后、panic 类已被 tokio 隔离的窄暴露面不成比例）。只在威胁模型变宽（接受非认证/低信任来源的 update）时才重新考虑隔离。

**上游已报（2026-07-21）**：搜索后发现不该新开 issue（会与既有重复）——yrs 的 [#415 "use errors instead of panic"](https://github.com/y-crdt/y-crdt/issues/415)（panic 类，维护者两年前就求一个可复现场景、含 AppFlowy 在内的报告者都给不出）和 [#373 "Memory safety issues (segfaults)"](https://github.com/y-crdt/y-crdt/issues/373)（UB 类）已覆盖。故把**最小复现**（52 字节合法 v1 update，翻 byte 9 → panic `block.rs:92`；翻 byte 50 → 非展开 abort / release UB，`core::str` 无效 UTF-8）作为**评论**贴到 #415：[issuecomment-5025461308](https://github.com/y-crdt/y-crdt/issues/415#issuecomment-5025461308)。复现只用 yrs 公开 API（`Update::decode_v1` + `apply_update`），发帖前已实测编译+触发。

读 yrs 源码后追发**跟进评论**：[issuecomment-5025540330](https://github.com/y-crdt/y-crdt/issues/415#issuecomment-5025540330)。① 自我纠正——`block.rs:92` 其实是 `debug_assert!`（release 编译掉），真正 release 相关的是解码路径的边界检查 panic + case (2) 的 UB；② 给出 UB 的**具体修复**：`src/encoding/read.rs:137` 与 `src/updates/decoder.rs:486` 两处 `from_utf8_unchecked` → 校验版 `from_utf8` + `Err`（两处都已返回 `Result`，代价可忽略）；`src/lib.rs:777` 的第三处是对 HEX 表、sound，不动。

### P0-1 空 block id 可写入 CRDT，是 7 月 19 日事故的块级同款 【实测】

`crates/app-core/src/documents.rs:200`

```rust
fn validate_block(block: &Block) -> DocumentOperationResult<()> {
  if block.kind.trim().is_empty() {
    return Err(DocumentOperationError::EmptyBlockType);
  }
  Ok(())          // block.id 从不校验
}
```

`insert_block`（同文件 :85）随后：`block_index(snapshot, "")` 查不到 → 不触发 `BlockAlreadyExists` → 空 id 的块连同 `""` 这个 child 引用一起落进 `document_yrs_base` 并广播给所有副本。

之后每次读都在 children 里遇到 `""`，报的就是事故当天那句 **`block not found:`（id 为空）**。CRDT 里删不掉，**和事故 A 一样自我延续**。

`set_blocks` 现在守住了 `meta.root`（`crates/mica-core/src/doc.rs:531`），但守不住 children 里的空 id。**同一个根因在下一层没有防线。**

### P0-2 `insert_block` 无环检查 → `delete_block` 无限循环并锁死文档 【实测】

同一个文件里，两条路径的防线不对称：

| 函数 | 环检查 |
| --- | --- |
| `move_block` `documents.rs:183` | ✅ `is_descendant` |
| `insert_block` `documents.rs:85` | ❌ 无，且原样接受调用方的 `children` |

造环：`{"type":"insert_block","block":{"id":"X","children":["root"]},"parent_id":"root"}`

触发：`documents.rs:145` 的级联删除**没有 visited 集**——

```rust
let mut delete_ids = vec![block_id.to_string()];
let mut cursor = 0;
while cursor < delete_ids.len() {
    if let Some(index) = block_index(snapshot, &current_id) {
        delete_ids.extend(...children...);   // 无去重
    }
    cursor += 1;
}
```

→ `[X, root, X, root, …]` 无限增长 → CPU 100% + 内存耗尽。而这发生在 `apply_derived_operations` 的事务内（已持 `FOR UPDATE` 锁），**该文档同时被永久锁死，且不产生任何信号**：无超时、无日志、无 500。

### P0-3 一个块两个父亲 → 读取时静默丢整棵子树 【子代理】

`insert_block` 不检查目标块是否已有父亲。配合 `crates/mica-core/src/doc.rs:126` 的 DFS `if !seen.insert(id) { continue; }` —— 第二次出现直接跳过，**那一支子树从树序里安静消失**，不报错。用户看到「我那段跑到文末了 / 不见了」，服务端毫无异常。

### P0-4 transfer/clone 读不到 payload 就用空文档顶上，然后删源 【子代理】

`crates/api-server/src/routes/documents.rs:2591`（transfer）和 `:2827`（clone）：

```rust
.unwrap_or_else(|| DocumentSnapshotPayload {
    root_block_id: "root".to_string(),
    blocks: Vec::new(),        // 「我没读到」被翻译成「它是空的」
})
```

随后 `remove_source == true` 时把源子树标删。**返回 200，计数照报，内容没了。** 与事故 A 完全同源：`unwrap_or_default` 落进持久化路径。

---

## P1 — 会产生静默的错误结果

### P1-1 Rust ↔ Dart 双表示红线**没有守住**：12 处确认漂移 【子代理，实测 1 处】

CLAUDE.md 原则 #2 要求两端同步。git 证据：`f1d418e`（"close 31 CommonMark gaps"）**只改了 Rust**，Dart 侧 0 行；`c87830b` Rust 改 520 行，Dart 只同步 24 行。

**实测确认的一条（往返丢内容）**：`escapeBlockLeader` —— Dart 侧 `clients/mica_flutter/lib/editor/marks.dart:1486` 的文档注释**逐字写着「Mirrors the Rust engine」，而它并没有**：

| Rust `crates/markdown/src/lib.rs:5002` | Dart `marks.dart:1487` |
| --- | --- |
| `compact`（先滤空格）判分隔线 → `-- -` 转义 | 无 → 原样输出 |
| `setext_like`（全 `=` 或全 `-`）→ `===`、`--` 转义 | **完全没有 `=` 处理** |

后果：正文里一行 `===`，导出不转义，**再导入把上一段吃成一级标题**。CLAUDE.md 原则 #4 明写「round-trip 是不变量」。

> 一个断言自己是镜像的注释，比没有注释更危险——它让每个后来者跳过核对。

**未复核的其余 11 处**（按代理给的严重性）：硬换行判定跨 code span、`matchingBracket` 不跳 code span、缺 `label_contains_link`（链接可嵌套链接）、链接 title 转义/实体解码不一致、code span 空格剥离条件、`mathRunSpans` 不跳 code span、`~~~` 回退步长、引用标签 case-folding、空白字符定义（Unicode vs ASCII）、空 label 链接、ATX 标题转义范围。

### P1-2 漂移检测机制**存在且设计正确**，但用例集只覆盖 3.6% 【实测】

这是本次审查最有价值的一条，也是推翻子代理结论后才得到的。

代理断言「Dart 侧没有 conformance 跑分」——**错的**。`clients/mica_flutter/test/markdown_conformance_test.dart` 存在，而且设计得很好：钉在和 Rust **同一份 gold fixture** 上，注释写着「任一侧漂移都会在这里失败」。

真正的问题是**规模**：

| | 用例数 |
| --- | --- |
| Rust `commonmark_scoreboard.rs` | **641**（官方 spec 全量） |
| Dart 共享 fixture（`crates/markdown/tests/fixtures/conformance/*.md`） | **23** |

**机制是对的，用例集只有 3.6%。** 这精确解释了 12 处漂移为什么能全部溜过去——不需要新建机制，只需要扩大 fixture 集。**本报告投入产出比最高的一项。**

### P1-3 导出、排序、搜索的静默失败 【子代理】

- **workspace markdown 导出** `documents.rs:1744`：`if let Ok(markdown) = export_markdown_with_assets(...)` **无 else** → 单页渲染失败则该页只剩标题、正文消失，HTTP 200。导出是用户当备份用的，「备份成功但内容是空的」正是事故 B 的形状。
- **`reorder_views`** `documents.rs:738`：从不检查 `rows_affected`，部分失败无条件报全量成功。**`ssh | tee` 同款**。
- **搜索** `documents.rs:197`：`if let Ok(Some(payload))` 吞掉损坏文档，**且一条日志都不留**（同仓库 `blob_gc.rs:137` 处理同一错误是记日志的）。若事故 A 那种损坏先表现为「搜不到」，这条路径会吃掉唯一的早期信号。

### P1-4 `data.indent` 不变量只在 Dart 里，且两端 clamp 不一致 【子代理】

`canNestUnder` 那条模式的**下一层，尚未修补**：

| 位置 | 行为 |
| --- | --- |
| Dart 读 `lib/editor/model.dart:61` | `.clamp(0, 8)` |
| 服务端 | **零校验**（`data` 是不透明 `Value`） |
| Rust 导出 `crates/markdown/src/lib.rs:2890` | `unwrap_or(0)`，**不 clamp** |

MCP/REST 客户端写入 `indent: 30` → 服务端接受 → 导出按 30 层渲染，编辑器按 8 层渲染。**同一份数据两端结果不同。** 且用户一旦碰这个块，Dart 写回 clamp 值，原始层级永久丢失。`data.quote`/`qbreak`/`li` 同理。

### P1-5 `await` 后跨异步间隙使用 `BuildContext` 【实测】

`clients/mica_flutter/lib/editor/editor.dart:4878` 和 `:4890`。widget 已卸载时会崩。analyzer 报了，但 CI 用 `--no-fatal-infos` 放行。

---

## P2 — 机制性缺口（杠杆最高，成本最低）

### P2-1 `cargo clippy` 在这个项目上**根本跑不起来** 【实测】

`invisible_characters` 是 deny 级，撞上 `crates/markdown/src/lib.rs:2095` 的 HTML 实体表（`"shy" => "­"`，一个**完全正确**的软连字符映射），clippy 在第一个 crate 就整体中止，**剩余 8 个 crate 从未被检查**。

这几乎肯定就是 CI 里至今没有 clippy 的原因——谁想加都会先撞上这堵墙。

放开这一条之后的结果意外地好：**27,111 行 Rust 只有 14 条提示，零 error**，其中 3 条还是误报。真正值得看的只有 `documents.rs:3104` 一个没人用的测试夹具。

> 加 clippy 到 CI 的成本 ≈ 一个 `#[allow]` + 13 条 trivial 修复。

### P2-2 167 个测试从不在 CI 执行（18.9%）【子代理，实测 1 条】

**实测确认**：`mica-core` 的 `store` feature 不在 default，导致 `crates/mica-core/src/store.rs` 的 **24 个测试从未被编译**：

```
cargo test -p mica-core                    → lib 跑 2 个测试
cargo test -p mica-core --features store   → lib 跑 26 个测试
```

这 24 个里包含本地 SQLite 的**全部迁移测试**（v3→v4 复合主键迁移、备份表校验）。

【子代理】的完整统计：

| 来源 | 从不执行 |
| --- | --- |
| `api-server` | 59 |
| `app-core` | 34 |
| `mica-core`（store feature gate）| 24 |
| `infra` | 4 |
| Flutter `integration_test/` | 46 |
| **合计** | **167** |

**最刺眼的一条**：`crates/app-core/src/sync.rs:381` 的 3 个测试 + `store.rs` 的 4 个 root 重建测试 = **7 个纯 Rust、零数据库、零环境变量的测试，只因 crate 名不在 `ci.yml` 的 `-p` 列表里而从不执行**。这 7 个里包含 root 擦除事故读路径修复的**全部覆盖**。

### P2-3 「测试真空」的活体标本 【子代理】

`crates/mica-core/tests/web_interop.rs:16`：

```rust
let Ok(b64) = std::env::var("MICA_WEB_STATE_B64") else {
    eprintln!("skipping ..."); return;
};
```

这是全仓库唯一一个**在 CI 里被执行、报告 PASS、但一条断言都没跑**的测试。它声称在测 yrs ↔ yjs 线格式兼容——那正是 CLAUDE.md 豁免 #7 依赖的核心假设。yrs 升级把 mark 编码彻底改掉，它照样绿。

更糟的是 `crates/app-core/tests/sync_pg.rs:13` 的 `pool()` 用 `.ok()` 把**连接失败**也变成跳过——开发者本地设了 `DATABASE_URL`、看到 `8 passed`，完全可能一次库都没连上。

**`docs/lessons.md:84` 逐字记录了这个陷阱，代码从未改。**

### P2-4 依赖零漏洞扫描 【实测】

491 个 Rust crate + 81 个 Dart 包，`cargo-audit`/`deny`/`machete` 一个都没装，CI 里也没有。**本次没有跑扫描**，所以这是空白而非「没问题」。

### P2-5 密钥扫描干净 【实测】

工作树和**全部历史**零真实令牌。`AKIAIOSFODNN7EXAMPLE` 是 AWS 官方文档的公开测试向量；`mica_pat_` 的命中全是前缀常量、文档占位符和测试夹具。

---

## P3 — 架构债

### P3-1 渲染注册表被从 `List + canHandle` 窄化成 `Map<kind, renderer>` 【实测】

`docs/render-architecture.md:43` 设计的是 `List<BlockRenderer>`（first `canHandle` wins）。实现是 `render.dart:474`：

```dart
static final Map<String, AtomicBlockRenderer> _renderersByKind = {
  for (final r in atomicRenderers) r.kind: r,     // 一个 kind 只能有一个 renderer
};
```

`MermaidRenderer` 已占住 `'code_block'`，真正的判定被藏进它的 `layout()`（`language != 'mermaid'` 返回 null）。

**代价很具体**：再加一个 Graphviz renderer 会被 map **静默覆盖**——无编译错、无断言、无测试。唯一出路是往 `MermaidRenderer.layout()` 里加 `language == 'graphviz'` 分支，**把红线想消灭的 if 链原样搬进 renderer 内部**。而 doc 的 Problem 段点名的正是 "Mermaid, Graphviz, footnote panel"。

修复很小：`kind` 换成 `claims(node)`，map 换成 `firstWhereOrNull`。但 `paintBackground` 故意按 kind 派发，那条要单独保留一张表。

### P3-2 命中测试完全在注册表之外 【子代理】

`render-architecture.md` 自认 hit-test dispatch "deferred"。但 TableRenderer 显然已经需要了——`render.dart:2362-2451` 有 **9 个独立的表格命中方法**，每个都自己全量扫 `_layouts` 再 `if (l.kind != 'table') continue`。

新 block 类型只要有任何指针交互，就必须在 render.dart 新开一组 `xxxAt(Offset)` + 一组 `_NodeLayout` 公开字段。**注册表在这条路径上等于不存在。**

### P3-3 选中态几何在注册表落地**之后**被加回 if 链 【子代理】

`render.dart:1501`（table）和 `:1519`（image），git 追溯到 `904e132` 和 `749a371`——**都在注册表 commit `cb6bf97` 之后**。接口有 `paintBackground`/`paint`/`paintOverlay` 三个钩子，唯独没有「选中高亮几何」，于是需求从这个洞流回了老形状。

教科书式的「新渲染能力没先抽象机制」。

### P3-4 四个巨型 State 类 【实测 + 子代理】

| 文件 | 行数 | 总改动 | **fix commit** |
| --- | --- | --- | --- |
| `main.dart` | 7,659 | 215 | **47** |
| `editor.dart` | 5,897 | 104 | **31** |
| `controller.dart` | 3,095 | 58 | **25** |
| `render.dart` | 2,889 | 75 | **19** |
| `documents.rs` | 3,510 | 33 | 7 |
| `markdown/lib.rs` | 5,268 | 27 | 4 |

**这四个 Dart 文件吃掉了全部 fix commit 的一大半。** Rust 侧尽管刚出过事故，历史上反而安静得多——`markdown/lib.rs` 5,268 行只有 4 次 fix。

【子代理】：`_WorkspaceShellState` 约 4,080 行、`_MicaEditorState` 约 4,950 行，`build()` 都在最后 60 行——即 99% 是 build 之前的方法体，网络 + SQLite + 同步 + 偏好持久化 + 全部 UI 装在同一个 State 里。

---

## 明确核实过、**没有**问题的地方

避免下次重复审查：

- **`set_blocks` 的 root 守卫**【实测】：正确，注释详尽，`root_invariant.rs` 3 个测试 + `sync_pg.rs` 回归，**且这 3 个测试真的在 CI 跑**。这是事故留下的最有价值资产。
- **页树 folder/page 不变量**【子代理】：服务端 7 处写 `parent_view_id` 的路径全过 `ensure_parent_accepts_children`，另有递归 CTE 环检查，DB 触发器兜底。**这条已修到位。**
- **`blob_gc.rs`**【子代理】：全仓库最谨慎的文件，读失败 fail-closed 整个 workspace，边界纯函数化 + 5 个测试。
- **`mcp-conformance` CI job**【子代理】：设计最好的检查——用官方 MCP inspector 驱动真实二进制，精确钉住两个曾上线的 bug。
- **emphasis 处理**【子代理】：`_flanking`/`_isMdPunct`/`_isCjk`/`_processEmphasis` 两端**逐位一致**，含 CJK 修正。inline 分支优先级顺序也完全相同。
- **Flutter 单元测试（573 个）**【子代理】：零 `skip:`、零环境门禁、零静默 early-return。这一侧是干净的。
- **`find_replace`**【子代理】：显式拒绝改动带 marks 的块，0 命中返回 `Err` 而非静默成功。**本次审查里唯一一处主动把「什么都没发生」变成错误的代码。**

---

## 建议的执行顺序

按「单位成本消除的风险」排，不按严重性排：

**第一梯队（几小时，永久生效）**

1. **P2-1 解开 clippy** — 一个 `#[allow]` + 13 条 trivial 修复，然后进 CI
2. **P2-2 把 `app-core`/`infra` 加进 `-p` 列表** — 一行 CI 改动，立刻激活 7 个纯 Rust 零依赖测试，含 root 事故读路径的全部覆盖
3. **P2-2 给 `mica-core` 加 `--features store`** — 一行，激活 24 个迁移测试
4. **P2-3 把 `pool()` 的 `.ok()` 改成 panic、`web_interop.rs` 改成缺环境变量即失败** — 让假绿变成真红

**第二梯队（1-2 天，堵住已知事故复现路径）**

5. **P0-1 / P0-2 / P0-3** — `validate_block` 补 id 非空 + `insert_block` 补环检查和 children 校验 + 级联删除加 visited 集。三条同源，一起改。
6. **P1-2 扩 conformance fixture 集** — 机制已存在，只需从 641 个 spec 用例里补进共享 fixture。**这是唯一能系统性防止双表示漂移复发的手段**，比逐条修 12 个漂移更重要。

**第三梯队（按需）**

7. P0-4 / P1-3 的静默失败逐条改成显式错误
8. P3-1 注册表改回 `claims(node)` —— **在加 Graphviz 之前必须做**
9. P3-4 拆 `main.dart` —— 收益最大但成本也最高，建议等前面做完再评估

**不建议现在做**：性能优化。没有 profile 数据，改的是风险不是性能。

---

## 本报告的局限

- 12 处 Markdown 漂移只复核了 1 处
- 服务端 13 条发现只复核了 2 条（P0-1、P0-2）
- 未跑依赖漏洞扫描
- 未做任何性能测量；P3 里所有「每帧执行」的陈述只说明代码位置，**不构成对影响大小的断言**
- 四个子代理各自独立工作，未做交叉验证

---

## 修复进度（2026-07-21）

已落地并推送（每条均配回归测试，且实测「退回修复即失败」）：

| commit | 覆盖 | 说明 |
| --- | --- | --- |
| `2e84422` | **P2-1 / P2-2 / P2-3** | 解开 clippy（源头一个 `#[allow]` + 13 条 trivial）并进 CI（`-D warnings`）；CI 加 `services: postgres` + `-p app-core -p infra` + `--features store`，**激活 121 个此前从不执行的测试**；`sync_pg.rs` 的假绿改真红，`web_interop.rs` 改 `#[ignore]`。CI 实跑全绿。 |
| `f48f704` | **P0-0 兜底** | FFI 45 处 `.lock().unwrap()` → 毒化恢复式取锁，一次 panic 不再让文档永久空白。 |
| `4170414` | **P0-0 收窄** | HTML 导出不再持文档锁跑整个渲染引擎（merman/数学，318 个 panic 点），移出锁作用域。 |
| `a02feb3` | 覆盖补强 | 钉住 LLM 输出形态的粘贴内容（嵌套围栏 / CJK / marks 边界）——排查 panic 的副产物，未复现但补上了此前缺失的覆盖。 |
| `278e2b9` | **P0-0 根因** | `contain_yrs_panic` 把 `load_doc` 的 yrs 调用包进 `catch_unwind` → `CorruptDoc`；yrs 0.27.0→0.27.3；记录上游 UB（`yrs_corrupt_input_is_unsound`，`#[ignore]`）。 |
| `9b117c0` | **P0-0 第 4 层** | 本地 blob CRC32（5 张表 sidecar 列，字节进 yrs 前验），补上 `restore_local_version`/`rollback_doc` 两条无防护路径。11 个新测试，均实测「移除 verify 即 FAILED」。设计经 workflow 对抗性评审——抓到一个几乎会发的 P0（UPSERT 的 SET 子句漏 crc → 每次编辑误判 CorruptDoc）。 |
| `0723857` | **P0-0 服务端** | `sync.rs` 5 处 yrs 调用包 `contain_yrs_panic` → 客户端 update 的 panic 类收成干净 400（原 `.map_err` 映射不变）。核实后**否掉进程隔离**（暴露面窄且认证后，tokio 已隔离 panic 类）。测试实测「移除 guard 即 FAILED」。UB 类留待上游。 |

### 无人值守批量修复（2026-07-21 夜，用户授权自主执行）

每条均配回归测试、实测「移除 fix 即 FAILED」（除注明外），本地起 dev postgres 实跑 pg-gated 测试。

| commit | 覆盖 | 说明 |
| --- | --- | --- |
| `96ad977` | **P0-1/2/3** | 块级不变量：`validate_block` 拒空 id；`insert_block` 拒非空 children（两个合法客户端都发 `[]`，一刀关掉成环+双父）；`delete_block` 加 visited 集防死循环。纯内存函数，3 个单测全红绿验证。 |
| `c3627aa` | **P1-1** | `escapeBlockLeader` 忠实镜像 Rust——补 setext（`===`/`--`）+ 带空格分隔线，堵住"正文 `===` 再导入把上段吃成标题"的 round-trip 丢内容。 |
| `4a46077` | **P1-5** | 图片复制/下载不在已卸载 widget 上用 `context`；消除 2 条 `use_build_context_synchronously` warning。 |
| `3141de8` | **P1-3 / P0-4** | 服务端静默失败改可见：导出丢正文→`?` 传播；`reorder_views` 查 `rows_affected`→并发删除返回 409；搜索吞错→`tracing::warn`。P0-4 核实为基本非 bug（`current_payload` 的 `None` 即空文档、读错误在破坏性事务前已传播），加 warn 让残留边界可诊断。 |
| `24ff9bd` | **P1-4** | 导出 list `indent` `.min(8)` 镜像 Dart 编辑器的 `clamp(0,8)`——非编辑器客户端写 `indent>8` 时两端渲染不再分叉。 |
| `6491ff2` | **P1-2** | 扩 conformance fixture（3 例，锁定 parity）+ 修引用标签 `ß→ss` case-fold 漂移（confirmed #9，一行镜像 Rust）。**确认为真、但故意保守未改**（Dart 核心 parser，无人值守有回归风险，留待带全神修）：**#3 nested-link**（`label_contains_link` 缺失）、**#4 link-title 转义**（Dart 吞任意反斜杠、Rust 只吞标点）。 |

| `e527445` | **修我自己的 flaky 测试** | CI 在 `24ff9bd` 报红，failing 的是我在 `278e2b9` 写的 `a_corrupt_snapshot_is_an_error_not_a_panic`——`from_blocks` mint **随机 client_id**，编码字节逐次不同，翻 byte 9 是"整合期 panic"还是"decode_v1 拒"成了掷骰子。**时绿时红比没有测试更糟。** 实测钉住 client_id 后**再无任何偏移能触发可捕获的整合期 panic**（172 字节逐偏移单进程扫过），故改成确定性两件套：直接喂 panic 闭包测守卫契约（实测移除 `catch_unwind` 即 FAILED）+ 端到端只断言"返回 Err 而非 unwind"。连跑 15 次 15/15。教训记入 `docs/lessons.md`。 |

**这轮全量验证**：Rust 全 workspace 36 个测试二进制 + `clippy -D warnings` 绿；Flutter 610 测试全过、0 个 error/warning 级 analyze。未发版（等用户指令）。

> 附带验证：过程中本地 dev postgres 容器停掉，`sync_pg` 8 个测试**全部报红**并打出 `DATABASE_URL is set but the connection failed: PoolTimedOut`——这正是 P2-3「假绿改真红」在野外自证：换作从前它们会静默跳过并报绿。

### 带全神修：最后两处漂移 + 一个指数级 DoS（2026-07-21，用户在场）

| commit | 覆盖 | 说明 |
| --- | --- | --- |
| `0f918ae` | **P1-2 剩余两处漂移** | ① link title 反斜杠：Dart 原本吃掉**任意**字符前的反斜杠，`[a](/u "C:\name")` → `C:name`（静默丢内容）；改成"扫边界跳 `\X`、末尾整体 `_unescapeMd`"，对齐 Rust。② 嵌套链接：补 CommonMark §6.3 的 `label_contains_link` 守卫。两个 fixture 归队，conformance 27 → **29**。 |
| `d70bc66` | **第三处漂移（审查发现）** | 空标签 `[](/url)`：Dart 的 `close > i + 1` 要求标签非空，Rust 只要括号匹配 → `x [](/url) y` 两端**文本和 mark 都不同**。改 `close > i`，引用形式补 `label.isNotEmpty`。新增 `link_parity_test.dart`（期望值全部取自 Rust 实际输出），顺带补上审查指出的"引用分支零覆盖"。 |
| `32fcd70` | **指数级 DoS（A）** | `label_contains_link` 的 memo **跨整棵递归共享**。此前 2·T(n-1)：每层为检查解析一遍、被拒后主扫描再解析一遍。实测(release) `[[[a](/u)](/u)]` depth 20 **834ms → 0ms**（depth 32 仍 0ms）；`[[[[a]]]]` depth 24 ~6s → 0ms。`import_markdown` 吃用户提交内容，~170 字节曾能占住一个核数分钟。输出不变（差分模糊 ~280 万输入零分歧 + 641 scoreboard 未变）。 |

**这一轮最该记住的**：第一版只做了惰性求值，我在注释和测试标题里宣称 DoS 已解决——**但那两个 perf 测试只钉了被治好的良性形状**，真正的向量绿着溜过去。对抗性审查抓出来，我复测确认后才补上共享 memo，并让两端守卫**同时钉住两种形状**。这是本轮第二次「测试声称超出证据」，两次都是我自己写的。

**仍未动**：
- **块层差异（新发现，未查）**：`[![a](/img)](/u)` 在 **inline 层两端完全一致**，但 Dart 的 `markdownToBlocks` 会把"只含一张图的段落"提升成 image 块并**丢掉外层链接**，Rust 保留段落 + image/link 两个 mark。与本轮改动无关，属块层既有差异。
- P3 全部（架构债，判断重、风险高，计划本就建议缓做）。

### 清空漂移清单：最后 3 处 + 序列化器丢链接（2026-07-21，用户在场）

先纠正上一轮两个报数错误：**剩余是 5 处不是 7 处**（我多报了）；**和 Docker 无关**——漂移验证是纯本地两步（Rust `GEN_GOLD` 生成 gold + Dart `flutter test` 比对），Postgres 只有 `sync_pg`/api-server 那批测试要用。

实测 5 处：**2 处证伪**（`mathRunSpans` 不跳 code span —— `` `$a` $b$ `` 两端逐字节相同，Dart 的 `_findMathCloser` 早已调用 `_codeSpanClose`；空白定义 Unicode vs ASCII —— 全角空格后的 `www.example.com` 两端都识别为 autolink），**3 处确认为真并已修**：

| commit | 覆盖 | 说明 |
| --- | --- | --- |
| （本轮） | **漂移 ①硬换行判定跨 code span** | 根因是**架构层级错位**：Dart 在**块层**用 `lines[i].endsWith('  ')`(`_OpenItem.endsHard`) 预判硬换行，而 code span 可以跨行——块层逐行看，根本无法判定。`` a `code␠␠⏎span` `` 被注入一个字面 `\` 进代码内容。照 Rust 的做法把判定**整体下移到行内层**（新增 `_canonicalizeBreaks`，跳过 code span/autolink/raw HTML），块层改为保留行尾空格、一律 `\n` 拼接（新增 `body`/`withTrailing`，对齐 Rust `body`），`endsHard` 连同 10 处引用一并删除。 |
| （本轮） | **漂移 ②`matchingBracket` 不跳 span** | `` [`a]b`](/x) ``：Rust 得到 code+link，Dart 只得到 code，**链接整个丢失**。新增 `_inlineSpanEnd`（镜像 Rust `inline_span_end`，含 autolink 与 raw HTML 分支），`matchingBracket` 扫描时整体跳过。 |
| （本轮） | **漂移 ③code span 空格剥离条件** | 判空用的是「非全 **空格**」(§6.1) 而不是「非全 **空白**」。Dart 用 `inner.trim().isNotEmpty`，`trim()` 连 tab 一起吃 → `` ` ⇥ ` `` 答"空"而跳过剥离。改为 `inner.codeUnits.any((u) => u != 0x20)`，对齐 Rust `!inner.bytes().all(|b| b == b' ')`。 |

新增 fixture `38-hardbreak-spans` / `39-bracket-spans` / `40-code-span-strip`，conformance 29 → **32**，三个 case 在 Dart 侧全部对上 Rust 生成的 gold。**至此 12 处原始清单全部有定论：10 处已修、2 处证伪。**

**补 fixture 当场炸出一个更严重的 bug（两端同病，与三处漂移无关）**：`render_span` 选下一个 mark 的键是 `(start, 最宽)`，而 `code` 分支是**终止性**的（写完字面量就 `pos = e; continue;`，从不渲染 `inner`）。当 code mark 与 link mark **范围完全重合**时，谁在数组里靠前谁赢——`` [`a`](/x) `` 导出成 `` `a` ``，**URL 静默消失**。`` [`useState`](https://react.dev/…) `` 这种形状在技术文档里遍地都是，任何一次导出/复制为 Markdown 都在毁真实内容。修法：终止性 kind（code/math/html/footnote）**输掉范围完全重合的平局**，让可嵌套的 mark 包在外面；同时给 autolink / 裸 `www.` 两个简写分支加 `inner.is_empty()` 守卫（它们写 `plain`、丢弃内嵌 mark，只有无内容可丢时才该用）。两端各配 5 条断言完全相同的回归测试（`linked_code_span.rs` / `linked_code_span_test.dart`），含两条**反向**守卫防止修过头。

**这一轮最该记住的**：① 漂移不都是"照抄漏了一行"——①是**架构层级放错**，块层被迫在信息不足的位置做判定，修法是搬层级而不是打补丁。② **fixture 机制本身比它当下钉住的用例更值钱**：这次三个新 fixture 里，真正的重头戏是它顺带炸出的序列化器丢链接——那个 bug 不在任何漂移清单上，是 round-trip 断言自己抓的。③ Dart 侧那个 bug 我起初是从**代码结构推断**的（探针编译失败没跑成），提交前把平局规则临时改回去实测它确实会红，才确认测试不是空跑。
