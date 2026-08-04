# Mica 路线图 — 剩余功能与优化点

> **这份文件只放没做完的事。** 做完的整条搬去 [`roadmap-done.md`](roadmap-done.md)
> —— 不是删,那些条目里写着「当初为什么必须这么排」(op 模型退役那六步是最好的例子),
> 那是判断依据,不是流水账。但把它们留在待办清单里,每次盘点都要重付一遍阅读成本:
> 2026-08-03 那轮核实,29 条未做项里有 3 条是幻影、1 条工作量标反 —— 而那还是在
> **只扫未做项**的前提下。
>
> 影响力从高到低;`(S/M/L)` = 工作量;`[需后端]` = 要动 Rust。
> `🟡` = 部分完成(条目里写清楚了哪半还欠);`⏸️` = 已拍板不做,不是遗留 TODO。
>
> **维护规矩(否则它会再烂一遍)**:
> 1. **一条做完了就当场整条搬走**,别只加 ✅ —— 发版流程里有这一步(`release.md` 步骤 5)。
> 2. **否定式条目(「无 X」「不校验 Y」)只会静默变假**:代码删个函数会编译报错,
>    文档说「没有」而实际有了,什么都不会响。所以它们要定期对着代码核,而不是靠记性。
> 3. **核实结论写进条目本身**(日期 + 依据文件),下一个人才知道这条是新鲜的还是祖传的。

## 可靠性与同步

- 🟡 **长离线重连 = 推送风暴**(2026-07-30 核实:原描述「无分批/背压/合并」已半假)—— **桌面已修**:`_flushUnacked` 在 append-log 路径上按 `_pushWindow = 64` 开窗,尾巴由 ack 回调驱动继续排空 —— 这既是分批也是背压(节奏由服务端定,不由 for 循环定)。~~① **web 仍然无界**~~ ✅ **已做(2026-08-03)**:内存 outbox 走同一个 `_pushWindow = 64`,尾巴由 ack 回调续排;`resendAll`(重连)先清 `sent`/`rejected` 再按窗口发。**顺带修掉一个只有开窗后才存在的死角**:被 `error` 拒绝的 push 永远等不到 ack,如果一直算「在飞」就会**永久占着窗口位**,攒够一批直接把排空焊死 —— 现在打 `rejected` 标记让位,但**不立刻重发**(这条路的重试本来就在重连)。节奏判定抽成纯函数 `pushSlice` 单测(`web_outbox_backpressure_test.dart`),因为真正的发送路径要先 bootstrap,而 bootstrap 要 CRDT 引擎 + 活服务端 —— 那是 `integration_test/cloud_sync_*` 的地盘,CI 里排除。**残留**:② **合并未做**(两条路径都没有):可先用 yrs merge 把尾巴合成一条再推。服务端那半仍然成立:每条 push 全档 decode+encode+upsert = O(条数×文档大小),见本文件「每次 push 重建+重编码+重写整档」。(`cloud_sync_session.dart` `_flushUnacked`, `sync.rs` `push_update`)(M) `[需后端]`

## 安全

> 上一轮安全 review 的落地清单。自托管一上公网,前几项是硬底线。
> 2026-07-22:refresh/rotation/改密撤销已落地(勾除);新增分享页 XSS、AI 密钥外泄、
> SSRF 等此前漏网的高危项。

- 🆕 **可上传携带脚本的 SVG,直开 blob 链接执行脚本**(**降级 low**,2026-07-22 复核)—— 允许 `image/svg+xml`,blob 端点(`blob_inner`)**302 跳存储的 `download_url`**(`public_base_url`/CDN 或 presigned GET,都是**存储源、非 app 源**)→ SVG 脚本跑在存储源、**碰不到 app 的 token,不是账号接管 XSS**。**仅当**运营者把 `public_base_url` 配成与 app 同源才成洞(部署误配)。且 302-跳存储架构下 app 不发字节,强制 attachment 别扭(要么上传即拒 SVG / 存成 text/plain,要么 presigned 加 `response-content-disposition`)。作为「防误配」的纵深项保留,非活跃洞。(`files.rs:350/364/537`)(S) `[需后端]`
- 🟡 **自托管 TLS 全靠运维 + `HTTP_ADDR` 默认明文**(2026-07-30 核实:「无启动告警」已假)—— ~~无启动告警~~ ✅ 早在 `9289ecf` 就有:`main.rs` 绑定后判 `!addr.ip().is_loopback()`,非环回即 warn「本进程只说明文 HTTP,请在前面终止 TLS,否则连 WS URL 里那个 token 都在网上裸奔」。默认 `127.0.0.1` 安全且**静默** —— 只有真承担了风险的部署才看到告警。(2026-07-30 顺手修了那条字符串:它被压成一行、中间留了三段连续空格,打出来有大段空白。)**残留**:TLS 本体仍全靠运维,且告警只是告警 —— 没有「prod + 非环回 + 无 TLS 即拒启动」的硬闸门(要拒得先能看见前面有没有反代,单机上看不见)。(M) `[需后端]`
- 🆕 **安全清单卫生**(low) —— 上面两条已勾除即本轮校准;后续改动请同步勾选,避免半真半假的清单掩盖真未修项。

## 生产运维与备份 🆕

> 2026-07-22 新增小节。节点是单机 docker(阿里云),生产当前处于「盲飞 + 静默失败」态。

