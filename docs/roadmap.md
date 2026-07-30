# Mica 路线图 — 剩余功能与优化点

> 2026-07-08 生成。来源:多代理系统扫过 `crates/` + `clients/mica_flutter/lib/` 的
> TODO/未做标记、`docs/` 的 pending 项、编辑器里程碑、后端硬化面,综合排优先级。
> 影响力从高到低;`(S/M/L)` = 工作量;`[需后端]` = 要动 Rust。
>
> 背景:v0.1.4。**M-R 云端数据安全里程碑已完成**(崩溃/切页/坏 update/流截断四类
> 丢数据 + 熔断可见,见 `phase2-offline-crdt.md` §13),自动重连见 §13.1。
>
> **2026-07-22 盲区扫描增补**:多维度只读审计(安全/运维/数据生命周期/客户端/测试/
> 同步/合规七维,每条经二轮怀疑者核实)补入了原清单未覆盖的项,以 `🆕` 标记;同时
> 勾掉了此前误记为待办、实则已实现的安全项。新增了「生产运维与备份」「数据生命周期与
> 增长」「产品与公开发布合规」三个小节。

## 可靠性与同步

- ~~**P2-M4 云同步流未真正建**~~ ✅ **主干早已上线**(2026-07-29 对代码核实,此条整条过期)—— `sync.rs` push_update 写流+fold、`catch_up_document` 按 since_rid 续传+剪枝缺口自动 Rebootstrap、`diff_from_base` SV 兜底;WS `sync.bootstrap/pull/push` 三 handler(`ws.rs:336-440`)+ 客户端 `_pullPayload(since_rid+sv)` 消费;真 PG 集成测试 `sync_pg.rs`。**它一直被记成未建,把整棵依赖树都记歪了** —— 实际解锁的下一步是 op 模型退役(见「数据生命周期」)。
- **实时字符级并发协同未落地**(2026-07-29 措辞修正)—— 传输/应用层是**真 yrs merge**(`sync.rs` guarded_apply、客户端 applyUpdate),不是 last-write;真实缺陷在编辑器 op 粒度:`update_block` 整段换文本(`doc.rs` set_text_and_marks = remove_range+insert),两端并发编辑同块合并成**重复/拼接**而非字符交织。字符级 API 已在(`text_insert`/`text_delete`)但编辑器热路径不用;细粒度化 = delta↔marks 映射,phase2 §12 明言的最大风险区。(L) `[需后端]`
- 🟡 **M-R 收尾**(2026-07-29 核实:4 项里 3 项早已落地)—— ~~C3 坏更新自愈+schema 版本~~ ✅(`store.rs` SCHEMA_VERSION=6 + 太新拒开 + 每 blob CRC-32 + yrs panic 包成 CorruptDoc;客户端坏副本冷 bootstrap 自愈、坏 remote 封顶熔断);~~D2 同步健康态~~ ✅(`sync_status.dart` → _SyncBadge + fault banner);~~A3 会话持久化 e2e~~ ✅(`cloud_sync_integrity_test.dart`:未 ack 编辑跨重启重推)。**残留仅 D1 尾巴**:关键故障已计数上报(onFault/_faultCount/_persistFails/_pushRejects),但 `onError:(_){}` 等带注释静默吞仍无通用计数设施。(S)
- **离线→在线 blob 自动 reconcile** —— `_reconcilePendingUploads` 全仓唯一调用点在 onReady(=重开文档);重连侧 `_onCloudOnline` 只扫文本 outbox 不扫 blob(`_sweepPendingOutboxes` 的注释自己点名了这个洞),且 onServerConnected 每会话只 latch 一次、会话内二次重连无回调可挂。(S,客户端为主)
- 🆕 **重连复用过期 token → 永久退避循环**(bug,2026-07-29 核实中发现)—— token 烤死在会话创建时的 `uri`(`cloud_sync_session.dart:62` 注释自认 + `:259` connect 用 this.uri),access TTL 1h 过后断线重连恒 401 → 退避循环,直到用户切文档才拿到新 token;长开会话表现为「静默停止同步」。修法:uri 改 getter/回调注入刷新后 token;顺手把服务端「exp 到期 close(4401)」一起做(即安全小节的 TTL 条)。待实测复现确认路径。(S-M)
- ~~**双向 state-vector 协商**~~ ✅ 已做(校准复核)—— P4-3:`ws.rs:508` `client_sv.and_then(|sv| sync::diff_from_base(base, sv))` 按 client SV 发最小 diff,base_message delta 分支 + 单测 `base_message_sends_delta_only_when_sv_yields_one`。
- ~~**broadcast lag 触发整档重载**~~ ✅ 已做(校准复核)—— 客户端 `_resyncFromLag` 发 `sync.pull` 带 cursor+SV 增量续拉,非整档重载。(`cloud_sync_session.dart`)
- ~~🆕 **`client_out_of_date` 客户端零处理 → 被跳过的更新永久静默丢失**~~ ✅ 已做(校准复核)—— `cloud_sync_session.dart:467` 收到 `code:'client_out_of_date'`(无 ack_id)即 `_resyncFromLag()` 触发 pull/bootstrap 补洞;server 侧 `ws.rs:170` 发 notice。
- ~~🆕 **离线 outbox 按文档滞留:重连后只有当前打开的文档会推送**~~ ✅ 已做(5b7536a)—— 上线/重连(`onServerConnected → _onCloudOnline`)跑 `_sweepPendingOutboxes`:枚举云视图,跳活跃文档,对每个有非空 outbox 的云文档起短命 headless `CloudSyncSession`(persistence=该 doc 的 store)connect→drainOutbox→dispose。桌面 only;串行 + 单例锁 + best-effort + 双检跳活跃 + 空 outbox 不连;blast radius 有界(mis-drain=幂等/超时不损坏)。**注**:纯客户端编排、组合已测原语,无新增自动化测试(端到端需真 WS+多文档集成环境),待实机冒烟。(`main.dart` `_sweepPendingOutboxes`)
- 🆕 **长离线重连 = 推送风暴**(medium) —— `_flushUnacked(resendAll:true)` 逐条重发整个 outbox,无分批/背压/合并;服务端每条 push 全档 decode+encode+upsert = O(条数×文档大小)。可先用 yrs merge 把尾巴合成一条再推,或分批节流。(`cloud_sync_session.dart:584`, `sync.rs:217`)(M) `[需后端]`
- ~~🆕 **协议无版本协商 / 无最低版本闸门**~~ ✅ **已做(2026-07-29)** —— 客户端在 WS URL 上声明 `v=<版本>`(`kSyncProtocolVersion`,现为 1),服务端 `check_protocol` 对照 `MICA_WS_MIN_PROTOCOL` 判定,太老的拒以 `client_too_old`(稳定机器码,客户端据此提示"请更新"而不是"请重新登录")。
  **三个刻意的设计点**:① **默认地板 0,谁都不拒** —— 闸门必须在需要它之前就存在,上线当天就开始拒会打死所有还没更新的桌面安装;要挡的恰恰是那批**不带版本**的老客户端,等到 S4 再加声明就晚了。② **判定排在鉴权之前** —— 否则一个 token 恰好过期的老客户端会先拿到 `unauthorized`、被支去重新登录,登录完重连再被同样拒掉,循环;版本是公开参数,不必等在密钥后面(端到端实测:不带 v + 坏 token 返回 `client_too_old` 而非 `unauthorized`)。③ **加性演进不许 bump** —— 版本号只标记"老客户端活不下去"的破坏性变更,为兼容改动动它会逼出没必要的升级。
  两端各有一条对称断言钉住版本号一致(`the_current_version_is_one` / 「版本号跟服务端对齐」),防它们在两种语言里无声漂移。(`ws.rs` `check_protocol`, `sync_client.dart`)
- ~~🆕 **Web IndexedDB 被驱逐 → 未推送离线编辑静默蒸发**~~ ✅ 已做(60b8b67)—— `WebIdbDocStore.open` 首次打开时 best-effort 调 `navigator.storage.persist()`(js_interop 绑定 StorageManager),请求持久化存储、显著降低驱逐概率;guarded 每会话一次、fire-and-forget、缺 API/拒绝均降级。(`web_idb_doc_store.dart` `_requestPersistentStorage`)
- **M-R 收尾:更细的「离线/重连中」状态提示** —— 见「客户端质量」小节的同步状态可见性条目。
- ~~客户端自动重连~~ ✅ 已做(branch `feat/cloud-auto-reconnect`,退避重连,§13.1)。

