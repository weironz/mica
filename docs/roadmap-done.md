# 路线图 — 已完成条目存档

> 从 [`roadmap.md`](roadmap.md) 搬出来的完成项,**原文一字未改**(含当时的日期、
> commit、核实记录)。搬而不删,是因为这里面很多条写的不是「做了什么」,而是
> **「为什么必须这么做/这么排」** —— op 模型退役那六步、协议版本门那三个刻意的设计点、
> 备份缺一条腿就整次失败的取舍,都属于下次遇到同类问题会想回头查的东西。
>
> 待办总账仍是 `roadmap.md`,**这里只进不出**。
>
> 2026-08-03 建档,当时搬进 68 条。

## 可靠性与同步

- ~~**P2-M4 云同步流未真正建**~~ ✅ **主干早已上线**(2026-07-29 对代码核实,此条整条过期)—— `sync.rs` push_update 写流+fold、`catch_up_document` 按 since_rid 续传+剪枝缺口自动 Rebootstrap、`diff_from_base` SV 兜底;WS `sync.bootstrap/pull/push` 三 handler(`ws.rs:336-440`)+ 客户端 `_pullPayload(since_rid+sv)` 消费;真 PG 集成测试 `sync_pg.rs`。**它一直被记成未建,把整棵依赖树都记歪了** —— 实际解锁的下一步是 op 模型退役(见「数据生命周期」)。
- 🟡 **M-R 收尾**(2026-07-29 核实:4 项里 3 项早已落地)—— ~~C3 坏更新自愈+schema 版本~~ ✅(`store.rs` SCHEMA_VERSION=6 + 太新拒开 + 每 blob CRC-32 + yrs panic 包成 CorruptDoc;客户端坏副本冷 bootstrap 自愈、坏 remote 封顶熔断);~~D2 同步健康态~~ ✅(`sync_status.dart` → _SyncBadge + fault banner);~~A3 会话持久化 e2e~~ ✅(`cloud_sync_integrity_test.dart`:未 ack 编辑跨重启重推)。~~D1 尾巴~~ ✅ **已做(2026-07-30),M-R 整条关闭** —— `lib/swallowed.dart`:`swallowed(tag)` 累加、`swallowedCounts()` 出不可写快照、`swallowedSummary()` 出一行给 bug 报告。5 处带注释的故意丢弃接上了(`cloud_ws_ready`/`cloud_ws_stream`/`cloud_ws_uri`/`presence_ws_ready`/`presence_ws_stream`/`ai_ws_ready`),**丢弃行为一个字没改** —— 改的只是「丢了多少次」不再无从得知。**刻意不接 `onFault`**:那是同步故障横幅的入口,而连不上服务器是**状态不是故障**,把普通离线送进去等于给一个 app 已经处理正确的状态贴红标。可见面在 Settings → 诊断,**仅在非零时出现**(每次都显示「无」只会训练眼睛跳过它,而那一行不在本身就是同一个意思),且**不受诊断开关门控** —— 没有内容要记录,价值全在「事后能回头看」,而那恰恰是提前打开开关不可能的场合。测试:`swallowed_test.dart` 5 条(含快照不可写穿),外加在 `sync_reconnect_token_test.dart` 那条既有测试上挂了**接线断言** —— 纯模块单测证明不了接线,已实测撤掉 `swallowed('cloud_ws_uri')` 它立刻变红。**已知空白**:web 上 Settings 的诊断区整段隐藏(`diagnosticsSupported=false`,没有文件系统),所以 web 目前**没有**读回计数的地方 —— 与既有诊断故事一致,不另开口子。
- ~~**离线→在线 blob 自动 reconcile**~~ ✅ 已做(2026-07-29,核实于 2026-07-30)—— 原文「`_reconcilePendingUploads` 全仓唯一调用点在 onReady」已假:现在 3 个调用点,`_onCloudOnline` 那一侧也扫 blob 了(`main.dart` 里 onReady、活动文档重连、`_sweepPendingOutboxes`)。
- ~~🆕 **重连复用过期 token → 永久退避循环**~~ ✅ 已修(2026-07-29,核实于 2026-07-30)—— `uri` 由 `final Uri` 改成 `final Future<Uri> Function()`,每次连接尝试重新求值,3 处构造点全部迁移;长开会话那一处会先 `await _ensureFreshSession()`。回归测试 `test/sync_reconnect_token_test.dart`(钉的是「每次尝试都重新问一次 URI」,不是「能连上」)。~~**服务端那半仍未做**:socket 建立后不再校验 exp,没有「exp 到期 close(4401)」心跳~~ —— ✅ **2026-08-03 已做**(`fd509f2`,见本文件安全小节「长连 WS 超 token TTL 不再认证」)。〔存档是原文留档,但一句已经变假的「仍未做」正是本次整顿要消灭的东西,所以补标注而不是改写。〕
- ~~🆕 **web 端在浏览器不报 locale 时启动即崩**~~ ✅ **已修(2026-07-30)** —— 触发条件定位到 `navigator.language === "C"`(POSIX 风格,非合法 BCP-47)。逐一遍历确认:`C` **CRASHED**,而未指定 / `''` / `en` / `en-US` / `zh-CN` / `en_US` 六种全部 BOOTED —— CI runner 的系统 locale 恰是 `C`。抛点在**引擎内部**的 `new Locale`(`RangeError`),Flutter 的 Dart 源码与 pub cache 里都搜不到这句原文,所以 Dart 侧根本捕不到。于是修在能修的那一层:`web/index.html` 里、`flutter_bootstrap.js` **之前**一段内联脚本,**仅当 `navigator.languages`/`language` 里没有任何合法标签时**才用 `en` 兜底;已经像语言标签的值原样保留,所以不会改变真实用户拿到的语言(实测那六种一个都没被改动)。web e2e 第 7 组断言守着,并实测过:从产物里摘掉守卫它立刻变红并打出原始错误。原文如下 —— headless Chromium(CI runner 上未指定 locale)加载页面后抛 `Error: Invalid argument(s): Incorrect locale information provided`,以 JS `pageerror` 形式逃出 —— 即发生在 `runZonedGuarded` **之前**、引擎层的 locale 解析阶段。后果:`main()` 走不到 `registerYjsSelfTest()`,`window.micaYjs*` 三个钩子全是 undefined,`document.body.children` 只剩 1 个(canvas 外壳)—— **整个应用没起来,而 console 里一条错误都没有**(未捕获异常不是 console message,是加了 `page.on('pageerror')` 才看见的)。本机 Chrome 撞不到,它带正常 locale。**尚未定位到具体哪一行**:`loadPersistedLocale` 只做 `Locale('zh'|'en')`,抛不出这个;`ArgumentError` 以 JS 异常形式出现,指向引擎 / `intl` 的系统 locale 探测。e2e 侧已用 `newContext({locale:'en-US', timezoneId:'UTC'})` 钉死 —— **那是给 harness 的确定性,不是这个 bug 的修复**。真实影响面待评估:locale 被隐私工具剥掉或环境异常的用户,可能只看到一个白页且无任何提示。(S-M,纯客户端)
- ~~🆕 **登录页「连不上服务器」会永久粘住**~~ ✅ 已修(2026-07-30)—— 两半都做了:**自动**有界退避重探(2s / 5s / 12s,够覆盖冷启动和「我刚把后端起起来」,然后停 —— 登录页无限探测是没人要的后台任务),**手动**在 `unreachable` 时才出现一个重探按钮(绿点旁边挂刷新按钮只会招人乱点;预算用完后它是唯一的回头路)。`dispose` 里取消定时器 —— 登录页恰恰是最容易在网络调用途中被 pop 的那一屏。复用已有 i18n key `commonRetry`,不新增。5 个回归测试(`sign_in_pane_test.dart`),并实测:撤掉 `_scheduleRetry` 后其中 3 个立刻变红。原文如下—— `SignInPane._probe()` 只在 `initState` 跑一次,失败后唯一的重试出口是「切到本地模式页再切回来」(`sign_in_pane.dart` 那段注释自己记了半个:「在真机上撞到过:开着这个屏才启动后端,行里一直说连不上」,当时只补了切 tab 这一路)。冷启动那一下网络/DNS 未就绪就会红着不动,而几秒后登录完全正常 —— 用户看到的就是「说连不上但我能登进去」。生产侧无关:连续 6 次 `GET /api/health` 全 200,DNS 5–9ms、总耗时 42–366ms,对 4s 超时有两个数量级余量。修法:给那行状态一个重试出口(点一下重测)+ 失败后退避重试;别再依赖运气。(S,纯客户端)
- ~~**双向 state-vector 协商**~~ ✅ 已做(校准复核)—— P4-3:`ws.rs:508` `client_sv.and_then(|sv| sync::diff_from_base(base, sv))` 按 client SV 发最小 diff,base_message delta 分支 + 单测 `base_message_sends_delta_only_when_sv_yields_one`。
- ~~**broadcast lag 触发整档重载**~~ ✅ 已做(校准复核)—— 客户端 `_resyncFromLag` 发 `sync.pull` 带 cursor+SV 增量续拉,非整档重载。(`cloud_sync_session.dart`)
- ~~🆕 **`client_out_of_date` 客户端零处理 → 被跳过的更新永久静默丢失**~~ ✅ 已做(校准复核)—— `cloud_sync_session.dart:467` 收到 `code:'client_out_of_date'`(无 ack_id)即 `_resyncFromLag()` 触发 pull/bootstrap 补洞;server 侧 `ws.rs:170` 发 notice。
- ~~🆕 **离线 outbox 按文档滞留:重连后只有当前打开的文档会推送**~~ ✅ 已做(5b7536a)—— 上线/重连(`onServerConnected → _onCloudOnline`)跑 `_sweepPendingOutboxes`:枚举云视图,跳活跃文档,对每个有非空 outbox 的云文档起短命 headless `CloudSyncSession`(persistence=该 doc 的 store)connect→drainOutbox→dispose。桌面 only;串行 + 单例锁 + best-effort + 双检跳活跃 + 空 outbox 不连;blast radius 有界(mis-drain=幂等/超时不损坏)。**注**:纯客户端编排、组合已测原语,无新增自动化测试(端到端需真 WS+多文档集成环境),待实机冒烟。(`main.dart` `_sweepPendingOutboxes`)
- ~~🆕 **协议无版本协商 / 无最低版本闸门**~~ ✅ **已做(2026-07-29)** —— 客户端在 WS URL 上声明 `v=<版本>`(`kSyncProtocolVersion`,现为 1),服务端 `check_protocol` 对照 `MICA_WS_MIN_PROTOCOL` 判定,太老的拒以 `client_too_old`(稳定机器码,客户端据此提示"请更新"而不是"请重新登录")。
  **三个刻意的设计点**:① **默认地板 0,谁都不拒** —— 闸门必须在需要它之前就存在,上线当天就开始拒会打死所有还没更新的桌面安装;要挡的恰恰是那批**不带版本**的老客户端,等到 S4 再加声明就晚了。② **判定排在鉴权之前** —— 否则一个 token 恰好过期的老客户端会先拿到 `unauthorized`、被支去重新登录,登录完重连再被同样拒掉,循环;版本是公开参数,不必等在密钥后面(端到端实测:不带 v + 坏 token 返回 `client_too_old` 而非 `unauthorized`)。③ **加性演进不许 bump** —— 版本号只标记"老客户端活不下去"的破坏性变更,为兼容改动动它会逼出没必要的升级。
  两端各有一条对称断言钉住版本号一致(`the_current_version_is_one` / 「版本号跟服务端对齐」),防它们在两种语言里无声漂移。(`ws.rs` `check_protocol`, `sync_client.dart`)
