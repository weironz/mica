# 全项目代码审查 — 2026-10-21(第三轮)

审查对象:`main` @ `95ae3f3c`(v0.13.63 + 触控板手势修复)。这是近期一系列重构之后的第一次全项目审查，重构包括：编辑器拆成 part、外壳控制器化、markdown 解析器分解、搜索索引重写、启动路径改造。

**方法:**
- 8 个只读评审代理按域切片审查：
  - api-server 鉴权/工作区/WS/AI
  - documents 路由 + app-core
  - mica-core + infra + FFI
  - markdown + interchange
  - cli + mcp-server
  - Flutter 编辑器
  - Flutter 外壳层
  - 跨模块架构
- 主会话逐行复核最高危的结论。
- 跑了下表的工具。

调整计划见 [`code-optimization/OPTIMIZATION_PLAN_ROUND4.md`](code-optimization/OPTIMIZATION_PLAN_ROUND4.md)。

## 证据等级

- **【复核】**:主会话逐行读过源码，确认触发路径成立。
- **【读码】**:评审代理给出了 file:line 和推演路径，方向可信，动手前再确认一次。
- **【需实测】**:性能结论是按代码推演的，修之前要先用基准或真机量一下。

## 工具结果(补上 10-19 报告「未跑测试 / 未扫依赖」的空白)

| 工具 | 结果 |
| --- | --- |
| `cargo clippy --workspace --all-targets` | **0 警告** |
| `cargo test --workspace`(`DATABASE_URL` 指向本地 Postgres,DB 测试**实跑**)+ `cargo test -p mica-core --features store` | **942 通过 / 0 失败**(18 个 ignored) |
| `flutter analyze` | 0 error / 0 warning / 147 info。值得一提的 info 有两类:<br>· 8 个 `_web` 变体还在用已弃用的 `dart:html`<br>· `main_shell.dart:3883` 有 1 处 `use_build_context_synchronously` |
| `flutter test` | **1826 通过** |
| `cargo audit`(workspace) | **13 个漏洞公告**,大部分可以靠兼容范围内的 `cargo update` 消掉:<br>· h2 0.4.14<br>· quinn-proto 0.11.14 ×5<br>· rustls 0.23.40 ×4<br>· rustls-webpki 0.103.13<br>· webpki-roots 1.0.7<br>`rmcp 0.9.1` 那条是 Streamable HTTP **服务端**传输的 DNS rebinding,本项目只用 stdio(`mcp-server/src/lib.rs:15`),**不受影响**。<br>另有 unmaintained/unsound 警告:anyhow `downcast_mut`、event-listener、cgmath、paste 等 |
| `cargo audit`(FFI crate `clients/mica_flutter/rust`) | 0 漏洞,11 条警告(同类 unmaintained/unsound) |
| `cargo machete` | 3 个未使用依赖:<br>· app-core `thiserror`<br>· mcp-server `tracing`<br>· infra `serde_json` |

**仍是空白，不代表「查过没问题」:**
- 没在真机上验证(Windows 桌面、触控板、IME)。
- 没做 fuzz 和压测。
- 下文标【需实测】的性能结论都没跑基准。

---

## 旧条目状态(10-18 / 10-19 两轮)

**已修:**
- 10-18:A3 `set_blocks` 差量化、A4/A5 恢复与搜索 panic、B1–B3、B8、B12、C1/C2/C5/C9/C10、A9、D2、C7 hub 清理。
- 10-19:#3 本地 snippet panic、外壳 `removeServer` 的 `!mounted`、`main_workspace_view.dart` 的 NUL 字节。

**仍未修**(本轮重新核对，证据见各域报告):