## 安全

> 上一轮安全 review 的落地清单。自托管一上公网,前几项是硬底线。
> 2026-07-22:refresh/rotation/改密撤销已落地(勾除);新增分享页 XSS、AI 密钥外泄、
> SSRF 等此前漏网的高危项。

- ~~🆕 **公开分享页存储型 XSS → 窃 token → 账号接管**~~ ✅ 已做(200c3b1)—— 分享响应加严格 CSP(`SHARE_CSP`:`default-src 'none'`、无 `script-src` → 内联 script + `on*` 处理器全挡)+ raw HTML 纵深净化(`strip_unsafe_attrs` 剥 `on*`/中和 `javascript:` URI,81ff653)。双层。(`documents.rs:2235/2244`, `markdown/lib.rs:3600`)
- ~~🆕 **分享链接在页面进回收站/「永久删除」后仍对外可读**~~ ✅ 已做(200c3b1)—— `public_share_page` 渲染前 `fetch_document_view`(过 `is_deleted=false`)→ 删/purge 后返回 None → 统一 404。(`documents.rs:2211`)
- ~~🆕 **任意登录用户可改全局 AI 配置 base_url → 服务端密钥外泄 + SSRF**~~ ✅ 已做(200c3b1)—— base_url 钉死服务端配置 / 忽略用户输入。(`ai.rs`)
- ~~🆕 **`files/import-url` 服务端抓取任意 URL —— 盲 SSRF**~~ ✅ 已做(200c3b1)—— 私网/元数据地址黑名单 + 解析后校验(测试 `ssrf_guard_blocks_private_and_metadata_addresses`,files.rs:670)。(`files.rs`)
- 🆕 **可上传携带脚本的 SVG,直开 blob 链接执行脚本**(**降级 low**,2026-07-22 复核)—— 允许 `image/svg+xml`,blob 端点(`blob_inner`)**302 跳存储的 `download_url`**(`public_base_url`/CDN 或 presigned GET,都是**存储源、非 app 源**)→ SVG 脚本跑在存储源、**碰不到 app 的 token,不是账号接管 XSS**。**仅当**运营者把 `public_base_url` 配成与 app 同源才成洞(部署误配)。且 302-跳存储架构下 app 不发字节,强制 attachment 别扭(要么上传即拒 SVG / 存成 text/plain,要么 presigned 加 `response-content-disposition`)。作为「防误配」的纵深项保留,非活跃洞。(`files.rs:350/364/537`)(S) `[需后端]`
- 🆕 **客户端令牌明文存储放大 XSS 后果**(medium / 部分记录) —— web `authToken`+`refreshToken` 明文写 localStorage(任意同源 JS 可读,直接放大分享页 XSS);桌面明文存 prefs(无 DPAPI/secure_storage)。(`prefs_web.dart:6`, `main.dart:475`)(M)(桌面部分见下方「桌面 token DPAPI」)
- ~~**无 refresh / 无撤销的 24h JWT**~~ ✅ refresh + rotation + reuse-detection + `revoke_family`/`revoke_user_sessions` 已落地;access JWT TTL 默认 **24h→1h**(`config.rs`,4a3042a),把「本该失效的 token 仍可用」窗口从 24h 压到 1h(客户端透明续期,无感)。更强的即时吊销(per-user token-version 表)仍可选,但收益已大幅下降。
- ~~**改密不失效旧令牌**~~ ✅ `change_password` 已 `revoke_user_sessions`(`auth.rs:246`);唯一残留是被盗 access JWT 在剩余 TTL 内仍活(同上,靠缩 TTL/token-version 收口)。
- ~~**登录/注册/refresh 无限流**~~ ✅ per-IP 令牌桶 + 全局 Argon2 并发门(`rate_limit.rs`);反代后取真实 IP 走「XFF 从右跳私网」对双跳(Traefik+nginx)/单机都对,自研无依赖。refresh 也纳入 per-IP 限流(但不占 Argon2 门——它不 hash,占了会饿死登录)。**WS 建连有意不限**:已 token 鉴权、低威胁,共享桶会误伤「同时开多文档」——按「不要过度设计」先不做并记因(CLAUDE.md 协作约定)。
- **自托管 TLS 全靠运维 + `HTTP_ADDR` 默认明文** —— 叠加 query token,未配 TLS 即明文泄露,且无启动告警(`config.rs`)。(M) `[需后端]`
- ~~**鉴权逐 handler 手写、非中间件**~~ ✅ 已做(校准复核)—— `auth.rs:618` `scope_guard` 是 router-wide **默认拒绝**中间件 + `is_public` 白名单;新路由默认已鉴权。
- **WS token 走 query string** —— 明文 JWT 落反代日志/浏览器历史(`ws.rs`)。(M) `[需后端]`
- **长连 WS 超 token TTL 不再认证** —— 过期前建的 socket 可授权数小时,无 re-auth 心跳(`ws.rs`)。(M) `[需后端]`
- ~~**CORS 全放行**~~ ✅ prod 默认拒跨源(`cors_layer`,4a3042a),`CORS_ALLOWED_ORIGINS` 放行指定 origin,dev 仍 permissive;顺带修了「prod 一直以 Development 运行」(compose 缺 `APP_ENV`,727ebab)——否则收紧在 prod 不生效。
- **桌面 token 明文存 prefs**(无 DPAPI)(`main.dart`)。(M)
- **开放注册无验证 + 弱口令(仅 ≥8)** —— 公网可无限刷号(`auth.rs`)。(M) `[需后端]`
- 🆕 **安全清单卫生**(low) —— 上面两条已勾除即本轮校准;后续改动请同步勾选,避免半真半假的清单掩盖真未修项。

## 生产运维与备份 🆕

> 2026-07-22 新增小节。节点是单机 docker(阿里云),生产当前处于「盲飞 + 静默失败」态。

- ~~🆕 **备份 sidecar 静默失败无任何告警**~~ ✅ 已做(校准复核)—— `mica-backup-loop.sh:16` 死人开关:成功/失败分别 ping `${HEALTHCHECK_URL}`(healthchecks.io 式),compose 已布线。
- ~~🆕 **Postgres 全库无自动异地备份**~~ ✅ 已做(校准复核)—— `mica-backup.sh:70` `pg_dump|gzip` 进 PGDUMP_DIR、rustic 顺带异地;`Dockerfile.cli` 装 postgresql-client-16。
- ~~🆕 **生产无任何外部探活**~~ ✅ 已做(校准复核)—— `.github/workflows/uptime.yml` cron `*/15` 打 `/api/ready`。
- ~~🆕 **容器 HEALTHCHECK 用不摸库的静态 `/api/health`**~~ ✅ 已做(校准复核)—— `Dockerfile.api:22` HEALTHCHECK + 部署验证均改打摸库的 `/api/ready`。
- 🆕 **磁盘慢渗(降级 low,2026-07-23 复核)** —— 原列 medium,核对后大半已做:① ✅ **日志上限**——compose 5 个服务全走 `*default-logging`(10m×3),最吓人的"日志无限涨"已堵;② ✅ **悬空镜像 prune**——`mica-deploy.sh:139` 每次部署 `docker image prune -f --filter until=168h`。**残留(慢渗、低危)**:③ 旧的**带 tag** 版本镜像累积(上面 prune 故意 NO `-a`、只清悬空、留回滚,每版多 3 个带 tag 镜像几百 MB);④ `/data/mica/pre-*.sql.gz` 手动还原点不自动清;⑤ 无磁盘水位告警。云盘几十 GB、慢渗不急。要做就是 `mica-deploy.sh` 尾部再加"保留最近 N 版镜像 + N 个还原点"。(S)
- ~~**坏迁移的「恢复」流程无文档**~~ ✅ backup.md 加「从 pg_dump 恢复/回滚坏迁移」runbook(停 api→drop/create→zcat|psql→钉旧 tag→health/ready 验证,0d9c404)。
- 🆕 **备份恢复演练纯手动、`rustic check` 不在自动流程**(medium) —— `backup.md:135` 自写「没恢复过的备份只是猜测」,但无 cron/CI/脚本承载,每日脚本也不跑 `rustic check`(OSS 端静默损坏只在恢复那天发现,prune 又最易放大损坏)。修:`rustic check` 进每周节拍,每季度恢复一个 workspace diff 并记日期。(S)
- ~~🆕 **单机兜底部署脚本 `deploy/deploy.sh` 已漂移**~~ ✅ 已做(2026-07-23)—— 对齐 justfile 权威版:`flutter build web` 补 `--no-web-resources-cdn`(修 CN 运行时拉 gstatic CanvasKit 不可用)、删 stale `--no-tree-shake-icons`、rsync→`rm -rf + cp -r`(Windows 无 rsync);`bash -n` 过。
- ~~🆕 **Postgres 大版本升级路径无文档**~~ ✅ 已做(deploy.md 早有升级 section,0d9c404;2026-07-23 补「PG16 上游支持到 ~2028、这是主动维护任务非顺手改 tag」)。
- 🟡 **Traefik:证书监控已做,配置仍不在仓库**(2026-07-29 核实)—— ~~过期无监控~~ ✅:`uptime.yml` 每 15 分钟对两域名(app + s3)openssl 查证书剩余有效期,< 10 天(CERT_MIN_DAYS)即 fail → Actions 失败邮件。**残留**:Traefik 配置本体在仓库外未纳管;ACME 卡死那类故障仍靠 `deploy.md:86` 的手动 runbook。(S,external)