- ~~🆕 **Web IndexedDB 被驱逐 → 未推送离线编辑静默蒸发**~~ ✅ 已做(60b8b67)—— `WebIdbDocStore.open` 首次打开时 best-effort 调 `navigator.storage.persist()`(js_interop 绑定 StorageManager),请求持久化存储、显著降低驱逐概率;guarded 每会话一次、fire-and-forget、缺 API/拒绝均降级。(`web_idb_doc_store.dart` `_requestPersistentStorage`)
- ~~**M-R 收尾:更细的「离线/重连中」状态提示**~~ ✅ **2026-08-03 核实:这是一条重复条目** ——
  它指向的「云文档离线/未同步状态零指示」早在 2026-07-26 就完成了(三态徽标 + 心跳,
  见本文件「客户端质量与兜底」小节)。一条只写着「见另一条」的待办,在另一条被勾掉时
  **不会跟着变** —— 交叉引用不是链接,是复印件。
- ~~客户端自动重连~~ ✅ 已做(branch `feat/cloud-auto-reconnect`,退避重连,§13.1)。

## 安全

> 上一轮安全 review 的落地清单。自托管一上公网,前几项是硬底线。
> 2026-07-22:refresh/rotation/改密撤销已落地(勾除);新增分享页 XSS、AI 密钥外泄、
> SSRF 等此前漏网的高危项。