| 原编号 | 问题 | 当前证据 |
| --- | --- | --- |
| 10-19 #2 | `purge_view` 能直接删活页面 | `trash.rs:108`。roadmap 记为「要不要收紧是独立决定」 |
| 10-19 #6a/#6b | li/quote 深度不 clamp;图片(以及链接)label 嵌套无深度上限 | `block.rs:1042`、`html.rs:149`、`inline.rs:976`。见本轮 P1-6/P1-7 |
| 10-19 #7(ZIP64) | ZIP64 extra 越界。fe0c0e0 只修了 writer | `zip/reader.rs:108-116` |
| 10-19 #8 | 协作模式下 Undo 会回滚/删除协作者的编辑 | `controller.dart:3618-3700` |
| 10-19 P2-3 | XFF 无条件采信 | `rate_limit.rs:41-52`。compose 不发布 api 端口，算是缓解 |
| 10-19 P2-5 | `GET /api/ai/settings` 向任意登录用户泄露 base_url/model/key_hint | `ai.rs:123-133` |
| 10-19 P2-6 | `mica-cli update` 无校验和;`contains` 比版本;还会**静默降级** | `update.rs:99/132/200` |
| 10-19 P2-7 | MCP / Flutter HTTP 层无超时 | `mcp-server/src/lib.rs:42`、`api/client.dart:118-133`。**CLI 不同**:reqwest blocking 默认有 30s 总超时，问题反而是太短，见 P2 |
| 10-19 P2-8 | 单条恢复两句 SQL 不在一个事务里;会复活早先单独删掉的子孙 | `trash.rs:38-76` |
| 10-19 P2-9 | `move_view` 环检查在事务外;12 处递归 CTE `UNION ALL` 不防环 | `documents/mod.rs:527-550` |
| 10-19 P2-10 | MCP id 裸拼 URL;缓存无上限且不感知外部编辑 | 本轮升为 **P1-1** |
| 10-19 P2-11/12 | 本地库 `updates_after` 不验 CRC;restore/rollback 无事务 | `store.rs:1318`、`1052-1100`、`1538-1567` |
| 10-19 P2-13 | `APP_ENV` fail-open | `config.rs:11-17`。后果比原报告多两条:<br>· 拼错会关掉 `origin.rs:37` 的 CSRF Origin 校验<br>· cookie 不带 `Secure`(`auth.rs:1156`) |
| 10-19 P2-14 | presence socket 掉线不重连 | `sync_client.dart:52`。token 在构造时就写死了，比原报告更糟 |
| 10-19 P2-15/16 | `reorder_views` 无上限;`transfer_roots` 全串行 | `transfer.rs:72-137`、`553-677`。原表说「批量 UPDATE 已修」是误记,fe0c0e01 改的是 `workspaces.rs` |
| 10-19 P2-18 | 破坏性工具 confirm 拒绝走 protocol error | 剩 6 处:`tools_admin.rs:51`、`tools_trash.rs:43/75/160/191/224` |
| 10-19 P2-19 | 日志泄露 token | 部分:镜像设了 `RUST_LOG=info`,但 `telemetry.rs:5` 默认值仍是 `tower_http=debug` |
| 10-19 P3-2/P3-6 | `upsert_snapshot` 重复解码;`guarded_from_update` 未推广 | `mica-core/store.rs:714`、`app-core/store.rs:171/464/491` |
| 10-19 P3-5 | `decode_gbk` 每次调用重建 2.4 万字符的表 | `zip/names.rs:67` |
| 10-19 P3-8 | 外壳 6 个 ChangeNotifier 未 dispose;`knownClientId` 死 API | `main_shell.dart:1242-1254`、`sync_reconcile_controller.dart:686` |
| 10-19 P3-9 | clipboard 兜底 load 挂起;`_runSelectionAi` 用旧选区 | `image_actions_web.dart:97`、`editor.dart:5862` |

---

## P1 — 严重(数据丢失 / 进程级崩溃 / 越权)

### P1-1 MCP 工具 id 路径穿越：一次提示注入就能硬删整个工作区 【复核】

**状态:✅ 已修(2026-10-21,id 全部改为 Uuid;C11 缓存失效一并修)**

- **位置**:`crates/mcp-server/src/lib.rs:137` `url()` 直接 `format!` 拼接;args 里的 id 全是 `String`(`args.rs:41` 等)。
- **触发**:
  1. 页面内容里的提示注入，诱导模型调用 `mica_unshare(document_id="../../../workspaces/<V>#")`。
  2. reqwest 按 WHATWG 规则解析 URL:`..` 被消掉,`#` 之后被丢弃，实际请求变成 `DELETE /api/workspaces/V`。
  3. 服务端 `workspaces::delete`(`workspaces.rs:324`)只检查调用者是不是 owner,然后**级联硬删**。
  4. MCP 层的 `confirm`/`expect_name` 护栏被整个绕开。其余 DELETE/POST 工具都能用同样手法打到任意端点。
