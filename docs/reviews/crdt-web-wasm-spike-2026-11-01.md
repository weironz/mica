# Web 端 CRDT 换成 mica-core wasm：可行性验证与迁移（2026-11-01）

> **已迁移（同日，用户决定）**：`crates/doc-wasm` + `scripts/build-doc-wasm.sh`（gzip 251,111 B，硬预算 262,144 B），
> Web 适配器 `cloud/sync_replica_web.dart` 与桌面同形，op 回放统一为 `cloud/block_op_mirror.dart`；yjs 及其 Dart 镜像全部删除。
> 验证：浏览器套件 8 个全过（新增 `web_doc_replica_test` 6 项）；本地真实路径上登录→打字→撤销/重做→服务端导出一致，
> 两个浏览器上下文实时互见编辑，`micaWebSyncTest` 两会话经真实服务端收敛。
>
> **性能（迁移后复测，同一份 2000 块文档、同一浏览器，`test/web_replica_bench_test.dart`；旧数据取迁移前提交 ff1c3c14 跑同一基准）**：
> 打开 115.9 → 29.4 ms，整篇读出 24.4 → 12.4 ms，每次按键（回放 + 生成增量）0.20 → 0.20 ms，
> 收到一次远端更新（合并 + 整篇读出）21.2 → 11.4 ms，撤销一批 23.3 → 10.5 ms。启动时两个引擎并行到位（本机均约 30 ms），
> 首屏不被拖慢；慢网下多出的约 251 KB 与 Markdown 引擎争带宽。待观察：远端更新后整篇读出随文档线性增长（2000 块约 10 ms），
> wasm 内存只增不减。下文为迁移前的验证记录。

> 只做验证，不做迁移。背景见 [单一实现专项 01 §9](../single-source/01-background.md#9-仓库里其他双份与后续专项不在本专项范围)：
> 服务端与桌面用 yrs（`mica-core` 的 `MicaDoc`），Web 用 yjs（`web/yjs_bundle.js`）加约 1,000 行 Dart 镜像
> （`lib/web/mica_ydoc.dart` 532、`marks_diff.dart` 132、`text_diff.dart` 84、`yjs_interop.dart` 64）。
> 收成一份的做法是 Web 也跑 `mica-core` 的 wasm，经与桌面 `MicaDocument` 同一套接口调用。

## 结论

**技术上可行，代价是 Web 首包多约 194 KB gzip。是否迁移由用户决定**（与 wasm opt-level 同类的体积取舍）。

| 项 | 结果 |
| --- | --- |
| 能否编到 `wasm32-unknown-unknown` | 能。唯一阻碍在 `mica-core` 自己：撤销历史用 `undo::Options { ..Default::default() }`，而 yrs 的 `Options: Default` 只在非 wasm 平台实现（默认时钟是 `SystemTime`）。已修（见下），原生行为不变 |
| 随机数 | yrs 经 `fastrand` 用 `getrandom 0.3`：绑定 crate 需开 `getrandom/wasm_js` 并以 `--cfg getrandom_backend="wasm_js"` 构建 |
| 功能（node 实测） | 从块建文档、`textInsert`、批次撤销/重做（三次插入一次撤回）、`stateVector`→`encodeDiff`→`applyUpdate` 同步、两副本并发编辑收敛，全部正确 |
| 性能 | 2,000 次单字插入 5.5–8.8 ms（每次一个事务，含 wasm 边界） |
| 体积（LTO、O3、`panic=abort`，wasm-bindgen 去 name/producers 段，gzip -9） | CRDT 核心单独 **244 KB**；与 Markdown 引擎合成一个模块 **470 KB**。现状 = Markdown wasm 249.6 KB + yjs 26.6 KB ≈ 276 KB → **净增约 194 KB**。两者几乎不共享代码（yrs 本身是大头） |
| 存量数据 | 不需要迁移：IndexedDB 里存的是 yjs 编码的 update，yjs↔yrs 线格式兼容由 `crates/mica-core/tests/web_interop.rs` 每次测试双向校验 |

## 已合入的改动

`crates/mica-core/src/doc/history.rs`：`UndoManager` 的 `Options` 改为显式给出全部字段，时钟用常量 `|| 1`。
`capture_timeout_millis` 本来就是 `u64::MAX`，经过的时间从不参与判定；常量**不能是 0**——yrs 把
`last_change == 0` 当作「刚停止捕获、不要合并」，零时钟会把每个批次拆开（第一次试 0 时 6 个历史测试失败，正是这个原因）。
改后 mica-core 217 项、桌面 FFI 历史测试全过。

## 如果决定迁移，工作量与风险

1. 新 crate（形同 `crates/md-wasm`）：把桌面 `MicaDocument` 的约 30 个操作（块/文本/标记编辑、历史、状态与差量）用
   wasm-bindgen 导出，JSON 进出，与 FFI 同形。
2. Dart：`MicaYDoc` 换成调这个 wasm 的薄封装，删掉 `marks_diff` / `text_diff` / `yjs_interop` 与 `yjs_bundle.js`；
   Web 与桌面的文档操作走同一份 Rust。`yjs` 从批准依赖里移除。
3. 合并或分开加载：合成一个模块省一次请求，但 Markdown 引擎首屏就要，CRDT 只在打开文档时要；分开加载可以把 244 KB 推迟。
4. 风险点：AppFlowy 曾把整个后端 wasm 化后撤回（我们只搬文档核心，范围窄）；wasm 内存只增不减，长会话要观察；
   浏览器套件要覆盖离线编辑、多标签页、重连合并。

验证用的绑定 crate 只放在会话草稿目录、未入库；复现步骤：以 `mica-core` 为路径依赖建 cdylib，导出上表操作，
按上述 getrandom 设置 `cargo build --release --target wasm32-unknown-unknown`，再用 `wasm-bindgen 0.2.129` 生成绑定。
