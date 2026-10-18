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

| 条目 | 状态 | 提交 |
| --- | --- | --- |
| — | — | — |