- **修法**:args 里的 id 改成 `uuid::Uuid`(schemars 开 uuid 特性);路径段改用 `Url::path_segments_mut` 拼接。

### P1-2 切换页签丢掉最后 ≤400ms 的输入,retry 定时器把旧文档的 op 发给当前会话 【复核】

**状态:✅ 已修(2026-10-21)**

- **位置**:
  - `main_shell.dart:1020` `_selectTab` 切换前不 flush。对照侧栏的 `main_workspace_view.dart:4703`,那里会先 `await _commandHook.flush()`。
  - 编辑器 `dispose`(`editor.dart:958-1005`)不 flush。
  - `EditorController.dispose`(`controller.dart:3727`)只取消 `_saveTimer`,不取消 `_retryTimer`(`:655`)。
- **触发**:打字后立刻点另一个页签，旧编辑器被销毁，防抖窗口里的最后一段输入永远提交不上去。残留的 `_retryTimer` 和 hook 还会经 `_applyEditorOperations` 把旧文档的 op 发到**当前活跃**的会话。
- **修法**:
  - 页签切换走同一个 flush。
  - 编辑器 dispose 时同步 flush 并取消 `_retryTimer`。
  - `onOps` 在创建时绑定 documentId。

### P1-3 撤销拖拽移动只改本地顺序,CRDT 里仍是移动后的顺序 【读码】

**状态:✅ 已修(2026-10-21)**

- **位置**:`moveBlock`(`controller.dart:3386`)发 `move_block`;`_diffOps`(`:3672-3700`)注释写着「编辑器从不重排块」,不会生成移动 op。
- **触发**:拖动一个块后按 Ctrl+Z,本地顺序还原了，但服务端/CRDT 没有。之后按下标插入的 `insert_block` 会落到错误位置，重载后块又跳回去。
- **测试是真空通过的**:`move_block_test.dart:40` 只断言了本地顺序，没检查发出的 op。

### P1-4 协作模式下 Undo 会回滚/删除协作者的编辑(10-19 #8,仍未修) 【读码】

**状态:🟡 已降级修复(2026-10-21):撤销只还原此后没人碰过的块;彻底方案见计划阶段 5**

机制按 10-19 第三轮复核的描述:历史栈里存的是远端编辑之前的全域快照,`_restore()` 整份还原后,`_diffOps` 会把远端新增的块推成 `delete_block`。修法要按历史栈来设计，见计划阶段 5。

### P1-5 导出再导入，页面标题被改名 + 正文多出一个标题(round-trip 红线) 【复核:代码路径】

**状态:✅ 已修(2026-10-21,含 front matter 与 `.Md` 后缀)**

- **位置**:`interchange/src/import.rs:275`,`title = fallback`。文档页的名字取自 sanitize 过的文件名,manifest 里的 `title` 只给 folder 用(`:143-147`)。
- **触发**:
  1. 页面「Meeting notes」导出为 `Meeting_notes.md`,正文开头是 `# Meeting notes`。
  2. 导入后页面名变成 `Meeting_notes`。
  3. 开头的 H1 因为和这个名字不完全相等，没有被剥掉，标题出现两次。
  - 名字含空格/标点的页面，以及同名带 `-2` 后缀的页面都会中招。唯一的闭环测试(`:931`)用的是「季度回顾」,sanitize 改不动它，所以测不到。
- **同族**:`strip_leading_h1` 不跳过 front matter(`import.rs:451-465` 对照 `lib.rs:184-199`),带属性的页面导入后同样多出标题。

### P1-6 quote/li 深度无上限 → 栈溢出或 `repeat` 内存耗尽，进程 abort(10-19 #6a,仍未修) 【复核】

**状态:✅ 已修(2026-10-21,`MAX_NESTING_DEPTH`=8,两端同值)**

