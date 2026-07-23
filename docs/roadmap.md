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

- **P2-M4 云同步流未真正建**(bigserial 单调流 + 断点续传 + SV 回退 + local-seq→Rid)—— 离线优先同步的主干,现有 op 模型本应随它退役。(L) `[需后端]`
- **实时字符级并发协同未落地** —— presence 光标已画(`render.dart`),但同块并发输入仍靠 last-write,「协同」名不副实。(L) `[需后端]`
- **M-R 收尾 C3/D1/D2/A3** —— 坏更新加载自愈 + schema 版本号、静默 `catch{}`→计数日志、同步健康态、会话持久化 e2e。(M)
- **离线→在线 blob 自动 reconcile** —— 现只在重开文档时懒重传;可挂到自动重连成功事件上。(M) `[需后端]`
- ~~**双向 state-vector 协商**~~ ✅ 已做(校准复核)—— P4-3:`ws.rs:508` `client_sv.and_then(|sv| sync::diff_from_base(base, sv))` 按 client SV 发最小 diff,base_message delta 分支 + 单测 `base_message_sends_delta_only_when_sv_yields_one`。
- ~~**broadcast lag 触发整档重载**~~ ✅ 已做(校准复核)—— 客户端 `_resyncFromLag` 发 `sync.pull` 带 cursor+SV 增量续拉,非整档重载。(`cloud_sync_session.dart`)
- ~~🆕 **`client_out_of_date` 客户端零处理 → 被跳过的更新永久静默丢失**~~ ✅ 已做(校准复核)—— `cloud_sync_session.dart:467` 收到 `code:'client_out_of_date'`(无 ack_id)即 `_resyncFromLag()` 触发 pull/bootstrap 补洞;server 侧 `ws.rs:170` 发 notice。
- ~~🆕 **离线 outbox 按文档滞留:重连后只有当前打开的文档会推送**~~ ✅ 已做(5b7536a)—— 上线/重连(`onServerConnected → _onCloudOnline`)跑 `_sweepPendingOutboxes`:枚举云视图,跳活跃文档,对每个有非空 outbox 的云文档起短命 headless `CloudSyncSession`(persistence=该 doc 的 store)connect→drainOutbox→dispose。桌面 only;串行 + 单例锁 + best-effort + 双检跳活跃 + 空 outbox 不连;blast radius 有界(mis-drain=幂等/超时不损坏)。**注**:纯客户端编排、组合已测原语,无新增自动化测试(端到端需真 WS+多文档集成环境),待实机冒烟。(`main.dart` `_sweepPendingOutboxes`)
- 🆕 **长离线重连 = 推送风暴**(medium) —— `_flushUnacked(resendAll:true)` 逐条重发整个 outbox,无分批/背压/合并;服务端每条 push 全档 decode+encode+upsert = O(条数×文档大小)。可先用 yrs merge 把尾巴合成一条再推,或分批节流。(`cloud_sync_session.dart:584`, `sync.rs:217`)(M) `[需后端]`
- 🆕 **协议无版本协商 / 无最低版本闸门**(medium) —— WS 握手不交换客户端/协议版本,未知帧与未知 error code 均静默忽略,兼容全靠「每次改动做成双向后向兼容」的纪律 + op-model REST 兜底。桌面是用户自装包、服务端本地独立部署,版本天然会漂;op 模型退役后没有任何机制(WS hello / min-version 拒连 / 健康检查版本比对)挡老客户端连上不再兼容的服务端并静默错乱。(`ws.rs:36`, `health.rs:10`)(M) `[需后端]`
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
- 🆕 **单机兜底部署脚本 `deploy/deploy.sh` 已漂移**(low) —— `flutter build web` 缺 `--no-web-resources-cdn`(CN 环境运行时拉 gstatic CanvasKit 直接不可用)、带被点名修过的 `--no-tree-shake-icons`、用 Windows 没有的 rsync。按文档首次部署会得到依赖 gstatic 的 bundle。(`deploy/deploy.sh:16`, `justfile:154`)(S)
- 🆕 **Postgres 大版本升级路径无文档**(low) —— 钉在 `postgres:16-alpine` + 命名卷,顺手改 tag 到 18 会 crash-loop(需 pg_upgrade/dump-restore)。PG16 支持到 2028,不急但三年后必忘。deploy.md 加三行「升级 = dump→新卷新镜像→restore,禁原地改 tag」。(S)
- 🆕 **共享 Traefik 证书无过期监控、配置不在仓库**(medium) —— 生产 HTTPS 由仓库外的 EXISTING Traefik 终结,`deploy.md:86` 记了 ACME 卡死需手动重启的真实故障;证书过期无监控,S3_DOMAIN 证书失效会让所有 presigned 图片 URL 浏览器端全挂。修:外部拨测顺带断言两域名证书剩余有效期;把 Traefik 配置纳入某受管仓库。(S)

