# Mica 代码库优化计划

审查基线：2026-09-29，`main` 的 `b838ebc`。本轮只读检查了 Rust workspace、Flutter 编辑器与外壳、Web 离线存储、测试、CI、容器与部署配置；**未修改源码，也未对生产发起破坏性验证**。仓库知识图谱项目未索引，查询返回 `project not found`，随后按 `AGENTS.md` 回退到源码与测试的定向检索。以下是可定位的风险清单，不把静态推演当作已经在生产复现的事故。

优先级：**P0** 为可能静默丢失持久数据、应先修复；**P1** 为高影响的数据完整性、安全或身份问题；**P2** 为中等影响、可观测性和测试覆盖；**P3** 为清理项。每项的“风险”同时说明当前影响和修复时需要留意的回归面；“预计收益”是定性预期，不是未测量的性能数字。

## P0 — 先保护持久数据

### P0-01 并发 Yrs 写入可互相覆盖

- **文件 / 位置**：[sync.rs:341-346](crates/app-core/src/sync.rs#L341-L346)、[sync.rs:496-539](crates/app-core/src/sync.rs#L496-L539)、[store.rs:470-484](crates/app-core/src/store.rs#L470-L484)。
- **问题 / 原因**：`push_update` 在事务中普通 `SELECT` 读取旧 base，离线折叠客户端更新，最后用 `ON CONFLICT DO UPDATE SET state = excluded.state` 写回整份状态。两个并发写者可能从同一旧 base 派生，后写者覆盖先写者；两条 `workspace_updates` 却都已提交。REST 路径锁 `documents` 行，而此路径在读取 base 前不取同一锁，混合写入也存在旧读覆盖窗口。这是高置信静态并发推演，尚需可控时序的数据库复现。
- **推荐修改方式**：在读取 base 前，以统一锁顺序锁住对应 `documents` 行；在锁内完成折叠、stream 写入和 base 更新。增加两连接并发 `push`、REST 与 `push` 交错测试，断言最终 base、`base_rid` 和搜索投影都包含两次编辑。现有 [sync_pg.rs:639](crates/app-core/tests/sync_pg.rs#L639) 只测试首次 bootstrap 并发。
- **风险**：当前可能静默丢失协作编辑；增加锁后须检查其他写路径的锁顺序与吞吐，避免死锁。
- **预计收益**：高；阻断最严重的数据完整性缺口。

## P1 — 高影响缺陷与安全边界

### P1-01 空实例的并发首账号注册可产生多个管理员

- **文件 / 位置**：[auth.rs:149-172](crates/api-server/src/routes/auth.rs#L149-L172)、[0001_initial.sql:3-10](migrations/0001_initial.sql#L3-L10)、[0021_ai_settings_and_admin.sql:25](migrations/0021_ai_settings_and_admin.sql#L25)。
- **问题 / 原因**：首账号特殊权限依赖 `INSERT … WHERE NOT EXISTS(users)`；PostgreSQL 默认 `READ COMMITTED` 下，两个并发注册语句可都看到空表并插入不同邮箱，数据库没有“仅一个首管理员”约束。
- **推荐修改方式**：用数据库串行化锁或原子 bootstrap 状态约束首账号创建；增加空库双请求并发测试，同时验证注册默认关闭时第二个请求被拒。
- **风险**：初装短窗口内可能意外授予第二个已验证管理员；修改首启流程需保留合法首账号体验。
- **预计收益**：高；首启管理员归属不再依赖请求时序。

### P1-02 预签名上传未绑定真实内容与大小

- **文件 / 位置**：[files.rs:74-132](crates/api-server/src/routes/files.rs#L74-L132)、[storage.rs:365-388](crates/infra/src/storage.rs#L365-L388)、[store.rs:676-702](crates/app-core/src/store.rs#L676-L702)。
- **问题 / 原因**：服务端依据客户端声明的 hash、字节数签发 object key；签名使用 `UNSIGNED-PAYLOAD`，只签 `host`，`complete` 也不读取对象校验。持有该工作区编辑权限者可用已知 key 写入不同字节覆盖既有附件，或 PUT 超大对象而不 complete，绕过元数据配额并留下难以回收的孤儿对象。攻击链基于静态代码，尚未对对象存储做破坏性复现。
- **推荐修改方式**：防止覆盖已存在 key，约束实际上传长度与内容摘要，complete 前校验对象存在、实际大小及摘要；以真实 S3 兼容存储做恶意大小、重复 key、缺失对象的集成测试。
- **风险**：当前附件完整性与存储额度可能被工作区编辑者破坏；修复会改变客户端上传协议和去重语义，须兼容现存对象。
- **预计收益**：高；让配额与内容寻址具有实际约束力。

### P1-03 外部图片导入的 DNS 检查与实际连接脱节

- **文件 / 位置**：[files.rs:238-270](crates/api-server/src/routes/files.rs#L238-L270)。
- **问题 / 原因**：先解析并过滤私网 IP，之后 `reqwest` 再次解析域名；源码注释明确承认 DNS rebinding 可在两次解析之间切换目标。禁止重定向不能堵住这一条路径。
- **推荐修改方式**：让实际连接固定使用已验证的公网 IP，同时维持原始 Host/TLS 主机名；覆盖解析变化、IPv4/IPv6、重定向测试。
- **风险**：当前可被用于请求内网或云元数据地址；修复涉及 DNS 与 TLS 的结合，必须防止错误拒绝正常 CDN。
- **预计收益**：高；关闭明确存在的 SSRF 绕过窗口。

### P1-04 外部图片响应在限额检查前整体进入内存

- **文件 / 位置**：[files.rs:282-302](crates/api-server/src/routes/files.rs#L282-L302)。
- **问题 / 原因**：`response.bytes().await` 将完整远端响应装入内存，之后才调用 `ensure_storable`。远端持续返回大数据时，API 在判定超限前已承担内存消耗。
- **推荐修改方式**：流式读取并在超过 `max_upload_bytes` 时停止；`Content-Length` 可作提前拒绝，但不能替代实际流量计数。
- **风险**：当前恶意或异常图片源可推高 API 内存；修改流式处理需要保留 MIME、hash 和错误映射行为。
- **预计收益**：高；单请求内存有明确上界。

### P1-05 已连接文档 WebSocket 不实时执行撤权

- **文件 / 位置**：[ws.rs:98-112](crates/api-server/src/routes/ws.rs#L98-L112)、[ws.rs:351-451](crates/api-server/src/routes/ws.rs#L351-L451)、[ws.rs:684-701](crates/api-server/src/routes/ws.rs#L684-L701)。
- **问题 / 原因**：连接升级时检查工作区角色，之后沿连接复用缓存的 `permissions`。成员被移除或降为只读后，原连接仍可发送 `sync.push`；JWT 默认有效期内不会自然触发角色重查。
- **推荐修改方式**：每次写入前重新校验成员权限，或在成员权限变更时主动关闭对应连接；补“连接建立→撤权→继续推送”的测试。
- **风险**：当前撤权不能即时阻止写入；逐次查询会增加数据库读取，主动踢连接则增加房间生命周期逻辑。
- **预计收益**：高；权限变更按用户预期立即生效。

### P1-06 编辑器旧 flush 完成会清除新输入的脏标记

- **文件 / 位置**：[controller.dart:499-524](clients/mica_flutter/lib/editor/controller.dart#L499-L524)。
- **问题 / 原因**：`_dirty` 只按 block ID 记录。A 批输入开始异步提交后，同块又输入 B；A 的 `whenComplete` 无条件 `removeAll(ids)`，可清掉 B 的标记，后续 debounce 看见空集合而跳过 B。
- **推荐修改方式**：为每块记录修改代次，只清除本批已成功提交且代次未变化的脏状态；以可控 `Completer` 覆盖 A/B 交错。
- **风险**：当前可丢失未持久化的新输入，随后被远端 reconcile 覆盖；改动需维护输入、flush 和切页时的代次一致性。
- **预计收益**：高；防止快速编辑时的静默丢字。

### P1-07 编辑器提交失败仍会清掉待保存内容

- **文件 / 位置**：[controller.dart:521-524](clients/mica_flutter/lib/editor/controller.dart#L521-L524)、[controller.dart:3288-3300](clients/mica_flutter/lib/editor/controller.dart#L3288-L3300)、[editor_op_fault_test.dart:19-41](clients/mica_flutter/test/editor_op_fault_test.dart#L19-L41)。
- **问题 / 原因**：`_send` 将 `onOps` 错误转为完成的 Future；`flushPending` 的 `whenComplete` 不区分结果即清脏。已有测试只数错误回调，不验证失败后的本地内容是否还可提交。
- **推荐修改方式**：保留明确的提交成功/失败结果；失败时保留待提交修改、提供重试，并在切页/销毁前阻止静默丢弃；增加故障注入测试。
- **风险**：当前 outbox/存储写失败后可能丢失输入；修复需避免无限重试和重复操作。
- **预计收益**：高；让错误处理真正保护数据耐久性。

### P1-08 立即结构操作冲刷 dirty 文本时遗漏格式数据

- **文件 / 位置**：[controller.dart:266-310](clients/mica_flutter/lib/editor/controller.dart#L266-L310)、[controller.dart:3266-3285](clients/mica_flutter/lib/editor/controller.dart#L3266-L3285)。
- **问题 / 原因**：文字编辑会更新 `data.marks` 偏移，但 `_sendNow` 为其他脏块生成的 `update_block` 只含 `text`，然后清脏；与正常 `flushPending` 同时提交 `text`、`data` 的做法不一致。带链接/粗体的文本输入后，400ms 内触发另一块的结构操作即可留下错位 mark。
- **推荐修改方式**：立即冲刷也提交对应 `data`；增加“编辑带 mark 的块→立即 Enter/插块→重载”的回归测试。
- **风险**：当前可能持久化错误的格式或链接范围；改动应防止重复发送覆盖新的远端格式更新。
- **预计收益**：高；结构操作前完整保存文本与格式。

### P1-09 reconcile 保留脏文本却覆盖同一块的本地格式

- **文件 / 位置**：[controller.dart:92-120](clients/mica_flutter/lib/editor/controller.dart#L92-L120)、[cloud_reconcile_test.dart:36-54](clients/mica_flutter/test/cloud_reconcile_test.dart#L36-L54)。
- **问题 / 原因**：本地块为 dirty 时，`reconcile()` 保留 `cur.text`，但无条件取远端 `src.data`。文字编辑已调整的 marks 可被旧快照的偏移覆盖，稍后再把“新文本+旧 marks”提交。现有测试仅覆盖无格式文本。
- **推荐修改方式**：dirty 时把文本与对应格式数据视为同一版本；明确远端非格式字段合并规则，补粗体、链接和远端旧快照交错测试。
- **风险**：当前格式/链接可错位；过度保护整个 `data` 也可能吞掉远端属性变更，合并粒度须先定清楚。
- **预计收益**：高；保住富文本编辑的一致性。

### P1-10 旧账号的异步会话刷新可在退出后复活

- **文件 / 位置**：[main.dart:1054-1063](clients/mica_flutter/lib/main.dart#L1054-L1063)、[session_refresher.dart:43-50](clients/mica_flutter/lib/api/session_refresher.dart#L43-L50)。
- **问题 / 原因**：刷新发起后若用户退出、切换服务器或登录另一账号，完成时只检查 `mounted` 就覆盖 `_session`；`_persistSession` 按当前 `_cloudOrigin` 保存旧凭据。全局单飞刷新器还可能让新会话复用旧会话的结果。
- **推荐修改方式**：刷新结果绑定发起时的用户、服务器、refresh token 与会话代次；身份变化时废弃旧结果，并用受控 Future 测试退出/切服交错。
- **风险**：当前可能退出后重新登录旧账号或混淆不同服务器身份；改动须避免使合法的并发 401 刷新重复执行。
- **预计收益**：高；恢复认证状态边界。

### P1-11 旧目录响应可覆盖新树并使 ETag 锁定陈旧内容

- **文件 / 位置**：[main.dart:6219-6229](clients/mica_flutter/lib/main.dart#L6219-L6229)、[main.dart:6245-6277](clients/mica_flutter/lib/main.dart#L6245-L6277)、[main.dart:6295-6315](clients/mica_flutter/lib/main.dart#L6295-L6315)。
- **问题 / 原因**：目录请求收到 200 后先存 ETag，等待正文 bootstrap 才提交树；期间另一轮目录刷新可提交新树和新 ETag，旧请求晚到后再覆盖树与离线镜像。后续 304 可能使陈旧树长期保留。
- **推荐修改方式**：按工作区使用请求代次/服务端版本，对树、ETag 和镜像实行一次性提交；加入响应乱序与 304 后续请求测试。
- **风险**：当前目录可回退、跨重启保留陈旧状态；修复需兼顾离线镜像与树事件的顺序。
- **预计收益**：高；避免列表与缓存互相矛盾。

### P1-12 Web IndexedDB 写锁早于写队列完成释放

- **文件 / 位置**：[web_idb_doc_store.dart:359-390](clients/mica_flutter/lib/cloud/web_idb_doc_store.dart#L359-L390)、[web_idb_doc_store.dart:455-464](clients/mica_flutter/lib/cloud/web_idb_doc_store.dart#L455-L464)、[web_idb_doc_store.dart:561-569](clients/mica_flutter/lib/cloud/web_idb_doc_store.dart#L561-L569)、[web_idb_doc_store_test.dart:46-55](clients/mica_flutter/test/web_idb_doc_store_test.dart#L46-L55)。
- **问题 / 原因**：`dispose()` 先释放单写者 Web Lock，之后才等 `_tail` 队列完成；新实例可抢锁、读到旧 outbox，再以整份 rows 覆写仍在提交的旧实例。现有重开测试总先 flush，避开这个交接时序。
- **推荐修改方式**：在队列 settled 后才释放锁；加入延迟 IndexedDB 事务的跨实例交接测试。
- **风险**：当前 Web 离线编辑可能在切页/重开时被覆盖；修复会延长锁占用，需处理失败队列的释放兜底。
- **预计收益**：高；保护离线 outbox 的单写者语义。

### P1-13 追更判 gap 与拉取更新之间允许并发裁剪

- **文件 / 位置**：[sync.rs:589-603](crates/app-core/src/sync.rs#L589-L603)、[sync.rs:699-728](crates/app-core/src/sync.rs#L699-L728)。
- **问题 / 原因**：`catch_up_document` 分三次读取最小 rid、base rid、updates；并发 prune 可在第一次 gap 判断后删掉旧更新，拉取时只剩更高 rid。客户端可能前跳 cursor，遗漏被删的更新。此项为高可信时序风险，需屏障复现确认客户端行为。
- **推荐修改方式**：在一致快照中判断并读取，或拉取后再次核对缺口、必要时返回 base 差分；补 prune 与 pull 交错测试。
- **风险**：当前离线追更可能不完整；修复可能增加事务持续时间或全量回退次数。
- **预计收益**：高；保证断线客户端不会永久漏更新。

### P1-14 全站 1 GiB 请求体上限让匿名端点承担导入成本

- **文件 / 位置**：[nginx.conf:15](deploy/nginx.conf#L15)、[nginx.conf:102-112](deploy/nginx.conf#L102-L112)、[routes/mod.rs:249-250](crates/api-server/src/routes/mod.rs#L249-L250)。
- **问题 / 原因**：为 ZIP 导入设置的 `client_max_body_size 1g` 放在 `http` 全局；登录等所有公开路径也继承 1 GiB 上限。Nginx 的 `proxy_request_buffering` 默认开启，完整请求体可在进入应用的 2/8 MiB 限制前被读取和缓冲。依据：[Nginx 请求体限制](https://nginx.org/en/docs/http/ngx_http_core_module.html#client_max_body_size)、[代理请求缓冲](https://nginx.org/en/docs/http/ngx_http_proxy_module.html#proxy_request_buffering)。这是资源耗尽风险，未做流量攻击复现。
- **推荐修改方式**：全局设小上限，仅给 `/api/workspaces/import` 精确路径开放 1 GiB，并保留应用端限制；增加 Nginx 配置测试验证匿名路径返回 413、合法 ZIP 仍可上传。
- **风险**：当前无认证请求可占用大量入口层缓冲/磁盘；路径写错会阻断合法导入。
- **预计收益**：中高；降低公开入口的资源放大系数。

## P2 — 稳定性、可观测性与覆盖缺口

### P2-01 已删除账号的旧 JWT 仍能调用付费 AI

- **文件 / 位置**：[auth.rs:712](crates/api-server/src/routes/auth.rs#L712)、[ai.rs:84-110](crates/api-server/src/routes/ai.rs#L84-L110)、[ai_ws.rs:37-46](crates/api-server/src/routes/ai_ws.rs#L37-L46)。
- **问题 / 原因**：删除账号只撤销 refresh token；AI REST/WS 入口校验 JWT，却不确认 `users` 行仍存在。旧 access token 在默认约 1 小时有效期内仍可消耗实例的 AI 配额。
- **推荐修改方式**：AI 入口校验用户仍有效，或统一引入可撤销 token 版本；增加删户后复用旧 JWT 的 REST/WS 测试。
- **风险**：当前有短时付费资源滥用窗口；修复将增加一次身份查询或 token 状态维护。
- **预计收益**：中高；删除账号后资源权限随即终止。

### P2-02 并发搜索索引刷新可用旧结果覆盖新结果

- **文件 / 位置**：[search.rs:107-156](crates/app-core/src/search.rs#L107-L156)。
- **问题 / 原因**：`BodyIndex::refresh` 在读锁下取 cursor，放锁查 DB，再拿写锁。旧查询若比新查询晚完成，仍可把旧文档内容写进 `docs`；只阻止 `seen` 回退不能阻止内容回退。这是需并发复现的时序风险。
- **推荐修改方式**：串行化 refresh，或对每行携带单调版本/`updated_at`，只接受较新的内容；补可控的查询完成顺序测试。
- **风险**：当前可能长期漏搜最新文本；串行化会影响刷新并发度。
- **预计收益**：中；搜索结果与数据库状态更一致。

### P2-03 远端光标每帧创建文本布局且不释放

- **文件 / 位置**：[render.dart:3234-3265](clients/mica_flutter/lib/editor/render.dart#L3234-L3265)。
- **问题 / 原因**：每次 paint 为每位协作者创建、layout `TextPainter`，未调用 `dispose()`，屏外光标仍走完整绘制准备；其他绘制辅助已有显式释放的先例。
- **推荐修改方式**：先按可视区域裁剪，再用 `try/finally` 释放布局资源；补多人长文档的重复 repaint 基准测试。
- **风险**：当前长时间协作可能增加原生文本资源与掉帧；裁剪边界处理错误可能藏掉靠近视口边缘的光标。
- **预计收益**：中；降低无效布局和持续重绘成本。

### P2-04 资料刷新可把旧账号资料写入新会话

- **文件 / 位置**：[main.dart:1033-1045](clients/mica_flutter/lib/main.dart#L1033-L1045)。
- **问题 / 原因**：资料 HTTP 请求期间切换账号，响应回来只确认当前 session 非空，不确认仍为发起请求的用户与服务器；旧 `User` 可能被持久化进新会话。
- **推荐修改方式**：与 P1-10 共用会话代次校验；加入资料请求等待期间切号测试。
- **风险**：当前可显示错误头像/名称并污染缓存；修复需避免丢掉正常的资料更新。
- **预计收益**：中；身份相关 UI 与当前凭据保持一致。

### P2-05 目录刷新网络错误可从 `unawaited` Future 逸出

- **文件 / 位置**：[main.dart:1592](clients/mica_flutter/lib/main.dart#L1592)、[main.dart:1602-1637](clients/mica_flutter/lib/main.dart#L1602-L1637)。
- **问题 / 原因**：WebSocket 事件触发的目录 HTTP 刷新未等待 Future，内部只捕获 `ApiException`；断网产生的客户端/Socket 异常可成为未捕获异步错误，与“瞬态错误被吞掉”的注释不一致。
- **推荐修改方式**：统一捕获网络异常并留下低噪音诊断；测试 WS 通知后 REST 失败。
- **风险**：当前正常离线场景可污染 crash log；过宽 catch 可能掩盖程序错误，建议只吞明确的网络类别。
- **预计收益**：中；错误日志更准确、重连体验更稳。

### P2-06 目录 WebSocket 未观察握手 Future 的失败

- **文件 / 位置**：[views_events.dart:84-102](clients/mica_flutter/lib/api/views_events.dart#L84-L102)。
- **问题 / 原因**：创建 `WebSocketChannel` 后只监听 stream，未观察 `channel.ready` 的异常。同仓库其他 WS 连接路径已专门 `ready.catchError`，因此这里在拒绝握手时可能抛出未捕获 zone 错误。
- **推荐修改方式**：观察并处理 `ready` 失败，按原重连策略回退；加入服务端拒绝握手测试。
- **风险**：当前断线诊断可能被额外未捕获异常污染；修复应避免 `ready` 与 `onDone` 重复安排重连。
- **预计收益**：中；网络故障路径可预测。

### P2-07 部署验证脚本记录状态码，却不据此失败

- **文件 / 位置**：[verify-prod.sh:17-23](scripts/verify-prod.sh#L17-L23)。
- **问题 / 原因**：脚本只强制检查 `/api/ready` 的版本和 bundle 下载；`/mcp`、`/` 的 HTTP 状态码只是 `echo`，`curl -s` 遇 4xx/5xx 也不让脚本失败。于是部署流程可在首页失效时仍显示验证成功。
- **推荐修改方式**：为首页断言 200；为 MCP 路由断言其预期状态集合，明确失败时退出非零。部署后对版本所改功能做独立冒烟，不能只看 bundle 可下载。
- **风险**：当前可能漏报局部生产故障；修改预期码时须尊重 `/mcp` 的真实方法语义。
- **预计收益**：中；部署结果更接近用户实际可用性。

### P2-08 三个 Windows 云端集成用例未进持续集成

- **文件 / 位置**：[flutter-integration.yml:13-18](.github/workflows/flutter-integration.yml#L13-L18)、[flutter-integration.yml:135-141](.github/workflows/flutter-integration.yml#L135-L141)；对应 `integration_test/migration_sync_test.dart`、`offline_image_reconcile_test.dart`、`page_switch_fidelity_test.dart`。
- **问题 / 原因**：工作流明确排除需要真实对象存储的三个用例，只运行无需 S3 的 `cloud_sync_test`。因此图片上传/重连、迁移和切页的 Windows 真链路变更不能由 CI 护航。这是已知基础设施取舍，不是“所有集成测试均已覆盖”。
- **推荐修改方式**：在支持 Linux 容器的 runner 增加可覆盖相同客户端链路的测试，或给 Windows runner 提供受控 S3 兼容服务；先确保失败不会静默跳过。
- **风险**：当前发布前存在这三条链路的测试盲区；新增栈会增加 CI 时间和维护成本。
- **预计收益**：中高；降低跨端存储/同步回归进入发布包的概率。

### P2-09 浏览器剪贴板实际链路缺持久 E2E 回归

- **文件 / 位置**：[copy_markdown_test.dart:39-73](clients/mica_flutter/test/copy_markdown_test.dart#L39-L73)、[web_e2e.mjs](e2e/web_e2e.mjs)。
- **问题 / 原因**：Dart 单测验证复制文本生成器，但 Web E2E 没有浏览器真实 Ctrl+A/C、`ClipboardItem` 与读回 `text/plain` 的断言；本次代码围栏缺陷只能靠一次性手工浏览器冒烟覆盖 UI 到系统剪贴板的组合路径。
- **推荐修改方式**：在固定小文档的浏览器用例里，授予测试页 clipboard 权限，分别验证“单个代码块纯文本”和“跨块 Markdown”，同时校验 CRLF/LF 归一化；保存失败截图。
- **风险**：当前控件、键盘与 Web 剪贴板 API 的接线可在单测全绿时回归；浏览器权限模拟可能带来少量测试脆弱性。
- **预计收益**：中；让这类用户可见复制缺陷由 CI 直接发现。

### P2-10 两份 Compose 的 API 环境变量允许清单已经漂移

- **文件 / 位置**：[docker-compose.yml:177-241](deploy/docker-compose.yml#L177-L241)、[docker-compose.single.yml:110-150](deploy/docker-compose.single.yml#L110-L150)、[.env.prod.example:79-99](deploy/.env.prod.example#L79-L99)。
- **问题 / 原因**：Traefik 版显式转发注册、配额、邮件等配置；单机版仅转发较小子集。示例 `.env.prod` 明确教用户设置 `MICA_REGISTRATION_ENABLED` 和邮件参数，但单机 Compose 不会把这些变量交给 API，配置成为静默无效项。这既是功能问题，也是两份大段配置重复维护造成的架构漂移。
- **推荐修改方式**：建立一份共享的 API environment 映射，或用脚本测试“示例声明的可调变量都进入两种 Compose 的 api 容器”；补 quickstart 配置冒烟。
- **风险**：当前自托管操作者可能误以为注册或邮件已开启；合并配置时要保留两种入口各自不同的网络、域名设置。
- **预计收益**：中；消除静默配置失效并减少重复维护。

### P2-11 自托管说明将旧 Compose 与任意新镜像配对

- **文件 / 位置**：[README.md:119-131](README.md#L119-L131)、[docs/deploy.md:30-44](docs/deploy.md#L30-L44)。
- **问题 / 原因**：说明要求 Compose 文件与镜像来自同一 release，却固定下载 `v0.13.17` 的 Compose 和 env 示例，同时让用户自选 `MICA_VERSION`。当前代码基线已是 v0.13.46，配置能力与镜像可能跨多个版本漂移。
- **推荐修改方式**：示例先定义一个 release 版本变量，再用同一变量下载 Compose/env 并填写 `MICA_VERSION`；发布检查中验证文档的固定示例不会落后于目标版本。
- **风险**：当前新装/升级可能沿用缺新配置的旧 Compose；改文档时需避免重新引用浮动 `main`。
- **预计收益**：中；自托管安装路径与实际发布物一致。

## P3 — 可顺手清理的负担

### P3-01 `_selectedMarkdown` 状态与显示分支已不可达

- **文件 / 位置**：[main.dart:404](clients/mica_flutter/lib/main.dart#L404)、[main.dart:7060](clients/mica_flutter/lib/main.dart#L7060)、[main.dart:11824-11841](clients/mica_flutter/lib/main.dart#L11824-L11841)；[roadmap.md:49](docs/roadmap.md#L49) 已记录。
- **问题 / 原因**：当前字段仅被多处置为 `null`，没有非空赋值，关联的 `selectedMarkdown != null` UI 分支不可达；状态传递和约二十处清空赋值增加外壳复杂度。
- **推荐修改方式**：在不恢复该功能的前提下，删除字段、传参和不可达显示分支；保留一条基础外壳回归。与路线图现有条目合并实施，不再创建重复待办。
- **风险**：当前主要是维护与理解成本；清理时注意不要误删仍被其他选择态使用的布局。
- **预计收益**：低到中；减少死状态和分支噪音。

## 建议执行顺序与验收

1. **先保数据**：P0-01 与 P1-06/07/08/09/12/13。每一项都先写可控交错或故障注入回归，再改实现；数据库并发用两连接屏障，不靠随机压力测试判通过。
2. **再收紧权限和输入边界**：P1-01/02/03/04/05/10/14。上传与 SSRF 用隔离测试服务/桶验证，避免碰生产数据；会话与撤权用可控 Future、持续 WS 测试。
3. **最后治理刷新、性能和交付**：剩余 P1、P2、P3。对搜索/渲染优化保留前后基准；Compose 和部署脚本先加配置/状态断言，再调整复用结构。

本轮刻意未把**已由用户拍板接受**的公开 S3 默认凭据和单机 HTTP 快速安装重新列为待修缺陷；它们的风险已在 [AGENTS.md](AGENTS.md)、[docker-compose.single.yml](deploy/docker-compose.single.yml) 与 [roadmap.md](docs/roadmap.md) 明示。审查发现中的并发攻击/时序项需要在隔离环境用上述回归确认影响范围，不能把“CI 目前通过”解释为这些交错已被覆盖。