- **位置**:`markdown/src/block.rs:1042` `quote_depth_of`、`html.rs:149` `li_of` 都不 clamp(Dart 侧已 clamp 到 0..8,`model.dart:66/74`)。
- **触发路径 ①**:一行几万个 `>` → 渲染时 `render_quote_group` 递归 N 层 → 栈溢出。分享页和 HTML 导出都会触发。
- **触发路径 ②**:成员经 MCP/REST 写入 `data.li=1e9`(`validate_block` 不校验 data)→ `export_md.rs:94` 执行 `" ".repeat(4e9)` → 内存耗尽。
- **修法**:`.min(8)`,两端用同一套上限。

### P1-7 内联链接/图片交替嵌套，疑似指数级回溯(连同 10-19 #6b) 【读码,需实测】

**状态:✅ 已修(2026-10-21):实测确为指数级(221 字节 release 1.3 s);缓存贯穿 + 嵌套预算 32,修后 <1 ms**

- **位置**:`inline.rs:976/869`。`push_link` 每次都新建 `LinkCache`。
- **触发**:`[![[![…a](u)](u)](u)](u)`。递推大约是 P(k)=2·P(k-2),几百字节就能卡死导入、MCP,以及 Dart 编辑器(`marks.dart:1278` 结构相同)。现有的 `nested_bracket_perf.rs` 只测了纯链接和纯括号。
- **修法**:缓存贯穿整次解析，再加深度预算(例如 32 层)。超出预算按字面文本处理。

### P1-8 `purge_view` 能直接永久删除活页面(已知，待拍板)

**状态:✅ 已修(2026-10-21,用户拍板收紧：只能删回收站里的页面)**

`trash.rs:108`。今天只有回收站界面会调它，所以「不咬人」。但 P1-1 那条路径和任意 REST 客户端都能直接打到它。顺带的修法见「精简」一节：单条操作复用批量函数，批量版本身就带 `is_deleted` 过滤。

---

## P2 — 中等

### 安全

| # | 位置 | 问题 | 证据 |
| --- | --- | --- | --- |
| S1 | `avatar.rs:98/120/219` | **头像接受 SVG**(`files.rs:1004` 的 `mime_to_ext` 收 `image/svg+xml`),原样写 Content-Type,公开 302 跳到存储域，绕过了 B4 的修复 | 【复核】 |
| S2 | `tokens.rs:62-67`、`auth.rs:930-947` | 被盗的 JWT(1 小时有效)能铸出**永久 PAT**:改密、删号之后照样可用;`resolve_pat` 不过滤已删账号 | 【读码】 |
| S3 | `ai.rs` `ListModelsQuery.api_key`、`client.dart:1131` | 明文 AI key 放在 GET 查询串里，会进 nginx access log | 【复核】 |
| S4 | `lib.rs:308`、`share.rs:141` | CSP 缺 `form-action` / `base-uri`;`<meta http-equiv=refresh>` 也不拦。结果：分享页可做钓鱼表单和开放重定向 | 【读码】 |
| S5 | `cli/src/client.rs:520-527`、`main.rs:862-877` | `mica-cli rehost-images` 在客户端做 SSRF:跟随重定向、不拦内网地址、不校验类型 | 【读码】 |
| S6 | `metrics.rs:724` | HTTP method 标签基数无上限，未登录请求就能让内存无界增长 | 【读码】 |
| S7 | `rate_limit.rs:41`、`ai.rs:123`、`config.rs:11` | 旧条目 XFF、ai/settings、APP_ENV,见上表 | — |

### 进程级 DoS / 性能

| # | 位置 | 问题 | 证据 |
| --- | --- | --- | --- |
| D1 | `auth.rs:1330/1377` | Argon2 同步跑在 tokio worker 上，没走 `spawn_blocking`;并发登录能卡住 WS 和 `/ready` | 【复核】 |
| D2 | `ws.rs:146` | WS 消息上限用的是 axum 默认的 64 MiB,REST 同类写入只允许 2 MB | 【读码】 |
| D3 | `ai_edits.rs:75/148` | AI 编辑预览的 markdown 解析不限大小、不放进 blocking 线程 | 【读码,需实测】 |
| D4 | `inline.rs:1002/67/184-190`、`html.rs:357-373` | 还有几处平方级:<br>· 未闭合的 `\(`<br>· `_a` 重复 n 次时的 email 扫描<br>· `*a* ` 重复 n 次时 html_span 的 O(M²) 取最小 | 【读码,需实测】 |
| D5 | `preview_raster.dart:113-124` + `render.dart:864` | 超过 48 个不同公式时，数学预览缓存会「驱逐 → 重新请求 → 再驱逐」,永不收敛，公式还会闪烁。图片缓存(上限 64)同理 | 【读码,需实测】 |