## 数据生命周期与增长 🆕

> 2026-07-22 新增小节。多处「删除不真删」+「无界追加」,单节点小盘上会慢慢暴雷。

- ~~**REST/MCP 写路径从不落自动版本快照**~~ ✅ `apply_derived_operations` 复用 push_update 的 auto 版本 INSERT(同事务、10min cadence、30 天;只写版本归档表、不碰双表示红线,6612330;连真 PG 测试)。
- ~~**删除 workspace 永久泄漏其全部 S3/RustFS 图片对象**~~ ✅ `workspaces::delete` 删库前枚举 `DISTINCT object_key` 逐个删存储对象(best-effort、objects-first,6612330)。
- ~~🆕 **`purge_view`「永久删除」只删 views 行**~~ ✅ 已做(927d7f7)—— `purge_view_subtree` 一条原子 CTE 删 views 子树 + 对 document 型视图删 `documents` 行,DB `ON DELETE CASCADE` 随之清空所有 document_* 表(yrs base/快照/版本/op/**分享 token**);blob 靠 blob_gc 惰性回收(去重共享,不急删)。DB 门控测试锁级联。(`documents.rs` `purge_view_subtree`)
- 🆕 **op 模型表无界增长**(medium) —— 每次 REST/MCP 写入落一整份 jsonb 全量快照进 `document_snapshots` + 一条 `document_updates`,两表全仓无 DELETE;该路径还追加 `workspace_updates` 但没有 push_update 那套修剪。op 模型「随 P2-M4 退役」是计划,退役前按「文档大小×写入次数」增长。(`store.rs:252`, `sync.rs:284`)(M) `[需后端]`
- 🆕 **无任何容量配额**(medium) —— 唯一限制是单文件 25MB + 导入 1GiB body;无 workspace 总量/单文档大小/用户级上限,WS 路径默认可收 64MiB 单条消息,大文档写放大(每 push 全量 base 覆写 + 每 10min 全量版本)。开放注册单节点最易被无意/恶意打爆盘。(`storage.rs:50`, `ws.rs:60`, `sync.rs:244`)(M) `[需后端]`
- ~~**`document_yrs_versions` 过期清理只挂在「该文档自己 push 撞 cadence」**~~ ✅ blob_gc 6h 循环加全局 `DELETE ... expires_at IS NOT NULL AND < now()`(只命中 auto、不碰命名检查点,6612330)。**残留**:`list_yrs_versions` 仍不过滤 expires_at(6h 扫前的过期行可能短暂现于面板,极小)。
- 🆕 **回收站无保留期限,永久堆积**(low) —— 纯 `is_deleted` 标志,无自动清空/保留期;blob GC 刻意把回收站引用算存活 → 图片 blob 也永久保留。`blob_gc.rs:43` 注释预设了一个不存在的「回收站保留期」。可能是有意的产品选择(如 Notion),但从未写成决定且与注释矛盾。(S) `[需后端]`
- ~~**`refresh_tokens` 只增不删**~~ ✅ blob_gc 6h 循环加 `DELETE ... expires_at < now()-7d`(6612330)。
- ~~🟡 **账号删除功能不存在**~~ ✅ 已做(18300d1,2026-07-23)—— `delete_account` 事务级联(密码门控 + 跨他人 workspace RESTRICT 阻塞回滚 409),详见「产品与公开发布合规」小节;级联顺序备忘 deploy.md 早有(0d9c404)。
- 🆕 **导出(及其上的每日备份)不含回收站内容**(low) —— `fetch_workspace_views` 过滤 `is_deleted=false`,回收站页面及其独有图片不进导出包;一个页面在回收站躺过备份保留窗口(7天/4周/6月)后,备份里最后副本也被 prune,唯一副本只剩生产库。`backup.md` 记了排除编辑历史/用户数据,回收站排除没写。(`documents.rs:2101`)(S)

## 编辑器与功能广度

- 🟡 **全文搜索**(2026-07-22 复核:原描述失实——不是「反序列化每篇快照」,是每查询把每篇 yrs base **全量 CRDT 解码**一遍,N 次 decode)—— **M1 已做**(aa4c5d8):加 `document_yrs_base.content_text` 派生列(migration 0012),搜索退化为**一条 LEFT JOIN + ILIKE SQL**,干掉 N 次 CRDT 解码。content_text 是 state 纯投影、三条写 base 路径同语句 co-write(红线#1 不漂移);启动一次性回填存量;LIKE 转义 + 命中处窗口 snippet;CJK 走子串(无扩展/无分词器)。**残留**:~~②导入未打开的文档正文不可搜~~ ✅(2026-07-22,导入两条路径 commit 后 best-effort `bootstrap_base` 即建 base+content_text,等价「导入即打开」,复用现有构建器无新写路径;回归测试锁住;postgres:16-alpine 自带 pg_trgm 已确认)。**故意缓做**(非尾巴,按「不过度设计」):① **pg_trgm GIN 索引**——当前 22MB 库、查询已被 (workspace_id,is_deleted,object_type) 索引收窄再 ILIKE,亚毫秒;GIN 只加速 ≥3 字子串(CJK 2 字仍 seq-scan),不是干净胜利。真到大规模(万级文档)时一行 `CREATE EXTENSION pg_trgm` + `CREATE INDEX` 升级,现在加是为假想规模优化;② 排序/高亮/分词——各自独立 UX 特性,另立项非 M1 尾巴。(各 S–L) `[需后端]`
- 🟡 **表格**(2026-07-22 复核:原描述大幅失实)—— 实测:**富行内单元格**(粗体/斜体/行内代码/链接,cell 存可重解析 md 源码、两端渲染+编辑,`cellDisplaySpan`/`CellEditController`)与**矩形/行列选区**(跨格拖选、点行/列把手选整行列、Ctrl+C/X 复制为 TSV+HTML、Delete 清空、Esc 清除)**本来就能用**;本轮仅补 **Shift+点击扩展选区**。**合并单元格有意不做**——8 家同类(Notion/AFFiNE/AppFlowy/Outline/siyuan/Joplin/logseq/anytype)调研定论:合并与「Markdown 权威 + round-trip 不变量」在 GFM 下**架构级互斥**(siyuan 能合并因它放弃了 md 权威;Joplin 同约束只能冻单向 HTML;Logseq/Notion 干脆不做)。要做只能另开 HTML 逃生舱块退出 round-trip,是独立决策。块级单元格/列宽 GFM 表达不了,同样不做。
- 🟡 **反向链接/引用面板/关系图** —— 正向 `[[` 已建;**引用面板已做**(云端页显示「谁链到我」可点跳转,`GET .../backlinks` 按需扫描、复用 page_link_targets,7de2c2a)。**残留**:~~①并发扫描~~ ✅(buffered(8),6612330);②规模成瓶颈再上维护式反向索引表(现在故意不建);③本地世界(offline)反链;④**关系图**(graph view)。(各 S–L)`[需后端]`
- 🟡 **页面属性/标签**(**M1 已完成**,2026-07-22)—— 走 front matter 权威路(调研定论:同类 md 权威系均如此,见 `docs/page-properties.md`)。**M1 全部落地**:① 数据/权威层——Rust `crates/markdown/src/properties.rs`(解析扁平子集 + 类型推断 + 外科式写回,round-trip 不变量经用户批准从字节保真降为规范化子集稳定)+ Dart 镜像 `properties.dart`,两端逐条测试一致(Rust 9 / Dart 10 全绿);② 页头属性面板 `property_panel.dart`(读 root 块 `data['front_matter']` → 类型化编辑:文本/数字/日期文本框、勾选、tags chips 增删 + 增/删属性 → 编辑经 `onApplyOperations` 单入口自分派写回 root 块,local/cloud-CRDT/cloud-REST 三模式通用,无需穿层新回调);flutter build windows 通过。tags = `tags:` list 属性。**Obsidian-lite 闭环已完成**:增删改属性(类型 text/number/checkbox/date/list)、tags chips、**可搜**(属性值折进 content_text,list 值以 `#值` 存)、**tag 点击精确跳页**(搜 `#值` 只命中真正带该标签的页,ce13cef)、**默认隐藏在页头 ⓘ 图标后**(不占版)+ **AppFlowy 式面包屑路径**(579272f)、AFFiNE 式紧凑面板(7379444)。**故意不做/另立项**:① 数据库视图级「按属性筛选/排序/看板」——是 Notion 数据库那套,与 markdown 权威+round-trip 架构互斥(要豁免 md 权威,AFFiNE/siyuan 路),独立大决策;② 存量页要下次编辑才索引属性(backfill 只填空行,属性是新功能故不强制全量重派生);③ 日期选择器 UI(现文本输入)。**数据库视图(带类型列/筛选/relation)另立项**——与 markdown 权威+round-trip 架构互斥,要么破双表示红线要么豁免 md 权威(AFFiNE/siyuan 路),是独立大决策。(L) `[需后端]`
- **评论/建议未建** —— 仅 `commenter` 角色打通,marks 模型本为 range 锚点预留。(L) `[需后端]`
- 🟡 **结构块 callout/toggle/embed/columns** —— **callout 已做**(GFM alert `> [!TYPE]` 5 类型,复用 quote 扁平模型、round-trip 干净、记分牌未降,e7ff038)。**残留/定论**(2026-07-22 调研):① **toggle** —— `<details>` 现已当 raw-HTML 直通 round-trip(不可编辑);可编辑结构化 toggle 需新 kind + 教导入器反解析 HTML(有损成本),留独立决策;② **columns** —— **红线不做**(标准 md 无多列表示,同表格合并;要做只能显式有损方言);③ embed 未做。附:render 注册表 P3-1 对这三种块**不是前置**(仅撞已有 kind 如 Graphviz 时才需)。(各 S–L)
- **无屏幕阅读器语义(a11y) / 无 RTL 双向文本** —— 自绘 RenderBox 无 Semantics;10+ 处硬编码 `TextDirection.ltr`(editor-engine, `render.dart`)。缓解:设置里有 85–140% 应用内字号(`EditorAppearance.fontScale`),覆盖低视力一部分。(各 L)
- 🆕 **无暗色模式**(2026-07-22 实测,独立特性) —— app `MaterialApp` 只设浅色 `theme`(**无 `darkTheme`/`themeMode`**),自绘编辑器 `EditorTheme` 全是静态浅色常量、零亮度判断 → **全 app 永远浅色**,系统暗色也不跟随。做法:① MaterialApp 加 `darkTheme` + `themeMode`(跟随系统/设置手切);② 自绘编辑器把 `EditorTheme` 常量 + 所有块装饰(quote/code/callout 色条、选区、caret、图片/mermaid/math 背景对比)改**主题感知**——这是自绘暗色的主要工作量;③ 结构块(callout 等)的浅色配色到时一并进主题(故 callout 现保持浅色、与全 app 一致是正确的,不单独适配)。先调研自绘编辑器怎么做主题感知最省再动手。(L)
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
- **无内置自动更新** —— 现靠手动;可采 AppFlowy 的 WinSparkle + appcast(desktop-plan)。(L)
- **window_manager→nativeapi / Turso 观望**(各 S,已隔离在 trait 后)。

## 客户端质量与兜底 🆕

> 2026-07-22 新增小节。离线功能面做得全,但崩溃/损坏/双开几处兜底缺失会真丢数据。

- ~~🆕 **客户端零崩溃/错误上报**~~ ✅ 已做(校准复核)—— `main.dart:149` `runZonedGuarded` + `FlutterError.onError`,未捕获异常落盘 diagnosticsDir。
- ~~🆕 **本地世界文档损坏 → 静默变空白且自毁恢复检查点**~~ ✅ 已做(校准复核)—— FFI `store.rs` `load_doc` 区分 None/corrupt(throw);`local_doc.dart:56` 捕获即 rethrow `LocalDocCorruptException`,不再 seed+saveDoc+checkpoint(§10 回滚网不再被冲)。
- ~~🆕 **桌面无单实例守卫,双开丢本地文档**~~ ✅ 已做(校准复核)—— `windows/runner/main.cpp:24` `CreateMutexW("Local\\MicaSingleInstance")` + `ERROR_ALREADY_EXISTS` 守卫(fail-open)。
- ~~🆕 **退出路径漏掉编辑器 400ms 防抖文本**~~ ✅ 已做(校准复核)—— `main.dart:1016` `_flushForExit` 先 `await _activeEditorFlush()` 再冲会话/后端。
- ~~🆕 **`prefs.json` 非原子写 + 损坏静默清空**~~ ✅ 已做(校准复核)—— `prefs_stub.dart:64` 写 `.tmp` 后 `renameSync`(同卷原子,含 Windows 覆盖处理)。
- ~~🆕 **编辑器 op 管道 `catchError((_){})` 吞掉本应浮出的 outbox 写失败**~~ ✅ 已做(校准复核)—— `controller.dart` 现 `opFaultCount++` + `onOpFault?.call` 上浮,不再吞(红线 #1)。
- 🆕 **云文档离线/未同步状态零指示**(medium) —— 唯一状态 UI 是 integrity-fault banner(且 count>3 才出),断网继续编辑云文档界面与在线零差别、无「离线中/N 条未同步/已保存」任何指示;配合滞留 outbox,用户有理由以为「看到了=已同步」直接换设备造成分叉。数据源现成(`outboxAfter(pushedClock).length`)。(`main.dart:1024`, `cloud_sync_session.dart:672`)(M)
- 🆕 **i18n 漏网**(low) —— 默认页名 `kUntitledPage='未命名页面'` 硬编码中文并持久化(英文用户新建页得到中文标题、且与 'Untitled' 双轨),代码块 AI 动作 prompt 全中文;语言仅 en+zh。(`models.dart:667`, `editor.dart:5109`)(S)

## 性能

- **长文档无虚拟化** —— paint 侧已有视口裁剪(`render.dart:1302`,±600px),但 `performLayout` 每次布局仍 dispose+重建全部节点 TextPainter,大档每击键全量重排。(L)
- ~~**图片纹理缓存无逐出策略**~~ ✅ `_imageCache` 改 LRU(64 上限,每帧 touch 可见图、逐出屏外静态图并 dispose,守 lessons.md §5 dispose 时序,253c53f)。
- **每次 push 重建+重编码+重写整档(写放大)** —— `from_update`→全档 `encode_state`+upsert,成本 O(文档) 而非 O(更新)(`sync.rs`)。(M) `[需后端]`
- **yrs base 无 squash/GC,无界增长** —— 只裁 stream 不压 base,长寿文档 base 越滚越大(`sync.rs`)。(L) `[需后端]`
- **本地持久化仅全量快照** —— §4 的增量队列 + squash 折叠推迟中。(M)
- **frb v2 热路径 FFI 基准待测** —— IME/逐字输入若过慢,热路径留 Dart(phase2 §12)。(M)

## 开发者体验 / CI / Markdown

- ~~🆕 **api-server 全部测试不进 CI;DB 测试本地也静默跳过**~~ ✅ 已做(校准复核)—— `ci.yml` 已有 postgres service + `-p mica-api-server`(测在 CI 实跑);`auth.rs:895` `pool()` **已带 sync_pg.rs 同款 CI-assert**(`assert!(env CI is_err, "DATABASE_URL unset in CI...")`)→ 缺库在 CI 里 panic、本地才 return None 跳过,那些 `else{return}` 在 CI 不会假绿。(校准注:审计曾误判为残留,因只看了 `else{return}` 调用点、未读 `pool()` 定义——同其 #1 的错。)
- ~~🆕 **页树不变量守卫 `ensure_parent_accepts_children` 零自动化测试**~~ ✅ 已做(校准复核)—— `documents.rs` `parent_guard_pg` 测 folder 接受/page 拒绝/缺失父 + 触发器 backstop(真 PG 门控)。
- ~~**Release 出的 Windows 安装包从未被自动安装-启动验证**~~ ✅ release.yml 加「安装-启动冒烟」(/VERYSILENT 装 + 启动 + 存活 10s + finally 清理,发布前拦,0d9c404)。**2026-07-23 根治 flaky**:冒烟测撞单实例 mutex 竞态偶发假失败(安装器 `[Run]` 自启一个 + 测试又自启一个,谁后抢到 `Local\MicaSingleInstance` 谁 `exit 0`;`[Run]` 触发时机随机)。结构性解法(ShareX 同款):`mica.iss` 的 `[Run]` 加 `Check: not CmdLineParamExists('/SKIPRUN')`,CI 安装传 `/SKIPRUN` → 安装器不自启 → 测试是唯一启动方 → mutex 永不争用,竞态从结构上消失(9c006e6),0.12.16 真 CI 跑绿实证。顺带 windows job 上 sccache 缓存本地 crate 编译(b5e7f04)。
- **CI 补 Windows 集成测试** —— 18 个 integration_test(≈46 测,多条是真丢数据 bug 回归)只能本地手跑,且两个全栈文件并跑会撞 debug-connection race(`dev-environment.md:137`);Postgres 依赖测试已随 2e84422 进 CI。(M) `[需后端]`
- 🆕 **全项目零自动化 e2e**(medium) —— 桌面 integration_test 手跑;web 端零 e2e,CLAUDE.md「playwright 截图」是人工手段,仓库无任何 `.spec.ts`/committed 脚本,CI 对 web 只验「能编译」。(L)
- 🆕 **三个不可信输入解析面零 fuzz**(medium) —— 无 `fuzz/` 目录,任何 Cargo.toml 不含 proptest/quickcheck/arbitrary/cargo-fuzz;而手写逐字节 xor 已实证挖出远程可达(需认证)的 yrs UB。markdown/interchange 是自家代码,cargo-fuzz 可直接落地为回归。(`store.rs:2202`)(M)
- 🆕 **本地 SQLite 真库升级冒烟不在发版清单**(medium) —— `upgrade_real_store_smoke`(`#[ignore]`+需手动设 `MICA_REAL_STORE`)是发版前手动步骤,但 `release.md` 全篇不含其字样 → 发版流程不会触发任何人想起它;而桌面自动更新后首启就地迁移本地库,迁移写坏=用户笔记不可见。(`store.rs:2083`, `local-first-p3-design.md:288`)(S)
- 🆕 **无覆盖率度量;`crates/cli`(备份导出引擎+MCP 代理,779 行)零测试**(medium) —— 无 tarpaulin/llvm-cov 配置;cli 是 prod backup sidecar 每天执行的导出命令本体 + 用户 MCP 接入代理层,唯一验证是 mcp-conformance 的握手/schema(不碰导出/REST 逻辑)。装 cargo-llvm-cov 让「0% 的洞」在数字上无法被忽视。(S)
- ~~🆕 **`just test` 漏 `--features store`**~~ ✅ 已做(校准复核)—— `justfile:133` 已有 `cargo test -p mica-core --features store`。
- 🆕 **Linux 桌面在仓库但从不在 CI 构建**(low) —— `linux/` runner + 托盘降级逻辑在库,CLAUDE.md 还为它写了约束,但 CI/release 都无 `flutter build linux` → 编译债不可见。flaky 债本身很轻(仅 2 个带理由 `#[ignore]`)。(M)
- **仅结构化日志,无 /metrics/telemetry** —— 同步后端生产盲飞(`telemetry.rs`)。(M) `[需后端]`
- **可选/later 基建:Redis、OTel、索引块表** —— 索引块表是搜索/反链/分析的底座(architecture.md)。(L) `[需后端]`
- **自研 parser vs 采用 comrak(读侧)未决** —— Milestone 8 决策点(editor-engine)。(M)
- **catch-up limit / stream 常量硬编码** —— 1000、KEEP_MARGIN/PRUNE_EVERY 应入 AppConfig(`ws.rs`)。(S) `[需后端]`
- **过时注释/文档批量清理** —— 多处 "M5+/later" 已实现却没更新(`main.dart`, `model.dart`, `lib.rs`, `preview_raster.dart`)。(S)

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

## 接下来最该做的 3–5 件(2026-07-22 重排)

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