- ~~🆕 **公开分享页存储型 XSS → 窃 token → 账号接管**~~ ✅ 已做(200c3b1)—— 分享响应加严格 CSP(`SHARE_CSP`:`default-src 'none'`、无 `script-src` → 内联 script + `on*` 处理器全挡)+ raw HTML 纵深净化(`strip_unsafe_attrs` 剥 `on*`/中和 `javascript:` URI,81ff653)。双层。(`documents.rs:2235/2244`, `markdown/lib.rs:3600`)
- ~~🆕 **分享链接在页面进回收站/「永久删除」后仍对外可读**~~ ✅ 已做(200c3b1)—— `public_share_page` 渲染前 `fetch_document_view`(过 `is_deleted=false`)→ 删/purge 后返回 None → 统一 404。(`documents.rs:2211`)
- ~~🆕 **任意登录用户可改全局 AI 配置 base_url → 服务端密钥外泄 + SSRF**~~ ✅ 已做(200c3b1)—— base_url 钉死服务端配置 / 忽略用户输入。(`ai.rs`)
- ~~🆕 **`files/import-url` 服务端抓取任意 URL —— 盲 SSRF**~~ ✅ 已做(200c3b1)—— 私网/元数据地址黑名单 + 解析后校验(测试 `ssrf_guard_blocks_private_and_metadata_addresses`,files.rs:670)。(`files.rs`)
- ~~**无 refresh / 无撤销的 24h JWT**~~ ✅ refresh + rotation + reuse-detection + `revoke_family`/`revoke_user_sessions` 已落地;access JWT TTL 默认 **24h→1h**(`config.rs`,4a3042a),把「本该失效的 token 仍可用」窗口从 24h 压到 1h(客户端透明续期,无感)。更强的即时吊销(per-user token-version 表)仍可选,但收益已大幅下降。
- ~~**改密不失效旧令牌**~~ ✅ `change_password` 已 `revoke_user_sessions`(`auth.rs:246`);唯一残留是被盗 access JWT 在剩余 TTL 内仍活(同上,靠缩 TTL/token-version 收口)。
- ~~**登录/注册/refresh 无限流**~~ ✅ per-IP 令牌桶 + 全局 Argon2 并发门(`rate_limit.rs`);反代后取真实 IP 走「XFF 从右跳私网」对双跳(Traefik+nginx)/单机都对,自研无依赖。refresh 也纳入 per-IP 限流(但不占 Argon2 门——它不 hash,占了会饿死登录)。**WS 建连有意不限**:已 token 鉴权、低威胁,共享桶会误伤「同时开多文档」——按「不要过度设计」先不做并记因(CLAUDE.md 协作约定)。
- ~~**鉴权逐 handler 手写、非中间件**~~ ✅ 已做(校准复核)—— `auth.rs:618` `scope_guard` 是 router-wide **默认拒绝**中间件 + `is_public` 白名单;新路由默认已鉴权。
- ~~**CORS 全放行**~~ ✅ prod 默认拒跨源(`cors_layer`,4a3042a),`CORS_ALLOWED_ORIGINS` 放行指定 origin,dev 仍 permissive;顺带修了「prod 一直以 Development 运行」(compose 缺 `APP_ENV`,727ebab)——否则收紧在 prod 不生效。
- ~~🟠 **compose 的注册默认已改对,但要下一版才到生产**~~ ✅ **0.13.8 上线**(2026-08-02)——
  同一版还带上:`JWT_SECRET` 生产环境拒绝占位/过短的值(模板不再发一个能用的 `change-me`)、
  备份编排从 shell 收进 `mica-cli backup`(部分成功不再静默 —— 缺一条腿默认让整次备份失败)、
  对象字节 rclone 直传 OSS、`pg_dump` 去 gzip 让 rustic 能去重、搜索能找到文件夹并回传
  `parent_view_id`、页面菜单的「复制页面内容」与「复制路径」、CI 新增 container job。
  **已做**:节点 `.env` 补了 `RUSTFS_S3_*` / `OSS_BLOB_BUCKET`;上线后首跑三条腿全部 `ran`
  (database / content / objects),对象字节第一次有了异地副本。**0.13.7 作废**——它的
  `images (cli)` job 挂在我写的 rclone 阶段上(见 `deploy/Dockerfile.cli` 注释),那一版
  只出了 api/web 镜像,修好后重发 0.13.8。下面留档原文 ——

- ~~**长连 WS 超 token TTL 不再认证**~~ ✅ **已做(2026-08-03,`fd509f2`)** —— 过期前建的 socket
  曾可授权数小时:每个 HTTP 请求都重读一次 token 所以每次都校验 `exp`,而 WebSocket 在 upgrade
  时认证一次就再也不看。现在 `session_from_token` 一次解出 `(user_id, exp)`,socket 自带一个
  到期即关的定时器,关闭码 **4401**。三个刻意的点:①**定时器只装一次、不被流量重置** ——
  忙碌的 socket 不能靠「一直在说话」把过期会话续命;②`time_left` 已过期返回 `None` 而非巨大
  duration —— `u64` 反向相减会环绕,环绕出的 deadline 就是「永不关闭」,恰好把这个 bug 原样
  复活,测试专钉它;③**4401 而非 1008** —— 4000–4999 是应用自己的区间,客户端必须能把「换新
  token」和「服务器连不上」分开,后者该退避,前者等再久也不会自己回来。**客户端无需新分支**:
  `_reconnectAttempts` 收到有效帧即归零、重连时 URI 供应器先 `await _ensureFreshSession()`,
  这条路本就通。活栈实测(`tool/ws_expiry_smoke.dart`,TTL=20s):第 19 秒收到
  `4401 "token expired"` —— 单测证明不了定时器有没有装在活路径上,`run_connection` 只有经过
  真 upgrade 才到得了。