## 数据生命周期与增长 🆕

> 2026-07-22 新增小节。多处「删除不真删」+「无界追加」,单节点小盘上会慢慢暴雷。

- ~~**REST/MCP 写路径从不落自动版本快照**~~ ✅ `apply_derived_operations` 复用 push_update 的 auto 版本 INSERT(同事务、10min cadence、30 天;只写版本归档表、不碰双表示红线,6612330;连真 PG 测试)。
- ~~**删除 workspace 永久泄漏其全部 S3/RustFS 图片对象**~~ ✅ `workspaces::delete` 删库前枚举 `DISTINCT object_key` 逐个删存储对象(best-effort、objects-first,6612330)。
- ~~🆕 **`purge_view`「永久删除」只删 views 行**~~ ✅ 已做(927d7f7)—— `purge_view_subtree` 一条原子 CTE 删 views 子树 + 对 document 型视图删 `documents` 行,DB `ON DELETE CASCADE` 随之清空所有 document_* 表(yrs base/快照/版本/op/**分享 token**);blob 靠 blob_gc 惰性回收(去重共享,不急删)。DB 门控测试锁级联。(`documents.rs` `purge_view_subtree`)
- ~~🆕 **新建 / 打开 / 重命名 workspace 一律 500**~~ ✅ 已修(2026-07-30)—— **从首次提交起就坏的**:`Workspace::page_count` 上写的是 `#[serde(default)]`,而那管的是「从 json 反序列化」,这里没人这么用;派生的 `FromRow` 仍然要求这一列,于是所有不 SELECT 它的查询(create / get / update 走的 `fetch_workspace_for_user{,_in_tx}`)解码失败 → `database error: no column found for name: page_count`。只有 `list` 的 SQL 带这一列,所以列表页正常、单个工作区的三个接口全挂。补 `#[sqlx(default)]`。**为什么活了这么久**:单测抓不到(解码只在真行上发生),而已有的 PG 测试没有一个从「短 SELECT」解码 `Workspace`。新增 DB 门控测试 `a_workspace_decodes_from_queries_that_omit_page_count` 同时解两种形状,并已验证撤掉修复它就变红。(S4 端到端实测时撞出来的,与 op 模型退役无关)
- ~~🆕 **op 模型表无界增长**~~ ✅ **整条关闭(2026-07-30,v0.13.4)** —— 曾经每次 REST/MCP 写入都落一整份 jsonb 全量快照进 `document_snapshots` + 一条 `document_updates`,两表全仓无 DELETE。**六步退役全部做完,三张表已从 schema 删除**(migration 0016),文档内容如今只存在于 `document_yrs_base` 一处。六步留档如下 —— 值得留着,因为每一步都有一个「不这么排就会坏」的理由:
  - ~~**S0 删死代码**~~ ✅ —— 删掉 7 个零调用者函数(文档只点名了 `create_named_version`/`restore_snapshot`,实际排查还有 `list_updates`/`list_versions`/`fetch_version`/`fetch_snapshot`/`fetch_snapshot_by_version_seq`)+ 随之成为孤儿的 `VersionRecord`,共 ~245 行。
  - ~~**S1 补齐「只走 op 模型」的路径**~~ ✅ —— **这是退役真正的拦路虎**:`create_document`/`transfer_view`/`clone_view` 三条只写 op-model 快照、不建 yrs base,全靠 `ensure_base_tx` 的惰性桥在首次编辑时**从快照**现造 base。而那座桥正是退役要拆的东西 —— 直接停写快照,这些文档首次编辑就 404。现已在三处提交后补 `bootstrap_base`(best-effort,与 import 两条路径同款),新文档一出生就有 base。回归测试 `a_created_document_owns_a_yrs_base_immediately`(真 PG 门控)。
  - ~~**S2 存量体检 + 回填**~~ ✅ **已完成并在生产跑完**(v0.13.3,2026-07-29)——
    上线后日志 `yrs base backfill: built 1428 base(s)`,复量 `missing_base = 0`、
    `document_yrs_base` 2307 → **3735 = 文档总数**(1428+2307 一篇不差)。**S4 的前提
    (每篇文档都有 base)现已成立。**原始量数与实现说明:
    **3735 篇文档里 1428 篇(38%)没有 yrs base**,但**全部都有快照**(0 篇两头皆空),
    所以全部可回填;`document_snapshots` 3973 行/22MB、`document_updates` 238 行/440kB、
    `document_yrs_base` 2307 行/24MB。实现 `sync::backfill_yrs_bases`(形状对齐已有的
    `backfill_content_text`:keyset 分页、可重入、坏行 warn 跳过)。**放后台 spawn 而不是
    阻塞启动** —— content_text 那条只走 migration 0012 留空的行,这条要走每一篇缺 base 的
    文档,阻塞会把 `/api/ready` 压到部署误判失败;而且它只是给 S4 铺路,惰性桥今天照常工作。
    并发安全:`ensure_base_tx` 是 `ON CONFLICT DO NOTHING`,和惰性路径抢同一篇也不会重复或损坏。
    **部署时注意**:这是带数据改动的上线(会插 ~1428 行),按规矩先落还原点。
  - ~~**S3 协议版本门**~~ ✅ **已做(2026-07-29)** —— 见「可靠性与同步」同名条目。闸门**默认惰性**(地板 0,谁都不拒),S4 时把 `MICA_WS_MIN_PROTOCOL` 抬到 1 即生效。
  - ~~**S4 停写**~~ ✅ **已做(2026-07-30)** —— `apply_derived_operations` 不再 INSERT
    `document_updates`/`document_snapshots`(yrs base + `workspace_updates` 成为唯一记录),
    `insert_initial_snapshot`/`insert_root_snapshot` 整个删掉,换成事务内的
    `sync::seed_base_tx`:create / import / transfer / clone 四条路径**和 `documents` 行同一个
    事务**建 base —— 比 S1 那个提交后 best-effort 更强,不存在「文档已存在但没内容可读」的窗口。
    `current_payload` 翻成 base 优先、快照仅作兜底(生产已 `missing_base = 0`,该分支预计永不触发,
    留到 S5 删表)。另外两处不改就会坏:`ensure_base_tx` 原先无快照即 404,现改为从
    `documents.root_block_id` 种空页;WS `send_bootstrap` 原先 `latest_snapshot(...).ok_or(())?`
    —— 停写后**每个新文档一连就断**,现发 `snapshot: null`(客户端本来就只取 `connection_id`)。
    回归测试 `a_write_no_longer_grows_the_op_tables`(写前写后各数一次两表行数 + 确认内容真落了)、
    `a_document_with_no_snapshot_builds_its_base_from_the_documents_row`。**实测**:本地跑 0.13.3
    走完注册→建页→写→导出/大纲/搜索/重开→WS bootstrap→import→clone,全工作区
    `docs=3 bases=3 snapshots=0 updates=0`。**注**:`seed_document` 那批夹具没挂 —— 快照兜底接住了,
    它们现在测的是「遗留文档仍可读」这条路,故意留着。
  - ~~**S5 删表**~~ ✅ **已做(2026-07-30)** —— migration `0016_drop_op_model.sql`。删前量到的生产存量:
    `document_snapshots` 3973 行 / **22 MB**(整库 71 MB 的三分之一)、`document_updates` 238 行、
    `document_versions` **3 行**。三者都不含可达数据:快照是 base 的冗余副本(S2 回填已把每一份折进
    base);`document_updates` **零读者**(`sync::pull_document_updates` 读的是 `workspace_updates`,
    同名不同表);`document_versions` 是 op 时代的命名检查点,被 0009 的 `document_yrs_versions`
    取代后**再没有任何代码读过**,那 3 个名字在产品里早就看不见了。**DROP 顺序固定**:
    `document_versions` 必须先删,它的 `snapshot_id` 是 `ON DELETE RESTRICT`,不然删快照表会被依赖挡住;
    刻意不用 `CASCADE` —— 本 schema 最具破坏性的一次迁移,不该带「静默」这个属性。
    读侧同批撤掉:`current_payload` 的快照兜底、`ensure_base_tx` 的快照分支、`latest_snapshot{,_tx}`、
    两个 DB 行结构,以及 `backfill_yrs_bases`(它的源表就是被删的那张,留着只会给无 base 的文档造空页)。
    回归测试 `the_op_model_tables_are_gone`(直接查 `information_schema`,谁再把表建回来就红)+
    `a_write_round_trips_through_the_base_alone` + `a_document_with_no_base_opens_as_an_empty_page`。
    **改造夹具时挖出两个哑测试**:op 快照把 `data` 当 jsonb 逐字返回,所以 backlink 夹具里那些
    **没有 `start`/`end` 的 link mark** 照样能被扫到;yrs 把 marks 存成文本 format,`marks_from_data`
    会丢弃无区间的条目 —— 补上真实区间后,`a_self_link_is_not_a_backlink`(断言「为空」)才从**空转通过**
    变成真的在测那条过滤。
  退役完成。(M) `[需后端]`
- 🆕 **无任何容量配额**(medium) —— 唯一限制是单文件 25MB + 导入 1GiB body;无 workspace 总量/单文档大小/用户级上限,WS 路径默认可收 64MiB 单条消息,大文档写放大(每 push 全量 base 覆写 + 每 10min 全量版本)。开放注册单节点最易被无意/恶意打爆盘。(`storage.rs:50`, `ws.rs:60`, `sync.rs:244`)(M) `[需后端]`
- ~~**`document_yrs_versions` 过期清理只挂在「该文档自己 push 撞 cadence」**~~ ✅ blob_gc 6h 循环加全局 `DELETE ... expires_at IS NOT NULL AND < now()`(只命中 auto、不碰命名检查点,6612330)。**残留**:`list_yrs_versions` 仍不过滤 expires_at(6h 扫前的过期行可能短暂现于面板,极小)。
- ⏸️ **回收站不做自动清空 —— 已拍板(2026-07-29),不是遗留 TODO**。现状:纯 `is_deleted` 标志,只有用户手动「恢复 / 永久删除 / 清空回收站」三个出口(`documents.rs` 的 trash/restore/purge + `mod.rs:164` 那条 DELETE),**没有任何定时任务**碰它;blob GC 刻意把回收站引用算存活(`blob_gc.rs` `referenced_file_ids` 不过滤 `is_deleted`),所以图片 blob 跟着一起留。**决定不做自动过期删除**,四条理由:
  1. **同类一致**:AFFiNE、AppFlowy、Notion、Obsidian 在「用户自己是唯一管理员」这个相同约束下**都没有**做"回收站到期自动永久删除"。AFFiNE 的 blob 宽限期只针对已失活 blob,页面本身不过期;AppFlowy 干脆没有 blob GC。这是跨产品收敛的答案,不是巧合。
  2. **它解决不了真正的风险**。真风险是"单节点小盘 + 开放注册被撑爆",根源是**没有容量配额**(见本小节「无任何容量配额」条)。用一个会误伤用户数据的机制去掩盖一个该用配额解决的运维问题,是治标且引入新的不可逆风险。
  3. **代价不对称,且备份补不上**。每日全库 `pg_dump` 含回收站的文本与 CRDT 历史,但**不含 blob**(第二对象存储)。自动清空后就算靠备份找回文字,配图也真的没了 —— 对自托管的单管理员,这个恢复成本大概率承担不起。
  4. **手动出口已经齐备**:`_confirmPurgeAll` / 单条永久删除都带二次确认,要腾空间今天就能腾,不需要系统替用户决定。
  **配套(小,未做)**:回收站占用可见性 —— 照 `export_all_stats` 现成模式统计"回收站里几页 / 独有 blob 多少字节",显示在回收站对话框标题栏,让用户自己看见再决定。这比替他自动删更符合上面四条。(S)
- ~~🆕 `blob_gc.rs` 注释预设了一个不存在的「回收站保留期」~~ ✅ 已修(2026-07-29)—— 原文写 effective margin 是 "recycle-bin retention + this",而回收站根本没有 retention 这个量;改成如实说明它是 [0, 用户手动清空) 的不定时长。
- ~~**`refresh_tokens` 只增不删**~~ ✅ blob_gc 6h 循环加 `DELETE ... expires_at < now()-7d`(6612330)。
- ~~🟡 **账号删除功能不存在**~~ ✅ 已做(18300d1,2026-07-23)—— `delete_account` 事务级联(密码门控 + 跨他人 workspace RESTRICT 阻塞回滚 409),详见「产品与公开发布合规」小节;级联顺序备忘 deploy.md 早有(0d9c404)。
- 🟡 **导出不含回收站;备份已含其文本、独有 blob 仍漏**(2026-07-29 更正)—— 导出确实过滤 `is_deleted=false`;但 2026-07-22 起每日备份走**全库 pg_dump**(`mica-backup.sh`,label=_pgdump),回收站页面的文本+CRDT 历史**在备份里**,原「备份里最后副本被 prune」的推演已不成立。**残留**:回收站页面独有的图片 blob(第二对象存储,pg_dump 不含)。(S)

## 编辑器与功能广度

- 🟡 **全文搜索**(2026-07-22 复核:原描述失实——不是「反序列化每篇快照」,是每查询把每篇 yrs base **全量 CRDT 解码**一遍,N 次 decode)—— **M1 已做**(aa4c5d8):加 `document_yrs_base.content_text` 派生列(migration 0012),搜索退化为**一条 LEFT JOIN + ILIKE SQL**,干掉 N 次 CRDT 解码。content_text 是 state 纯投影、三条写 base 路径同语句 co-write(红线#1 不漂移);启动一次性回填存量;LIKE 转义 + 命中处窗口 snippet;CJK 走子串(无扩展/无分词器)。**残留**:~~②导入未打开的文档正文不可搜~~ ✅(2026-07-22,导入两条路径 commit 后 best-effort `bootstrap_base` 即建 base+content_text,等价「导入即打开」,复用现有构建器无新写路径;回归测试锁住;postgres:16-alpine 自带 pg_trgm 已确认)。**故意缓做**(非尾巴,按「不过度设计」):① **pg_trgm GIN 索引**——当前 22MB 库、查询已被 (workspace_id,is_deleted,object_type) 索引收窄再 ILIKE,亚毫秒;GIN 只加速 ≥3 字子串(CJK 2 字仍 seq-scan),不是干净胜利。真到大规模(万级文档)时一行 `CREATE EXTENSION pg_trgm` + `CREATE INDEX` 升级,现在加是为假想规模优化;② 排序/高亮/分词——各自独立 UX 特性,另立项非 M1 尾巴。(各 S–L) `[需后端]`
- 🟡 **表格**(2026-07-22 复核:原描述大幅失实)—— 实测:**富行内单元格**(粗体/斜体/行内代码/链接,cell 存可重解析 md 源码、两端渲染+编辑,`cellDisplaySpan`/`CellEditController`)与**矩形/行列选区**(跨格拖选、点行/列把手选整行列、Ctrl+C/X 复制为 TSV+HTML、Delete 清空、Esc 清除)**本来就能用**;本轮仅补 **Shift+点击扩展选区**。**合并单元格有意不做**——8 家同类(Notion/AFFiNE/AppFlowy/Outline/siyuan/Joplin/logseq/anytype)调研定论:合并与「Markdown 权威 + round-trip 不变量」在 GFM 下**架构级互斥**(siyuan 能合并因它放弃了 md 权威;Joplin 同约束只能冻单向 HTML;Logseq/Notion 干脆不做)。要做只能另开 HTML 逃生舱块退出 round-trip,是独立决策。块级单元格/列宽 GFM 表达不了,同样不做。
- 🟡 **反向链接/引用面板/关系图** —— 正向 `[[` 已建;**引用面板已做**(云端页显示「谁链到我」可点跳转,`GET .../backlinks` 按需扫描、复用 page_link_targets,7de2c2a)。**残留**:~~①并发扫描~~ ✅(buffered(8),6612330);②规模成瓶颈再上维护式反向索引表(现在故意不建);③本地世界(offline)反链;④**关系图**(graph view)。(各 S–L)`[需后端]`
- 🟡 **页面属性/标签**(**M1 已完成**,2026-07-22)—— 走 front matter 权威路(调研定论:同类 md 权威系均如此,见 `docs/page-properties.md`)。**M1 全部落地**:① 数据/权威层——Rust `crates/markdown/src/properties.rs`(解析扁平子集 + 类型推断 + 外科式写回,round-trip 不变量经用户批准从字节保真降为规范化子集稳定)+ Dart 镜像 `properties.dart`,两端逐条测试一致(Rust 9 / Dart 10 全绿);② 页头属性面板 `property_panel.dart`(读 root 块 `data['front_matter']` → 类型化编辑:文本/数字/日期文本框、勾选、tags chips 增删 + 增/删属性 → 编辑经 `onApplyOperations` 单入口自分派写回 root 块,local/cloud-CRDT/cloud-REST 三模式通用,无需穿层新回调);flutter build windows 通过。tags = `tags:` list 属性。**Obsidian-lite 闭环已完成**:增删改属性(类型 text/number/checkbox/date/list)、tags chips、**可搜**(属性值折进 content_text,list 值以 `#值` 存)、**tag 点击精确跳页**(搜 `#值` 只命中真正带该标签的页,ce13cef)、**默认隐藏在页头 ⓘ 图标后**(不占版)+ **AppFlowy 式面包屑路径**(579272f)、AFFiNE 式紧凑面板(7379444)。**故意不做/另立项**:① 数据库视图级「按属性筛选/排序/看板」——是 Notion 数据库那套,与 markdown 权威+round-trip 架构互斥(要豁免 md 权威,AFFiNE/siyuan 路),独立大决策;② 存量页要下次编辑才索引属性(backfill 只填空行,属性是新功能故不强制全量重派生);③ 日期选择器 UI(现文本输入)。**数据库视图(带类型列/筛选/relation)另立项**——与 markdown 权威+round-trip 架构互斥,要么破双表示红线要么豁免 md 权威(AFFiNE/siyuan 路),是独立大决策。(L) `[需后端]`
- 🟡 **评论**(2026-07-26:**服务端 + 渲染 + API 三层已闭环,剩面板 UI**)—— 锚点=yrs sticky index 存独立表(`comment_threads`/`comments`,migration 0014),**正文 Markdown 一字不动 → round-trip 红线零改动、评论永不进导出**。已落地:锚点原语 `sticky_for_range`/`resolve_range`(2672201,8 单测)+ store 层与 5 个端点(354f946,gated on `commenter`——能评论但不能改正文)+ **Postgres 集成测试 8 项 CI 真跑**(736639c,含"锚点经 push_update 落库后仍随文字位移")+ 客户端 API 层(af03743,6 单测)+ **渲染期高亮**(08f221d,**纯 paint、绝不 relayout**,5 widget 测证明 caret 几何不变)。**Phase 1 已闭环**(f3bf181):评论面板(`ui/comment_panel.dart`,9 widget 测)+ 右键「添加评论」(offset 用 UTF-16、只看 `onAddComment` 不看 `canEdit`,故 commenter 能评论不能改正文)+ main.dart 接线(`onReady` 拉取 → 过滤 `isHighlightable` → `commentHighlights`;变更后重拉,服务端是锚点/orphan 唯一真相源;拉取失败只是没评论、文档照常打开)。**残留仅观感**:Dialog 形态/面板宽度/图标位置/高亮浓度/跨块高亮 需 `just app` 或发版后真机看一眼(清单见 `docs/comments-plan.md`「待真机确认」)。**实测修正了设计假设**:yrs 保留 tombstone,删掉锚定文字后锚点**仍能解析、只是塌缩成零长** → orphan 判定必须"解不出 **或** 塌缩"两者同等对待(只信 None 会漏掉最常见的删除情形)。**建议(suggest mode)**仍有意另立项(正文内 overlay,与评论不共用存储)。(残留 M)`[需后端]`
- 🆕 **本地工作区没有工作区级搜索**(medium) —— 云端搜索靠 `document_yrs_base.content_text`
  派生列 + ILIKE(见上条),本地库没有对应投影,`onSearch` 原本接的是返回 `const []` 的
  空实现 → 每次本地搜索都答「没有找到与「x」匹配的内容」,把"页面不在这个工作区里"当
  事实说了出来。**已先止血**(改成 null-means-absent + 诚实文案 + 指向页内 Ctrl+F,
  6668976 一批),真做需要本地 store 侧同样维护一份正文投影并给 FFI 查询接口 ——
  和云端那条是同一套设计,不是两套。(M)
- 🟡 **结构块 callout/toggle/embed/columns** —— **callout 已做**(GFM alert `> [!TYPE]` 5 类型,复用 quote 扁平模型、round-trip 干净、记分牌未降,e7ff038)。**残留/定论**(2026-07-22 调研):① **toggle** —— **已拍板(2026-07-29):分两步,先做渲染层折叠 UI**。关键区分:`<details><summary>` 是**合法的 CommonMark/GFM raw-HTML block**(GitHub 官方推荐写法),属于"我们的解析器没接住",**不是**像合并单元格/多列那样"标准 md 表达不了"——所以**不适用**那两条的「红线不做」先例。现状比参照产品还弱一档:AppFlowy(原生 `toggle_list` block)和 AFFiNE(`collapsed` 做成 list/heading 通用属性)**编辑器里都有可点的折叠 UI**,只在导出 md 时降级;我们是连折叠 UI 都没有,`<details>` 源码当代码块摆着。**第一步(S,建议先做)**:纯渲染层——`code_block`+`raw:true` 的解析/序列化**一个字不改**,只在渲染层认出规范形态的 `<details>` 就换折叠外观,走 `AtomicBlockRenderer` 注册表(不堆 if,守渲染红线);折叠态复用现有 `data.collapsed` 那条路。round-trip 零改动。**第二步(M–L,另议)**:规范子集结构化成新 kind(summary 富文本 + body 复用 `data.li` 扁平容器),非规范形态退回现状直通;成本大头在教导入器反解析真实世界五花八门的 `<details>`。**顺带定论**:折叠状态**是文档数据不是视图状态**——AppFlowy(`updateNode` 事务)、AFFiNE(`store.updateBlock`)、以及本仓库自己的代码块折叠先例(`controller.dart` `toggleCollapsed` 走 `update_block`,但 `collapsed` **从不进 md 字节**)三方一致;`<details open>` 让我们有机会比前两家更彻底(折叠态也能 round-trip),但那意味着"点一下折叠"变成一次真实文本编辑,留到第二步再定;② **columns** —— **红线不做**(标准 md 无多列表示,同表格合并;要做只能显式有损方言);③ embed 未做。附:render 注册表 P3-1 对这三种块**不是前置**(仅撞已有 kind 如 Graphviz 时才需)。(各 S–L)
- **无屏幕阅读器语义(a11y) / 无 RTL 双向文本** —— 自绘 RenderBox 无 Semantics;10+ 处硬编码 `TextDirection.ltr`(editor-engine, `render.dart`)。缓解:设置里有 85–140% 应用内字号(`EditorAppearance.fontScale`),覆盖低视力一部分。(各 L)
- ~~无暗色模式~~ ✅ **已做**(v0.13.2,2026-07-29):语义色 token 层(`ui/theme_tokens.dart`,AppFlowy 式角色分组)贯穿外壳 / 自绘画布(挂 `EditorAppearance.tokens`)/ 语法高亮(`_Rule` 存角色,emit 时解析)/ mermaid(merman host theme + mermaid.js themeVariables);跟随系统或设置手切,冷启动在 runApp 前就位。**位图与记忆化缓存的 key 都带调色板**(公式/mermaid 栅格、代码 span memo),否则浅色下烤出的产物会画到深色页上。
- ~~**文档内查找/替换缺失**~~ ✅ Ctrl+F 查找栏(导航/计数/当前匹配高亮)原已具备;2026-07-22 补齐**替换**(`replaceRange`/`replaceAll` 走既有 op 路径,9fe9ae8)+ F3/Shift+F3。**全部匹配高亮**有意不做(要动 render.dart 加第二遍选区叠绘,超 MVP)。
- ~~**行内数学未排版**~~ ✅ 2026-07-16:`$…$` 真排进行里(基线对齐、随字号缩放),公式为不可进入的原子(`inline_atoms.dart`,render-architecture.md Decision 4)。
- **Web IME/光标滚动实况调优** —— Milestone 1 遗留(合成态/游离换行、caret scroll-into-view)。(M)
- **AI 离线为空 stub / 无拼写检查**;~~字数统计~~ ✅ 已做(右下角角标,253c53f)。(M / M)
- 🆕 **集成 AI 升级为「能操作文档的 agent」(候选,已调研背书,2026-07-23)** —— 现
  `/ai/complete`(`ai.rs`)只是**单次 prompt→Markdown 补全**,无 tools/agent 循环;而
  mica MCP(`mica-cli mcp`)是给**外部** agent(Claude)的工具面,两者不共享——集成 AI
  **天然不具备** MCP 能力。候选:把集成 AI 升级成 tool-use agent,能读/写/组织自己的文档。
  **调研定论**(派子代理扒了 Notion/AFFiNE/AppFlowy/Coda 等 + 框架):① "内置 AI 变
  agent"是主流路,**Notion 3.0 Agents(2025-09)是标杆**(用户权限内、多步、可回滚);
  ② "**一套工具对外 MCP + 对内 agent**"机制成熟——**Block `goose`** 自家 agent 就跑在自家
  内建 MCP 上,Vercel AI SDK / LangChain / OpenAI Agents SDK 都把"MCP 工具装进自家 agent"
  做成一行 API;**但笔记品类无公开先例**(Notion 同时有 agent + MCP server 但未证实共享注册表)。
  **架构选型**:后端保留**一套操作层**,`MCP 面`(外部通用、探路、啰嗦 Markdown)和`内置
  agent 面`(紧凑、预 scoped、认识当前会话)当作它的**两个薄投影**,**不要把 MCP 工具原样复用**。
  **三个坑(均有引用)**:①MCP 工具为外部通用 agent 设计→太粗太啰嗦、费 token、稀释工具选择
  (Datadog MCP tool design);②agent 循环比一次性补全**更慢更贵**(多轮+schema 开销);
  ③鉴权用**用户真实会话身份**(非 MCP 服务账号 scope)+ **写前确认 UX**(Notion Custom
  Agents 同规)。(L) `[需后端]`

## 平台覆盖

- **无触屏选择手势** —— 无长按选词/选择手柄/放大镜,手机端文本选择基本不可用。(L)
- **Windows 未签名(SmartScreen 告警)** —— 路径:SignPath CA 证书接入 Inno SignTool(desktop-plan)。(M)
- 🟡 **自动更新器不校验下载完整性/哈希/签名**(medium) —— **完整性校验已做**(508808e):下载后经纯函数 `installerMatches` 验 `size`(GitHub `assets[].size`,恒有→拒截断)+ `sha256`(`assets[].digest`,GitHub 服务端算→拒换包/损坏),不符即删+`updaterIntegrityFailed`,绝不运行;无 digest 老 release 退化仅 size。**残留**:① **Authenticode 签名**——安装包本身仍未签名(需代码签名证书,独立项);② digest 依赖 GitHub 是否为我方资产填充(未填时仅 size 兜底),要更强可让 release CI 自发 `SHA256SUMS`。(`updater_desktop.dart` `installerMatches`)(证书项 M / 其余已做)
- **window_manager→nativeapi / Turso 观望**(各 S,已隔离在 trait 后)。

## 客户端质量与兜底 🆕

> 2026-07-22 新增小节。离线功能面做得全,但崩溃/损坏/双开几处兜底缺失会真丢数据。

- ~~🆕 **客户端零崩溃/错误上报**~~ ✅ 已做(校准复核)—— `main.dart:149` `runZonedGuarded` + `FlutterError.onError`,未捕获异常落盘 diagnosticsDir。
- ~~🆕 **本地世界文档损坏 → 静默变空白且自毁恢复检查点**~~ ✅ 已做(校准复核)—— FFI `store.rs` `load_doc` 区分 None/corrupt(throw);`local_doc.dart:56` 捕获即 rethrow `LocalDocCorruptException`,不再 seed+saveDoc+checkpoint(§10 回滚网不再被冲)。
- ~~🆕 **桌面无单实例守卫,双开丢本地文档**~~ ✅ 已做(校准复核)—— `windows/runner/main.cpp:24` `CreateMutexW("Local\\MicaSingleInstance")` + `ERROR_ALREADY_EXISTS` 守卫(fail-open)。
- ~~🆕 **退出路径漏掉编辑器 400ms 防抖文本**~~ ✅ 已做(校准复核)—— `main.dart:1016` `_flushForExit` 先 `await _activeEditorFlush()` 再冲会话/后端。
- ~~🆕 **`prefs.json` 非原子写 + 损坏静默清空**~~ ✅ 已做(校准复核)—— `prefs_stub.dart:64` 写 `.tmp` 后 `renameSync`(同卷原子,含 Windows 覆盖处理)。
- ~~🆕 **编辑器 op 管道 `catchError((_){})` 吞掉本应浮出的 outbox 写失败**~~ ✅ 已做(校准复核)—— `controller.dart` 现 `opFaultCount++` + `onOpFault?.call` 上浮,不再吞(红线 #1)。
- ~~🆕 **云文档离线/未同步状态零指示**~~ ✅ 2026-07-26 完成(69ff98f 地基+信号 / 8e1318d 徽标 / 6832dea 心跳)—— 扒了 8 家(AFFiNE/SiYuan/Logseq/Anytype/Google Docs/Notion/Obsidian/AppFlowy)后定**最小形态**:三态克制徽标(已同步→**什么都不画**、同步中→faint 慢转圈、离线→cloud-off + tooltip),摆文档面包屑右上、**仅云工作区**显示。**不做数字计数**(同类无一家做)、**不可点击/不做手动同步**(AFFiNE/AppFlowy/Anytype 同样没有;mica 本就自动重连 + 自动 flush)。信号从 `CloudSyncSession` 四个真实转移点 emit,推导是纯函数 `deriveSyncPhase`(`sync_status.dart`,4 单测)。**关键补丁**:加了**心跳**(8s ping + 20s 帧静默看门狗)——否则拔网线是 TCP 半开、不发 WS close 帧,`_onDone` 永不触发 → 一直误判在线(用户实测拔线发现徽标不动才暴露);服务端 `ws.rs:267` 本就 `ping→pong`,零改动。〔"别人都没做"的印象来自 AppFlowy:它的 `sync_indicator.dart` 当前是**死代码**(重构后未挂载),且有未关闭的需求 #5729 求做回。〕
- 🆕 **i18n 漏网**(low) —— 默认页名 `kUntitledPage='未命名页面'` 硬编码中文并持久化(英文用户新建页得到中文标题、且与 'Untitled' 双轨),代码块 AI 动作 prompt 全中文;语言仅 en+zh。(`models.dart:667`, `editor.dart:5109`)(S)

## 性能

- ~~**长文档性能**~~ ✅ **性能线已闭环**(2026-07-23)—— 设计 `docs/editor-virtualization-plan.md`。三刀叠加后每击键 = O(改动块)真推导 + O(N) 平凡重定位:**Phase 1**(da25075)painter 缓存复用,干掉逐帧 dispose+重建全部 TextPainter;**代码高亮记忆化**(7fe1997)未变代码块不再重新分词;**Phase 2**(b750d88)整块 layout 缓存 `_layoutCache`,未变块跳过 marks/span/高亮/rect 全部推导只 `shiftBy` 重定位,dirty 判定用 identity(实证 controller 只重赋值不原地改 text/data)。回归 `test/{painter_cache,code_span_memo,layout_reuse}_test.dart`,全量 728 通过。**残留(L,有意不做)**:真·视口虚拟化(屏外跳过排版)——两条架构约束(performLayout 无滚动偏移、编辑器不自管视口)使其为独立架构项,ROI 仅万级块/超长档,剧本留档待需。
- ~~**图片纹理缓存无逐出策略**~~ ✅ `_imageCache` 改 LRU(64 上限,每帧 touch 可见图、逐出屏外静态图并 dispose,守 lessons.md §5 dispose 时序,253c53f)。
- **每次 push 重建+重编码+重写整档(写放大)** —— `from_update`→全档 `encode_state`+upsert,成本 O(文档) 而非 O(更新)(`sync.rs`)。(M) `[需后端]`
- **yrs base 无 squash/GC,无界增长** —— 只裁 stream 不压 base,长寿文档 base 越滚越大(`sync.rs`)。(L) `[需后端]`
- 🟡 **本地持久化:云文档已增量,纯本地文档仍全量**(2026-07-29 更正)—— 云端(在线)文档早已 append+squash(`store_cloud_doc_store.dart` appendOutbox/appendUpdate/compact,每 32 次 append 检查、日志 >256 squash;store 层 `append_update`/`squash` 全备)。「仅全量快照」只剩**纯本地(离线)文档**的 `local_doc.dart` debounce saveDoc 路径,接线即可。(S/M)
- **frb v2 热路径 FFI 基准待测** —— IME/逐字输入若过慢,热路径留 Dart(phase2 §12)。(M)

## 开发者体验 / CI / Markdown

- ~~🆕 **api-server 全部测试不进 CI;DB 测试本地也静默跳过**~~ ✅ 已做(校准复核)—— `ci.yml` 已有 postgres service + `-p mica-api-server`(测在 CI 实跑);`auth.rs:895` `pool()` **已带 sync_pg.rs 同款 CI-assert**(`assert!(env CI is_err, "DATABASE_URL unset in CI...")`)→ 缺库在 CI 里 panic、本地才 return None 跳过,那些 `else{return}` 在 CI 不会假绿。(校准注:审计曾误判为残留,因只看了 `else{return}` 调用点、未读 `pool()` 定义——同其 #1 的错。)
- ~~🆕 **页树不变量守卫 `ensure_parent_accepts_children` 零自动化测试**~~ ✅ 已做(校准复核)—— `documents.rs` `parent_guard_pg` 测 folder 接受/page 拒绝/缺失父 + 触发器 backstop(真 PG 门控)。
- ~~**Release 出的 Windows 安装包从未被自动安装-启动验证**~~ ✅ release.yml 加「安装-启动冒烟」(/VERYSILENT 装 + 启动 + 存活 10s + finally 清理,发布前拦,0d9c404)。**2026-07-23 根治 flaky**:冒烟测撞单实例 mutex 竞态偶发假失败(安装器 `[Run]` 自启一个 + 测试又自启一个,谁后抢到 `Local\MicaSingleInstance` 谁 `exit 0`;`[Run]` 触发时机随机)。结构性解法(ShareX 同款):`mica.iss` 的 `[Run]` 加 `Check: not CmdLineParamExists('/SKIPRUN')`,CI 安装传 `/SKIPRUN` → 安装器不自启 → 测试是唯一启动方 → mutex 永不争用,竞态从结构上消失(9c006e6),0.12.16 真 CI 跑绿实证。〔sccache 曾加在 windows job(b5e7f04)后于 v0.12.18 撤除:tag 触发的 job 命中率恒 0%(GHA cache 按 ref 隔离、release 只在 tag 上跑),且长杆是 Flutter 构建非 Rust——详见 `docs/lessons.md`。〕
- 🟡 **CI 补 Windows 集成测试**(2026-07-23:离线子集已进)—— 新 `.github/workflows/flutter-integration.yml`:windows-latest **串行**跑 **14 个离线/客户端**集成测试(文件间杀 `mica_flutter.exe`,化解单实例守卫导致的 debug-connection race——已复现:残留进程锁住下次启动)。**残留**:4 个需活的 dev 栈(postgres+rustfs+api)的测试仍排除(`cloud_sync_test`/`migration_sync_test`/`offline_image_reconcile_test`/`page_switch_fidelity_test`,含那对 race 文件)——CI 里起全栈超范围。且 12/14 是读文件头判定离线(实跑了 2)、首次 CI 真跑确认。(M) `[需后端]`(残留部分)
- 🟡 **e2e:桌面已进 CI,web 仍为零**(2026-07-29 更正:「全项目零 e2e」失实)—— 桌面 14 个离线集成测试已在 windows-latest **真 app 实跑**(flutter-integration.yml,2026-07-23 起,那就是 e2e);web 端仍零(无 spec,CI 只验能编译;playwright 截图是人工手段)。(web 侧 L)
- 🟡 **不可信输入解析面 fuzz**(2026-07-23:markdown + interchange 已上,yrs 待)—— 三个吃不可信字节的面:markdown 解析、ZIP 导入、yrs 二进制更新。**已做**:proptest 属性 fuzz 覆盖前两个自家解析面——`markdown/tests/proptest_parse.rs`(`import_markdown` 灌任意字节 + markdown-ish 片段,never-panic)+ `interchange/tests/proptest_zip.rs`(`read_zip→normalize_entries→expand_nested_zips` 灌任意/PK-前缀字节)。本轮**未挖出 panic**(解析器稳),但落成**快回归门**(各 ~2–5s,随 `cargo test` 进 CI;`PROPTEST_CASES=100000` 可本机长跑)。**残留**:yrs 二进制更新那面——手写 xor 已实证挖出远程可达(需认证)UB,但 UB 要 **cargo-fuzz + sanitizer(ASan)** 才抓得住,proptest(只抓 panic)不够 → 留 Linux/CI 的 cargo-fuzz。(`store.rs:2202`)(M)
- 🆕 **本地 SQLite 真库升级冒烟不在发版清单**(medium) —— `upgrade_real_store_smoke`(`#[ignore]`+需手动设 `MICA_REAL_STORE`)是发版前手动步骤,但 `release.md` 全篇不含其字样 → 发版流程不会触发任何人想起它;而桌面自动更新后首启就地迁移本地库,迁移写坏=用户笔记不可见。(`store.rs:2083`, `local-first-p3-design.md:288`)(S)
- 🟡 **cli 测试 + 覆盖率度量**(2026-07-23:起步)—— 原 `crates/cli` 零测试 + 无覆盖率工具。**已做**:9 个纯逻辑单测(`url_file_name`/`slugify`/`sanitize_rel` 路径防穿越/`workspace_dir`/`mirror` 备份 reconcile 增删剪/`Config` serde),`ci.yml` 测试步补 `-p mica-cli`(进 CI),`just coverage`(`cargo llvm-cov`,不入 CI 门)。**残留**:REST `Client` 方法需活服务端未测;`config_path/load/save` 走进程级 env + 真实用户配置目录,未注入点故略(用 serde 落盘形状覆盖)。覆盖率数字化了但远非高覆盖。(S)
- ~~🆕 **`just test` 漏 `--features store`**~~ ✅ 已做(校准复核)—— `justfile:133` 已有 `cargo test -p mica-core --features store`。
- 🆕 **Linux 桌面在仓库但从不在 CI 构建**(low) —— `linux/` runner + 托盘降级逻辑在库,CLAUDE.md 还为它写了约束,但 CI/release 都无 `flutter build linux` → 编译债不可见。flaky 债本身很轻(仅 2 个带理由 `#[ignore]`)。(M)
- **仅结构化日志,无 /metrics/telemetry** —— 同步后端生产盲飞(`telemetry.rs`)。(M) `[需后端]`
- **可选/later 基建:Redis、OTel、索引块表** —— 索引块表是搜索/反链/分析的底座(architecture.md)。(L) `[需后端]`
- **自研 parser vs 采用 comrak(读侧)未决** —— Milestone 8 决策点(editor-engine)。(M)
- **catch-up limit / stream 常量硬编码** —— 1000、KEEP_MARGIN/PRUNE_EVERY 应入 AppConfig(`ws.rs`)。(S) `[需后端]`
- ~~**过时注释/文档批量清理**~~ ✅ 大部分(2026-07-23)—— 改了 4 处确认为 stale 的:`model.dart`×2(表格/void 节点/marks 已落地)、`preview_raster.dart`(web mermaid 已 JS interop)、`mica-core/lib.rs`(本地 SQLite store 已在 `store` feature 落地)。`main.dart` 的 M4/M5 保留——它们是准确的里程碑出处标注,非"没做"声明。

## 产品与公开发布合规 🆕

> 2026-07-22 新增小节。生产节点已上公网 + 开放注册,这类义务是上线后才暴露的。
>
> **2026-07-23 进度**:三项 medium 硬缺口(关注册开关 / 账号删除 / 密码找回,后者顺带
> 建起邮件底座)已全部落地并发版 0.12.16 上线端到端验证。剩下均为 low:AGPL 源码入口、
> 隐私声明·条款、OFL.txt 随附。

- ~~🆕 **AGPL-3.0 但客户端无「获取源代码」入口**~~ ✅ 已做(2026-07-23)—— About 弹窗加「源代码(AGPL-3.0)」链接,点开 github.com/weironz/mica(版本号已在弹窗上方,可对到 tag);复用 in-house `openUrl`(无 url_launcher 依赖),web/桌面同一 About 段共享故两端都覆盖。满足 AGPL §13 向远程交互用户显著提供 Corresponding Source。(`dialogs.dart` `_showAboutDialog`)
- ~~🆕 **无账号自助注销/数据删除入口**~~ ✅ 已做(18300d1 后端 + 43b4dae 客户端,2026-07-23)—— `DELETE /auth/me`(密码门控)事务级联删本人拥有的全部 workspace(内容随 `workspace_id` CASCADE)+ tokens/成员;跨他人 workspace 的协作内容(RESTRICT)阻塞时整事务回滚返 409。设置里危险按钮 + 密码确认弹窗 + 成功后登出。DB 门控测试锁级联(方案 A 全删,被遗忘权)。
- ~~🆕 **无密码找回/重置,忘密码=永久锁死**~~ ✅ 已做(5680547,2026-07-23)—— `POST /auth/password/forgot`(恒 204,不做账号枚举 oracle)+ 服务端渲染无 JS `/reset-password` 页(token 单次性、存 sha256、1h 过期、条件 UPDATE 花掉、改密撤销全会话)。**邮件底座就此建立**:`infra::Mailer` trait(默认 LogMailer,把链接打日志、无服务商也能跑)+ 阿里云 DirectMail 实现(SingleSendMail v1 RPC HMAC-SHA1 签名,复用 reqwest,不引 lettre),`MICA_MAIL_BACKEND=directmail` 切换、缺项 WARN 回退。端到端实测收信 + 重置通过。见 `docs/password-reset.md`。
- ~~🆕 **开放注册无法关闭**~~ ✅ 已做(39ac1ee,2026-07-23)—— `MICA_REGISTRATION_ENABLED=false` 让 `/auth/register` 返 403(login/refresh 不受影响);默认开、只认显式 off 值(`false/0/no/off`)防误锁。运营者一个开关把公网节点转私有。**残留**:邮箱验证/验证码/邀请制仍无(找回已带来邮件底座,验证可后续叠)。
- 🆕 **已上线实例无隐私声明/服务条款**(low) —— 正面:诊断 opt-in 默认关、无 telemetry 回传,产品内隐私姿态好;缺口是外部合规面,仓库无任何面向用户的隐私政策/条款文本。(M)
- 🆕 **打包 Noto Sans SC 走 OFL 1.1 但没随附 OFL.txt**(low) —— `fonts/NOTICE.md` 自写「include the full OFL.txt alongside for strict compliance」,但 fonts/ 只有 NOTICE.md,OFL 要求许可证正文与字体一同分发。(`fonts/NOTICE.md:9`)(S)

## 接下来最该做的(2026-07-29 重排)

> 本轮 7 路并行对代码逐条核实 53 项候选:翻掉 **5 个整条误报**(P2-M4 云同步流、撤销/重做按钮、版本富预览、登录页服务器选择器、证书监控)+ **6 处部分失实**,挖出 **1 个真 bug**(重连复用过期 token)。排序原则:先闭环、再立规、后大件。

1. **同步「离线→在线」闭环三小件**(各 S/S-M)—— ① 修重连过期 token 死循环(bug);② blob 重传挂上重连(现只在重开文档时);③ outbox 分批背压(重连风暴)。全是小活,合起来把离线体验真正关上门。
2. **op 模型退役启动**(M)`[需后端]` —— P2-M4 主干已上线,阻塞解除:先删死写入器(S),再拍修剪/退役;这是库里唯一无界增长的大头。
3. **协议版本门**(S-M)`[需后端]` —— 桌面自装包版本天然漂;退役 op 模型(现在的 REST 兜底)之前先把 WS min-version 闸门立好,退役才敢做。
4. **快赢打包**(合计约一个下午)—— OFL.txt 附上、0.0.0.0 明文绑定启动告警、`upgrade_real_store_smoke` 写进 release.md 清单、catch-up 常量进 AppConfig、残余静默 catch 接计数。
5. **拍板项(先决策不写码)**—— ~~回收站保留期~~ ✅ 已拍板不做(见「数据生命周期」);~~toggle 要不要新 kind~~ ✅ 已拍板分两步(见「编辑器与功能广度」);~~MCP inspector v2~~ ✅ 已拍板**暂不升**(官方迁移指南 #1822 未完成、v2 当天 tree-swap 上线自家 CI 都在抖;`@1` 仍收安全补丁,改法已备:命令里补 `-e KEY=VALUE` 且必须放在 target 之后);**剩** M8 comrak 取舍。

**中期(用户挑)**:安全三件(token DPAPI / 邮箱验证 / WS token 出 query string);平台两件(Authenticode 签名 / Linux CI 出包);产品大件(同块字符级协同 L / 本地全文搜索 M-L / 本地反链 / 视口虚拟化 L / 触屏选择 L)。

## 上一批「最该做」(2026-07-22 排,当批已全部完成)

> 数据安全里程碑已收口后,重心转向「公网自托管的硬底线」——发出去前一次事故就不可挽回的类型。

1. ~~**分享页安全三件套**~~ ✅ 完成(200c3b1/81ff653)—— export_html 白名单净化(strip_unsafe_attrs)+ 分享响应 CSP + 渲染前校验 view 存活。存储型 XSS→token 接管 与「删了还在公网」两个高危都堵上。
2. ~~**备份可信化**~~ ✅ 完成(死人开关 + pg_dump 异地 DR + `/api/ready` 探活)—— 仅 `rustic check`/恢复演练排期这一件 S 未做(见「生产运维」小节)。
3. ~~**AI 配置授权 + 收口 base_url**~~ ✅ 完成(200c3b1)—— base_url 钉死服务端配置,密钥外泄 + SSRF 堵上。
4. ~~**CI 锁住数据面回归**~~ ✅ 完成 —— api-server 测进 CI(postgres service)+ auth.rs `pool()` CI-assert(缺库即 fail)+ 页树守卫补测 + real_store_smoke。
5. ~~**客户端兜底三件**~~ ✅ 完成 —— 崩溃上报(runZonedGuarded)+ 单实例守卫(CreateMutexW)+ 本地损坏兜底(LocalDocCorruptException,不再自毁恢复点)。
6. ~~**限流 + 收紧 CORS + Token 撤销收口**~~ ✅ 2026-07-22 完成:认证端点(含 refresh)per-IP 令牌桶 + Argon2 并发门、CORS prod 拒跨源、access JWT 24h→1h、修 prod 误认作 dev。WS 建连限流有意不做(已鉴权低威胁,见 CLAUDE.md「不要过度设计」);per-user token-version 即时吊销可选。
7. ~~**文档内查找/替换**~~ ✅ 2026-07-22 完成(9fe9ae8):查找侧原已具备,补齐替换 + F3。至此本「最该做」清单全部清空——下一批优先级见下方各小节(反链、表格、虚拟化等)。

---

**整体判断**(2026-07-22 校准后):上面「发出去前必须补的底线」——安全(分享页/AI 密钥/限流)、备份可信化、CI 回归网、客户端桌面丢数据三路径——**均已落地**(本轮对着代码逐条核实,勾除了 18+ 项 roadmap 陈旧误报的已做项)。剩的高价值真·未做偏「基建/成熟度」:更新器 Authenticode 签名、协议版本协商、恢复演练排期、op 模型表 GC、长文档虚拟化;产品广度上**虚拟化 + 表格 + 反链**决定它像不像一个成熟笔记。