- 🆕 **磁盘慢渗(降级 low,2026-07-23 复核)** —— 原列 medium,核对后大半已做:① ✅ **日志上限**——compose 5 个服务全走 `*default-logging`(10m×3),最吓人的"日志无限涨"已堵;② ✅ **悬空镜像 prune**——`node-deploy-policy.sh:139` 每次部署 `docker image prune -f --filter until=168h`。**残留(慢渗、低危)**:③ 旧的**带 tag** 版本镜像累积(上面 prune 故意 NO `-a`、只清悬空、留回滚,每版多 3 个带 tag 镜像几百 MB);④ `/data/mica/pre-*.sql.gz` 手动还原点不自动清;⑤ 无磁盘水位告警。云盘几十 GB、慢渗不急。要做就是 `node-deploy-policy.sh` 尾部再加"保留最近 N 版镜像 + N 个还原点"。(S)
- 🟡 **备份恢复演练:已有脚本 + 已实跑一次,仍未自动化**(2026-07-30)—— ~~纯手动、无脚本承载~~ ✅:`deploy/restore-drill.sh` + `just restore-drill <basename>`,一条命令恢复进一次性库 → 断言 → DROP(不碰 `mica`、不重启容器),并顺带跑 `rustic check`。三条硬门槛:恢复错误 0、`documents` > 0、**可读页数 > 0**(走每次读都要走的 `views→documents→document_yrs_base` join,要求 `length(state)>0 AND content_text<>''`)—— 因为一次产出空 `state` blob 的恢复能通过所有「表在不在」式断言。**首次实跑(这条路径此前从未被走过)**:错误 0、`_sqlx_migrations`=15、S5 删掉的三张表都回来了、行数与备份时记录逐项一致、32 FK + 19 PK、3331 可读页;`rustic check` 170 snapshot 全过。**残留 = 自动化,而它被一条刻意的安全边界挡着**:CI 那把 key 不是 shell key,`~mica-deploy/.ssh/authorized_keys` 用 `restrict,command=/usr/local/sbin/mica-deploy` 钉死,只能执行 `deploy <version> <sha>`;要让 Actions 定时跑演练,得在节点上装一条新的 pinned 命令 + 一把新 key —— 那是生产侧的凭据/策略变更(deploy.yml 自己写着 CI「may READ the fence that limits it, never install it」),**该由用户决定并执行**,不该由 agent 代办。在那之前:发版落还原点后手动 `just restore-drill` 一次,以及 `rustic check` 进每周节拍,每季度恢复一个 workspace diff 并记日期。(S)
- 🆕 **生产当前没有任何自动化告警 —— 挂了只能靠人发现**(2026-08-04,`uptime.yml` 已删除)——
  **为什么删**:那个 workflow 声称 `cron: */15` 拨测 `/api/ready`,实测(08-02T17:14Z →
  08-03T14:23Z,21 小时)只跑了 **12 次**而非应有的 84 次(**14%**),间隔从 ~60 分钟退化到
  95 / 214 / 214 / 212 / 149 分钟;而 08-03 的故障窗口是 16:19–16:45Z,上一次运行停在
  14:23Z —— **整段故障它一次都没跑**,是用户自己发现服务挂了。根因不是配置写错:
  **GitHub Actions 的 schedule 是 best-effort**,短间隔被丢得最狠。它把一个可靠性承诺建在
  尽力而为的调度上,而**没有任何东西会在这个前提失效时报错** —— 维护规矩第 2 条的活标本。
  留着比没有更糟:它让人以为有人看着。
  **本条已收敛,但不是靠告警补上的**(2026-08-04):`/metrics` 已上;**采集刻意不在 Mica 的
  部署里**(示例栈在 `deploy/monitoring/`,谁抓是运维的决定);**告警已拍板不做**。所以标题
  这句"挂了只能靠人发现"到今天**仍然字面为真** —— 它现在是一个被接受的代价,不是待办。
  **删除同时丢掉的两样,别忘了**:① 应用层拨测;② **TLS 证书过期检查**(见下条)。
  ②和应用监控不是一回事,Prometheus 本体也不覆盖(要 blackbox_exporter),而 ACME 卡死是
  **静默的全站故障**。(M) `[需后端]`
  〔08-03 故障复盘,存这里免得重查:同机跑 `docker build`(neostor 的 vite + cargo,~1.4G)
  把 3.5G 无 swap 的机器压到 load 23 / IO 压力 98%,所有健康探针卡在 D 状态 → api/pg 标
  unhealthy,Mica 进程本身没死。已加 4G swap(`vm.swappiness=10`,写进 fstab)作缓冲;
  **真正的解是把构建挪走**。〕
- 🆕 **Traefik:证书过期重新变成无人看守 + 配置仍不在仓库**(2026-08-04 更新)——
  ~~`uptime.yml` 每天查两域名(app + s3)证书剩余有效期,< 10 天即告警~~ **该 workflow 已删**
  (上一条),所以**证书过期这条线又空了**。这一半本来是工作正常的:证书是天级的,即便退化到
  每 1–3.5 小时跑一次也完全够用 —— 删掉是删应用拨测的连带损失,不是因为它不好使。
  重建时它属于 blackbox 探测那一档,不是 `/metrics` 那一档 —— 而 **blackbox 探测已于 2026-08-04
  拍板不做**(见排序清单末尾的 ⏸️ 三项)。所以这条线是**明知空着**,不是忘了。
  **残留照旧**:Traefik 配置本体在仓库外未纳管;ACME 卡死那类故障仍靠 `deploy.md:86` 的
  手动 runbook。(S,external)
- 🆕 **provisioning 层不存在:「给台新机器就能起全套」今天做不到**(2026-08-02 写下方案,未实施)——
  仓库里只有 `deploy/docker-compose.yml`;Traefik、`/data/mica` 目录、`.env`、受限部署账号与
  `/usr/local/sbin/mica-deploy`、ACR 登录全是当年手工装的,没有一条能重放的路径(上面 Traefik
  那条是它的一个切片)。同源的第二个症状:**一个 `vX.Y.Z` tag 焊住三条节奏不同的发布线** ——
  0.13.6 为送一行 compose 配置付了一次完整 Windows 构建 + 给所有桌面用户推了个空更新,
  0.13.7 因单个 `images (cli)` job 挂掉整版作废。症状、拆分方案与优先级在 `docs/cd-plan.md`。
  **刻意不含实施**:最关键的一步(手工走一遍 provisioning 并记下每条命令)还没人做过,
  没走过就写 IaC 等于把猜测固化。(L)