- ~~**桌面 token 明文存 prefs(无 DPAPI)**~~ ✅ **已做(2026-08-03,`13d7dfa`)** —— 会话 token 与
  refresh token 一直明文躺在 `{APPDATA}/mica/prefs.json`(实测改动前 access 165 / refresh 72
  字符,直接可用)。现在 Windows 上过 DPAPI `CryptProtectData` 落盘,密钥绑 Windows 用户账户:
  拷到别的机器、或以别的用户读,都解不开。四个决定:①**`dart:ffi` 直绑 crypt32.dll,零新依赖**
  (两个调用+一个 struct,紧挨着已有的 `window_snapped_win.dart`;要替代它的 secure-storage 包
  会在每个平台拉 plugin 去解决一个只存在于一个平台的问题);②**按 key 名判定机密**,不让调用方
  显式选 —— 显式 `saveSecret` 只保护「有人记得调它」的键,下一个 token 键会一直明文躺着,
  按名字判定是默认保护、忘了也朝安全方向失败;③**`protect` 幂等**,这是写迁移时抓出的真坑 ——
  迁移每次冷启动都跑,非幂等则第二次启动二次加密,`unprotect` 只剥一层返回 `dpapi1:` 字符串
  并作为 `Authorization` 头发出去,失败现场跟成因完全不像;④**解不开返回 `null` 而非密文** ——
  那是 profile 被拷走的情形,正确答案是登录页。**迁移**在首次读取时一次性做(不等下次写入:
  做那次写入的是登录或刷新,一个一直登录着的用户会把明文留到会话结束,而那正是要关的窗口);
  旧明文仍可读,**升级不以重新登录为代价**。**边界**:DPAPI 挡不住以该用户身份运行的代码
  (它同样能调 `CryptUnprotectData`)—— 桌面上没有方案挡得住,系统钥匙串也一样;消除的是
  **离线副本**。非 Windows 桌面老实降级明文,`secretsAreEncrypted` 把这件事说出来。
  **单向性(2026-08-03 实测补记)**:0.13.10 加密过 token 之后,**回滚到旧版会被登出** ——
  老版本不认 `dpapi1:` 前缀,把密文当 token 用,认证失败退回登录页。**不是数据丢失**
  (笔记都在,重登即可),但它是这条改动的真实后果,而当时只想到了「向前兼容」那一半。
  撞见方式很偶然:想用安装版(0.13.9)驱动另一个功能的验收,发现它退到了登录页。
  单测 7 条在真 DPAPI 上跑过;端到端实测:迁移后为密文(527/399)、键数不变、**登录态完好**
  (工作区 733 页加载)—— 只加密而读不回来等于把用户踢下线,两半都要成立。

- ~~**web 凭据:localStorage 明文 + WS token 出 URL**~~ ✅ **两处一起关掉(2026-08-03,`264d018`/`537422d`)**
  —— 它们是同一个问题的两面,一个机制解决:**HttpOnly cookie**。**推翻的前提**:此前的条目和
  代码注释都写着「web 只能把 token 放 URL,因为浏览器 WebSocket API 设不了 header」——
  前半句对,结论错。**WS 握手就是一个普通 HTTP 请求**,同源 cookie 自动跟着走;「设不了自定义
  header」被当成了「没法鉴权」。扒 AFFiNE 源码(`affine_session` cookie,其 WS gateway 测试
  就是从 cookie 取 session)才撞破 —— 正是原则 #6 说的那类假前提。**落地**:服务端 cookie 成为
  token 第三来源(header > cookie > `?token=` 兼容尾巴;头赢 cookie 是有意的 —— 显式的头是
  一次决定,cookie 是环境自带的),login/refresh 下发 `mica_session`+`mica_refresh`,logout 清除
  (**即使 token 已不认识也要清**,否则「退出登录」在最要紧那种情况下是句假话);web 侧机密键
  只进内存并**主动删掉旧版留在 localStorage 的副本**(否则升级只是装样子),WS URL 里不再有任何
  凭据,刷新页面用 refresh cookie 换回内存 token。**CSRF 是本方案引入的新风险**,按最小充分解
  做单层 `SameSite=Strict`(token 只从 cookie 读取、不与环境状态组合);**边界写进注释**:哪天
  API 与 app 不同源,这套推理要重做而不是往上加。**活栈端到端抓到一个单测抓不到的缺口**:
  cookie 只加进了 `scope_guard`,而 `user_id_from_headers` / `ai_ws::token_from` 自己取 token、
  不走中间件 —— **一半端点认、一半不认比全都不认更糟**,故障是逐端点的,读起来像服务器时好时坏
  (`537422d` 修)。冒烟 `tool/web_cookie_smoke.dart` 14 条断言全过(含「WS 只靠 cookie 建连」)。
  **刻意不测** Chrome 会不会带 cookie:那是同源规范行为,测它要驱动登录表单即自动输密码。

- ~~**弱口令:唯一校验是 `len() >= 8`**~~ ✅ **已做(2026-08-03,`d82f96c`)** —— 原策略放行
  `password`、`12345678` 和用户自己的邮箱前缀。**不是在防暴力破解**(在线猜测早被 per-IP
  令牌桶 + Argon2 并发门挡着),挡的是**不需要猜很多次的那两种**:所有人都在用的那个,
  和用账号自己名字拼出来的那个。形状照 **NIST SP 800-63B**,它对「不要做什么」说得异常
  具体:**长度地板仍是 8、没有提高** —— 提高它是组合规则换马甲,挡不住 `Password123` 却
  会催生套路;不加组合规则、不强制轮换。加的是:常用口令表(内嵌、短、手挑)、**无信息量
  结构**(单字符重复/整串顺逆序/键盘行,抓固定表抓不到的长尾)、**不能用账号自身标识拼**
  (邮箱前缀 + 显示名,双向匹配;**4 字符以下片段忽略** —— 否则姓 `wu` 会把 `powerful`
  一并禁掉,而拒绝好口令的规则只会教人绕过;**邮箱域名不算身份** —— 自托管实例上所有人
  共用一个域)。顺带收掉一处**双表示**:注册与改密各写了一份 `len() < 8`,现在同过一个
  `password_strength::check`;改密的校验放在**验过当前口令之后**,否则未鉴权调用方能探测
  策略。**测试抓到 12 条死数据**:`qwerty`/`letmein`/`monkey` 这些短于 8 的表项永远读不到
  (长度检查先拦),已删并加不变量测试钉住 —— **一个看起来覆盖了它其实覆盖不到的东西的
  清单,比一个诚实的短清单更糟**。**刻意不做**:不查泄露口令库(HIBP k-anonymity 要在注册
  路径上加一次出站 HTTPS,失败即拒则谁都注册不了、失败即放行则检查静默停止生效)。