### 数据正确性

| # | 位置 | 问题 | 证据 |
| --- | --- | --- | --- |
| C1 | `transfer.rs:499→771-786` | 跨工作区移动期间，协作者新建的子页会被软删但没有拷贝，用户看到的却是「移动成功」 | 【读码】 |
| C2 | `import.rs:511→1229/1316`、`crud.rs:34/120/183` | 父节点检查是 TOCTOU:后台导入跑到一半，目标文件夹被删进回收站，后续页面就挂到已删的父下面 → 清空回收站时 CASCADE 把这些页面删掉，它们的 documents/share 行留下成泄漏。修法：触发器 `views_parent_must_be_folder` 同时要求父节点未删除、`workspace_id` 一致 | 【读码】 |
| C3 | `mica-core/store.rs:1503` | `delete_doc` 不删 `doc_version`,「永久删除」的页面和被移除服务器的文档，全文仍留在 SQLite 里 | 【复核】 |
| C4 | `mica-core/store.rs:757` 等 4 处 | 本地搜索/反链/图谱不按 origin 和工作区过滤，会混入云端镜像和其他本地工作区的页面 | 【复核】 |
| C5 | FFI `api/store.rs:841` | `clone_view` 的子树遍历没有 visited 集合：遇到环就在 UI 线程上死循环直到 OOM;还会把子树里已在回收站的页面以「未删除」状态复活 | 【复核】 |
| C6 | `local_doc.dart:157`、FFI `store.rs:1032` | 本地 `appendUpdate` 失败被吞(返回 0 不检查),之后 compact 时这次编辑丢失 | 【读码】 |
| C7 | `mica-core/store.rs:1090-1097` | restore 的顺序是「先写库再解码」:坏掉的旧版本先成为 live base,下次打开页面就报损坏 | 【读码】 |
| C8 | `controller.dart:3643/3663` | redo 前不 flush 待提交输入:Ctrl+Z → 打字 → 400ms 内 Ctrl+Y,刚打的字丢失 | 【读码】 |
| C9 | `controller.dart:190-200` | 远端编辑到达时，本地选区被折叠成光标，且光标偏移不随之调整 | 【读码】 |
| C10 | `zip/reader.rs:108` | ZIP64 extra 只和 `end` 比较，不和 `extra.len()` 比较 → 越界 panic(旧 #7) | 【复核】 |
| C11 | `mcp-server/tools_organize.rs:336` | `mica_restore_version`、trash、purge 之后不清 MCP 读缓存，之后的写基于旧文本 | 【读码】 |

### 客户端可用性

| # | 位置 | 问题 | 证据 |
| --- | --- | --- | --- |
| U1 | `main_shell.dart:3022-3062` | `_selectView` 的结果写进**完成时**的活动页签：页面加载期间切了页签，加载结果会覆盖另一个页签;连续点两个页面也没有序号守卫 | 【读码】 |
| U2 | `main_shell.dart:659-770` | `_restoreSession` 不防服务器切换：网络慢时切到服务器 B,A 的 token 会写进 B 名下，还会被发往 B | 【读码】 |
| U3 | `main_shell.dart:978-995` | 后台页签加载失败时，移除方式不修正活动下标 → RangeError,还会泄漏 WS | 【读码】 |
| U4 | `updater_desktop.dart:151` | 「更新并重启」直接 `exit(0)`,绕过了退出前的 flush | 【复核】 |
| U5 | `import_controller.dart:177` | 导入轮询一次网络抖动就判定失败，取消按钮也随之消失 | 【读码】 |
| U6 | `cli/src/client.rs:481-575` | CLI 30s 总超时会打断大文件 Notion 导入的上传 | 【读码】 |

## P3 — 优化(精选)

- **启动预热与回填的竞态**(本轮 0.13.63 引入):`main.rs:70-80/166-186`。回填的 UPDATE(`sync.rs:337`)不改 `updated_at`,如果它在预热之后才把 `content_text` 补上，增量刷新就永远拉不到这条正文。只影响从迁移 0012 之前升级上来的实例。【复核】
- **事务时间戳与 5 s 安全回退**:大的 transfer/clone 事务里 `updated_at=now()` 取的是事务开始时刻，事务超过 5 s 就可能被索引游标漏掉。修法：派生列 upsert 改用 `clock_timestamp()`。【读码】
- **`kDevAutoLogin` 默认 true,正式版也带着**:`main.dart:114`,发版构建都没传 define。后端是 localhost 时，会尝试用 demo 账号登录或注册，目前只是被弱密码表拦下了。修法:`&& !kReleaseMode`。【复核】
- **编辑器性能**:
  - 光标在代码块里移动，会整段重新高亮(`render.dart:618-628/1331`)。
  - undo 保存 200 份全文档拷贝。
  - `setHover` 每次鼠标移动做 7 遍线性扫描。
- **CLI/MCP 杂项**:
  - 第三方图片返回 401 时，提示用户重新登录(`client.rs:526`)。
  - token 文件先写后 chmod,中间有一段窗口(`config.rs:55`)。
  - `mica_read_documents` 不走服务端的 batch-read。
- **api-server 杂项**:
  - views socket 不复查成员资格。
  - 重置密码、邮箱验证这些多步写不在事务里。
  - 多处漏了已删账号(墓碑)过滤。
  - AI 出站请求无超时。
  - 名称字段不限长度。
  - forgot/resend 存在时序上的邮箱枚举。
- **本地库**:FFI `save_view` 每行都全表扫描(`store.rs:521`);子树操作没有事务。
- **cargo machete**:3 个未使用的依赖(见工具表)。

---

## 精简、死代码与重复(估算)

| 区域 | 内容 | 估算 |
| --- | --- | --- |
| 死代码(Rust) | 以下都是零引用，或只有测试在用,`grep -rnw` 已核:<br>· `sync.rs` 的 `pull_workspace_updates` / `workspace_head` / `pull_document_updates` / `document_head` / `document_base`<br>· `store.rs:984` `fetch_file_by_key`<br>· `properties.rs:147` `infer_value`<br>· `transfer.rs` 的 `created_document_ids`(只写不读)<br>· WS 的 `document.update` 处理(没有客户端发送)<br>· `greet`/`add` 冒烟函数 | −250 |
| 一次性工具 | `api-server/src/legacy_orphans.rs`(322 行)及其唯一依赖 `roxmltree`。生产回填已在 v0.13.47 跑完 | −322(等宽限期结束) |
| 死代码(Dart) | 以下都是零引用或只有测试在用:<br>· `api/models.dart:1253` `collectSubtreeIds`(只有测试在调，属于真空通过)<br>· `knownClientId`<br>· `SessionController.seedViews`<br>· `render.dart:1943` `_paintMathPreview`(约 105 行，光标永远进不去;顺带去掉唯一的 `MaskFilter.blur`) | −200 |
| 单条复用批量 | `delete_view` / `restore_view` / `purge_view` 改为调用 `*_batch(&[id])`:顺带补上事务，并修掉 P1-8 | −90 |
| 重复 SQL / 插入循环 | View 列清单重复 7 处;transfer 和 clone 的插入循环;三处 `insert_page` | −200 |
| 三处 base 写入 | 合成一个 `write_base_tx`:<br>· 只拍平一次<br>· 派生列一次算完<br>· 用 `guarded_from_update` 解码<br>D1 兄弟路径、views.name 漂移、guarded 推广三条一起解决 | −40 |
| 跨 crate 重复 | 以下都是多处各写一份:<br>· `unique_asset_name` 三份，大小写规则已经不一致<br>· `safe_segment`、`percent_decode` 各两份<br>· `Block` 结构两份(mica-core / markdown) | −120 |
| mcp-server | 模块化后留下的样板:<br>· 7 份重复的 `//!` 头注释<br>· 24 处 `ensure_writable` 样板<br>· confirm 处理改为 `require_confirm`,顺带修掉 P2-18<br>· 手写的 urlencode | −190 |
| api-server | · AI 两条路径(REST / WS)重复<br>· token 取值函数逐字重复<br>· `normalize_email` 重复<br>· `_in_tx` 版本 SQL 重复<br>· 约 20 处 `reqwest::Client::new()` 改为共享 client | −200 |
| mica-core | · 手写的事务 4 处改用 RAII<br>· 9 处 `pragma_table_info` 列探测抽成函数<br>· 投影派生合并 | −150 |
| Dart 重复 | · 图片 MIME 嗅探两份，已漂移<br>· SHA-256 两份(而且为此引入了 `crypto` 依赖)<br>· 前缀 diff 三份 | −120 |
| 外壳层 | · 控制器状态在 State 里还留着同步副本(shell 11 个字段，view 5 个)<br>· 约 8 个一行适配器<br>· presence 并进 `CloudSyncSession` 后可删 `sync_client.dart`(193 行) | −300 |
| 编辑器/markdown | · `block_index` 线性查找改成映射，同时解决 O(B²)<br>· 反引号分支重复<br>· `matching_bracket` 改用 `BracketMemo` | −70 |
| **合计** | | **约 −2,200 行**,另有约 3,000 行可从巨石文件迁出 |

还有一批**错位或过时的注释**:文档注释挂错了函数(`sync.rs:98-132`、`html.rs:719`、`store.rs:32-54`、`config.rs:27`、`ws.rs:579` 等十余处),以及描述已经不成立的注释(`doc.rs:786` 仍写「wholesale 替换」、`controller.dart:3672` 仍写「从不重排」、`sync.rs:496` 仍写「op-model snapshot」)。这类注释误导下一个读的人，代价不比 bug 小。

## 架构结论

1. **「兄弟路径漏守卫」第三次大面积命中。** 本轮至少 7 例：单条 vs 批量 purge/restore、本地 vs 云端 clone 子树、头像 vs 文件上传 MIME、forgot/resend vs 登录时序、views vs document socket 复查、FFI vs 服务端 `next_position`、REST `UpdateBlock` vs push 写 `views.name`。

   结构性的解法是**单条复用批量、一个入口多个调用方**,而不是每次修一处。计划阶段 4 按这个思路收口。

2. **校验只在客户端 = 没校验**(lessons §2)。本轮新增两例：
   - `reject_mangled_latex` 只在 MCP 代理层生效。
   - quote/li 深度只在 Dart 侧 clamp。

   两者都应该下沉到服务端写入或渲染路径。

3. **巨石文件的棘轮在按文件计数时被 `part` 绕过。**
   - main.dart 那组 part 文件合起来 20,512 行，编辑器那组 7,132 行。
   - 拆进 part 文件只是换了个位置，耦合度没降。
   - FFI crate 不在扫描范围内。

   棘轮应该改为按库计算。

4. **外壳层的「双表示」**:本地世界的 6 个 `_local*` 字段和 SessionController 是同一形状的两份状态,`_unifiedWorkspaceView` 里有 81 处 `local ?` 三元分派。抽一个 `WorldBackend` 接口统一分派，比继续加分支便宜。

5. **渲染注册表红线**:9 月以来没有新增按 kind 的分支。存量里该收进注册表的有：
   - 表格 hit-test,9 个方法
   - `setHover` 里的 image/code/table 分支
   - `_drawAtomicHighlight` 的 table/image 分支

   建议给 `AtomicBlockRenderer` 加 `hitTest` / `paintSelection` 两个方法来承接。

6. **没有契约测试**:服务端 DTO 和 cli / mcp / Dart 三个客户端之间没有任何契约测试。mcp-server 的 `tools_*.rs`(约 1,480 行)没有一个测试校验实际发出的 URL 和请求体 —— P1-1 就藏在这里。

7. **工程链**:
   - `rustfmt.toml` 从来没被执行过，存在 2/4 空格混用。
   - `just check` 用 `2>/dev/null ||` 吞掉了 clippy 的失败。
   - workflow 的 setup 步骤重复 6 次,action 版本混用。
   - `architecture.md` 仍写着 Tantivy / Redis / 无 FFI,和代码全面不符。
   - 依赖豁免清单漏登了 ratex-\*、roxmltree、webview2-com、crypto 等。