## 数据生命周期与增长 🆕

> 2026-07-22 新增小节。多处「删除不真删」+「无界追加」,单节点小盘上会慢慢暴雷。

- 🟡 **容量配额:blob 已封,其余仍无**(2026-07-30)—— ~~无任何容量配额~~ 部分已做:
  **每工作区字节配额**上线,`MICA_WORKSPACE_QUOTA_BYTES` 默认 **1 GiB**(`0` = 不限)。
  四个存字节的入口全部经过 `files.rs` 的 `ensure_storable` —— presign / complete /
  import-url / 工作区导入的图片 re-host;`complete` 也查,因为 presign 只是建议(客户端可以
  跳过它、或复用一个 URL),真正建行的是 complete。用量 = 按 workspace 的 `sum(byte_size)`,
  **含已标记 unreferenced 但还没被 GC 扫掉的** —— 那些字节还在盘上,不算的话「删了再传」是个
  永不触顶的循环;dedup 已自然体现(同样字节 `insert_file` 返回既有行,只加引用不加体积)。
  拒绝时返回机器可读的 `workspace_quota_exceeded`。配套 migration 0017 建
  `idx_files_workspace_bytes`(带 `INCLUDE (byte_size)`,走 index-only scan)—— 这个求和进了
  上传热路径,不加索引就是全实例顺序扫,成本随**别人**的上传增长。**默认值按实测选**:生产
  最大工作区 57 MB,1 GiB 是 ~18 倍余量。解析上有一处刻意的反常:**乱码保留默认值而不是变成
  「不限」** —— 对一个安全上限,不可解析绝不能解析成「没有上限」。
  **仍然没有的**:单文档大小上限、用户级/实例级总量上限;WS 路径默认仍可收 64MiB 单条消息;
  大文档写放大照旧(每 push 全量 base 覆写 + 每 10min 全量版本)。**未实测的一处**:import-url
  的配额检查没能端到端验到 —— SSRF 守卫先拒了 loopback 测试 URL(顺序正确),代码路径与另三条
  一致但只经代码核实。
  (`files.rs` `ensure_storable`, `store.rs` `workspace_bytes_used`, `ws.rs:60`, `sync.rs`)(M) `[需后端]`
- ⏸️ **回收站不做自动清空 —— 已拍板(2026-07-29),不是遗留 TODO**。现状:纯 `is_deleted` 标志,只有用户手动「恢复 / 永久删除 / 清空回收站」三个出口(`documents.rs` 的 trash/restore/purge + `mod.rs:164` 那条 DELETE),**没有任何定时任务**碰它;blob GC 刻意把回收站引用算存活(`blob_gc.rs` `referenced_file_ids` 不过滤 `is_deleted`),所以图片 blob 跟着一起留。**决定不做自动过期删除**,四条理由:
  1. **同类一致**:AFFiNE、AppFlowy、Notion、Obsidian 在「用户自己是唯一管理员」这个相同约束下**都没有**做"回收站到期自动永久删除"。AFFiNE 的 blob 宽限期只针对已失活 blob,页面本身不过期;AppFlowy 干脆没有 blob GC。这是跨产品收敛的答案,不是巧合。
  2. **它解决不了真正的风险**。真风险是"单节点小盘 + 开放注册被撑爆",根源是**没有容量配额**(见本小节「无任何容量配额」条)。用一个会误伤用户数据的机制去掩盖一个该用配额解决的运维问题,是治标且引入新的不可逆风险。
  3. **代价不对称,且备份补不上**。每日全库 `pg_dump` 含回收站的文本与 CRDT 历史,但**不含 blob**(第二对象存储)。自动清空后就算靠备份找回文字,配图也真的没了 —— 对自托管的单管理员,这个恢复成本大概率承担不起。
  4. **手动出口已经齐备**:`_confirmPurgeAll` / 单条永久删除都带二次确认,要腾空间今天就能腾,不需要系统替用户决定。
  **配套(小,未做)**:回收站占用可见性 —— 照 `export_all_stats` 现成模式统计"回收站里几页 / 独有 blob 多少字节",显示在回收站对话框标题栏,让用户自己看见再决定。这比替他自动删更符合上面四条。(S)

## 编辑器与功能广度