- ~~**yrs base 无 squash/GC,无界增长**~~ ❌ **2026-08-03 实测为假,整条撤销**(不是做完了,是它
  从来不成立)—— 原文说「只裁 stream 不压 base,长寿文档 base 越滚越大」。**实测**:插入
  40 000 字符后 base 为 40 137 字节,**删除后回到 150 字节**(空文档 123),经 `from_update`
  → `encode_state` 往返仍是 150。yrs 的 doc 用 `Options::default()` 建(`skip_gc = false`),
  加载即回收已删内容,而 `push_update` **每次写都做一趟全档 decode→encode**,那趟 round trip
  本身就是一次 squash。base 里持续增长的只有墓碑 ID 区间与 per-client 状态项 —— 前者按删除
  次数、后者按设备数,都不是「按编辑量无界」。
  **这条的价值不在于删掉它,而在于它与隔壁那条的关系**:「每次 push 重写整档」是真的,
  而修它最显然的办法(改成追加 + 定期 squash)**会把这条不存在的无界增长真造出来**。
  两条曾被(包括我自己在 2026-08-03 的排序里)判为「同根、要一起设计」—— 方向对,内容错:
  **一个是另一个不发生的原因**。已加回归测试
  `base_compaction::deleted_content_does_not_survive_in_the_base` 钉住,并把这段推理写在
  测试文档注释里,改写路径的人会先看到它。

- ~~🆕 **本地工作区没有工作区级搜索**~~ ✅ **已做(2026-08-03,`63617b3`/`283aa16`/`d40986e`)**
  —— 云端搜索靠 `document_yrs_base.content_text` 派生列,本地库没有对应投影,`onSearch` 接的是
  null,搜索框只能诚实地说「本地模式不支持」。**先把风险拆掉再动手**:原条目要碰
  `SCHEMA_VERSION`,而动它就触发 release.md 步骤 4(桌面自动更新后首启就地迁移本地库,写坏 =
  用户笔记打不开)。**加一个可空列是纯增量的,不用 bump**(`doc_version` 先例):老版本打开新库
  只是不 SELECT 这列,新版本把老版本留下的 NULL 当「还没索引」。**没有强制迁移,就没有那条路**,
  也不需要拿真 store.db 冒险。**四条写路径收成一个入口**:`doc_snapshot` 原有四处一模一样的
  UPSERT,云端守「派生列跟着 state 走」靠四处各记一遍同语句 co-write;这里改成只有一个
  `upsert_snapshot` 能写这张表 —— 两列**不可能**漂移,第五条写路径得刻意绕开。比云端更强也更省。
  **派生失败绝不让保存失败**(存 NULL,只是搜不到 —— 笔记是用户的,索引是我们的)。
  **web 保持 `onSearch: null` 且这是对的**:web 没有 on-device store,「这个世界搜不了」和
  「什么都没搜到」是两句不同的话。**过程中抓到两个自己的半成品**:① 插入位置吃掉了
  `list_views` 的 `#[frb(sync)]`,生成出的 `searchLocal` 是同步的 —— 而注释正写着「绝不能同步」,
  看生成产物才发现(注释与代码相反是最难查的一类);② `onOpenSearchResult` 在本地模式是空实现,
  **搜得到点不开**;③ 回填函数写了**没人调用**,真库 indexed=0 —— 每一层单独都对,单测抓不到。
  **实测(真实库 588 快照)**:列已加、版本仍是 6、重启后 588/588 已索引、536 篇有正文、
  日志 0 异常 —— FFI 通道由此实证。Rust 侧 7 条行为测试(通配符字面化、回收站不回来、老库回填、
  snippet 按字符切…),flutter 1070 passed。

## 生产运维与备份 🆕

> 2026-07-22 新增小节。节点是单机 docker(阿里云),生产当前处于「盲飞 + 静默失败」态。

- ~~🆕 **备份 sidecar 静默失败无任何告警**~~ ✅ 已做(校准复核)—— `mica-backup-loop.sh:16` 死人开关:成功/失败分别 ping `${HEALTHCHECK_URL}`(healthchecks.io 式),compose 已布线。
- ~~🆕 **Postgres 全库无自动异地备份**~~ ✅ 已做(校准复核)—— `mica-backup.sh:70` `pg_dump|gzip` 进 PGDUMP_DIR、rustic 顺带异地;`Dockerfile.cli` 装 postgresql-client-16。
- ~~🆕 **生产无任何外部探活**~~ ✅ 已做(校准复核)—— `.github/workflows/uptime.yml` cron `*/15` 打 `/api/ready`。
- ~~🆕 **容器 HEALTHCHECK 用不摸库的静态 `/api/health`**~~ ✅ 已做(校准复核)—— `Dockerfile.api:22` HEALTHCHECK + 部署验证均改打摸库的 `/api/ready`。
- ~~**坏迁移的「恢复」流程无文档**~~ ✅ backup.md 加「从 pg_dump 恢复/回滚坏迁移」runbook(停 api→drop/create→zcat|psql→钉旧 tag→health/ready 验证,0d9c404)。
- ~~🆕 **单机兜底部署脚本 `deploy/deploy-from-source.sh` 已漂移**~~ ✅ 已做(2026-07-23)—— 对齐 justfile 权威版:`flutter build web` 补 `--no-web-resources-cdn`(修 CN 运行时拉 gstatic CanvasKit 不可用)、删 stale `--no-tree-shake-icons`、rsync→`rm -rf + cp -r`(Windows 无 rsync);`bash -n` 过。
- ~~🆕 **Postgres 大版本升级路径无文档**~~ ✅ 已做(deploy.md 早有升级 section,0d9c404;2026-07-23 补「PG16 上游支持到 ~2028、这是主动维护任务非顺手改 tag」)。

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
- ~~🔴 **未闭环:配额在生产还不可调**~~ ✅ **已闭环(0.13.6,2026-07-30 上线并在节点核实)** ——
  `deploy-prod` 从 tag 取 compose + sha256 指纹校验,所以 `81276a2` 那行必须发一版才生效;0.13.6 就是那一版。
  **生产实测**:容器里 `MICA_WORKSPACE_QUOTA_BYTES=` 已存在(空 = 走代码默认 1 GiB),
  即「设了到不了进程」这个根因消失。同批把允许清单缺的另外 8 个变量也补齐了(见「开发者体验」小节
  catch-up 常量那条),**`MICA_WS_MIN_PROTOCOL` 从此才真的可设** —— S4 那句「抬到 1 即生效」在这之前是空话。
  **仍未在生产实测的一半**:极小配额→拒绝→带 `workspace_quota_exceeded` 的三态,需要一个已登录账号发真上传,
  agent 手上没有凭据;本地三态实测过(3000 放行 / 6000 拒 / 边界 4500 放行)。要在生产补这一刀,就是
  `.env` 临时设小值 → `docker compose up -d --no-deps api` → 用自己的账号传个超限文件 → 复原,窗口内会误伤真实上传。
  原文如下 —— `MICA_WORKSPACE_QUOTA_BYTES` **没有**进
  `deploy/docker-compose.yml` 的 api `environment:` 块,而那是一份**显式允许清单**:没列进去的
  变量到不了进程。0.13.5 冒烟时抓到 —— 节点 `.env` 把配额设成 5000,一个 6000 字节的上传仍被
  接受。**当前生产状态**:配额在生效,但锁在代码默认的 1 GiB、不可调(不算失去保护,只是没有
  可调性)。修复已在 main(`81276a2`),但**重启不生效** —— `deploy-prod` 从 tag 取 compose,
  且 deploy 有 sha256 指纹校验(一个版本不能配着不属于它的 compose 部署),所以必须发 0.13.6。
  另外**生产侧只验证了「1 GiB 默认在生效」这一半**,极小配额那次因这个 bug 没跑成;本地是三态
  实测过的(3000 放行 / 6000 拒并带 `workspace_quota_exceeded` / 边界 4500 放行)。
  一般化教训:**加配置项要同时接三处** —— `config.rs` 读、compose 透传、文档写。
