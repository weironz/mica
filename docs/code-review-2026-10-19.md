# 全项目代码审查 — 2026-10-19(第二轮)

审查对象:`main` @ `55ec288`(v0.13.55)。方法:9 个按域切片的只读评审代理
(api-server 核心/认证路由/文档路由/mica-core+infra/markdown+interchange/
cli+mcp-server+app-core/Flutter 重构回归/Flutter 编辑器同步/Web 端存在性),
最高危结论由主会话逐行对照源码复核,并运行了 `cargo clippy --workspace
--all-targets`(**0 警告**,补齐 10-18 报告"clippy 冷编译超时未出结果"的空白)。

## 证据等级

- **【复核】** —— 主会话逐行读了相关源码确认(含在 `origin/claude/eager-johnson-blcmyx` 分支上验证修复状态)。
- **【读码】** —— 评审代理带 file:line 报告,方向可信,动手前再确认一次。

本报告**没有**做到的:未跑测试套件、Flutter 未编译验证、未做依赖漏洞扫描。
这些是空白,**不是「查过没问题」**。

---

## 与 2026-10-18 评审(`claude/eager-johnson-blcmyx`)的关系

两轮为**相互独立**的全项目评审,结果高度互补:

| 桶 | 数量 | 说明 |
|---|---|---|
| 重叠项:分支已修 | ~14 | owner 降级(A2)、tagfilter(A6)、导出 XSS/CSP(B5)、zip(A1/ZIP64)、resend 限流(B3)、登录时序(C10)、重置强度(B2)、WS 撤销滞后(B6)、expires_in_days(C1)、push 3× flatten(D1)、DocumentHub 清理(C7)、错误泄露(C3)、未闭合 `[<` 平方级(A8 待办,结论一致) |
| 本轮净新增 | ~20 | P1×3(见下 #2/#3/#8)+ P2 ~13 + P3 若干;已在分支 ref 上逐条验证未修 |
| 对方独有 | ~15 | **A3 `set_blocks` 全量重建吞并发/离线编辑(两轮中最严重的一条)**、A9 AI key 串号、B4 存储域 XSS、B7 配额、B9–B12 WS 协议级、B13/B14 round-trip、C2/C5/C6/C8/C9 等 |

两条对方提出的结构性结论在本轮净新增里**再次命中**:

- 「兄弟路径漏守卫」——本轮 #3(本地 snippet 是对方 A5 的兄弟路径,分支只修了 `search.rs`,`store.rs:116` 原样未动)、#2(`purge_view` 是 `batch_purge_views` 的兄弟,批量版有 `is_deleted` 过滤、单条版没有)。
- 「无上界=进程级」——本轮 #6b(图片 label 嵌套无深度上限)、#8 的 `_diffOps` 全量重建思路与 A3 同族。

---

## P1 — 严重(数据损坏 / 越权 / 进程崩溃)

### #1 工作区 admin 可把 owner 降级,owner 永久失控 【复核】⚠ 分支已修
- **位置**:`crates/api-server/src/routes/workspaces.rs:469` `add_member`。
- **成因**:`INSERT … ON CONFLICT DO UPDATE SET role=EXCLUDED.role` 无 `ensure_not_workspace_owner`;删除工作区按 `role='owner'` 判定(`:303`),降级后工作区成为无 owner 孤儿。
- **分支状态**:已修(A2 + migration 0028 触发器兜底)。合并分支即闭环。

### #2 `purge_view` 可物理删除未进回收站的活页面整棵子树 【复核】🔴 净新增未修
- **位置**:`crates/api-server/src/routes/documents/trash.rs:105-157`。
- **成因**:递归子树种子 `SELECT … WHERE id=$1 AND workspace_id=$2` 缺 `is_deleted=true` 过滤;`batch_purge_views`(`:323` 注释)加了过滤,单条版没有——标准「兄弟路径」。任意 editor 一条 DELETE 永久抹掉活页 + documents + yrs + share。
- **分支验证**:`git show <branch>:trash.rs` 该区域 `is_deleted` 计数 = 0。
- **修法**:种子加 `AND is_deleted = true`,未进站返回 404/409;配 pg 回归测试。

### #3 本地搜索 snippet 跨字符串字节索引,用户数据可触发 panic 【复核】🔴 净新增未修
- **位置**:`crates/mica-core/src/store.rs:116`(FFI 侧 `Mutex<LocalStore>` 包裹,panic 会毒化锁——正是 2026-07-20 报告 P0-1 的失败类别)。
- **成因**:`at` 是 `to_lowercase()` 结果的字节偏移,却切原 `body`;`ẞ→ß`、`İ→i̇` 折叠改变字节长 → 非字符边界 panic / out of bounds。任何含异形折叠字符的文档 + 普通搜索词即触发。
- **分支验证**:对方 A5 修的是 `routes/documents/search.rs` 的 `snippet_for`;`store.rs:116` 在分支上原样未动。
- **修法**:边折叠边推进产出 char 下标(与 `quote_match.rs` 归一化做法一致),补 `İ/ẞ` 回归用例。

### #4 `tagfilter` 把非 ASCII 全部变成乱码 【复核】⚠ 分支已修(A6)
- `crates/markdown/src/html.rs:656` `out.push(s.as_bytes()[i] as char)` Latin-1 语义,中文 raw HTML 在分享/导出全变 mojibake。对方已改为整段 `push_str`(分支上已验证该字节 push 不存在)。

### #5 导出 HTML 无 CSP + href 无 scheme 白名单 + `align` 属性注入 ⚠ 分支已修(B5)
- `html.rs:433` 链接 href 直通 `javascript:`;`lib.rs:281` `export_html_document` 无 meta CSP,「另存为」后分享页 CSP 失效;`html.rs:532` 表格 `align` 无白名单可属性注入。对方 `safe_url` + CSP meta + 白名单已覆盖。

### #6 层级/嵌套深度无钳制 → 栈溢出 abort(非可捕获 panic) 【读码+分支抽查】🟡 部分净新增
- (a) `data.li`/`data.quote` 可经 MCP/REST 写任意值,渲染侧 `li_of`(html.rs:143)/`quote_depth_of`(block.rs:2030)不 clamp(`list_indent` 有 `.min(8)`,这两处漏);分支上 `min(8)` 仍仅 1 处,B15 修的是**导出侧** children 成环。**渲染侧未修**。
- (b) 图片 label 嵌套无深度上限(链接 label 有 `label_has_link_cached`,图片没有):`![` 套娃百万层 → 递归嵌套持有 `Vec<char>` 帧,O(n²) 活栈内存 → abort。**未见修复**。
- **修法**:渲染侧 clamp `.min(8)`;`parse_inline_memo` 加深度计数(如 64 层),超限按字面文本处理。

### #7 导入 zip bomb + ZIP64 extra 越界 panic ⚠ 分支已修(A1 + fe0c0e0)
- `reader.rs:127` 按声明 `uncomp_size` 预分配 + 无总量上限(1 GiB 包 → OOM abort);ZIP64 解析不查 `end <= extra.len()`。对方 zip_limits.rs + ZIP64 修正已覆盖。

### #8 协作模式下 Undo 会回滚/删除协作者的编辑 【读码+机制推演】🔴 净新增未修
- **位置**:`clients/mica_flutter/lib/editor/controller.dart:3618-3701`。
- **成因**:`_undoStack` 存全域快照,`reconcile()` 合入远端 CRDT 时**不更新**快照;undo 的 `_diffOps` 对远端插入块发 `delete_block`、对远端改过的块 LWW 回滚,并经 op 路径推给所有协作者。
- **分支验证**:`git diff HEAD <branch> -- editor/controller.dart` 中 undo 相关行 diff 为空。
- **修法(建议)**:短期在协作者在场时降级/禁用全域 undo;中期 `reconcile()` 对每个快照按块 id 归并;长期放弃全域快照,改记录本地 op 逆操作(Yjs UndoManager tracked-origins 思路)。

### #9 `/api/auth/resend-verification` 不限流的邮件轰炸通道 ⚠ 分支已修(B3)
- `rate_limit.rs:174` 未列入;未认证可无限触发真实发信。`email_verify.rs:176` 注释声称"`/auth/` 前缀有限流",与事实不符(注释本身也是 bug)。

## P2 — 中等(精选;⚠=分支已修,🔴=净新增)

| # | 位置 | 问题 | 状态 |
|---|---|---|---|
| 1 | `auth.rs:239` | 登录账号枚举时序信道(未命中不跑 Argon2) | ⚠ C10 |
| 2 | `password_reset.rs:145` | 重置密码绕过强度策略(且按字节计长) | ⚠ B2 |
| 3 | `rate_limit.rs:41` | 无代理直暴时 XFF 伪造可完全绕过 per-IP 限流;仅当 socket peer 属可信代理段才应采信 XFF | 🔴 |
| 4 | `ws.rs:453` | 被移除成员在已建立 WS 上继续收文档内容至 token 过期 | ⚠ B6(30s 复查) |
| 5 | `routes/ai.rs:119` | `GET /api/ai/settings` 向任意登录用户泄露 operator 的 base_url/model/key_hint | 🔴 |
| 6 | `cli/src/update.rs:132` | `mica-cli update` 无校验和/签名;`contains("0.13.5")` 误中 `"0.13.55"` | 🔴 |
| 7 | `cli/src/client.rs:172`、`mcp-server/src/lib.rs:42`、flutter `api/client.dart:1526` | 三个客户端 HTTP 层均无超时,半死连接永久挂起 | 🔴 |
| 8 | `trash.rs:38-76` | 恢复无「删除批次」标记,恢复父级会复活早前单独删掉的子孙;批量恢复缺孤儿提升(对方 A4 修此条);单条恢复两段无事务 | 🟡 A4 部分 |
| 9 | `transfer.rs:20-64` | `move_view` 环检查 check-then-act,并发互换可造环;全库递归 CTE `UNION ALL` 不防环,成环卡死工作区 | 🔴 |
| 10 | `mcp-server/src/lib.rs:137` | 模型给的 id 裸拼 URL path(无 UUID 校验/百分号编码);读写缓存无上限且不感知外部编辑,写路径 LWW 杀评论锚点 | 🔴 |
| 11 | `mica-core/src/store.rs:1340` | `updates_after` 是唯一不验 CRC 的日志读路径——磁盘腐坏**上行**污染权威流,与 CRC 体系立意相反 | 🔴 |
| 12 | `store.rs:1106/1579` | `restore_local_version`/`rollback_doc` 多语句无事务,断电后被丢弃的编辑静默复活 | 🔴 |
| 13 | `infra/src/config.rs:10` | `APP_ENV` 解析 fail-open:`Production`/尾随空格/笔误全落入更宽松的 Development(连锁跳过 JWT 强度校验、CORS 变宽松、结算 `MICA_SEED_TEST_USER`)。安全开关失败方向反了 | 🔴 |
| 14 | flutter `api/sync_client.dart:43` | presence 光标 socket 掉线后永不重连(`onDone` 为空;自家 `ViewsEventsChannel` 就有 capped backoff) | 🔴 |
| 15 | `transfer.rs:74` | `reorder_views` 无批量上限;每 id 3 条串行查询 | 🟡 D5(批量 UPDATE 已修,上限未见) |
| 16 | `transfer.rs:541-665` | `transfer_roots` 逐文档/逐 blob 全串行,大子树迁移分钟级易留孤儿字节 | 🟡 D3 部分 |
| 17 | `inline.rs` 多处 | 未闭合分隔符 O(n²) 扫描(`[<`×4000 等) | ⚠ A8 已记待办,结论一致 |
| 18 | `tools_admin.rs:51` 等 7 处 | 破坏性工具 confirm 拒绝走 protocol error,与自家「可操作拒答走 isError」约定矛盾 | 🔴 |
| 19 | `ai_ws.rs:221` + `main.rs:270` | query 里的 token/探针 key 会被 TraceLayer debug 级记入日志 | 🔴 |
| 20 | `tokens.rs:84` | `expires_in_days` 无校验(负值铸已过期 token;超大值 chrono panic) | ⚠ C1 |
| 21 | `workspaces.rs:462`、`auth.rs:1338` | 已认证用户枚举:add_member 404 vs 成功;开放注册 409 | 🔴(P3 级) |

## P3 — 优化项(精选)

| # | 位置 | 问题 | 状态 |
|---|---|---|---|
| 1 | `app-core/src/sync.rs:129/145/220` | 每次 push(含每个按键)全量 flatten×3 + 重编码;`title_from_doc` 注释自称避免该成本,与实现不符 | ⚠ D1 |
| 2 | `mica-core/src/store.rs:741` | **D1 的兄弟路径**:`upsert_snapshot` 对文档做两遍完整 yrs 解码 + `to_blocks` | 🔴 |
| 3 | `app-core/src/rooms.rs:97` | `DocumentHub`/`views_hub` 死 `Weak` 条目永不清理;`:61` presence 的 Mutex poison `.expect` 可改降级 | ⚠ C7(清理) |
| 4 | `export.rs:664`、`lib.rs:588` | 交互式全量导出逐页串行解码、全结果驻留内存;`block_index` 线性查找 O(B²) | 🟡 D3 |
| 5 | `interchange/src/zip/names.rs:67` | `decode_gbk` 每次调用重建 23,940 字符表(`OnceLock` 即可) | 🔴 |
| 6 | `app-core/src/store.rs:171/460` | 自读 base/只读路径仍用裸 `MicaDoc::from_update`,未推广 `guarded_from_update` | 🔴 |
| 7 | `infra/src/error.rs:39` | `ApiError::Database` 把 sqlx 原文回客户端 | ⚠ C3 |
| 8 | flutter `main_shell.dart` 等 | 重构系列遗留:6 个 ChangeNotifier 只有 1 个被 `dispose()`;`SyncReconcileController.knownClientId` 死 API(`:686` 注释引用的读取点不存在);`removeServer` 丢了原实现的 `!mounted` 中止语义 | 🔴 ×3 |
| 9 | flutter `image_actions_web.dart:97`、`preview_raster.dart:98`、`editor.dart:5861` | clipboard 兜底 load 失败挂起泄漏 DOM;preview key 缓存逐出后条目常驻;`_runSelectionAi` 用对话框前的 stale selection | 🔴 ×3 |
| 10 | `main_workspace_view.dart` | 源码含字面 NUL 字节,grep 跳过整文件 | ⚠ C8(对方独有发现) |

## 对方独有、本轮未发现但值得知晓的(分支状态见其报告)

**A3** `set_blocks` 先删光再插全新 YMap/YText,并发/离线编辑静默丢失(两轮中最严重,已修为按块 id 差量);A9 AI key 按 provider 串号;B4 存储域同站 XSS(未修,方案已定);B7 配额绕过(部分修);B9 ack 先于广播 cursor 越界;B10 `expected_seq` 看不到 WS 编辑;B11 缺依赖 update 被 ack(仅告警);B12 presence 无上限;B13/B14 round-trip 口子;C2 AI 流中文跨块乱码;C5 光标停在 emoji 中间;C6 Dart/Rust 镜像差异;C9 IPv6 限流绕过。

## 建议修复顺序(在合并 `claude/eager-johnson-blcmyx` 之后)

1. **净新增 P1**:#2 purge 活页(一行过滤 + pg 测试)、#3 本地 snippet panic(折叠推进重写 + 回归用例)、#8 协作 undo(先降级兜底再立结构方案)。
2. **#6 渲染侧深度钳制 + 图片 label 深度**:与 B15 同一主题,立「渲染前 clamp + 输入校验」规则一次做全。
3. **安全散件**:P2-3 XFF 可信段、P2-5 ai/settings 读收敛、P2-13 APP_ENV fail-closed、P2-19 日志 token。
4. **客户端可用性**:P2-6/7 超时、P2-14 重连、P2-12 本地库事务化 + CRC 上行。
5. P2-9 `move_view` 环竞态(事务 + FOR UPDATE;纵深:递归 CTE 改 UNION/加深度上限)。

**注**:`claude/eager-johnson-blcmyx` 同时**删除了** `docs/notion-gap-analysis.md`(无替代文件)。合并前需与发起方确认是误删还是有意(本文档默认按误删处理,main 上保留并更新)。
