# Mica 代码库优化计划

审查基线：2026-09-29，`main` 的 `b838ebc`。初始审查只读检查了 Rust workspace、Flutter 编辑器与外壳、Web 离线存储、测试、CI、容器与部署配置；未对生产发起破坏性验证。随后按条目实施修复，实施与验证结果写在各项下。仓库知识图谱项目未索引，查询返回 `project not found`，随后按 `AGENTS.md` 回退到源码与测试的定向检索。原始风险描述保留审查时的判断；以每项后续的实测结果与剩余边界为准。

优先级：**P0** 为可能静默丢失持久数据、应先修复；**P1** 为高影响的数据完整性、安全或身份问题；**P2** 为中等影响、可观测性和测试覆盖；**P3** 为清理项。每项的“风险”同时说明当前影响和修复时需要留意的回归面；“预计收益”是定性预期，不是未测量的性能数字。

## P0 — 先保护持久数据

### P0-01 并发 Yrs 写入可互相覆盖 — DONE

- **文件 / 位置**：[sync.rs:341-346](crates/app-core/src/sync.rs#L341-L346)、[sync.rs:496-539](crates/app-core/src/sync.rs#L496-L539)、[store.rs:470-484](crates/app-core/src/store.rs#L470-L484)。
- **问题 / 原因**：`push_update` 在事务中普通 `SELECT` 读取旧 base，离线折叠客户端更新，最后用 `ON CONFLICT DO UPDATE SET state = excluded.state` 写回整份状态。两个并发写者可能从同一旧 base 派生，后写者覆盖先写者；两条 `workspace_updates` 却都已提交。REST 路径锁 `documents` 行，而此路径在读取 base 前不取同一锁，混合写入也存在旧读覆盖窗口。这是高置信静态并发推演，尚需可控时序的数据库复现。
- **推荐修改方式**：在读取 base 前，以统一锁顺序锁住对应 `documents` 行；在锁内完成折叠、stream 写入和 base 更新。增加两连接并发 `push`、REST 与 `push` 交错测试，断言最终 base、`base_rid` 和搜索投影都包含两次编辑。现有 [sync_pg.rs:639](crates/app-core/tests/sync_pg.rs#L639) 只测试首次 bootstrap 并发。
- **风险**：当前可能静默丢失协作编辑；增加锁后须检查其他写路径的锁顺序与吞吐，避免死锁。
- **预计收益**：高；阻断最严重的数据完整性缺口。
- **已实施（2026-09-29）**：`push_update` 在事务开头、读 base **之前**取 `documents` 行锁（`store::lock_document_tx`，由私有改为 `pub(crate)` 并加注释说明两条写路径必须同序取锁）。锁序与 REST 路径一致（先 `documents` 后 `document_yrs_base`），因此二者互相串行而不会死锁。折叠、stream 插入、base 更新、`views.name` 投影、版本快照与修剪全部落在锁内同一事务。
- **回归测试**：`sync_pg.rs` 新增两个，均用**可控交错**（外层事务持锁 → 派生任务阻塞在锁上 → 提交释放）而非随机压力：
  - `concurrent_pushes_both_reach_the_base`：两连接并发 push。断言最终 base 同时含 `AAA`/`BBB`、`base_rid` 指向最后一次折叠、`content_text` 同步、**且从 base 起把整个 stream 折叠一遍必须重现存储的 base**（最后一条是流对追赶客户端的承诺，正是被丢更新违反的性质）。
  - `op_write_and_push_interleave_without_losing_either`：REST/MCP 写与 WS push 交错，断言两条路径的编辑都在最终 base 里。
  - **有效性已实测**：临时移除锁后 `concurrent_pushes_both_reach_the_base` 失败，错误信息 `both pushes must survive in the base, got "HelloBBB"` —— A 的编辑静默丢失，即本项描述的丢更新；恢复修复后 24 项全过（23 passed / 1 ignored）。

## P1 — 高影响缺陷与安全边界

### P1-01 空实例的并发首账号注册可产生多个管理员 — DONE

- **文件 / 位置**：[auth.rs:149-172](crates/api-server/src/routes/auth.rs#L149-L172)、[0001_initial.sql:3-10](migrations/0001_initial.sql#L3-L10)、[0021_ai_settings_and_admin.sql:25](migrations/0021_ai_settings_and_admin.sql#L25)。
- **问题 / 原因**：首账号特殊权限依赖 `INSERT … WHERE NOT EXISTS(users)`；PostgreSQL 默认 `READ COMMITTED` 下，两个并发注册语句可都看到空表并插入不同邮箱，数据库没有“仅一个首管理员”约束。
- **推荐修改方式**：用数据库串行化锁或原子 bootstrap 状态约束首账号创建；增加空库双请求并发测试，同时验证注册默认关闭时第二个请求被拒。
- **风险**：初装短窗口内可能意外授予第二个已验证管理员；修改首启流程需保留合法首账号体验。
- **预计收益**：高；首启管理员归属不再依赖请求时序。
- **已实施（2026-09-29）**：注册插入抽成 `insert_registered_user(&mut tx, …)`，事务内先取**事务级** `pg_advisory_xact_lock(REGISTRATION_LOCK_KEY)` 再执行原语句。选 advisory lock 而非唯一索引：这两个竞争者用的是**不同邮箱**，`users_email_key` 永远不会触发；而「至多一个 admin」在 schema 上无法表达 —— 角色按设计是要能移交的（migration 0021 解释了为什么用列而不是名字）。用事务级而非会话级是为了 commit/rollback 自动释放，没有忘记解锁的错误路径。
- **对原始描述的修正**：审查写的是「第二个请求被拒」，这只对**注册关闭**时成立。开放注册时 `WHERE $4 OR NOT EXISTS (…)` 因 `$4 = true` 恒真，两个账号都创建是**合法**的；真正被破坏的不变量是 `is_admin = NOT EXISTS (SELECT 1 FROM users)` —— 同一个语句里的两个 `NOT EXISTS` 各自读语句快照，两个请求都算出 `true`，于是**两个管理员**。测试两半都断言了。
- **回归测试**：`concurrent_first_registrations_cannot_both_become_admin`（`routes/auth.rs` 的 `refresh_pg` 模块），抛库 `CREATE DATABASE` 建空实例，用**可控交错**：A 取锁插入但不提交 → B 调**同一个生产函数**并被锁挡住 → 观察 `pg_locks` 确认阻塞 → 提交 A。断言开放时恰好一个 admin、关闭时恰好一个账号。
  - **有效性已实测**：临时移除 `pg_advisory_xact_lock` 后测试失败，`left: 2, right: 1` —— 两个管理员，正是本项缺陷；恢复后 api-server 全套 209 项通过。
  - **测试自身的两处返工已修正**：(a) 初版断言「第二个请求被拒」，实际失败原因是该断言写错了（见上）；(b) 屏障断言 `assert!(blocked)` 让失败落在「没有串行点」而非「两个管理员」上，改为有界等待 + 观测，由真实不变量判定。

### P1-02 预签名上传未绑定真实内容与大小 — 已缓解，PUT 前大小上限仍待收紧

- **文件 / 位置**：[files.rs:74-132](crates/api-server/src/routes/files.rs#L74-L132)、[storage.rs:365-388](crates/infra/src/storage.rs#L365-L388)、[store.rs:676-702](crates/app-core/src/store.rs#L676-L702)。
- **问题 / 原因**：服务端依据客户端声明的 hash、字节数签发 object key；签名使用 `UNSIGNED-PAYLOAD`，只签 `host`，`complete` 也不读取对象校验。持有该工作区编辑权限者可用已知 key 写入不同字节覆盖既有附件，或 PUT 超大对象而不 complete，绕过元数据配额并留下难以回收的孤儿对象。攻击链基于静态代码，尚未对对象存储做破坏性复现。
- **推荐修改方式**：防止覆盖已存在 key，约束实际上传长度与内容摘要，complete 前校验对象存在、实际大小及摘要；以真实 S3 兼容存储做恶意大小、重复 key、缺失对象的集成测试。
- **风险**：当前附件完整性与存储额度可能被工作区编辑者破坏；修复会改变客户端上传协议和去重语义，须兼容现存对象。
- **预计收益**：高；让配额与内容寻址具有实际约束力。
- **已实施（2026-09-29/30）**：
  1. **`complete` 不再相信客户端**。新增 `S3Config::presign_head_object`（服务端侧 HEAD），`complete` 在建行前 HEAD 一次并把结果交给纯函数 `uploaded_object_verdict(status, stored_len, declared_len)`：404 → 拒（对象从未上传）；非 2xx → Internal；**store 没报长度 → 拒（fail closed）**，不能把「无法验证」当成「已验证」；`stored != declared` → 拒。记账用的是 **store 报的实际长度**，不是客户端声明的数。
  2. **已存在的 key 不再签发新的上传 URL**。命中 `store::fetch_file_by_key` 时返回 `existing`（现有行 + 下载 URL）且**不含** `upload` 字段，挡住登记后再次向 API 索取 URL 的路径。
  3. **新签发的 PUT 同时绑定不可覆盖条件和真实 SHA-256**。`S3Config::presign_put_if_absent` 把 `If-None-Match: *` 与 `x-amz-checksum-sha256` 一起放进 SigV4 的签名头；客户端必须带这两个头，不能自行删除或调包。RustFS 在写入时拒绝同 key 覆盖，并按 checksum 校验实际字节。`UNSIGNED-PAYLOAD` 仍用于预签名流程，但已不能绕过这两个已签名条件。
  4. **协议变更**：`PresignResponse` 由扁平的 `upload_url/method/expires_in/max_byte_size` 改为 `upload: Option<PresignUpload>` + `existing: Option<FileResponse>`，两者互斥且用 `skip_serializing_if` **省略**而非填 null；`upload` 还携带必须发送的 `if_none_match` 与 `checksum_sha256`。已同步 Dart 客户端 [`client.dart`](clients/mica_flutter/lib/api/client.dart) 的 `uploadImage` 与集成测试里的镜像实现。旧客户端与新服务端的上传协议不兼容，发版前需明确升级路径。
- **回归测试**：
  - 单测 `complete_requires_the_store_to_confirm_the_upload`（404 / 5xx / 尺寸两个方向 / **无长度 fail closed** / 0 字节是合法尺寸）。
  - 单测 `an_existing_object_yields_no_upload_url`（无 `upload`、返回现有行、序列化后 `upload` **缺席而非 null**）。测试注释里如实写明了它的局限：它证明不了「查询确实执行了」，所以补了下一条。
  - **真栈端到端实测（2026-09-30）**：对着 dev 栈（api + 真 rustfs）跑完整 presign → PUT → complete。实测结论： (a) rustfs 的 HEAD **确实返回 Content-Length**，因此 `complete` 的校验不是纸面推断； (b) 谎报尺寸被拒，`{"code":"bad_request","message":"...uploaded object is 64 bytes but 9999 was declared"}`； (c) 同一 hash 二次 presign 返回 `has upload field: False` / `has existing: True`，且 `existing.file.id` 与首次 complete 的行一致。探针文件已清理。
- **有效性已实测（判别力）**：把尺寸校验弱化成「相信客户端声明」后 `complete_requires_the_store_to_confirm_the_upload` 立刻失败；把 `skip_serializing_if` 去掉后 `an_existing_object_yields_no_upload_url` 失败在 `"upload":null`（正是客户端无法区分的那两种情形）。恢复后 api-server 全套 214 项通过。
- **追加回归与实测（2026-09-30）**：`hex_sha256_becomes_the_s3_checksum_for_the_same_bytes` 固定了 hex→base64 摘要转换，`browser_write_once_url_signs_the_precondition` 固定签名头；针对本机实际运行的 RustFS rc.3，显式执行 `signed_conditional_put_refuses_replay`：漏签名头得 403、内容与 checksum 不符得 400、首次 PUT 成功、同一 URL 重放得 412，GET 仍是首次写入字节。浏览器预检也实测允许 `content-type,if-none-match,x-amz-checksum-sha256`。依据 [AWS 条件写入文档](https://docs.aws.amazon.com/AmazonS3/latest/userguide/conditional-writes.html)与本机真栈结果，不把 AWS 语义未经检验地推定给 RustFS。
- **完整登记与断线恢复**：`complete` 用要求 checksum 的 HEAD 对照对象 key 中的 SHA-256；没有登记行的新对象若缺 checksum 或摘要不符就拒绝。Dart 客户端遇到首次 PUT 已成功、重试得 412 时仍调用 `complete`，由服务端复核对象后完成登记。已有 `files` 行的重试先复核对象存在和长度，再幂等返回；旧对象没有 checksum 元数据时不阻断读取。服务端自行 PUT 也发送 SHA-256 checksum，避免重写同 key 时抹掉这项元数据。以上流程已用 RustFS 的 PUT/HEAD/replay 和客户端 412 回归测试覆盖。
- **存量边界**：修复前已签发的旧 URL 不含写入条件，在其 TTL 到期前仍可覆盖旧对象；新服务端无法撤销对象存储已发出的旧 URL。上线后须等最长 TTL 窗口结束（当前开发配置为 7 天），才可认为全部客户端上传 URL 都受新规则约束。
- **仍有大小缺口**：签名 SHA-256 证明对象字节与客户端申报的 hash 一致，却不能证明它与客户端申报的 `byte_size` 一致。编辑者可以先计算超大文件的真实 hash，向 presign 谎报很小的 `byte_size`，再在直连对象存储的 PUT 中上传大文件；`complete` 会拒绝登记，P2-12 最终会清理，但宽限期内的存储占用没有被预先限制。因此不能把“完成登记时严格校验”表述成“对象存储层已限制 PUT 大小”。后续需实测并采用存储端可执行的长度约束（如支持 `content-length-range` 的上传策略或等效网关），保留浏览器直传与实际尺寸回归。
- **孤儿对象**：新签发 URL 与服务端写入路径已由 P2-12 的台账跟踪；升级前从未登记的历史对象不在台账内，仍需一次性核对清理。

### P1-03 外部图片导入的 DNS 检查与实际连接脱节 — DONE

- **文件 / 位置**：[files.rs:238-270](crates/api-server/src/routes/files.rs#L238-L270)。
- **问题 / 原因**：先解析并过滤私网 IP，之后 `reqwest` 再次解析域名；源码注释明确承认 DNS rebinding 可在两次解析之间切换目标。禁止重定向不能堵住这一条路径。
- **推荐修改方式**：让实际连接固定使用已验证的公网 IP，同时维持原始 Host/TLS 主机名；覆盖解析变化、IPv4/IPv6、重定向测试。
- **风险**：当前可被用于请求内网或云元数据地址；修复涉及 DNS 与 TLS 的结合，必须防止错误拒绝正常 CDN。
- **预计收益**：高；关闭明确存在的 SSRF 绕过窗口。
- **已实施（2026-09-29）**：审查结果通过 `reqwest::ClientBuilder::resolve_to_addrs(&host, &pinned)` **钉在客户端上**，连接只能落到已检查过的那批地址，要么成功要么失败，不再有第二次解析可供调换。Host/SNI 仍由 reqwest 从 URL 取，不受影响 —— 这正是让钉住对 CDN 可用、而不是把 CDN 一起拒掉的原因。端口用 URL 的 `port_or_known_default()`（原来是硬编码 80/443 的 match），因此显式 `:8443` 也能正确钉住。
- **回归测试**：两个，都在 `routes/files.rs` 的 `tests` 模块。
  - `ssrf_vetting_refuses_private_and_metadata_targets`：URL 级，覆盖 loopback / 10.8 / 192.168 / 172.16 / **169.254.169.254 云元数据** / CGNAT / 0.0.0.0 / IPv6 loopback / ULA / link-local / IPv4-mapped。
  - `ssrf_vetting_refuses_a_host_with_any_private_answer`：**混合地址表**，即「同一域名同时解析出公网与私网」这一条。这是判别的关键 —— 见下。
  - **有效性已实测（含一次失败的自省）**：先把规则弱化成 `all` 再跑，**URL 级那个测试照样通过**（IP 字面量只解析出一个地址，`any`/`all` 在单元素上等价）—— 也就是说，只写 URL 级用例时，这个测试名字虽叫 rebinding，实际根本没覆盖 rebinding 形状。为此把规则抽成接收地址表的纯函数 `addresses_are_safe(&[SocketAddr])`，混合用例直接喂列表；再弱化成 `all` 时，混合用例失败（`a host with a private answer must be refused even when it also answers publicly`），URL 级仍通过。恢复后 api-server 全套 212 项通过。

### P1-04 外部图片响应在限额检查前整体进入内存 — DONE

- **文件 / 位置**：[files.rs:282-302](crates/api-server/src/routes/files.rs#L282-L302)。
- **问题 / 原因**：`response.bytes().await` 将完整远端响应装入内存，之后才调用 `ensure_storable`。远端持续返回大数据时，API 在判定超限前已承担内存消耗。
- **推荐修改方式**：流式读取并在超过 `max_upload_bytes` 时停止；`Content-Length` 可作提前拒绝，但不能替代实际流量计数。
- **风险**：当前恶意或异常图片源可推高 API 内存；修改流式处理需要保留 MIME、hash 和错误映射行为。
- **预计收益**：高；单请求内存有明确上界。
- **已实施（2026-09-29）**：改为 `response.bytes_stream()` 逐块读取，每块过 `append_within_cap`，**累计超限即刻拒绝**，缓冲区不会再增长（越界的那一块根本不会 append）。另加一道廉价的提前拒绝：`Content-Length` 已声明且超限时直接拒 —— 但它**不是**执行点，声明值来自远端、可缺失可撒谎，真正的约束是实际字节数。MIME 判定、hash、`ensure_storable` 与错误映射都保持原样。
- **回归测试**：`streamed_chunks_are_capped_before_they_are_buffered`（`routes/files.rs` 的 `tests` 模块）。覆盖：未超限保留、**恰好等于上限放行**（边界是 `> cap` 拒绝而非 `>= cap`）、越界一块被拒**且缓冲区长度不变**（证明不是先塞后查）、多块小 chunk 累加越界（chunked 响应没有可用的 `Content-Length`）、空块无害。
  - **有效性已实测**：把 `>` 改成 `>=` 后测试立刻失败（恰好等于上限那一档被误拒）；恢复后 api-server 全套 212 项通过。
  - 为使其可测，限长逻辑抽成纯函数 `append_within_cap(body, chunk, max_bytes)` —— 原先是埋在需要 `AppState` + 网络的 handler 内联代码，测不到。

### P1-05 已连接文档 WebSocket 不实时执行撤权 — DONE

- **文件 / 位置**：[ws.rs:98-112](crates/api-server/src/routes/ws.rs#L98-L112)、[ws.rs:351-451](crates/api-server/src/routes/ws.rs#L351-L451)、[ws.rs:684-701](crates/api-server/src/routes/ws.rs#L684-L701)。
- **问题 / 原因**：连接升级时检查工作区角色，之后沿连接复用缓存的 `permissions`。成员被移除或降为只读后，原连接仍可发送 `sync.push`；JWT 默认有效期内不会自然触发角色重查。
- **推荐修改方式**：每次写入前重新校验成员权限，或在成员权限变更时主动关闭对应连接；补“连接建立→撤权→继续推送”的测试。
- **风险**：当前撤权不能即时阻止写入；逐次查询会增加数据库读取，主动踢连接则增加房间生命周期逻辑。
- **预计收益**：高；权限变更按用户预期立即生效。
- **已实施（2026-09-30）**：新增 `current_write_permissions`，两个写分支（`document.update` 与 `sync.push`）在**写前**各自重查角色：`Ok(None)`（已不是成员）与「是成员但只读」给出**不同**的错误文案，便于客户端区分。选了「写前重查」而非「主动踢连接」：后者要把房间生命周期接到成员变更的所有入口（成员管理、工作区删除、角色调整），漏一处就是同一个 bug 换个形状；重查是一次走 `(workspace_id, user_id)` 主键的索引查询，而客户端写是 **400ms debounce** 的批量推送、不是逐字符，所以不落在按键热路径上。`AuthResponse`/bootstrap 里发给客户端的 `permissions` 仍按升级时计算 —— 它只是**告知**，不再被服务端用作授权依据。
- **顺手加固**：把 `handle_client_message` 的 `permissions: DocumentPermissions` 参数**整个删掉**（而不是加下划线保留）。留着它就是把「不可信的快照」摆在手边，下次有人顺手再读一次就复现同一 bug；现在是编译期保证 —— 该函数拿不到缓存值，只能走重查。
- **回归测试**：`ws.rs` 新增 `revoked_pg` 模块，两个数据库门控用例：`a_demoted_member_loses_write_on_the_next_check`（editor → viewer：重查必须看到降级；再删除成员关系 → 读为 `None` 而非陈旧角色）与 `a_promoted_member_gains_write_on_the_next_check`（viewer → editor，方向相反，防止「查询坏成永远拒绝」也能通过）。
  - **有效性已实测**：把重查弱化成「返回升级时缓存的 editor」后，两个用例**双双失败**（`the re-check must see the demotion; a cached value would still allow the write`）；恢复后 api-server 全套 216 项通过。
  - 测试写明了它测的是**规则**而非帧处理器：后者需要构造完整 `AppState`（config / hub / S3 / mailer），那样测的是脚手架。调用点由类型系统保证 —— 帧处理器已经拿不到权限快照了。

### P1-06 编辑器旧 flush 完成会清除新输入的脏标记 — DONE

- **文件 / 位置**：[controller.dart:499-524](clients/mica_flutter/lib/editor/controller.dart#L499-L524)。
- **问题 / 原因**：`_dirty` 只按 block ID 记录。A 批输入开始异步提交后，同块又输入 B；A 的 `whenComplete` 无条件 `removeAll(ids)`，可清掉 B 的标记，后续 debounce 看见空集合而跳过 B。
- **推荐修改方式**：为每块记录修改代次，只清除本批已成功提交且代次未变化的脏状态；以可控 `Completer` 覆盖 A/B 交错。
- **风险**：当前可丢失未持久化的新输入，随后被远端 reconcile 覆盖；改动需维护输入、flush 和切页时的代次一致性。
- **预计收益**：高；防止快速编辑时的静默丢字。
- **已实施（2026-09-29）**：`_dirty` 由 `Set<String>` 改为 `Map<String, int>`（block id → 该块最后一次被标记时的 `_editGen` 读数）。`flushPending`/`_sendNow` 发送前快照本批的 id→代次，完成后由 `_clearCommitted(batch)` 只清「代次未变」的项；往返期间又被编辑的块保留脏标记，下一次 debounce 会发出新文本。`_markDirty` 每块自增代次。顺带把 `flushPending` 里 `ops.isEmpty` 分支的 `_dirty.clear()` 也改成只清本批 —— 否则同一竞态会从另一条路径复现。
- **回归测试**：[editor_pending_commit_test.dart](clients/mica_flutter/test/editor_pending_commit_test.dart) 的 `an edit made while a batch is in flight is still sent afterwards`（用 `Completer` 闸住 A 批，A 在飞时输入 B，断言 B 最终落盘），外加反向用例 `a batch that carries no NEW edit does not hold back a clean block`（未被触碰的块仍要正常清脏，否则每次 flush 都会重发）。
- **有效性已实测**：把 `_clearCommitted` 还原为无条件删除后，测试失败 `Expected: 'AB' / Actual: 'A'` —— 新输入被旧 flush 的完成清掉；恢复修复后通过。

### P1-07 编辑器提交失败仍会清掉待保存内容 — DONE

- **文件 / 位置**：[controller.dart:521-524](clients/mica_flutter/lib/editor/controller.dart#L521-L524)、[controller.dart:3288-3300](clients/mica_flutter/lib/editor/controller.dart#L3288-L3300)、[editor_op_fault_test.dart:19-41](clients/mica_flutter/test/editor_op_fault_test.dart#L19-L41)。
- **问题 / 原因**：`_send` 将 `onOps` 错误转为完成的 Future；`flushPending` 的 `whenComplete` 不区分结果即清脏。已有测试只数错误回调，不验证失败后的本地内容是否还可提交。
- **推荐修改方式**：保留明确的提交成功/失败结果；失败时保留待提交修改、提供重试，并在切页/销毁前阻止静默丢弃；增加故障注入测试。
- **风险**：当前 outbox/存储写失败后可能丢失输入；修复需避免无限重试和重复操作。
- **预计收益**：高；让错误处理真正保护数据耐久性。
- **已实施（2026-09-29）**：`_send` 的签名由 `Future<void>` 改为 **`Future<bool>`**（成功 `true`／失败 `false`，仍不重抛，否则会打断编辑热路径），`flushPending` 同样返回 `Future<bool>`。失败时**保留全部脏标记**并通过 `_scheduleRetry()` 自行重排一次 flush（退避 `400ms × 2^n`，上限 5 次 ≈ 12 秒）—— 上限是刻意的：后端真的挂了不该被无限重试，且 `onOpFault` 已经报告过。原先失败后什么都不排，用户停止输入（或切页）时那段编辑就永远留在内存里。
- **回归测试**：[editor_pending_commit_test.dart](clients/mica_flutter/test/editor_pending_commit_test.dart) 的 `a failed commit keeps the edit pending and re-sends it without new input`。判据是**重试尝试次数**（`rec.batches.length` 增加），不是「最后发出的文本」—— 失败那次本身也携带了那个文本。
  - **有效性已实测**：还原为「无条件清脏、不排重试」后测试失败，5 秒内尝试次数停在 1（永不重试）；恢复修复后通过。
  - **测试自身的两次返工已修正**：(a) 初版用「手工再调一次 `flushPending`」证明，那恰好替旧代码掩盖了缺陷 —— 换成不碰文档、只等自动重试；(b) 第二版断言「重试批次 ≠ 首批」是错的，重试本就应重放同一批 op（服务端幂等）。

### P1-08 立即结构操作冲刷 dirty 文本时遗漏格式数据 — DONE

- **文件 / 位置**：[controller.dart:266-310](clients/mica_flutter/lib/editor/controller.dart#L266-L310)、[controller.dart:3266-3285](clients/mica_flutter/lib/editor/controller.dart#L3266-L3285)。
- **问题 / 原因**：文字编辑会更新 `data.marks` 偏移，但 `_sendNow` 为其他脏块生成的 `update_block` 只含 `text`，然后清脏；与正常 `flushPending` 同时提交 `text`、`data` 的做法不一致。带链接/粗体的文本输入后，400ms 内触发另一块的结构操作即可留下错位 mark。
- **推荐修改方式**：立即冲刷也提交对应 `data`；增加“编辑带 mark 的块→立即 Enter/插块→重载”的回归测试。
- **风险**：当前可能持久化错误的格式或链接范围；改动应防止重复发送覆盖新的远端格式更新。
- **预计收益**：高；结构操作前完整保存文本与格式。
- **已实施（2026-09-29）**：`_sendNow` 为其他脏块生成的补丁 op 补上 `'data': node.data`，与 `flushPending` 一致。判据很直接：marks 是**描述该文本的偏移**，只发其中之一永远是错的（新文本 + 服务端旧 marks = 链接/粗体指向别的字符，且会被持久化）。同时 `_sendNow` 改用与 debounce 路径相同的「快照本批代次 + 成功后 `_clearCommitted`」规则，而不是无条件 `_dirty.clear()`；并且仍然合成**一个** `_send` 批次（pending 文本在前、结构操作在后）—— 那句「先冲刷文本以保证顺序」的保证就靠这一点。
- **回归测试**：[editor_pending_commit_test.dart](clients/mica_flutter/test/editor_pending_commit_test.dart) 的 `an immediate structural send carries the dirty block's marks, not just its text`：编辑带粗体的块（marks 偏移随新文本重算）→ 在 debounce 窗口内对**另一块**做 `splitAtCaret()`（走 `_sendNow`）。
  - **有效性已实测**：去掉 `data` 后测试失败（该块的 marks 为 `null`，即发出去的是纯文本）；恢复后通过。另外用一次性探针确认过成功路径发出的确实是 `data: {marks: [{start: 0, end: 5, type: bold}]}` —— 偏移已随 `Xbold` 正确重算。

### P1-09 reconcile 保留脏文本却覆盖同一块的本地格式 — DONE

- **文件 / 位置**：[controller.dart:92-120](clients/mica_flutter/lib/editor/controller.dart#L92-L120)、[cloud_reconcile_test.dart:36-54](clients/mica_flutter/test/cloud_reconcile_test.dart#L36-L54)。
- **问题 / 原因**：本地块为 dirty 时，`reconcile()` 保留 `cur.text`，但无条件取远端 `src.data`。文字编辑已调整的 marks 可被旧快照的偏移覆盖，稍后再把“新文本+旧 marks”提交。现有测试仅覆盖无格式文本。
- **推荐修改方式**：dirty 时把文本与对应格式数据视为同一版本；明确远端非格式字段合并规则，补粗体、链接和远端旧快照交错测试。
- **风险**：当前格式/链接可错位；过度保护整个 `data` 也可能吞掉远端属性变更，合并粒度须先定清楚。
- **预计收益**：高；保住富文本编辑的一致性。
- **已实施（2026-09-29）**：dirty 块走「本地版本」——本地 `text` 与本地 `marks` 一起保留。做法是覆盖 `data` 前先取出**本地** `data['marks']`，合入远端非格式字段后再把本地 marks 放回（本地本来没有 marks 就 `remove('marks')`，不能用远端的补上）。粒度是刻意的：只保护 marks，远端其他属性变更（如 kind、level）仍然合并，不会因为一个块的文字还在飞就整份 `data` 都不采纳。
- **回归测试**：[editor_pending_commit_test.dart](clients/mica_flutter/test/editor_pending_commit_test.dart) 的 `reconcile keeps a dirty block's LOCAL marks alongside its local text`：本地把 `hi` 改成 `hi!`（marks 随之重算）后，收到携带**旧文本 + 旧偏移**的服务端快照。
  - **有效性已实测**：还原为「无条件取远端 `data`」后测试失败，`Expected: contains 'start: 0' / Actual: '[{start: 1, end: 2, type: bold}]'` —— 正是错位 mark；恢复后通过。
- **附带清理**：初版实现里有一行 `cur.text = cur.text;` 自赋值（无意义，仅注释用），已改为注释说明。

### P1-10 旧账号的异步会话刷新可在退出后复活 — DONE

- **文件 / 位置**：[main.dart:1054-1063](clients/mica_flutter/lib/main.dart#L1054-L1063)、[session_refresher.dart:43-50](clients/mica_flutter/lib/api/session_refresher.dart#L43-L50)。
- **问题 / 原因**：刷新发起后若用户退出、切换服务器或登录另一账号，完成时只检查 `mounted` 就覆盖 `_session`；`_persistSession` 按当前 `_cloudOrigin` 保存旧凭据。全局单飞刷新器还可能让新会话复用旧会话的结果。
- **推荐修改方式**：刷新结果绑定发起时的用户、服务器、refresh token 与会话代次；身份变化时废弃旧结果，并用受控 Future 测试退出/切服交错。
- **风险**：当前可能退出后重新登录旧账号或混淆不同服务器身份；改动须避免使合法的并发 401 刷新重复执行。
- **预计收益**：高；恢复认证状态边界。
- **已实施（2026-09-30）**，两半：
  1. **异步结果绑定「发起时的身份」**（`main.dart`）。新增两个指纹：`_sessionIdentity` = `origin|user.id|refreshToken`，`_accountIdentity` = `origin|user.id`。`_ensureFreshSession` 用严格的那个（refresh token 轮换即视为换了会话），`_refreshProfile` 用宽松的那个。**刻意分成两个**：profile 回答的是「这是谁」，refresh token 在底下轮换不该让一个合法的头像更新作废 —— 用严格指纹会在慢网络（刷新与轮询重叠最多的地方）静默退化成「头像不再刷新」。另外 401 分支也加了同一道守卫：针对**用户已经离开的那个**会话的拒绝，不该结束他现在正用的会话。选指纹而非代次计数器：计数器要在每处赋值点自增，第一个被忘掉的地方就又是同一个 bug。
  2. **刷新器的单飞闩按 refresh token 分键**（`session_refresher.dart`）。原来是一个单字段，跨该 App 的所有登录共享 —— 为账号 A 发起的刷新会被 B 拿走，两者只靠时序相关。
- **回归测试**：`auth_refresh_test.dart` 新增 `two different sign-ins do not share a refresh`（两个会话的刷新用 `Completer` 闸成真正并发，必须各自发起、各自拿到自己 token 换来的会话）与反向的 `the same sign-in still shares one refresh`（同一会话仍只花一次，防止前一条是靠破坏规则 2 换来的）。
  - **有效性已实测**：把闩退回「取第一个条目」后，前一条失败 `Expected: Set:['rt_A', 'rt_B'] / Actual: Set:['rt_A']` —— B 根本没发起请求，拿的是 A 的结果；恢复后 22 项全过。
  - **修复过程中我自己引入并修掉的一个 bug（如实记录）**：分键的第一版写成 `final f = refresh(key).whenComplete(remove); _inFlight[key] = f;` —— `refresh` 若**同步完成**，`whenComplete` 会在赋值**之前**先跑（删掉尚不存在的条目），随后把**已完成**的 future 存进去，再没有任何回调清理它。于是下一次调用拿到那个陈旧 future —— 正好是本项要修的 bug 换了个形状。**实测表现是测试直接挂死 30 秒**（不是断言失败），改用先建 `Completer` 入表、再挂回调的顺序后消除。这条值得记住：修竞态时，回调与赋值的先后本身就是竞态。

### P1-11 旧目录响应可覆盖新树并使 ETag 锁定陈旧内容 — DONE

- **文件 / 位置**：[main.dart:6219-6229](clients/mica_flutter/lib/main.dart#L6219-L6229)、[main.dart:6245-6277](clients/mica_flutter/lib/main.dart#L6245-L6277)、[main.dart:6295-6315](clients/mica_flutter/lib/main.dart#L6295-L6315)。
- **问题 / 原因**：目录请求收到 200 后先存 ETag，等待正文 bootstrap 才提交树；期间另一轮目录刷新可提交新树和新 ETag，旧请求晚到后再覆盖树与离线镜像。后续 304 可能使陈旧树长期保留。
- **推荐修改方式**：按工作区使用请求代次/服务端版本，对树、ETag 和镜像实行一次性提交；加入响应乱序与 304 后续请求测试。
- **风险**：当前目录可回退、跨重启保留陈旧状态；修复需兼顾离线镜像与树事件的顺序。
- **预计收益**：高；避免列表与缓存互相矛盾。
- **已实施（2026-09-30）**：
  1. **按工作区的请求代次**（新增 [`tree_request_seq.dart`](clients/mica_flutter/lib/api/tree_request_seq.dart)）：两条取树路径（后台铃声 `_refreshTreeFromServer`、工作区加载 `_loadSelectedWorkspaceViews`）在发请求前 `claim`，响应回来后 `isCurrent` 不成立就直接丢弃。**「最后到的响应」并不等于「最新的响应」** —— 旧请求可能后到，而它带着**旧的 ETag**；ETag 会被持久化，于是损害跨重启：下次冷启动发这个 ETag、拿到 304、继续用一个服务端已经走过去的树（症状是「树看着完全正常，但某个页面显示不存在的红字」）。
  2. **树与 ETag 一次性提交**：新增 `_commitTree(workspaceId, views, etag)`，把「写 ETag」和「写树」合成一步。它保的那条不变量写在代码注释里 —— **存了 ETag 就意味着镜像里已经有那棵树**；两个调用点原来各写各的，正是「A 响应的树配上 B 响应的 ETag」的来源。304 不带 ETag 时不推进标签（旧标签仍然正确地描述着手上这棵树）。
  3. **顺带**：加载路径里那个 `setState` 原本重复写了一遍树（`_commitTree` 已写），已收窄为只管选中项与正文。
- **`TreeRequestSeq` 性质**：**按工作区隔离**——取 B 的树不能作废 A 的在途请求（它们各自都是自己那个问题的正确答案；用全局计数器会无故丢掉一个）。
- **回归测试**：[`tree_request_seq_test.dart`](clients/mica_flutter/test/tree_request_seq_test.dart) 5 条：新请求开始后旧请求不再当前、**工作区之间不互相作废**、多次重叠只认最新、**未碰过的工作区首次 claim 必须算当前**（差一错误会让首次加载静默什么都不做）、`forget`/`clear` 之后编号继续增长（被遗忘的旧 claim 不会因遗忘而重新变成当前）。
  - **有效性已实测**：把 `isCurrent` 弱化成「总是返回 true」（即旧代码的 last-writer-wins）后，两条用例失败（`Expected: false / Actual: <true>`）；恢复后 5 项全过。
  - 测试写明它测的是**规则**：把「两个响应乱序到达」对着真实的 HTTP 客户端与 socket 摆出来是可行的，但「指望这个顺序在生产环境里恰好发生」正是被修掉的东西。

### P1-12 Web IndexedDB 写锁早于写队列完成释放 — 已实施，未实测（环境受限）

- **文件 / 位置**：[web_idb_doc_store.dart:359-390](clients/mica_flutter/lib/cloud/web_idb_doc_store.dart#L359-L390)、[web_idb_doc_store.dart:455-464](clients/mica_flutter/lib/cloud/web_idb_doc_store.dart#L455-L464)、[web_idb_doc_store.dart:561-569](clients/mica_flutter/lib/cloud/web_idb_doc_store.dart#L561-L569)、[web_idb_doc_store_test.dart:46-55](clients/mica_flutter/test/web_idb_doc_store_test.dart#L46-L55)。
- **问题 / 原因**：`dispose()` 先释放单写者 Web Lock，之后才等 `_tail` 队列完成；新实例可抢锁、读到旧 outbox，再以整份 rows 覆写仍在提交的旧实例。现有重开测试总先 flush，避开这个交接时序。
- **推荐修改方式**：在队列 settled 后才释放锁；加入延迟 IndexedDB 事务的跨实例交接测试。
- **风险**：当前 Web 离线编辑可能在切页/重开时被覆盖；修复会延长锁占用，需处理失败队列的释放兜底。
- **预计收益**：高；保护离线 outbox 的单写者语义。
- **已实施（2026-09-29）**：`dispose()` 改为在 `_tail.whenComplete` 里**先放锁、再关连接**，即等写队列 settled 后才交接。同时重写了那段「立刻放锁」的理由 —— 原文说这样能让同 Tab 的会话重建（B3 sync-retry）马上重新拿到可写镜像，而那个取舍正是缺陷本身：后继者在窗口内 `_hydrate` 读到前任**尚未写完**的 outbox 行，再用自己那份短内存副本整份重写 `rows`，未推送的离线编辑就此消失（正是单写者锁存在的理由）。代价有界：tail 只是若干个 IndexedDB 事务，重建等的是毫秒级。失败链也会释放 —— `whenComplete` 出错也执行，且 `_mirror` 已把写失败吞进 `_broken`，不会卡住交接。
- **回归测试**：`web_idb_doc_store_test.dart` 新增 `P1-12: a disposed store hands over only AFTER its queued writes land`。刻意**不复用** `reopen()`：那个辅助先 `flush()` 再 `dispose()`，恰好绕开要检的时序；这里改为连排 40 条 outbox 后**不 flush 直接 dispose**，再竞速重开，断言后继者看到全部 40 条。
- **⚠️ 未实测，如实记录**：本机 `flutter test --platform chrome` 无法运行 —— 表现为测试套件加载后 Chromium 启动、随后长时间挂起（日志实测 `[+372773 ms] Shutting down Chromium`，是我自己 timeout 杀掉的；末尾的 `The Dart compiler exited unexpectedly` 是被杀之后的收尾噪音，不是编译错误）。**已确认与本次改动无关**：`git stash` 掉 P1-12 改动后跑**基线**，同样挂起；另用一个不 import 任何应用代码的平凡浏览器测试作探针，也是同样症状 → 环境问题（本机 Chrome/浏览器测试链路），不是回归。代码本身 `dart analyze` 干净。**待有可用浏览器环境时补跑该测试。**
- **顺带排查记录**：`build/4d8a75823b0a70673c723f38357fb42a.cache.dill.track.dill` 曾只有 48 MB（同目录其它同类缓存 76–79 MB），是截断的增量 dill，已定点删除（非 `rm -rf .dart_tool/flutter_build`）。删除后症状不变，说明它不是本次挂起的原因。

### P1-13 追更判 gap 与拉取更新之间允许并发裁剪 — DONE

- **文件 / 位置**：[sync.rs:589-603](crates/app-core/src/sync.rs#L589-L603)、[sync.rs:699-728](crates/app-core/src/sync.rs#L699-L728)。
- **问题 / 原因**：`catch_up_document` 分三次读取最小 rid、base rid、updates；并发 prune 可在第一次 gap 判断后删掉旧更新，拉取时只剩更高 rid。客户端可能前跳 cursor，遗漏被删的更新。此项为高可信时序风险，需屏障复现确认客户端行为。
- **推荐修改方式**：在一致快照中判断并读取，或拉取后再次核对缺口、必要时返回 base 差分；补 prune 与 pull 交错测试。
- **风险**：当前离线追更可能不完整；修复可能增加事务持续时间或全量回退次数。
- **预计收益**：高；保证断线客户端不会永久漏更新。
- **已实施（2026-09-29）**：`catch_up_document` 的三条读（`MIN(rid)`、`base_rid`、拉取更新）改为落在**同一个 `REPEATABLE READ` 事务**里，一次快照。`Rebootstrap` 分支的 base 也改用 `ensure_base_tx(&mut tx, …)` 在同一快照内取 —— 原来调用的 `bootstrap_base` 会另开连接看到更晚的状态，正是同一个「快照不一致」形状。选 `REPEATABLE READ` 而非 `SERIALIZABLE`：这三条读需要彼此一致，不需要对并发提交做校验；prune 本身与读交错是安全的（它只删已被 base 吸收的行），错的是读到一半。
- **回归测试**：`catch_up_decision_and_pull_share_one_snapshot`，用**真屏障**构造交错 —— 表级 `ACCESS EXCLUSIVE` 锁住 `document_yrs_base` 卡住**第二条读**（`base_rid`，此时第一条 `MIN(rid)` 已完成），轮询 `pg_locks` 确认它确实阻塞（不靠固定 sleep 假设），在阻塞窗口内提交 prune。断言：返回的行集必须**恰好等于决策快照当时可见的那批行**。
  - **有效性已实测**：还原为修复前形状后测试失败，`left: [392,393,394,395,396]` / `right: [391,…,396]` —— 被 prune 删掉、客户端尚未收到的 391 被跳过，即本项描述的 cursor 前跳；恢复修复后 24 项全过（24 passed / 1 ignored）。
  - **一处自身缺陷已修正**：初版断言写成「rid 必须连续」，这是错的 —— `rid` 来自表级序列，同一文档的行号本来就有洞（并行测试插入别的行），全套运行时以 `expected 242, got 243` 暴露。改为与决策快照的可见集合做集合相等，既正确又有判别力。

### P1-14 全站 1 GiB 请求体上限让匿名端点承担导入成本 — DONE

- **文件 / 位置**：[nginx.conf:15](deploy/nginx.conf#L15)、[nginx.conf:102-112](deploy/nginx.conf#L102-L112)、[routes/mod.rs:249-250](crates/api-server/src/routes/mod.rs#L249-L250)。
- **问题 / 原因**：为 ZIP 导入设置的 `client_max_body_size 1g` 放在 `http` 全局；登录等所有公开路径也继承 1 GiB 上限。Nginx 的 `proxy_request_buffering` 默认开启，完整请求体可在进入应用的 2/8 MiB 限制前被读取和缓冲。依据：[Nginx 请求体限制](https://nginx.org/en/docs/http/ngx_http_core_module.html#client_max_body_size)、[代理请求缓冲](https://nginx.org/en/docs/http/ngx_http_proxy_module.html#proxy_request_buffering)。这是资源耗尽风险，未做流量攻击复现。
- **推荐修改方式**：全局设小上限，仅给 `/api/workspaces/import` 精确路径开放 1 GiB，并保留应用端限制；增加 Nginx 配置测试验证匿名路径返回 413、合法 ZIP 仍可上传。
- **风险**：当前无认证请求可占用大量入口层缓冲/磁盘；路径写错会阻断合法导入。
- **预计收益**：中高；降低公开入口的资源放大系数。
- **已实施（2026-09-30）**：
  1. `http` 全局的 `client_max_body_size` 由 **1g 降为 1m** —— 匿名路径（登录、找回密码、验证邮件、公开 blob 链接）合法请求都是几 KB 级，1 MiB 绰绰有余。注释里写明了「大额度属于某一个路由，不属于整台服务器」。
  2. 新增 `location = /api/workspaces/import`，**精确匹配**（`=`），单独放开 `1g`。必须是精确匹配：前缀匹配会让 `/api/workspaces/` 下一切继承它，等于把刚关上的匿名面重新打开。
  3. 该 location 额外设 `proxy_request_buffering off`：导入体本来就要落成临时文件，在 nginx 先缓冲一遍等于把 1 GiB 写两遍盘。上限仍然生效。
  4. **路径必须与 `routes/mod.rs` 保持同步**，两处各写 1 GiB；漂移的症状就是「大 ZIP 导入静默失败」。
- **回归测试**：新增 [`scripts/nginx-limit-check.sh`](scripts/nginx-limit-check.sh)，沿用本仓库 `scripts/` 里「断言而非提醒」的形态（`release-check.sh` 同源）。它**按块解析配置、断言大额度可以出现在哪**，而不是 grep 一个字符串 —— 后者会连合法的那个 `1g` 一起匹配到，正好放过本项描述的形状。三项断言：http 作用域上限 ≤ 1m、导入路由存在且精确匹配并放开 ≥ 64 MiB、两个文件里的路径一致。
  - **有效性已实测（两个方向）**：把 `1g` 挪回 `http` 全局 → 拒绝（`the http-scope body cap is '1g' … must be <= 1m`，退出码 1）；把导入 location 改成前缀匹配 → 拒绝（`no exact-match location … block`，退出码 1）；恢复后通过。
  - 顺手把提示语里的非 ASCII 省略号换成 ASCII（Windows 控制台实测出现乱码 `��import��`），并删掉了一个未跟踪的空目录 `deploy/nginx.conf;C`。

## P2 — 稳定性、可观测性与覆盖缺口

### P2-01 已删除账号的旧 JWT 仍能调用付费 AI — DONE

- **文件 / 位置**：[auth.rs:712](crates/api-server/src/routes/auth.rs#L712)、[ai.rs:84-110](crates/api-server/src/routes/ai.rs#L84-L110)、[ai_ws.rs:37-46](crates/api-server/src/routes/ai_ws.rs#L37-L46)。
- **问题 / 原因**：删除账号只撤销 refresh token；AI REST/WS 入口校验 JWT，却不确认 `users` 行仍存在。旧 access token 在默认约 1 小时有效期内仍可消耗实例的 AI 配额。
- **推荐修改方式**：AI 入口校验用户仍有效，或统一引入可撤销 token 版本；增加删户后复用旧 JWT 的 REST/WS 测试。
- **风险**：当前有短时付费资源滥用窗口；修复将增加一次身份查询或 token 状态维护。
- **预计收益**：中高；删除账号后资源权限随即终止。
- **已实施（2026-09-30）**：新增 `ensure_user_exists(&PgPool, user_id)`（`Err(Unauthorized)`）与 `existing_user_id_from_headers`，接到**两个花钱入口**：REST `POST /api/ai/complete` 与 WS `/ws/ai`。选 401 而非 403/404 —— 一个指向已不存在用户的 token 不是「合法调用者做了越权的事」，而是它已经不指向任何人；客户端对 401 的既有处理正是「会话结束、重新认证」。
- **为什么不做成全局**：access token 是无状态 JWT，本来就不可撤销。`delete_account` 的级联能删掉 `users` 行、`refresh_tokens`、`api_tokens`，但删不掉调用方手里的那个 JWT。把存在性检查加到 `user_id_from_headers`（几乎每个请求都走）会给全部流量加一次查询，只为关一个**只在花钱处**才有意义的窗口 —— 所以只在花钱的地方付费。「可撤销 token 版本」是另一个正经设计（加一列按请求校验），但相对本条不成比例。
- **`list_models` 无需改动**：它走 `admin_id_from_headers`，本来就会读 `users.is_admin`，删号后自然为 false。
- **回归测试**：`a_deleted_account_stops_passing_the_existence_check`（`auth.rs` 的 `refresh_pg` 模块，数据库门控）：账号存在时通过 → 删掉行 → 必须 401；另断言一个从未存在的 id 同样 401（无法区分「已删」与「从来没在」，而这两者都不指向任何人，这是对的）。
  - **有效性已实测**：把检查弱化成「永不过期」（即原缺陷：只解码不查库）后测试失败，恢复后 api-server 全套 217 项通过。
  - `ensure_user_exists` 收 `&PgPool` 而非 `&AppState` —— 这样规则能对着真库测，不必构造整个 app（AI 配置、hub、mailer），与 `delete_user_and_owned` 从 `delete_account` 里拆出来是同一个理由。

### P2-02 并发搜索索引刷新可用旧结果覆盖新结果 — DONE

- **文件 / 位置**：[search.rs:107-156](crates/app-core/src/search.rs#L107-L156)。
- **问题 / 原因**：`BodyIndex::refresh` 在读锁下取 cursor，放锁查 DB，再拿写锁。旧查询若比新查询晚完成，仍可把旧文档内容写进 `docs`；只阻止 `seen` 回退不能阻止内容回退。这是需并发复现的时序风险。
- **推荐修改方式**：串行化 refresh，或对每行携带单调版本/`updated_at`，只接受较新的内容；补可控的查询完成顺序测试。
- **风险**：当前可能长期漏搜最新文本；串行化会影响刷新并发度。
- **预计收益**：中；搜索结果与数据库状态更一致。
- **已实施 / 验证（2026-09-30）**：`BodyIndex` 新增独立 `refresh_lock`，串行化取 cursor→数据库查询→应用快照；索引读取不持这把锁。可控交错测试让旧快照暂停、新快照就绪，断言最终只命中新文本；临时去锁后测试失败，恢复后 `mica-app-core` 44 项单测通过。代价是并发刷新排队，慢查询可能增加搜索等待时间。

### P2-03 远端光标每帧创建文本布局且不释放 — DONE

- **文件 / 位置**：[render.dart:3234-3265](clients/mica_flutter/lib/editor/render.dart#L3234-L3265)。
- **问题 / 原因**：每次 paint 为每位协作者创建、layout `TextPainter`，未调用 `dispose()`，屏外光标仍走完整绘制准备；其他绘制辅助已有显式释放的先例。
- **推荐修改方式**：先按可视区域裁剪，再用 `try/finally` 释放布局资源；补多人长文档的重复 repaint 基准测试。
- **风险**：当前长时间协作可能增加原生文本资源与掉帧；裁剪边界处理错误可能藏掉靠近视口边缘的光标。
- **预计收益**：中；降低无效布局和持续重绘成本。
- **已实施 / 验证（2026-09-30）**：先按节点与精确 clip 跳过屏外远端光标，`TextPainter` 从 layout 到 paint 都在 `try/finally` 中释放。新增 `remote_cursor_paint_test.dart` 验证远/近屏外不绘制、屏内仍可见；撤去精确裁剪后测试由 0 变 2，定向测试通过。

### P2-04 资料刷新可把旧账号资料写入新会话 — DONE（由 P1-10 覆盖）

- **文件 / 位置**：[main.dart:1033-1045](clients/mica_flutter/lib/main.dart#L1033-L1045)。
- **问题 / 原因**：资料 HTTP 请求期间切换账号，响应回来只确认当前 session 非空，不确认仍为发起请求的用户与服务器；旧 `User` 可能被持久化进新会话。
- **推荐修改方式**：与 P1-10 共用会话代次校验；加入资料请求等待期间切号测试。
- **风险**：当前可显示错误头像/名称并污染缓存；修复需避免丢掉正常的资料更新。
- **预计收益**：中；身份相关 UI 与当前凭据保持一致。
- **已覆盖**：P1-10 的会话身份修复已让 `_refreshProfile` 在 await 后检查 `_accountIdentity`；旧账号或旧服务器响应不能写进当前 session。本项未重复加入另一套代次机制。

### P2-05 目录刷新网络错误可从 `unawaited` Future 逸出 — DONE

- **文件 / 位置**：[main.dart:1592](clients/mica_flutter/lib/main.dart#L1592)、[main.dart:1602-1637](clients/mica_flutter/lib/main.dart#L1602-L1637)。
- **问题 / 原因**：WebSocket 事件触发的目录 HTTP 刷新未等待 Future，内部只捕获 `ApiException`；断网产生的客户端/Socket 异常可成为未捕获异步错误，与“瞬态错误被吞掉”的注释不一致。
- **推荐修改方式**：统一捕获网络异常并留下低噪音诊断；测试 WS 通知后 REST 失败。
- **风险**：当前正常离线场景可污染 crash log；过宽 catch 可能掩盖程序错误，建议只吞明确的网络类别。
- **预计收益**：中；错误日志更准确、重连体验更稳。
- **已实施 / 验证（2026-09-30）**：后台刷新调用边界移至 `api/tree_fetch.dart`，仅吞 `ApiException`、`ClientException`、`TimeoutException` 等预期网络失败并计数，程序错误仍向上抛；受控失败测试撤去 `ClientException` 分支后失败，恢复后定向测试通过。

### P2-06 目录 WebSocket 未观察握手 Future 的失败 — DONE

- **文件 / 位置**：[views_events.dart:84-102](clients/mica_flutter/lib/api/views_events.dart#L84-L102)。
- **问题 / 原因**：创建 `WebSocketChannel` 后只监听 stream，未观察 `channel.ready` 的异常。同仓库其他 WS 连接路径已专门 `ready.catchError`，因此这里在拒绝握手时可能抛出未捕获 zone 错误。
- **推荐修改方式**：观察并处理 `ready` 失败，按原重连策略回退；加入服务端拒绝握手测试。
- **风险**：当前断线诊断可能被额外未捕获异常污染；修复应避免 `ready` 与 `onDone` 重复安排重连。
- **预计收益**：中；网络故障路径可预测。
- **已实施 / 验证（2026-09-30）**：`ViewsEventClient` 观察 `channel.ready` 失败并交给既有重连路径；测试注入握手拒绝，去掉处理后出现未捕获 `Bad state: handshake refused`，恢复后定向测试通过。

### P2-07 部署验证脚本记录状态码，却不据此失败 — DONE

- **文件 / 位置**：[verify-prod.sh:17-23](scripts/verify-prod.sh#L17-L23)。
- **问题 / 原因**：脚本只强制检查 `/api/ready` 的版本和 bundle 下载；`/mcp`、`/` 的 HTTP 状态码只是 `echo`，`curl -s` 遇 4xx/5xx 也不让脚本失败。于是部署流程可在首页失效时仍显示验证成功。
- **推荐修改方式**：为首页断言 200 且内容确为 Flutter 入口；部署后对版本所改功能做独立冒烟，不能只看 bundle 可下载。`/mcp` 实际被 SPA fallback 返回首页，MCP 是本机 `mica-cli mcp` 的 stdio 代理，故删掉误导性的 `/mcp` HTTP 探针。
- **风险**：当前可能漏报局部生产故障；首页断言需接受正常构建产物的入口结构。
- **预计收益**：中；部署结果更接近用户实际可用性。
- **已实施 / 验证（2026-09-30）**：`verify-prod.sh` 现在拒绝首页 500、重定向和错误 HTML；新增离线回归脚本覆盖五种响应，实跑通过，并对当前生产 v0.13.46 执行无破坏冒烟通过。部署脚本不再把 `/mcp` 的 SPA 200 误报为 MCP 正常。

### P2-08 三个 Windows 云端集成用例未进持续集成 — 已接入，待 CI 真跑

- **文件 / 位置**：[flutter-integration.yml:13-18](.github/workflows/flutter-integration.yml#L13-L18)、[flutter-integration.yml:135-141](.github/workflows/flutter-integration.yml#L135-L141)；对应 `integration_test/migration_sync_test.dart`、`offline_image_reconcile_test.dart`、`page_switch_fidelity_test.dart`。
- **问题 / 原因**：工作流明确排除需要真实对象存储的三个用例，只运行无需 S3 的 `cloud_sync_test`。因此图片上传/重连、迁移和切页的 Windows 真链路变更不能由 CI 护航。这是已知基础设施取舍，不是“所有集成测试均已覆盖”。
- **推荐修改方式**：在支持 Linux 容器的 runner 增加可覆盖相同客户端链路的测试，或给 Windows runner 提供受控 S3 兼容服务；先确保失败不会静默跳过。
- **风险**：当前发布前存在这三条链路的测试盲区；新增栈会增加 CI 时间和维护成本。
- **预计收益**：中高；降低跨端存储/同步回归进入发布包的概率。
- **已实施，待验证**：Windows cloud job 改为下载并校验与 Compose 相同的 RustFS rc.3 官方 Windows 发布包，启动 Postgres、RustFS、API 后串行运行四个 live 用例；本机已验证二进制启动与 ready=200，工作流语法检查通过。**GitHub runner 尚未执行新流程，暂不标 DONE。**

### P2-09 浏览器剪贴板实际链路缺持久 E2E 回归 — 本机 DONE，待 CI

- **文件 / 位置**：[copy_markdown_test.dart:39-73](clients/mica_flutter/test/copy_markdown_test.dart#L39-L73)、[web_e2e.mjs](e2e/web_e2e.mjs)。
- **问题 / 原因**：Dart 单测验证复制文本生成器，但 Web E2E 没有浏览器真实 Ctrl+A/C、`ClipboardItem` 与读回 `text/plain` 的断言；本次代码围栏缺陷只能靠一次性手工浏览器冒烟覆盖 UI 到系统剪贴板的组合路径。
- **推荐修改方式**：在固定小文档的浏览器用例里，授予测试页 clipboard 权限，分别验证“单个代码块纯文本”和“跨块 Markdown”，同时校验 CRLF/LF 归一化；保存失败截图。
- **风险**：当前控件、键盘与 Web 剪贴板 API 的接线可在单测全绿时回归；浏览器权限模拟可能带来少量测试脆弱性。
- **预计收益**：中；让这类用户可见复制缺陷由 CI 直接发现。
- **已实施 / 验证（2026-09-30）**：测试专用 Flutter Web 入口渲染真实 `MicaEditor`，仅用公开 hook 聚焦；Playwright 发真实 Ctrl+A/C 并读回 `ClipboardItem` 的 plain/html。单代码块得到无围栏源码，跨块得到 Markdown 围栏；本机两场景通过，CI 已接入独立构建与失败截图，待推送后观察。

### P2-10 两份 Compose 的 API 环境变量允许清单已经漂移 — DONE

- **文件 / 位置**：[docker-compose.yml:177-241](deploy/docker-compose.yml#L177-L241)、[docker-compose.single.yml:110-150](deploy/docker-compose.single.yml#L110-L150)、[.env.prod.example:79-99](deploy/.env.prod.example#L79-L99)。
- **问题 / 原因**：Traefik 版显式转发注册、配额、邮件等配置；单机版仅转发较小子集。示例 `.env.prod` 明确教用户设置 `MICA_REGISTRATION_ENABLED` 和邮件参数，但单机 Compose 不会把这些变量交给 API，配置成为静默无效项。这既是功能问题，也是两份大段配置重复维护造成的架构漂移。
- **推荐修改方式**：建立一份共享的 API environment 映射，或用脚本测试“示例声明的可调变量都进入两种 Compose 的 api 容器”；补 quickstart 配置冒烟。
- **风险**：当前自托管操作者可能误以为注册或邮件已开启；合并配置时要保留两种入口各自不同的网络、域名设置。
- **预计收益**：中；消除静默配置失效并减少重复维护。
- **已实施 / 验证（2026-09-30）**：补齐单机 Compose 的 API 配置映射，使两份清单均为 31 键；新增 `docker compose config` 回归脚本检查 19 项可调设置的实际透传并接入 CI，本机验证通过。

### P2-11 自托管说明将旧 Compose 与任意新镜像配对 — DONE

- **文件 / 位置**：[README.md:119-131](README.md#L119-L131)、[docs/deploy.md:30-44](docs/deploy.md#L30-L44)。
- **问题 / 原因**：说明要求 Compose 文件与镜像来自同一 release，却固定下载 `v0.13.17` 的 Compose 和 env 示例，同时让用户自选 `MICA_VERSION`。当前代码基线已是 v0.13.46，配置能力与镜像可能跨多个版本漂移。
- **推荐修改方式**：示例先定义一个 release 版本变量，再用同一变量下载 Compose/env 并填写 `MICA_VERSION`；发布检查中验证文档的固定示例不会落后于目标版本。
- **风险**：当前新装/升级可能沿用缺新配置的旧 Compose；改文档时需避免重新引用浮动 `main`。
- **预计收益**：中；自托管安装路径与实际发布物一致。
- **已实施（2026-09-30）**：
  1. README 与 `docs/deploy.md` 的 quickstart 改为**先定一个 `RELEASE=0.13.46`**，两条 `curl` 用 `v$RELEASE`，`MICA_VERSION` 用 `sed` 从**同一个**变量写入 —— 版本只在一个地方命名。原先固定 `v0.13.17` 的 URL 配一句「自己挑个 release」，正是漂移的来源。
  2. 两条命令之外补了「为什么」：compose 的 `environment:` 是**显式允许清单**，旧 compose 配新镜像会让新镜像期待的变量根本到不了进程 —— 症状是「设置静默无效」，按构造就看不见。
  3. `scripts/release-check.sh` 新增一道**拒绝式**门槛：README.md 与 docs/deploy.md 里必须各有一个 `RELEASE=x.y.z`，且必须等于本次发布版本。选「拒绝」而非「提醒」的理由与文件里其它门槛一致 —— 「发版时记得改文档」正是这个文件存在的原因，没人记得。
- **有效性已实测**：把 README 的 `RELEASE` 改回 `0.13.17` 后门槛拒绝（`README.md pins the quickstart to 0.13.17 but this release is 0.13.46`，退出码 1）；恢复后通过。`just release` 必经此脚本，所以漂移在发版那一刻被拦住，而不是等到有人装了才发现。

### P2-12 未完成上传的孤儿对象无法回收 — DONE（新上传）

- **文件 / 位置**：[blob_gc.rs:160-265](crates/api-server/src/blob_gc.rs#L160-L265)、[files.rs:71-142](crates/api-server/src/routes/files.rs#L71-L142)。
- **问题 / 原因**：`sweep_workspace` **只遍历 `files` 表的行**（`SELECT … FROM files WHERE workspace_id = $1`），从不枚举桶里的对象。因此「PUT 了但从未调用 `complete`」的对象在库里没有行 —— 对 GC 永远不可见，既不计入配额也不被回收。P1-02 已约束新签发 URL 的尺寸、内容与重放；签名有效的首次 PUT 本来仍允许上传而不 complete。
- **推荐修改方式**：给 sweep 增加一路「按前缀枚举」：列出 `workspaces/{ws}/` 下的对象，对照该工作区的 `files.object_key` 集合，把**陌生且早于宽限期**的对象删除（宽限期必须大于 presign TTL，否则会删掉「已 PUT、正在等 complete」的合法上传 —— 注意 dev compose 的 `S3_PRESIGN_TTL_SECONDS` 是 604800，即 7 天，远超现有的 30 天 unreferenced 宽限期这一段是安全的，但**生产节点的 TTL 配置需一并确认**）。同时加一个 gauge 暴露孤儿对象数与字节数，否则这类泄漏只会无声增长。
- **风险**：枚举+删除是对桶的破坏性操作，前缀算错会删掉别人的对象。必须先做 dry-run 模式并对照 `files` 全表校验。宽限期设短于 presign TTL 会删掉正在进行的合法上传。
- **预计收益**：中；堵住一条只能靠人工清理的存储泄漏。
- **实施方式（2026-09-30）**：采用比全桶枚举更收敛的 `pending_file_uploads` 台账（migration 0027）。浏览器 presign 在返回 URL 前记录 key 与到期时间；同 key 重签只延长截止时间。服务端图片导入、字节存储及跨工作区复制也在 PUT 前提交台账，所以 PUT 成功而数据库插入失败或进程中断时，GC 仍知道该对象。`complete`、presign、PUT→登记与 GC 共用对象键事务锁；GC 在锁内再次检查台账、`files` 行、URL 到期时间和对象的 Last-Modified，30 天宽限且 URL 到期后再多等一天，只删无登记行的对象。删除后才移除台账；dry-run 只报告不删，日志和指标记录候选数与字节。候选按 200 条分页，单轮最多 1 万键或 2 分钟。
- **验证**：纯规则测试覆盖 URL 延期、对象新近写入及宽限期；真实 PostgreSQL + RustFS 测试覆盖 dry-run、逾期孤儿删除、有效 URL 留存、`complete` 未提交时 GC 阻塞并在提交后保留，以及重复 presign 延期。`cargo check`、Clippy 和相关单测通过。
- **剩余边界**：升级前没有台账的孤儿对象不会被这条扫描发现，需单独做一次历史盘点与人工审阅后清理；`avatar/` 前缀不属于此工作区文件 GC。若最旧的 1 万候选长期网络失败，后续候选可能被反复扫描同一批而延迟回收，后续可加入失败退避或持久游标。生产对象至少 1 天的 Last-Modified 宽限由纯规则测试覆盖，真实对象测试无法回拨存储时钟，使用零对象年龄模拟该分支。

## P3 — 可顺手清理的负担

### P3-01 `_selectedMarkdown` 状态与显示分支已不可达 — DONE

- **文件 / 位置**：[main.dart:404](clients/mica_flutter/lib/main.dart#L404)、[main.dart:7060](clients/mica_flutter/lib/main.dart#L7060)、[main.dart:11824-11841](clients/mica_flutter/lib/main.dart#L11824-L11841)；[roadmap.md:49](docs/roadmap.md#L49) 已记录。
- **问题 / 原因**：当前字段仅被多处置为 `null`，没有非空赋值，关联的 `selectedMarkdown != null` UI 分支不可达；状态传递和约二十处清空赋值增加外壳复杂度。
- **推荐修改方式**：在不恢复该功能的前提下，删除字段、传参和不可达显示分支；保留一条基础外壳回归。与路线图现有条目合并实施，不再创建重复待办。
- **风险**：当前主要是维护与理解成本；清理时注意不要误删仍被其他选择态使用的布局。
- **预计收益**：低到中；减少死状态和分支噪音。
- **已实施 / 验证（2026-09-30）**：删除字段、19 处无效清空、构造参数和不可达 UI 分支；全库文本检索无残留引用，Flutter 定向测试与分析通过。

## 建议执行顺序与验收

1. **先保数据**：P0-01 与 P1-06/07/08/09/12/13。每一项都先写可控交错或故障注入回归，再改实现；数据库并发用两连接屏障，不靠随机压力测试判通过。
2. **再收紧权限和输入边界**：P1-01/02/03/04/05/10/14。上传与 SSRF 用隔离测试服务/桶验证，避免碰生产数据；会话与撤权用可控 Future、持续 WS 测试。
3. **最后治理刷新、性能和交付**：剩余 P1、P2、P3。对搜索/渲染优化保留前后基准；Compose 和部署脚本先加配置/状态断言，再调整复用结构。

本轮刻意未把**已由用户拍板接受**的公开 S3 默认凭据和单机 HTTP 快速安装重新列为待修缺陷；它们的风险已在 [AGENTS.md](AGENTS.md)、[docker-compose.single.yml](deploy/docker-compose.single.yml) 与 [roadmap.md](docs/roadmap.md) 明示。审查发现中的并发攻击/时序项需要在隔离环境用上述回归确认影响范围，不能把“CI 目前通过”解释为这些交错已被覆盖。