- ~~**`document_yrs_versions` 过期清理只挂在「该文档自己 push 撞 cadence」**~~ ✅ blob_gc 6h 循环加全局 `DELETE ... expires_at IS NOT NULL AND < now()`(只命中 auto、不碰命名检查点,6612330)。**残留**:`list_yrs_versions` 仍不过滤 expires_at(6h 扫前的过期行可能短暂现于面板,极小)。
- ~~🆕 `blob_gc.rs` 注释预设了一个不存在的「回收站保留期」~~ ✅ 已修(2026-07-29)—— 原文写 effective margin 是 "recycle-bin retention + this",而回收站根本没有 retention 这个量;改成如实说明它是 [0, 用户手动清空) 的不定时长。
- ~~**`refresh_tokens` 只增不删**~~ ✅ blob_gc 6h 循环加 `DELETE ... expires_at < now()-7d`(6612330)。
- ~~🟡 **账号删除功能不存在**~~ ✅ 已做(18300d1,2026-07-23)—— `delete_account` 事务级联(密码门控 + 跨他人 workspace RESTRICT 阻塞回滚 409),详见「产品与公开发布合规」小节;级联顺序备忘 deploy.md 早有(0d9c404)。
- ~~🔴 **异地备份里根本没有数据库**~~ ✅ **当天补上(2026-07-30)** —— 节点 `.env` 接上
  `MICA_BACKUP_PGURL`(不需发版)、重建 backup 容器、手动跑一次完整备份;
  `rustic snapshots --filter-label _pgdump` 从 **`total: 0`** 变成 **1 个快照(23.7 MiB)**。
  数据库第一次有了节点之外的副本,DB 的 RPO 从 **∞ → ≤24h**。
  **残留**(均已在 `docs/dr-plan.md` 立项):① 对象字节仍只靠内容导出间接覆盖,定了走 rclone
  直传 S3(要发版);② `pg_dump | gzip` 让 rustic 的 dedup 几乎失效,该改成不压缩交给 rustic 压;
  ③ **端到端恢复演练没人走过,RTO 至今无答案**;④ 跨账号第二份**已拍板不做**(用户判断概率够低)。
  下面留档原始发现,因为它是「机制写好了但没人接线 + 监控对此是绿的」这类失败的标本 ——
- ~~🔴 原始发现:**每日备份是「只有内容」的**~~(2026-07-30 生产实测,**推翻了本条更早的表述**)——
  原文说「2026-07-22 起每日备份走全库 pg_dump(label=_pgdump),回收站页面的文本+CRDT 历史在备份里」,
  **是假的**:机制在 `mica-backup.sh` 里写好了,但节点 `.env` **从来没设过 `MICA_BACKUP_PGURL`**,
  脚本因此按设计降级、打一条 WARN、只备内容。取证:今天备份日志原文
  `WARN: MICA_BACKUP_PGURL unset — skipping pg_dump (content-only backup, no DB disaster recovery)`,
  且 `rustic snapshots --filter-label _pgdump` = **`total: 0 snapshot(s)`**(全仓 170 个快照)——
  不是最近坏的,是一次都没跑过。**后果**:异地只有 22 个工作区的 Markdown+图片;账号、成员关系、
  CRDT 编辑历史、版本检查点、**评论**(独立表,正文里一个字都没有)、分享 token 全都只存在于
  节点本地盘,而 DB 的唯一副本是**发版前手工落的** `/data/mica/pre-*.sql.gz`(与 DB 同盘)。
  节点盘一坏 = 这些永久消失。**为什么活这么久**:死人开关对它是**绿的** —— 跳过 pg_dump 是一次
  *成功*的运行,监控抓得住「备份没跑」,抓不住「备份少备了一半」。**修法一行**(节点 `.env` 加
  `MICA_BACKUP_PGURL`,不需发版)+ 配套让降级不再算成功。完整方案、威胁模型、RPO/RTO 见
  **`docs/dr-plan.md`**。

## 编辑器与功能广度

- ~~无暗色模式~~ ✅ **已做**(v0.13.2,2026-07-29):语义色 token 层(`ui/theme_tokens.dart`,AppFlowy 式角色分组)贯穿外壳 / 自绘画布(挂 `EditorAppearance.tokens`)/ 语法高亮(`_Rule` 存角色,emit 时解析)/ mermaid(merman host theme + mermaid.js themeVariables);跟随系统或设置手切,冷启动在 runApp 前就位。**位图与记忆化缓存的 key 都带调色板**(公式/mermaid 栅格、代码 span memo),否则浅色下烤出的产物会画到深色页上。
- ~~**文档内查找/替换缺失**~~ ✅ Ctrl+F 查找栏(导航/计数/当前匹配高亮)原已具备;2026-07-22 补齐**替换**(`replaceRange`/`replaceAll` 走既有 op 路径,9fe9ae8)+ F3/Shift+F3。**全部匹配高亮**有意不做(要动 render.dart 加第二遍选区叠绘,超 MVP)。
- ~~**行内数学未排版**~~ ✅ 2026-07-16:`$…$` 真排进行里(基线对齐、随字号缩放),公式为不可进入的原子(`inline_atoms.dart`,render-architecture.md Decision 4)。

## 客户端质量与兜底 🆕

> 2026-07-22 新增小节。离线功能面做得全,但崩溃/损坏/双开几处兜底缺失会真丢数据。