- 🟡 **全文搜索**(2026-07-22 复核:原描述失实——不是「反序列化每篇快照」,是每查询把每篇 yrs base **全量 CRDT 解码**一遍,N 次 decode)—— **M1 已做**(aa4c5d8):加 `document_yrs_base.content_text` 派生列(migration 0012),搜索退化为**一条 LEFT JOIN + ILIKE SQL**,干掉 N 次 CRDT 解码。content_text 是 state 纯投影、三条写 base 路径同语句 co-write(红线#1 不漂移);启动一次性回填存量;LIKE 转义 + 命中处窗口 snippet;CJK 走子串(无扩展/无分词器)。**残留**:~~②导入未打开的文档正文不可搜~~ ✅(2026-07-22,导入两条路径 commit 后 best-effort `bootstrap_base` 即建 base+content_text,等价「导入即打开」,复用现有构建器无新写路径;回归测试锁住;postgres:16-alpine 自带 pg_trgm 已确认)。**故意缓做**(非尾巴,按「不过度设计」):① **pg_trgm GIN 索引**——当前 22MB 库、查询已被 (workspace_id,is_deleted,object_type) 索引收窄再 ILIKE,亚毫秒;GIN 只加速 ≥3 字子串(CJK 2 字仍 seq-scan),不是干净胜利。真到大规模(万级文档)时一行 `CREATE EXTENSION pg_trgm` + `CREATE INDEX` 升级,现在加是为假想规模优化;② 排序/高亮/分词——各自独立 UX 特性,另立项非 M1 尾巴。(各 S–L) `[需后端]`
- 🟡 **表格**(2026-07-22 复核:原描述大幅失实)—— 实测:**富行内单元格**(粗体/斜体/行内代码/链接,cell 存可重解析 md 源码、两端渲染+编辑,`cellDisplaySpan`/`CellEditController`)与**矩形/行列选区**(跨格拖选、点行/列把手选整行列、Ctrl+C/X 复制为 TSV+HTML、Delete 清空、Esc 清除)**本来就能用**;本轮仅补 **Shift+点击扩展选区**。**合并单元格有意不做**——8 家同类(Notion/AFFiNE/AppFlowy/Outline/siyuan/Joplin/logseq/anytype)调研定论:合并与「Markdown 权威 + round-trip 不变量」在 GFM 下**架构级互斥**(siyuan 能合并因它放弃了 md 权威;Joplin 同约束只能冻单向 HTML;Logseq/Notion 干脆不做)。要做只能另开 HTML 逃生舱块退出 round-trip,是独立决策。块级单元格/列宽 GFM 表达不了,同样不做。
- 🟡 **反向链接/引用面板/关系图** —— 正向 `[[` 已建;**引用面板已做**(云端页显示「谁链到我」可点跳转,`GET .../backlinks` 按需扫描、复用 page_link_targets,7de2c2a)。**残留**:~~①并发扫描~~ ✅(buffered(8),6612330);②规模成瓶颈再上维护式反向索引表(现在故意不建);③本地世界(offline)反链;④**关系图**(graph view)。(各 S–L)`[需后端]`
- 🟡 **页面属性/标签**(**M1 已完成**,2026-07-22)—— 走 front matter 权威路(调研定论:同类 md 权威系均如此,见 `docs/page-properties.md`)。**M1 全部落地**:① 数据/权威层——Rust `crates/markdown/src/properties.rs`(解析扁平子集 + 类型推断 + 外科式写回,round-trip 不变量经用户批准从字节保真降为规范化子集稳定)+ Dart 镜像 `properties.dart`,两端逐条测试一致(Rust 9 / Dart 10 全绿);② 页头属性面板 `property_panel.dart`(读 root 块 `data['front_matter']` → 类型化编辑:文本/数字/日期文本框、勾选、tags chips 增删 + 增/删属性 → 编辑经 `onApplyOperations` 单入口自分派写回 root 块,local/cloud-CRDT/cloud-REST 三模式通用,无需穿层新回调);flutter build windows 通过。tags = `tags:` list 属性。**Obsidian-lite 闭环已完成**:增删改属性(类型 text/number/checkbox/date/list)、tags chips、**可搜**(属性值折进 content_text,list 值以 `#值` 存)、**tag 点击精确跳页**(搜 `#值` 只命中真正带该标签的页,ce13cef)、**默认隐藏在页头 ⓘ 图标后**(不占版)+ **AppFlowy 式面包屑路径**(579272f)、AFFiNE 式紧凑面板(7379444)。**故意不做/另立项**:① 数据库视图级「按属性筛选/排序/看板」——是 Notion 数据库那套,与 markdown 权威+round-trip 架构互斥(要豁免 md 权威,AFFiNE/siyuan 路),独立大决策;② 存量页要下次编辑才索引属性(backfill 只填空行,属性是新功能故不强制全量重派生);③ 日期选择器 UI(现文本输入)。**数据库视图(带类型列/筛选/relation)另立项**——与 markdown 权威+round-trip 架构互斥,要么破双表示红线要么豁免 md 权威(AFFiNE/siyuan 路),是独立大决策。(L) `[需后端]`
- 🆕 **评论 Phase 2 + 建议(suggest mode)**(2026-08-03 立,Phase 1 整条已归档)—— Phase 1 已闭环并
  经真机验收(评论栏、跨块高亮、锚点随文字位移),整条见 `roadmap-done.md`。**没做的三件**:
  ① **orphan 模糊重锚** —— 锚定文字被删后 thread 只剩 `quote`,现在是死的,按 quote 模糊重锚未做;
  ② **@提及 / 通知** —— 需要通知底座(现在没有),不是评论本身的活;
  ③ **建议(suggest mode)** —— **刻意另立项**:建议是正文内 insert/delete overlay,与评论
  (side-store、正文一字不动)是两个问题,`comments-plan.md` 明写「别共用存储设计」。
  (①S / ②M `[需后端]` / ③L)
- 🟡 **结构块 callout/toggle/embed/columns** —— **callout 已做**(GFM alert `> [!TYPE]` 5 类型,复用 quote 扁平模型、round-trip 干净、记分牌未降,e7ff038)。**残留/定论**(2026-07-22 调研):① **toggle** —— **已拍板(2026-07-29):分两步,先做渲染层折叠 UI**。关键区分:`<details><summary>` 是**合法的 CommonMark/GFM raw-HTML block**(GitHub 官方推荐写法),属于"我们的解析器没接住",**不是**像合并单元格/多列那样"标准 md 表达不了"——所以**不适用**那两条的「红线不做」先例。现状比参照产品还弱一档:AppFlowy(原生 `toggle_list` block)和 AFFiNE(`collapsed` 做成 list/heading 通用属性)**编辑器里都有可点的折叠 UI**,只在导出 md 时降级;我们是连折叠 UI 都没有,`<details>` 源码当代码块摆着。~~**第一步(S)**~~ ✅ **已做(2026-08-03)**:纯渲染层折叠上线,`code_block`+`raw:true` 的解析/序列化一字未动,折叠态复用 `data.collapsed`(**实测 round-trip 字节不变**:点开折叠只写 `data.collapsed`,服务端读回的 `<details>` 仍无 `open`)。走 `AtomicBlockRenderer` 注册表 —— **顺带关掉了 P3-1**:注册表原本是 kind→renderer 的 Map,而 `code_block` 已被 Mermaid 占住,第二个 renderer 会**静默替换**它、无编译错误;现在是 kind→List,按注册顺序第一个不返回 null 的胜出(`render-architecture.md` 已改)。**但覆盖面比原计划小,原因是解析形态**:`<details>` 两种形态解析结果完全不同 —— **紧凑形态**(不留空行)是**一个** raw 块,已折叠;**GitHub 文档推荐的空行形态**(留空行让正文按 Markdown 解析)因为空行终止 type-6 HTML 块,解析成**三个块**(`<details>+<summary>` / 真正的 Markdown 正文 / `</details>`)。折叠后者要隐藏的是**一段范围**,而 `_layouts[i]` 在渲染器里全按节点下标索引,跳过节点会整体错位 → 得改成「零高度隐藏布局」,牵动选区/光标/命中测试/拖拽把手多条路径 —— **那是第二步的量级,不是第一步的尾巴**。空行形态今天仍是「两段源码夹着正文」,与改动前一致、不更差。**第二步(M–L,另议)**:规范子集结构化成新 kind(summary 富文本 + body 复用 `data.li` 扁平容器),非规范形态退回现状直通;成本大头在教导入器反解析真实世界五花八门的 `<details>`。**顺带定论**:折叠状态**是文档数据不是视图状态**——AppFlowy(`updateNode` 事务)、AFFiNE(`store.updateBlock`)、以及本仓库自己的代码块折叠先例(`controller.dart` `toggleCollapsed` 走 `update_block`,但 `collapsed` **从不进 md 字节**)三方一致;`<details open>` 让我们有机会比前两家更彻底(折叠态也能 round-trip),但那意味着"点一下折叠"变成一次真实文本编辑,留到第二步再定;② **columns** —— **红线不做**(标准 md 无多列表示,同表格合并;要做只能显式有损方言);③ embed 未做。附:render 注册表 P3-1 对这三种块**不是前置**(仅撞已有 kind 如 Graphviz 时才需)。(各 S–L)
- **无屏幕阅读器语义(a11y) / 无 RTL 双向文本** —— 自绘 RenderBox 无 Semantics;硬编码 `TextDirection.ltr`(editor-engine, `render.dart`)。
  **2026-08-03 核实:比原文写的更糟** —— 编辑器目录里 `Semantics` **0 处**(不是「少」,是没有),
  `TextDirection.ltr` **32 处**(原文写「10+」)。缓解:设置里有 85–140% 应用内字号(`EditorAppearance.fontScale`),覆盖低视力一部分。(各 L)
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
- ⏸️ **Windows 未签名(SmartScreen 告警)—— 已拍板不做(2026-08-04)**,不是遗留 TODO。路径本身清楚(SignPath CA 证书接入 Inno SignTool,desktop-plan),**卡的是买证书**。代价:每个新用户看到的第一样东西就是「不可信」。(M)
- 🟡 **自动更新器不校验下载完整性/哈希/签名**(medium) —— **完整性校验已做**(508808e):下载后经纯函数 `installerMatches` 验 `size`(GitHub `assets[].size`,恒有→拒截断)+ `sha256`(`assets[].digest`,GitHub 服务端算→拒换包/损坏),不符即删+`updaterIntegrityFailed`,绝不运行;无 digest 老 release 退化仅 size。**残留**:① **Authenticode 签名**——安装包本身仍未签名(需代码签名证书,独立项);② digest 依赖 GitHub 是否为我方资产填充(未填时仅 size 兜底),要更强可让 release CI 自发 `SHA256SUMS`。(`updater_desktop.dart` `installerMatches`)(证书项 M / 其余已做)
- **window_manager→nativeapi / Turso 观望**(各 S,已隔离在 trait 后)。

## 客户端质量与兜底 🆕

> 2026-07-22 新增小节。离线功能面做得全,但崩溃/损坏/双开几处兜底缺失会真丢数据。

- 🆕 **i18n 漏网**(low) —— 默认页名 `kUntitledPage='未命名页面'` 硬编码中文并持久化(英文用户新建页得到中文标题、且与 'Untitled' 双轨),代码块 AI 动作 prompt 全中文;语言仅 en+zh。(`models.dart:667`, `editor.dart:5109`)(S)

## 性能

- ⏸️ **每次 push 重建+重编码+重写整档(写放大)—— 已量化,拍板暂不做**(2026-08-04)——
  `push_update` 每次:`from_update` 全档解码 → apply → `encode_state` 全档重编码 →
  重新派生 `content_text` → upsert 整行。成本 O(文档) 而非 O(更新)。
  **动手前必读**:隔壁那条「yrs base 无 GC、无界增长」经实测为**假**(已归档),而它之所以
  为假,**正是因为这趟昂贵的 round trip 本身就是一次 squash**(yrs 默认 `skip_gc=false`,
  加载即回收已删内容)。所以「别每次都重建 base,改成追加 + 定期 squash」这个显而易见的修法
  **会把那条不存在的无界增长真造出来** —— 两条不是同一个根因,一个是另一个不发生的原因。
  回归测试 `base_compaction::deleted_content_does_not_survive_in_the_base` 守着这条,
  改写路径时它会红。
  **另一条约束**:`content_text` 是 `state` 的纯投影、同语句 co-write(红线#1)。base 一旦
  改成惰性重建,搜索索引就会在两次 squash 之间陈旧 —— 那是个**产品取舍**,必须摆到台面上
  决定,不能顺手做掉。
  **2026-08-04 实测,结论与写下这条时的直觉相反**。生产 3779 篇文档的 state 体积:
  p50 **6.6 kB** / p90 27 kB / p99 **68 kB** / 最大 **312 kB**。按这三档跑
  `cargo run --release -p mica-app-core --example bench_push`(解码+重编码+to_blocks):
  **0.08 ms / 1.04 ms / 3.60 ms**。而端到端一次 push 实测 **~21 ms** —— 这趟「昂贵的
  round trip」只占 **1–15%**,其余是数据库往返和网络。
  「O(文档) 而非 O(改动)」结构上完全成立,**但乘数比预想小一个量级**。拿最大文档 3.6 ms
  去换「自己重新实现 GC」+「搜索索引陈旧」两个风险,不划算 —— 用户 2026-08-04 拍板暂不做。
  **触发重新拿出来的条件**(任一):① 最大文档超过 **2 MB**(按线性外推 ≈ 25 ms);
  ② `mica_crdt_push_duration_seconds` 的 **p95 超过 200 ms**;③ 出现真实的编辑卡顿反馈。
  bench 留在仓库里,下次判断重跑一次就有数,不用重新猜。(M) `[需后端]`
- **frb v2 热路径 FFI 基准待测** —— IME/逐字输入若过慢,热路径留 Dart(phase2 §12)。(M)

## 开发者体验 / CI / Markdown

- 🟡 **CI 补 Windows 集成测试**(2026-07-23:离线子集已进)—— 新 `.github/workflows/flutter-integration.yml`:windows-latest **串行**跑 **14 个离线/客户端**集成测试(文件间杀 `mica_flutter.exe`,化解单实例守卫导致的 debug-connection race——已复现:残留进程锁住下次启动)。**残留**:4 个需活的 dev 栈(postgres+rustfs+api)的测试仍排除(`cloud_sync_test`/`migration_sync_test`/`offline_image_reconcile_test`/`page_switch_fidelity_test`,含那对 race 文件)——CI 里起全栈超范围。且 12/14 是读文件头判定离线(实跑了 2)、首次 CI 真跑确认。(M) `[需后端]`(残留部分)
- 🟡 **不可信输入解析面 fuzz:三个面都已覆盖,yrs 那面挖出三类问题、已报上游并提 PR**(2026-08-05)—— 三个吃不可信字节的面:markdown 解析、ZIP 导入、yrs 二进制更新。**前两个**(`markdown/tests/proptest_parse.rs`、`interchange/tests/proptest_zip.rs`,2026-07-23)未挖出 panic,落成快回归门。
  **第三个 2026-08-05 补上**(`mica-core/tests/proptest_yrs.rs`),而**搁置它的理由本身是错的**:原条目写「UB 要 cargo-fuzz + sanitizer 才抓得住,proptest 只抓 panic 不够」—— 实际普通 proptest **几秒就撞上了**,根本没用上 ASan。搁置的代价是这个面白空了两周。
  **挖出三类**(yrs 0.27.3,最新版):① `assert!` panic(`block.rs:92`)—— unwinding,服务端 `catch_unwind` 兜得住;② **UB** `invalid value for char` —— 非 unwinding,兜不住,release 下是静默 UB;③ **无界分配** —— **21 字节让 yrs 要 215 TB**,分配失败直接 abort,debug/release 都复现。②③ 从 `push_update` 可达 = **任何已认证客户端都能打挂 api 进程**。
  **调研结论:别人没规避掉。** y-crdt#415(2024-04 至今 open,标 bug,已指派)提交者原话是生产机器被打挂;AppFlowy 在同一 issue 下报同样问题;#373(evanw)是另一类堆损坏/segfault,同样 open。AppFlowy 的 CRDT 层用的也是 `catch_unwind`,和我们一模一样 —— **挡住的是同一类,漏掉的也是同一类**。
  **已做的处置**:③ 的复现器发到 #415;**PR y-crdt/y-crdt#644** 修 ②③ 两类(`any.rs` 的 `with_capacity` → `try_reserve`,沿用该仓库自己在 `block.rs`/`update.rs` 的既有模式;两处 `from_utf8_unchecked` → 检查版),含回归测试,yrs 全量 375+34 通过,回滚任一处补丁测试即 abort。
  **残留 = 等上游**。这一侧兜不住:限制输入大小没用(才 21 字节),预校验等于重写解码器,进程隔离业界无一家这么做、在单用户实例上不成比例。本地 `proptest_yrs.rs` 全部 `#[ignore]`(两类会 abort 测试进程,不 ignore 就是把 CI 打挂而不是报告),上游合并后去掉 ignore 即变回归门。
  **⚠️ 触发条件:开放注册前必须解决。** 今天风险低是因为注册关闭、只有一个账号;有第二个用户那天,任何普通成员都能让实例反复重启,而且不需要技巧 —— 我是随机灌字节撞出来的。(残留:等上游) `[需后端]`

- 🟡 **cli 测试 + 覆盖率度量**(2026-07-23:起步)—— 原 `crates/cli` 零测试 + 无覆盖率工具。**已做**:9 个纯逻辑单测(`url_file_name`/`slugify`/`sanitize_rel` 路径防穿越/`workspace_dir`/`mirror` 备份 reconcile 增删剪/`Config` serde),`ci.yml` 测试步补 `-p mica-cli`(进 CI),`just coverage`(`cargo llvm-cov`,不入 CI 门)。**残留**:REST `Client` 方法需活服务端未测;`config_path/load/save` 走进程级 env + 真实用户配置目录,未注入点故略(用 serde 落盘形状覆盖)。覆盖率数字化了但远非高覆盖。(S)
- 🆕 **Linux 桌面在仓库但从不在 CI 构建**(low) —— `linux/` runner + 托盘降级逻辑在库,CLAUDE.md 还为它写了约束,但 CI/release 都无 `flutter build linux` → 编译债不可见。flaky 债本身很轻(仅 2 个带理由 `#[ignore]`)。(M)
- 🟡 **可观测性:`/metrics` + dashboard 已上,抓取器移出部署改为示例栈,告警⏸️不做,缺机外黑盒探测**(2026-08-04)——
  ~~仅结构化日志,无 /metrics~~ ✅ 已做。**顺带修正原条目的两处错**:它写 `telemetry.rs`,
  而那文件在 `crates/infra` 不在 api-server,且只有 **12 行**——只初始化 tracing 的输出格式,
  没有任何计数器。所以起点不是「有 telemetry 但没暴露」,是**什么都没数**。
  已落地:自研 exporter(`api-server/src/metrics.rs`,**不引第三方 metrics 库** —— 文本格式
  + 计数器 map 约 150 行,CLAUDE.md「粘合层用成熟包」针对的是要背三套平台原生层的情形,
  这里没有那个乘数)。指标选的是**能解释 08-03 那次故障**的:HTTP 请求数/延迟直方图(按
  **路由模式**打标签,不是原始路径 —— 实测两个不同 UUID 合并成一条 `{workspace_id}`,
  爬虫撑不爆基数)、**PG 连接池 idle/in_use**(那次的真实症状是 acquire 3.3s)、WS 连接数
  (drop guard,提前返回或 panic 都不会漏计)、build_info。
  **暴露方式是结构性的**:挂在顶层 `/metrics` 而非 `/api` 下,nginx 只转发 `/api|/ws|/s|`
  两个邮件链接,所以公网够不到。**实测机制与直觉相反**:公网 `GET /metrics` **不是 404**,
  是 SPA 兜底 `try_files` 返回 **200 + index.html**(text/html,不含任何 `mica_` 序列)。
  安全结论不变,但「它会 404」是会被人当依据的半对说法。
  采集:**不在 Mica 的部署里**(2026-08-04 用户拍板:「prometheus 不是必须的,属于可选项」)。
  它先是作为一个 service 进了 prod compose,当天就被摘出来 —— `/metrics` 是应用的一部分,
  谁抓、留多久、谁能看是运维的决定;分开之后监控栈升级/挂掉/写满磁盘都带不走应用,而已经有
  Prometheus 的人可以直接指过来。示例栈(prometheus + grafana,含 provisioning 好的数据源)
  在 `deploy/monitoring/`,以**外部网络**接进 Mica 的 compose 网络 —— 那是通往 `/metrics`
  的唯一路径。两个 UI 都只绑 127.0.0.1,看图走 SSH 隧道。
  **2026-08-04 二轮补齐(5 族 → 23 族,80 条序列)**,按「这个产品会怎么坏」而不是「监控
  系统通常有什么」选:
  ① **CRDT 完整性**`mica_crdt_integrity_failures_total{kind}` —— 红线 #1 是「绝不静默分歧」,
  而被拒绝的更新此前只是一行日志。**刻意以 0 值序列出现**:只在触发后才存在的计数器无法在
  触发前配告警,而那正是唯一重要的时刻(`absent()` 规则是不这么做的补丁);
  ② **push 成本**`mica_crdt_push_{duration_seconds,bytes_total,rejected_total}` —— 写放大
  那条从此有数,不再是争论;③ **客户端落后**`mica_crdt_lag_notices_total`;
  ④ **池 acquire 探针**`mica_db_acquire_probe_seconds` —— idle/in_use 是 15s 采样的瞬时
  gauge,会在两次采样间漏掉尖峰,而 08-03 的真实症状正是「acquire 3.3s」。**是探针不是全量
  埋点**(sqlx 在每个 `fetch_*` 内部隐式 acquire,全量要动所有调用点),名字里写明了;
  ⑤ **进程 RSS / FD**(读 `/proc`,Linux only —— Windows 开发机**省略该序列而不是报 0**,
  报 0 会被读成「没用内存」);⑥ **in-flight 请求数**;⑦ **blob GC** sweeps/scanned/deleted/
  bytes_freed/failures(日志里本来就有这些数字);⑧ **容量**:users/workspaces/documents 计数 +
  `mica_storage_bytes{scope="total"}` + **每工作区** `mica_workspace_bytes_used{workspace_id}`
  + `mica_workspace_info{workspace_id,name}` + 生效中的配额值,DB 快照缓存 60s(抓取每 15s,
  不能让「看系统」变成系统的负载),查询失败**返回陈旧值而非丢序列**(丢序列看起来和「归零了」
  一模一样)。
  **per-workspace 这一版是纠错**:第一版只发聚合(总量 + 最大的那个),理由是「工作区数量无界」。
  错在原则上 —— **exporter 只暴露事实,策略归告警规则**,node_exporter 从不预先滤掉「还没满的
  盘」。预筛的代价具体有三:阈值被烤进二进制(改一次要重新部署)、**丢历史**(工作区跨过阈值才
  突然出现,于是算不出增长速率,而增长速率是唯一能提前预警的东西)、画不出「用量前 N」。名字走
  **info metric** 而不是打在每个样本上,否则一次改名就会把序列历史劈成两半。基数上限 1000 是
  **上限不是过滤**,按用量降序,撞上了由 `mica_workspace_series_truncated` **明说**。
  容量 gauge 与配额执行是**同一张表上的两条 SQL**=典型双表示,`the_capacity_gauge_matches_
  what_the_quota_enforces` 把两者钉在一起。
  **残留**:① ⏸️ **告警规则与通知通道:已拍板不做**(2026-08-04 用户决定,理由与代价见排序
  清单第 1 条 —— 单人自托管、没有值班轮转,一条没人 ack 的告警和没有告警区别有限);
  ② **node 级指标**(CPU/磁盘/内存)要 node_exporter;③ **外部黑盒探测**(含 TLS 证书剩余
  天数)必须跑在这台机器之外 —— 08-03 那次整机 IO 饥饿,同机采集器一样会被饿死。(残留 M)
- **可选/later 基建:Redis、OTel、索引块表** —— 索引块表是搜索/反链/分析的底座(architecture.md)。(L) `[需后端]`

## 产品与公开发布合规 🆕

> 2026-07-22 新增小节。生产节点已上公网 + 开放注册,这类义务是上线后才暴露的。
>
> **2026-07-23 进度**:三项 medium 硬缺口(关注册开关 / 账号删除 / 密码找回,后者顺带
> 建起邮件底座)已全部落地并发版 0.12.16 上线端到端验证。剩下均为 low:AGPL 源码入口、
> 隐私声明·条款、OFL.txt 随附。

- 🆕 **已上线实例无隐私声明/服务条款**(low) —— 正面:诊断 opt-in 默认关、无 telemetry 回传,产品内隐私姿态好;缺口是外部合规面,仓库无任何面向用户的隐私政策/条款文本。(M)

## 接下来最该做的(2026-08-04 重排)

> **这是 top-N,不是清单全文。** 上面正文有 **37 条**,其中只有 3 条是 ⏸️ 已拍板不做,
> **其余 34 条都有真实的未做部分**。这一节只排「下一步最该动手的」,所以它短不等于活干完了 ——
> 2026-08-04 它缩到一项,是因为用户当天拍板删掉三项 + 归档一项,不是因为没事可做。
> **别拿这一节的长度当进度**:要看还剩什么,读上面的分区。
> 这一节短的真实含义是:**剩下的都不是缺陷、也不是底线**,该由真实需求驱动排期。

> **旧清单第 1 项那四件安全底线已关**(WS token TTL 心跳、token 出 query string、桌面
> DPAPI + web 凭据走 HttpOnly cookie、弱口令),整条搬进存档。⚠️ **这不等于「安全整档已关」**
> —— §安全 小节仍有 4 条(SVG 纵深、TLS 硬闸门、compose 注册默认、清单卫生),只是都不到
> 底线级。(2026-08-04 复读时抓到的:上一版这里就写成了「整档已关」。)
> 08-03→08-04 又关掉五件:评论 Phase 1、`<details>` 折叠第一步、web outbox 背压、本地文档
> 增量持久化、同块并发编辑的**文本**那半。
>
> **上一版这里有一句错的,已删**:它说「数据面无界增长(yrs base 无 GC + push 写放大)是
> 最该先动的」,而 **yrs base 无 GC 早已实测为假并归档** —— 那句话会把人带去修一个不存在的
> 问题。写放大那条仍然成立,但它单独不构成「无界增长」。这正是维护规矩第 3 条要求把核实
> 结论写进条目本身的原因:结论进了条目,却没同步到引用它的排序清单里。

1. **provisioning 层**(L,方案在 `docs/cd-plan.md`,**刻意不含实施**:手工走一遍还没人做过,
   没走过就写 IaC 等于把猜测固化)。
   **这是清单里仅剩的一项。** 同批出清的:M8(读侧 parser)已拍板继续自研并整条搬进存档;
   **写放大已量化后拍板暂不做** —— 最大文档 3.6 ms、占一次 push 的 1–15%,换不来「自己重新
   实现 GC」+「搜索陈旧」两个风险,触发条件写在 §性能 那条里;另外三项见下方 ⏸️。

### ⏸️ 2026-08-04 用户拍板从这份清单里拿掉的三项

条目本身留在各自小节里(它们是真实的活),只是不再占「下一步该做什么」的位置。**代价要写明**,
否则这份清单会用沉默假装问题不存在。

- ⏸️ **机外黑盒探测**(原第 1 项)—— **代价**:① **TLS 证书过期无人看守**,而 ACME 卡死是
  **静默的全站故障**;② 08-03 那种**整机 IO 饥饿**看不见 —— `/metrics`、同机采集器、存活探针
  会一起被饿死,装在这台机器上的任何东西都照不到它。**触发重拿出来的条件**:有了第二个用户、
  或开始对外承诺可用性、或证书真的过期过一次。
- ⏸️ **Windows Authenticode 签名**(原第 2 项)—— **代价**:每个新用户装包时看到 SmartScreen
  说「不可信」,而那是他们看到的**第一样**东西。更新器的 size+sha256 校验只防传输损坏,
  不解决这个。**本来也卡在买证书**,不是工程问题。
- ⏸️ **成熟度大件「挑一个」**(原第 3 项)—— 剩下两个候选**都不是缺陷,是功能广度**:
  反链剩余(云端引用面板**已能用**,剩的是本地世界对等 + 关系图 —— 后者是产品下注)、
  `<details>` 折叠第二步(条目自己写着当前「与改动前一致、不更差」,而成本要把布局改成
  零高度隐藏,牵动选区/光标/命中测试/拖拽把手)。留着只会重演刚清掉的**幻影选项** ——
  一份没人会去点的菜单。真有需求时条目就在 §编辑器与功能广度 里。

**清单缩到两项本身就是个信号**:剩下的既不是缺陷也不是底线,而是一个待拍的决定 + 一笔基建
欠账。产品面已经进入「稳定期」,再排期就该由真实需求驱动,不该由清单自己驱动。

**下一轮核实的重点**:**未核实的 2 条**(fuzz 覆盖面、隐私声明/ToS),以及所有 🟡 条目的
「剩下那半」—— 部分完成的条目最容易整条烂掉:它已经有了一个 ✅,看上去就像做完了。

**这份排序清单本身也要核**,而且它烂得更隐蔽:读的人以为它是结论的索引,于是不会去对条目。
两轮实例:2026-08-03 那版留了一句「yrs base 无 GC 是最该先动的」,而那条早已实测为假并归档;
08-04 重排后复读,又抓到三处 —— 「安全整档已关」(实际还剩 4 条)、「表格补齐」(实际全是有意
不做)、`/metrics` 在第 1 和第 5 项各排了一次。**规矩**:改完条目就回来对一遍这份清单,它引用
的每个结论都要能在条目里找到同样的话。
