# 全项目代码审查 — 2026-10-18（v0.13.56）

审查对象：`main` @ `579e333`（v0.13.56）。方法：四个按「问题域」切片的只读子代理
（鉴权/权限、实时同步/CRDT、文件/导入导出/GC、Markdown 引擎 + Dart 镜像），
最严重的结论由主会话逐条对照源码复核，能跑的当场复现。

## 证据等级

- **【实测】** —— 主会话亲自跑了复现，或逐行读了两侧源码确认。
- **【PoC】** —— 子代理在 scratch crate 里写 PoC 复现过，主会话读码确认了成因。
- **【读码】** —— 子代理读码得出、带 file:line，主会话未逐条复跑。方向可信，动手前再确认一次。

本报告**没有**做到的：clippy 冷编译 15 分钟超时未出结果；本机无 Dart 工具链，Dart 侧结论
全部是【读码】；未做依赖漏洞扫描。这些是本报告的空白，**不是「查过没问题」**。

修复进度见文末[「修复进度」](#修复进度)。

---

## 摘要：三个结构性结论

**① 「一处有守卫、兄弟路径没有」是本轮最多的形状。** `update_member`/`remove_member`
防了「动 owner」，`add_member` 没防（A2）；单条恢复会把孤儿提到顶层，批量恢复不会（A4）；
`update_settings` 修过「拿 A 厂商的 key 配 B」，`list_models` 没修（A9）；`change_password`
吊销会话，PAT 不在「会话」里（B1）。lessons#2 说的是「不变量只写在客户端等于没写」，
这里是它的近亲：**不变量只写在一条路径上，等于只写了一半。**

**② 输入大小/形状没有上界的地方，后果都是整个进程，而不是一个请求。** zip 头声明大小直接
`with_capacity`（A1，abort）、`children` 成环递归导出（B12，栈溢出 abort）、搜索切片越界
（A5，panic 断连接）、段落 inline 重解析立方级（A8）。Rust 的 abort 不会被 `spawn_blocking`
或 tokio 任务边界接住 —— 「有 catch_unwind」不等于「不会挂」。

**③ round-trip 红线还有几个口子**，全在「转义」：`\[` 被读回成数学块（A7）、块首转义只做第一行
（B13）、空待办丢 `[ ]` 后的空格（B14）。fuzz 跑出 ~5900 个不稳定样例，多数是符号沙拉，
上面三条是能从编辑器正常操作走到的。

---

## A — 严重（数据丢失 / 进程级 DoS / 越权）

### A1 恶意 zip 让 API 进程 abort 【PoC】
- **位置**：`crates/interchange/src/zip/inflate.rs:21` `Vec::with_capacity(size_hint)`；
  `size_hint` 来自 zip 头声明的解压大小（`reader.rs:73/92`）。
- **路径**：`POST /api/workspaces/import` → `run_import` → `spawn_blocking(read_zip)` → `inflate`。
  声明 `1<<60` 字节 → 分配失败 → **进程 abort（SIGABRT）**，不是 panic，接不住。
  125 字节的 zip 即可，任何登录用户可反复触发；每次断开全部 WebSocket、中断全部导入。
- **相关**：导入路由 body 上限 1 GiB（`routes/mod.rs`），256 MiB 检查在 body 读完之后；解压
  总量无上限（`import.rs` 注释自承）。
- **修法**：预分配夹到 `min(size_hint, 压缩长度×N, 8 MiB)`；inflater 内设单条目/总解压字节上限。

### A2 工作区 admin 能把 owner 降级，owner 永久失去控制 【实测】
- **位置**：`crates/api-server/src/routes/workspaces.rs` `add_member`。
- **成因**：`INSERT … ON CONFLICT DO UPDATE SET role = EXCLUDED.role` 会改写已有成员行；
  `update_member`/`remove_member` 都调 `ensure_not_workspace_owner`，`add_member` 没调。
- **场景**：admin `POST /members {"email":"<owner>","role":"viewer"}` → owner 变 viewer，
  所有 owner 判定读的是 `workspace_members.role`，owner 再也删不了/改不了/管不了成员，无法自救。
- **修法**：`add_member` 补 `ensure_not_workspace_owner`；DB 触发器兜底（owner 行的 role 不可改）。

### A3 每次 REST/MCP 写都重建全部块，并发/离线编辑会静默丢失 【实测（读码）】
- **位置**：`crates/app-core/src/store.rs:216` → `crates/mica-core/src/doc.rs` `set_blocks`
  （先删光 `blocks` map 的全部 key，再逐块插入全新 YMap/YText）。
- **成因**：lessons.md 把代价写成「并发块级后写胜」，实际更大：**未被这次写入触及的块**也被换成新对象。
  另一个客户端对旧 YText 的插入在合并时落到已删除的父对象上，按 CRDT 语义直接成为已删除内容。
- **场景**：桌面端离线（或 400 ms 防抖内）改了段落 X，同时 MCP `append` 在页尾加一段 →
  重连后 X 的修改在所有端消失，无报错。评论锚点同理每次写都失效。
- **修法**：`set_blocks` 改成按块 id 差量：保留存活块的 map，文本用已有的 `set_text_and_marks`
  （做 splice），props/children 只改变化的部分，只对真正增删的块 remove/insert。

### A4 批量恢复把页面留在已删除的父目录下，清空回收站时连带删除 【实测】
- **位置**：`crates/api-server/src/routes/documents/trash.rs` `restore_views_batch`；
  对照单条 `restore_view` 会把父不在活跃集合里的页面提到顶层。
- **场景**：回收站里文件夹 F 含页面 P → 只批量恢复 P（MCP `mica_restore_views` 走这条）→
  P 活着但父被删，树里不可见 → 清空回收站 → `parent_view_id ON DELETE CASCADE` 把活着的 P 删掉。
- **修法**：批量恢复在同一事务里做同样的「提到顶层」。

### A5 搜索摘要切片越界 panic 【实测（读码）+ PoC】
- **位置**：`crates/api-server/src/routes/documents/search.rs` `snippet_for`。
- **成因**：在 `to_lowercase()` 后的串里数字符位置，再去切原文的字符数组；`İ` 这类字符小写后变长，
  `start` 超过 `chars.len()` → `chars[start..end]` panic。
- **场景**：任何成员写一页 ~100 个 `İ` + `foo`，之后所有命中它的搜索都断连接。
- **修法**：位置换算在原文上做（逐字符小写比较），并夹紧 `start <= end`。

### A6 raw HTML 过滤把非 ASCII 全部变成乱码 【实测】
- **位置**：`crates/markdown/src/html.rs` tagfilter：`out.push(s.as_bytes()[i] as char)`，
  把每个 UTF-8 字节当成 Latin-1 字符。
- **复现**：`<p align="center">居中</p>` → `<p align="center">å±\u{85}ä¸\u{ad}</p>`。
  分享页、HTML 导出、行内 HTML 都中招；中文 README 里 `<details><summary>` 极常见。
- **修法**：按字节扫描但整段 `push_str(&s[start..i])`，同文件 `strip_unsafe_attrs` 已是这么做的。

### A7 round-trip：只有 `[…]` 的段落被读回成数学块 【实测】
- **位置**：`crates/markdown/src/inline.rs` `escape_inline` 把 `[` 写成 `\[`；
  重导入时 `block_handlers.rs` `math_block` 把以 `\[` 开头的行当 display math。
- **复现**：`[TODO]` → 导出 `\[TODO\]` → 重导入 `math_block`。块类型变了，等于丢数据。
- **修法**：行首的 `\[` 改写为不会被识别成数学的形式；Dart `escapeInline` 同步。

### A8 普通多行段落解析是立方级耗时 【实测】
- **位置**：`crates/markdown/src/block_handlers.rs` 段落续行时每行对整段重跑 inline；
  `inline.rs` `extended_autolink` 在每个词边界 `chars[i..].collect()` + 小写化。
- **复现**：`"a\n" × 2000`（4 KB）1.9 s；8 KB 14.7 s。另有短恶意输入：`"[<" × 4000` 8.7 s。
  导入 / MCP / 粘贴都走这里。Dart 镜像（`markdown.dart` `reapply`、`marks.dart`）同构。
- **修法**：段落关闭时只做一次 inline；`extended_autolink` 先在 `chars[i..i+8]` 上比前缀；
  `matching_bracket`/`inline_span_end` 记忆化。

### A9 AI「拉取模型列表」把当前启用厂商的 key 发给另一家 【实测】
- **位置**：`crates/api-server/src/routes/ai.rs` `list_models`；客户端 `ui/dialogs.dart` `_fetchModels`。
- **成因**：切到厂商 B 没填 key 时，请求只带 B 的 `base_url`，服务端回落到**当前启用**厂商 A 的 key。
  与 migration 0022 修过的是同一类 bug —— `update_settings` 修了，`list_models` 漏了。
- **后果**：A 的密钥发到 B 的服务器；B 自己存了 key 却显示「key 被拒」。
- **修法**：请求带 `provider_id`，服务端回落用该厂商自己保存的行，不再回落到启用中的配置。

---

## B — 中等

### 安全 / 权限
- **B1 改密/重置密码不吊销 PAT** 【实测】`auth.rs` `revoke_user_sessions` 只动 `refresh_tokens`；
  PAT 默认永不过期。盗号者建个 PAT，受害者重置密码后照样读写。
- **B2 重置密码绕过强度策略且按字节计长** 【实测】`password_reset.rs` 只查 `len() < 8`，
  不走 `password_strength::check`（注释称它是「唯一的门」）。
- **B3 `/auth/resend-verification` 不限流** 【实测】`rate_limit.rs` `is_rate_limited` 未列入；
  `email_verify.rs` 注释说它被限流，不实。可轰炸收件箱、烧发信额度。
- **B4 存储域同站 XSS** 【读码】预签名 PUT 不签 Content-Type，`build_object_key` 保留 `.html`，
  GET 不强制 `attachment`；SVG 被当图片接受。存储域与应用通常同站，`SameSite=Strict` 照样带上；
  WS 握手不校验 `Origin`。
- **B5 HTML 导出文件可执行脚本** 【读码 + PoC】下载的 HTML 没有 CSP：`javascript:` 链接、
  `<!-- --!>` 注释绕过、表格 `aligns` / alert 属性未转义。
- **B6 被移除成员在已建立的 WS 上继续收实时编辑**直到 token 过期 【读码】`ws.rs` bootstrap/pull/广播不复查成员资格。
- **B7 存储配额可被绕过** 【读码】presign 不把未完成上传算入配额；每账号可无限建工作区（每个 5 GiB）；
  跨工作区移动不查目标配额。
- **B8 认证后的 Argon2 端点无限流无闸门** 【读码】`POST /auth/password`、`DELETE /auth/me`。

### 同步
- **B9 ack 先于广播送达，cursor 越过未收到的 rid** 【读码】`ws.rs` 在 incoming 分支发 ack，
  排队中的广播之后才投递；客户端按 ack 推进 cursor，lag 重同步/重连 `since_rid=cursor` 永久跳过。
- **B10 MCP `expected_seq` 看不到 WS 编辑** 【读码】`push_update` 不动 `current_seq`，乐观锁失效。
- **B11 `push_update` 对缺依赖的 update 也回 ack** 【读码】yrs 放进 pending 返回 Ok，内容永不可见。
- **B12 presence 无大小/频率限制且与 update 共用通道** 【读码】viewer 能把别人挤到 Lagged，触发 B9。

### Markdown
- **B13 块首转义只做第一行** 【PoC】硬换行后紧跟 `- `/`# `/`> `/`1. ` 的文本 round-trip 后变块。
- **B14 空待办丢勾选** 【PoC】`- [x] ` 行尾空格被裁，重导入变项目符号。
- **B15 `children` 成环导出栈溢出 abort** 【PoC，crate 级】`html.rs`/`export_md.rs` 递归无 visited 集合。

### 文件 / GC
- **B16 GC 与去重竞态** 【读码】去重命中旧行不清 `unreferenced_since`，Collect 删除前不复查。
- **B17 GC 不看命名版本** 【读码】恢复旧版本后图片已被回收。

---

## C — 低

- **C1** `tokens.rs` `expires_in_days` 超大值 chrono panic 【实测】
- **C2** AI 流按网络分块 `from_utf8_lossy`，中文跨块出 `�` 【实测（读码）】`ai_ws.rs`
- **C3** `Database`/`Internal` 错误把 SQL 原文回给客户端；头像上传失败回显预签名 URL 【读码】
- **C4** 删账号不删头像对象；协作者因 `created_by` RESTRICT 永远删不掉账号 【读码】
- **C5** 编辑器左右键按 UTF-16 单元移动，光标停在 emoji 中间 【读码，Dart】
- **C6** Dart/Rust 镜像差异：`~` 未闭合、Unicode 空白、`mathRunSpans` 不跳代码段、空链接被丢 【读码】
- **C7** 分页 `ORDER BY` 缺唯一兜底键；搜索负 offset 500；导入任务查询/取消不校验属主；
  `DocumentHub` 房间条目永不清理 【读码】
- **C8** `main_workspace_view.dart` 源码含字面 NUL 字节，grep/rg 当二进制跳过整个文件 【实测】
- **C9** IPv6 限流按单地址，换 /64 内地址即绕过 【读码】
- **C10** `login` 未知邮箱不跑 Argon2（时序可枚举账号） 【读码】

---

## D — 可优化项

- **D1 同步热路径**：每次 push 在行锁内解码全文 → 拍平 3 次 → 编码；自动版本
  `INSERT … WHERE NOT EXISTS` 把整份 state 再传一次而几乎总被丢弃；每次 push 改写 `views.name`
  触发全工作区侧栏重拉。
- **D2 PAT 请求**解析两遍 token + 两次 `UPDATE last_used_at`。
- **D3 整工作区导出**串行下载图片、全量进内存再拷贝。
- **D4 GC** 每 6 小时解码所有文档找文件引用。
- **D5** 若干路径持有 DB 事务等 S3 PUT/HEAD；`reorder` 每 id 一条 UPDATE；评论列表 O(线程×评论)。

---

## 修复进度

（随修复提交更新。每条修复配回归测试；提交信息写根因。）

| 条目 | 状态 | 说明 |
| --- | --- | --- |
| A1 zip abort | ✅ 已修 | 预分配夹紧 + inflater 输出上限 + 1 GiB 总预算（嵌套共用）+ 偏移 saturating；`interchange/tests/zip_limits.rs` |
| A2 owner 被降级 | ✅ 已修 | `add_member` 补守卫 + migration 0028 触发器；`workspaces.rs` `owner_role_pg` |
| A3 set_blocks 全量重建 | ✅ 已修 | 按块 id 差量：存活块保留 map，文本 splice，props 只写变化键，children 只换中间段；`mica-core/tests/sync.rs`；lessons.md 更正 |
| A4 批量恢复丢页 | ✅ 已修 | 同事务内提到顶层；`pg_tests.rs` |
| A5 搜索 panic | ✅ 已修 | 小写化时记录原字符索引 |
| A6 tagfilter 乱码 | ✅ 已修 | 整段 `push_str` |
| A7 `[TODO]` → 数学块 | ✅ 已修 | 仅在数学块真会触发时（`\[` 独占一行或以 `\]` 结尾）写成 `&#91;`；Rust + Dart |
| A8 段落解析立方级 | ✅ 已修 | `extended_autolink` 只看 8 字符前缀；`<` 扫描遇 `<`/空白即停（Rust + Dart）；续行只记原文、扫描结束一次性 inline 解析（Rust）。4000 行段落 release ~15 s → 0.7 ms。括号类恶意输入（`[<`/`[`/`![`/`[^` 成串）改为一次栈扫描记忆化后近线性（Rust `BracketMemo` + Dart 镜像）；`nested_bracket_perf.rs` |
| A9 AI key 串号 | ✅ 已修 | 按 `provider_id` 回落，且已存 key 只发往其保存时的 URL；服务端 + 客户端 |
| B1 改密不吊销 PAT | ✅ 已修 | `revoke_user_sessions` 同事务删 PAT（同 Google app passwords） |
| B2 重置密码强度 | ✅ 已修 | 走 `password_strength::check`，在消耗链接之前 |
| B3 resend 不限流 | ✅ 已修 | |
| B4 存储域同站 XSS | ✅ 已修 | 预签名 PUT 签入 `content-type`；html/svg/xml/js 等可执行类型一律落为 `application/octet-stream`（`stored_content_type`）；生产环境下带 Origin 的 cookie 写请求与三个 WS 入口校验来源（`origin.rs`）。真 RustFS 1.0.0-rc.3 实测：伪造类型 403、html 以下载类型返回 |
| B5 HTML 导出 XSS | ✅ 已修 | `safe_url` 滤 `javascript:`/`vbscript:`/非图片 `data:`；`--!>`/`<!-->` 注释边界；`align`/alert 白名单；导出文件加 CSP meta |
| B8 认证后 Argon2 端点 | ✅ 已修 | `POST /auth/password`、`DELETE /auth/me` 纳入每 IP 限流与 Argon2 并发闸门（按方法匹配，`GET /auth/me` 不受影响） |
| B6 移除成员仍收广播 | ✅ 已修 | 文档 socket 每 30 s 复查，关闭码 4403；bootstrap/pull 复查 |
| B7 配额可绕过 | ✅ 已修 | 未完成的浏览器上传计入配额（migration 0029）；跨工作区移动检查目标配额。新增按 owner 汇总的 `MICA_OWNER_QUOTA_BYTES`（默认 20 GiB，0 = 不限），错误码 `owner_quota_exceeded`；决策见下文 |
| B9 ack 先于广播导致 cursor 越界 | ✅ 已修（客户端） | lag 重同步与重连改发带 sv 的 `sync.bootstrap`（按状态向量补差，与 rid 到达顺序无关）；防抖窗口内的 lag 通知补发一次尾随重同步 |
| B10 expected_seq 看不到 WS 编辑 | ✅ 已修 | `push_update` 同事务推进 `current_seq`；`sync_pg.rs` |
| B11 缺依赖 update 被 ack | ✅ 已修 | y-protocols 式双向 sv 交换：ack 带 `missing` + 服务端 sv，`sync.base` 带服务端 sv 与 `ahead`；客户端回推对该 sv 的 diff（每连接 ≤3 次）。全为加字段，不升协议版本 |
| B12 presence 无限制 | ✅ 已修 | 4 KiB 上限 + 每连接 30/s 令牌桶 |
| B13 块首转义只做首行 | ✅ 已修 | 段落/引用逐行；补 `1)`、制表符、`>x`、空标记；Rust + Dart |
| B14 空待办丢勾选 | ✅ 已修 | 接受裸 `- [ ]`/`- [x]`；Rust + Dart |
| B15 children 成环 abort | ✅ 已修 | 导出前 `export_safe_tree`（去重 + 深度 128 封顶并提升）；`is_descendant` 迭代 |
| B16 GC 与去重竞态 | ✅ 已修 | 去重命中（presign/complete/`insert_file_tx`）清 GC 标记；GC 回收前持对象键锁复查标记 |
| B17 GC 不看命名版本 | ✅ 已修 | 命名版本中的 `file_id` 计入引用集 |
| C1 expires_in_days panic | ✅ 已修 | 限 1..=3650，否则 400 |
| C2 AI 流中文乱码 | ✅ 已修 | 按字节缓冲，只解码完整行 |
| C3 错误响应泄露 | ✅ 已修 | `Database`/`Migration` 只记日志、对外回 `internal database error`（HTTP 与 WS 同口径）；存储请求错误用 `without_url()`，不再回显预签名 URL |
| C4 删账号 | ✅ 已修 | 删除后清理头像对象；改为墓碑（migration 0031 `users.deleted_at`）：自有工作区照删，users 行保留为匿名作者、个人信息全部抹除、邮箱可重新注册，评论按 0014 约定删除；所有"是不是用户"的判断排除墓碑 |
| C7 分页 / 任务属主 / 房间清理 | ✅ 已修 | 分页加 `id` 兜底排序；负 offset 回 400；导入任务查询/取消校验属主；新建房间时清理失效条目 |
| C5 光标停在 emoji 中间 | ✅ 已修（Dart） | 左右键按字素簇移动 |
| C6 Dart/Rust 镜像差异 | ✅ 已修 | Dart `mathRunSpans` 跳过代码段、空链接 mark 不再丢弃；Rust 未开启的 `~` 连续段整段字面量（`tilde_runs.rs`）；Dart 改用与 Rust 相同的 Unicode White_Space 判定 |
| C8 源码 NUL 字节 | ✅ 已修 | 改为 `\u0000` 转义 |
| C9 IPv6 限流可绕过 | ✅ 已修 | IPv6 按 /64 计桶，IPv4 映射地址按 IPv4 |
| C10 登录时序可枚举账号 | ✅ 已修 | 未知邮箱也对一个固定的随机哈希跑一次 Argon2 |
| D2 PAT 请求写放大 | ✅ 已修 | `last_used_at` 一分钟内只写一次；`scope_guard` 把解析结果放进 task-local，`user_id_from_headers` 同 token 直接复用（客户端无法伪造） |
| D3 整工作区导出 | ✅ 已修 | ZIP64（超 4 GiB 不再截断）；全流式：`ZipStreamWriter` + `TreeExport` 逐页产出，服务端边解码边下载边写 `Body::from_stream`，内存 = 一页 + 一张图；输出与旧实现逐字节相同（指纹钉死）；并发上限 2 → 4 |
| D4 GC 解码所有文档 | ✅ 已修 | `document_yrs_base.file_ids` 派生列（migration 0030），所有写入点同语句维护 + 启动回填；GC 读列，NULL 回退解码 |
| D5 杂项 | ✅ 已修 | `reorder` 单条批量 UPDATE；评论列表按线程预分组；S3 往返移出事务：短事务持锁刷新 ledger 过期 + 清 GC 标记 → 无事务 PUT/HEAD → 短事务持锁登记 |
| D1 同步热路径 | ✅ 已修 | 每次 push 只拍平一次；自动版本先查再写，不再每次把整份 state 发给 Postgres |

**验证状态**（2026-10-19，`f0187cf`）：
- Rust 全量 `cargo test --workspace`（连本地 Postgres 16，DB 集成测试真实执行）**778 通过、0 失败**、
  13 忽略；`cargo clippy --workspace --all-targets -D warnings` 干净。
- 需要真对象存储的 `#[ignore]` 测试在本地 RustFS 1.0.0-rc.3 上全过（签名 PUT 防重放 / 防伪造类型、
  html 以下载类型返回、pending GC 竞态、工作区删除 GC），唯一例外 `backlink_scan_cost` 是要生产快照的基准。
- 云端容器装了 Flutter 3.47.1（`.fvmrc` 钉的版本）：`flutter analyze` 无 error/warning，
  `flutter test` **1821 通过**。装上后当场发现并修了两处此前没编译过的 Dart 错误（`marks.dart` 未定义变量、
  `onListAiModels` 签名不匹配）。
- D3 / D5 端到端实测：本地起服务 + RustFS，导入带图工作区（走新的无事务 PUT），下载工作区导出与全部
  工作区导出：chunked 传输、zip 校验通过、两页共用的图只存一份且字节一致。
- 此前出现过一次 `mica-cli` 备份锁测试的并行偶发失败（本审查未改 `crates/cli`），留作单独排查。

## 第四批决策（2026-10-19，先调研同类再拍板）

- **B7 配额按什么算**：AFFiNE 把存储记在工作区 owner 名下（owner 的套餐即工作区额度）；Outline 自托管
  直接不允许多团队。两家都不限工作区"个数"（空工作区不占盘）。→ 保留每工作区配额，**加一层按 owner
  汇总的上限**（默认 20 GiB = 4 个满工作区，约生产最大真实账号 350 倍），成员上传记在 owner 头上。
- **C4 协作者删账号**：Outline 删用户是软删 + 匿名化，内容留在团队里。→ **墓碑**：他人工作区里的页面 /
  上传 / 流记录是那个工作区的共同产出，留给匿名作者；身份与凭据全部抹除；评论按 0014「账号带走评论」
  的既有约定删除。外键改 SET NULL 不取：每个读者都要处理空作者，且 `NOT NULL` 约束要全表改。
- **B4 存储域同站**：Outline 把 content-type / disposition 签进上传；AFFiNE 用黑名单 + attachment + nosniff。
  → 签 content-type + 可执行类型落为 octet-stream（不依赖对象存储是否支持 disposition 覆写），
  再补 WS 的 Origin 校验（WebSocket 不受 CORS 保护，是这条链里真正能读数据的一环）。
- **B11 缺依赖 update**：y-protocols 的 SyncStep1/2、AFFiNE 的 pullAndPush、AppFlowy 的 `MissUpdate`
  都是"双方交换状态向量、各自补对方缺的"。→ 不拒收（旧客户端只会重发同一份），ack / base 里附服务端 sv，
  客户端补推 diff。已知边界：sv 不含删除，"服务端只丢了删除"测不出来；y-protocols 靠每次重连无条件
  发 SyncStep2 覆盖，在这里代价是每次重连一条流记录，不取。
- **D5 事务内等 S3**：原先认为是防 GC 竞态的必要代价。复查两类清扫的删除条件后发现，锁真正保证的只是
  "I/O 期间清扫不碰这个 key"，而清扫删除前都在同一把锁下复核（标记 / ledger 过期）——所以只需在 I/O
  之前用一个短事务让复核必然失败，I/O 本身不必在事务里。
- **D3**：两遍解码（第一遍只取资产名，保证与旧版逐字节一致）换内存有界；导出偶发，CPU 换内存划算。

## 本地发布流程（GitHub Actions 额度用尽时）

`just ci` 本地跑 CI 门禁；`just ship X.Y.Z` 一条龙：发版提交 + tag → 推送 → 本机能构建的产物
（Windows：安装包 + 冒烟 + CLI，有 Docker 时顺带静态 Linux CLI；Linux：静态 CLI）→ 镜像 → 以 draft
上传 → 必备产物齐了转 latest。部署 `deploy-prod.sh` 在本机没有 ansible 时自动改在 `alpine/ansible`
容器里跑（Windows 也可用）。细节见 `docs/ops/release.md`「本地发布」。

## 待办（未做）

- **其余平方级恶意输入**：`<a ` 成串（HTML/autolink 扫描）、`$a ` 成串（行内数学）、长短不一的反引号串，
  约每 100 KB 0.4–1 s。括号类已修；这几类各需一套类似的记忆化，单独做。
- **B11 边界**：仅删除的分歧（见上）。