- ~~🆕 **客户端零崩溃/错误上报**~~ ✅ 已做(校准复核)—— `main.dart:149` `runZonedGuarded` + `FlutterError.onError`,未捕获异常落盘 diagnosticsDir。
- ~~🆕 **本地世界文档损坏 → 静默变空白且自毁恢复检查点**~~ ✅ 已做(校准复核)—— FFI `store.rs` `load_doc` 区分 None/corrupt(throw);`local_doc.dart:56` 捕获即 rethrow `LocalDocCorruptException`,不再 seed+saveDoc+checkpoint(§10 回滚网不再被冲)。
- ~~🆕 **桌面无单实例守卫,双开丢本地文档**~~ ✅ 已做(校准复核)—— `windows/runner/main.cpp:24` `CreateMutexW("Local\\MicaSingleInstance")` + `ERROR_ALREADY_EXISTS` 守卫(fail-open)。
- ~~🆕 **退出路径漏掉编辑器 400ms 防抖文本**~~ ✅ 已做(校准复核)—— `main.dart:1016` `_flushForExit` 先 `await _activeEditorFlush()` 再冲会话/后端。
- ~~🆕 **`prefs.json` 非原子写 + 损坏静默清空**~~ ✅ 已做(校准复核)—— `prefs_stub.dart:64` 写 `.tmp` 后 `renameSync`(同卷原子,含 Windows 覆盖处理)。
- ~~🆕 **编辑器 op 管道 `catchError((_){})` 吞掉本应浮出的 outbox 写失败**~~ ✅ 已做(校准复核)—— `controller.dart` 现 `opFaultCount++` + `onOpFault?.call` 上浮,不再吞(红线 #1)。
- ~~🆕 **云文档离线/未同步状态零指示**~~ ✅ 2026-07-26 完成(69ff98f 地基+信号 / 8e1318d 徽标 / 6832dea 心跳)—— 扒了 8 家(AFFiNE/SiYuan/Logseq/Anytype/Google Docs/Notion/Obsidian/AppFlowy)后定**最小形态**:三态克制徽标(已同步→**什么都不画**、同步中→faint 慢转圈、离线→cloud-off + tooltip),摆文档面包屑右上、**仅云工作区**显示。**不做数字计数**(同类无一家做)、**不可点击/不做手动同步**(AFFiNE/AppFlowy/Anytype 同样没有;mica 本就自动重连 + 自动 flush)。信号从 `CloudSyncSession` 四个真实转移点 emit,推导是纯函数 `deriveSyncPhase`(`sync_status.dart`,4 单测)。**关键补丁**:加了**心跳**(8s ping + 20s 帧静默看门狗)——否则拔网线是 TCP 半开、不发 WS close 帧,`_onDone` 永不触发 → 一直误判在线(用户实测拔线发现徽标不动才暴露);服务端 `ws.rs:267` 本就 `ping→pong`,零改动。〔"别人都没做"的印象来自 AppFlowy:它的 `sync_indicator.dart` 当前是**死代码**(重构后未挂载),且有未关闭的需求 #5729 求做回。〕

## 性能

- ~~**长文档性能**~~ ✅ **性能线已闭环**(2026-07-23)—— 设计 `docs/editor-virtualization-plan.md`。三刀叠加后每击键 = O(改动块)真推导 + O(N) 平凡重定位:**Phase 1**(da25075)painter 缓存复用,干掉逐帧 dispose+重建全部 TextPainter;**代码高亮记忆化**(7fe1997)未变代码块不再重新分词;**Phase 2**(b750d88)整块 layout 缓存 `_layoutCache`,未变块跳过 marks/span/高亮/rect 全部推导只 `shiftBy` 重定位,dirty 判定用 identity(实证 controller 只重赋值不原地改 text/data)。回归 `test/{painter_cache,code_span_memo,layout_reuse}_test.dart`,全量 728 通过。**残留(L,有意不做)**:真·视口虚拟化(屏外跳过排版)——两条架构约束(performLayout 无滚动偏移、编辑器不自管视口)使其为独立架构项,ROI 仅万级块/超长档,剧本留档待需。
- ~~**图片纹理缓存无逐出策略**~~ ✅ `_imageCache` 改 LRU(64 上限,每帧 touch 可见图、逐出屏外静态图并 dispose,守 lessons.md §5 dispose 时序,253c53f)。

## 开发者体验 / CI / Markdown

- ~~🆕 **api-server 全部测试不进 CI;DB 测试本地也静默跳过**~~ ✅ 已做(校准复核)—— `ci.yml` 已有 postgres service + `-p mica-api-server`(测在 CI 实跑);`auth.rs:895` `pool()` **已带 sync_pg.rs 同款 CI-assert**(`assert!(env CI is_err, "DATABASE_URL unset in CI...")`)→ 缺库在 CI 里 panic、本地才 return None 跳过,那些 `else{return}` 在 CI 不会假绿。(校准注:审计曾误判为残留,因只看了 `else{return}` 调用点、未读 `pool()` 定义——同其 #1 的错。)
- ~~🆕 **页树不变量守卫 `ensure_parent_accepts_children` 零自动化测试**~~ ✅ 已做(校准复核)—— `documents.rs` `parent_guard_pg` 测 folder 接受/page 拒绝/缺失父 + 触发器 backstop(真 PG 门控)。
- ~~**Release 出的 Windows 安装包从未被自动安装-启动验证**~~ ✅ release.yml 加「安装-启动冒烟」(/VERYSILENT 装 + 启动 + 存活 10s + finally 清理,发布前拦,0d9c404)。**2026-07-23 根治 flaky**:冒烟测撞单实例 mutex 竞态偶发假失败(安装器 `[Run]` 自启一个 + 测试又自启一个,谁后抢到 `Local\MicaSingleInstance` 谁 `exit 0`;`[Run]` 触发时机随机)。结构性解法(ShareX 同款):`mica.iss` 的 `[Run]` 加 `Check: not CmdLineParamExists('/SKIPRUN')`,CI 安装传 `/SKIPRUN` → 安装器不自启 → 测试是唯一启动方 → mutex 永不争用,竞态从结构上消失(9c006e6),0.12.16 真 CI 跑绿实证。〔sccache 曾加在 windows job(b5e7f04)后于 v0.12.18 撤除:tag 触发的 job 命中率恒 0%(GHA cache 按 ref 隔离、release 只在 tag 上跑),且长杆是 Flutter 构建非 Rust——详见 `docs/lessons.md`。〕
- ~~**e2e:桌面已进 CI,web 仍为零**~~ ✅ **整条关闭(2026-08-03 核实)** —— 「web 仍为零」已假:
  `ci.yml` 有 `web-e2e` job、`justfile` 有 `just web-e2e`(浏览器里的 yjs 与服务端 yrs 经真 WS
  收敛、服务端渲染路由压过 SPA 兜底、入口文件缓存头、POSIX locale 下仍能启动)。**这条是
  「否定式条目静默变假」的标准样本**:web e2e 落地时没人回来改它,而文档说「没有」而实际
  有了,不会有任何东西报错。原文留档如下 ——
- 🟡 原文:**e2e:桌面已进 CI,web 仍为零**(2026-07-29 更正:「全项目零 e2e」失实)—— 桌面 14 个离线集成测试已在 windows-latest **真 app 实跑**(flutter-integration.yml,2026-07-23 起,那就是 e2e);web 端仍零(无 spec,CI 只验能编译;playwright 截图是人工手段)。(web 侧 L)
- ~~🆕 **本地 SQLite 真库升级冒烟不在发版清单**~~ ✅ 已做(核实于 2026-07-30)—— `release.md` 步骤 4 已写进去,连命令带理由:「它默认 `#[ignore]`、要手动设环境变量,所以不写进这里就等于不存在」,并钉住触发条件(改过 `crates/mica-core` 的 store schema / `SCHEMA_VERSION` 时必跑)。
- ~~🆕 **`just test` 漏 `--features store`**~~ ✅ 已做(校准复核)—— `justfile:133` 已有 `cargo test -p mica-core --features store`。
- ~~**catch-up limit / stream 常量硬编码**~~ ✅ 已做(2026-07-30)—— `SyncTuning{catch_up_limit, keep_margin, prune_every}` 进 `AppConfig`
  (`MICA_CATCH_UP_LIMIT` / `MICA_STREAM_KEEP_MARGIN` / `MICA_STREAM_PRUNE_EVERY`,默认 1000/64/32 = 原常量值),
  `push_update` / `restore_yrs_version` 收 `&SyncTuning`,`app-core` 的两个 `pub const` 删除。**解析上有一处不给面子的地方**:
  `0`、负数、乱码一律回落默认,**没有任何拼法能把它们设成 0** —— `prune_every=0` 会让 `rid % 0` 在 push 热路径上 panic,
  `keep_margin=0` 会把刚插进去的那条更新删掉;两者都不是运维会想要的状态,于是不给入口(同 `workspace_quota` 那条原则)。
  单测 `sync_tuning_parse` 三条钉住:默认值 == 老常量、正数原样、0/负/乱码回落。
  **顺带修掉一整类同款陷阱**:`deploy/docker-compose.yml` 的 api `environment:` 是显式允许清单,
  比对 `config.rs` 读的 16 个变量后发现**有 9 个没列进去、在生产等于不存在** —— 其中
  **`MICA_WS_MIN_PROTOCOL` 最要命**:本文件 S3 条目写着「S4 时把它抬到 1 即生效」,而它**根本没法在生产设置**。
  已补齐 8 个(`MICA_WS_MIN_PROTOCOL` / 三个 sync 旋钮 / `CORS_ALLOWED_ORIGINS` / 两个 token TTL /
  `DATABASE_MAX_CONNECTIONS`),全以 `${VAR:-}` 透传(空 = 走代码默认);**刻意不加 `MICA_SEED_TEST_USER`** ——
  生产必须永不可达。YAML 用拒绝重复键的 loader 验过。
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
- ~~🆕 **打包 Noto Sans SC 走 OFL 1.1 但没随附 OFL.txt**~~ ✅ 已做(核实于 2026-07-30)—— `clients/mica_flutter/fonts/OFL.txt` 在库里,`NOTICE.md:11-12` 也从「应当随附」改成了如实陈述「the full licence text ships beside it as `OFL.txt`」。

## 上一批「最该做」(2026-07-22 排,当批已全部完成)

> 数据安全里程碑已收口后,重心转向「公网自托管的硬底线」——发出去前一次事故就不可挽回的类型。

1. ~~**分享页安全三件套**~~ ✅ 完成(200c3b1/81ff653)—— export_html 白名单净化(strip_unsafe_attrs)+ 分享响应 CSP + 渲染前校验 view 存活。存储型 XSS→token 接管 与「删了还在公网」两个高危都堵上。
2. 🟡 **备份可信化 —— 原标「✅ 完成」不成立**(2026-07-30 更正)。死人开关 ✅、`/api/ready` 探活 ✅、
   `rustic check` + 恢复演练 ✅(已实跑),但当时那句「pg_dump 异地 DR」**是假的**:节点从未设
   `MICA_BACKUP_PGURL`,异地一份 DB 快照都没有。见「数据生命周期」小节的 🔴 与 `docs/dr-plan.md`。
3. ~~**AI 配置授权 + 收口 base_url**~~ ✅ 完成(200c3b1)—— base_url 钉死服务端配置,密钥外泄 + SSRF 堵上。
4. ~~**CI 锁住数据面回归**~~ ✅ 完成 —— api-server 测进 CI(postgres service)+ auth.rs `pool()` CI-assert(缺库即 fail)+ 页树守卫补测 + real_store_smoke。
5. ~~**客户端兜底三件**~~ ✅ 完成 —— 崩溃上报(runZonedGuarded)+ 单实例守卫(CreateMutexW)+ 本地损坏兜底(LocalDocCorruptException,不再自毁恢复点)。
6. ~~**限流 + 收紧 CORS + Token 撤销收口**~~ ✅ 2026-07-22 完成:认证端点(含 refresh)per-IP 令牌桶 + Argon2 并发门、CORS prod 拒跨源、access JWT 24h→1h、修 prod 误认作 dev。WS 建连限流有意不做(已鉴权低威胁,见 CLAUDE.md「不要过度设计」);per-user token-version 即时吊销可选。
7. ~~**文档内查找/替换**~~ ✅ 2026-07-22 完成(9fe9ae8):查找侧原已具备,补齐替换 + F3。至此本「最该做」清单全部清空——下一批优先级见下方各小节(反链、表格、虚拟化等)。

---

**整体判断**(2026-07-22 校准后):上面「发出去前必须补的底线」——安全(分享页/AI 密钥/限流)、备份可信化、CI 回归网、客户端桌面丢数据三路径——**均已落地**(本轮对着代码逐条核实,勾除了 18+ 项 roadmap 陈旧误报的已做项)。剩的高价值真·未做偏「基建/成熟度」:更新器 Authenticode 签名、协议版本协商、恢复演练排期、op 模型表 GC、长文档虚拟化;产品广度上**虚拟化 + 表格 + 反链**决定它像不像一个成熟笔记。

