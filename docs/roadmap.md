# Mica 路线图 — 剩余功能与优化点

> **这份文件只放没做完的事。** 做完的整条搬去 [`roadmap-done.md`](roadmap-done.md)
> —— 不是删,那些条目里写着「当初为什么必须这么排」(op 模型退役那六步是最好的例子),
> 那是判断依据,不是流水账。但把它们留在待办清单里,每次盘点都要重付一遍阅读成本:
> 2026-08-03 那轮核实,29 条未做项里有 3 条是幻影、1 条工作量标反 —— 而那还是在
> **只扫未做项**的前提下。
>
> 影响力从高到低;`(S/M/L)` = 工作量;`[需后端]` = 要动 Rust;
> **`[等用户]` = 卡在只有用户能做的一步**(生产凭据/策略变更、要在真机上先走一遍),
> 不是没排上 —— 分开标是因为这两者留在清单上的样子一模一样,但前者催我没用。
>
> **维护规矩(否则它会再烂一遍)**:
> 1. **一条做完了就当场整条搬走**,别只加 ✅ —— 发版流程里有这一步(`release.md` 步骤 5)。
> 2. **否定式条目(「无 X」「不校验 Y」)只会静默变假**:代码删个函数会编译报错,
>    文档说「没有」而实际有了,什么都不会响。所以它们要定期对着代码核,而不是靠记性。
> 3. **核实结论写进条目本身**(日期 + 依据文件),下一个人才知道这条是新鲜的还是祖传的。

## 可靠性与同步

> **这一节空了(2026-08-06)。** 不是漏删:同块字符级协同(文本 + marks)、长离线重连
> (背压 + 合并)两条都已关闭并搬进 [`roadmap-done.md`](roadmap-done.md) —— 后者 08-05
> 撤回过一次,08-06 连同它撤回的原因一起做完了。留着标题是因为这是最该有人盯的一档 ——
> 空着本身是个状态,而不是这一档不存在。

## 安全

> **这一节空了(2026-08-05)。** 拍板不做的条目已整条删除,理由留在那次提交的信息里。
> 留着标题是因为这是一个真实存在的分类 —— 空是个**状态**,不是这一档不存在。



## 生产运维与备份 🆕

> 2026-07-22 新增小节。节点是单机 docker(阿里云),生产当前处于「盲飞 + 静默失败」态。

- 🆕 **磁盘慢渗(降级 low,2026-07-23 复核)** —— 原列 medium,核对后大半已做:① ✅ **日志上限**——compose 5 个服务全走 `*default-logging`(10m×3),最吓人的"日志无限涨"已堵;② ✅ **悬空镜像 prune**——`node-deploy-policy.sh:139` 每次部署 `docker image prune -f --filter until=168h`。**残留(慢渗、低危)**:③ 旧的**带 tag** 版本镜像累积(上面 prune 故意 NO `-a`、只清悬空、留回滚,每版多 3 个带 tag 镜像几百 MB);④ `/data/mica/pre-*.sql.gz` 手动还原点不自动清;⑤ 无磁盘水位告警。云盘几十 GB、慢渗不急。要做就是 `node-deploy-policy.sh` 尾部再加"保留最近 N 版镜像 + N 个还原点"。(S)
- 🟡 **备份恢复演练:已有脚本 + 已实跑一次,仍未自动化**(2026-07-30)—— ~~纯手动、无脚本承载~~ ✅:`deploy/restore-drill.sh` + `just restore-drill <basename>`,一条命令恢复进一次性库 → 断言 → DROP(不碰 `mica`、不重启容器),并顺带跑 `rustic check`。三条硬门槛:恢复错误 0、`documents` > 0、**可读页数 > 0**(走每次读都要走的 `views→documents→document_yrs_base` join,要求 `length(state)>0 AND content_text<>''`)—— 因为一次产出空 `state` blob 的恢复能通过所有「表在不在」式断言。**首次实跑(这条路径此前从未被走过)**:错误 0、`_sqlx_migrations`=15、S5 删掉的三张表都回来了、行数与备份时记录逐项一致、32 FK + 19 PK、3331 可读页;`rustic check` 170 snapshot 全过。**残留 = 自动化,而它被一条刻意的安全边界挡着**:CI 那把 key 不是 shell key,`~mica-deploy/.ssh/authorized_keys` 用 `restrict,command=/usr/local/sbin/mica-deploy` 钉死,只能执行 `deploy <version> <sha>`;要让 Actions 定时跑演练,得在节点上装一条新的 pinned 命令 + 一把新 key —— 那是生产侧的凭据/策略变更(deploy.yml 自己写着 CI「may READ the fence that limits it, never install it」),**该由用户决定并执行**,不该由 agent 代办。在那之前:发版落还原点后手动 `just restore-drill` 一次,以及 `rustic check` 进每周节拍,每季度恢复一个 workspace diff 并记日期。(S)`[等用户]`
- 🆕 **Traefik:证书过期重新变成无人看守 + 配置仍不在仓库**(2026-08-04 更新)——
  ~~`uptime.yml` 每天查两域名(app + s3)证书剩余有效期,< 10 天即告警~~ **该 workflow 已于
  2026-08-04 删除**(它声称 `cron: */15` 拨测,实测 21 小时只跑了 12 次而非 84 次 —— GitHub
  Actions 的 schedule 是 best-effort,08-03 那次故障窗口它一次都没跑),所以**证书过期这条线
  又空了**。这一半本来是工作正常的:证书是天级的,即便退化到
  每 1–3.5 小时跑一次也完全够用 —— 删掉是删应用拨测的连带损失,不是因为它不好使。
  重建时它属于 blackbox 探测那一档,不是 `/metrics` 那一档 —— 而 **blackbox 探测已拍板不做**
  (2026-08-04)。所以这条线是**明知空着**,不是忘了。
  **残留照旧**:Traefik 配置本体在仓库外未纳管;ACME 卡死那类故障仍靠 `deploy.md:86` 的
  手动 runbook。
  **⚠️ 但证书这半和被否决的应用拨测不是一回事**:应用拨测要分钟级,GH Actions 给不了;
  证书是**天级**的,那 14% 的触发率对它绰绰有余。两者当初被归成一档一起砍了,约束却完全不同。
  后果也不对称 —— 证书过期 = **全站不可访问**,而现在没有任何东西在看。(S,external)`[等用户]`
- 🆕 **provisioning 层不存在:「给台新机器就能起全套」今天做不到**(2026-08-02 写下方案,未实施)——
  仓库里只有 `deploy/docker-compose.yml`;Traefik、`/data/mica` 目录、`.env`、受限部署账号与
  `/usr/local/sbin/mica-deploy`、ACR 登录全是当年手工装的,没有一条能重放的路径(上面 Traefik
  那条是它的一个切片)。同源的第二个症状:**一个 `vX.Y.Z` tag 焊住三条节奏不同的发布线** ——
  0.13.6 为送一行 compose 配置付了一次完整 Windows 构建 + 给所有桌面用户推了个空更新,
  0.13.7 因单个 `images (cli)` job 挂掉整版作废。症状、拆分方案与优先级在 `docs/cd-plan.md`。
  **刻意不含实施**:最关键的一步(手工走一遍 provisioning 并记下每条命令)还没人做过,
  没走过就写 IaC 等于把猜测固化。(L)`[等用户]`
  **2026-08-06 缩掉一块**:要人肉配的凭据从三样降到**零**(v0.13.16)—— `JWT_SECRET`
  服务端首启自铸存库;`POSTGRES_PASSWORD` 默认(两份 compose 都不发布 postgres 端口,安全);
  **S3 那对也默认了 —— 这一个不安全,是用户当天明确拍板的取舍**:rustfs `:9000` 有意对外,
  默认值又在公开仓库里,装机不改就是可写的桶,因此生产用着默认值会 `warn!`。
  同批还补了自动建桶(以前是埋在 Traefik 章节的手工 `mkdir`,快速上手看不到 → 全栈 healthy
  但上传全 404);实现走 S3 接口(`bucket::ensure_bucket`),不绑 RustFS,换 MinIO/OSS/S3 都成立 ——
  方案与调研见 `docs/bucket-provisioning-plan.md`。主干没动(Traefik / 受限部署账号仍无可重放路径),
  但「新机器起栈」现在只要填 `SERVER_IP` + `MICA_VERSION`。
  **剩下的真问题不是"要填几个变量",是那个不安全的默认** —— 要彻底解掉,得让 `:9000` 不再
  对外(nginx 同源反代 S3),那样这对凭据就能像 PG 口令一样安全地默认。当时评估为工作量与风险
  都更大(SigV4 经代理、CORS、Traefik 栈同步),没做。文档见 `docs/deploy.md` 的
  “Secrets: what you generate, and what generates itself” 一节。

## 数据生命周期与增长

> **这一节空了(2026-08-06)。** 建档(2026-07-22)时的问题是**多处**「删除不真删」+「无界追加」,
> 单节点小盘上会慢慢暴雷 —— 这类问题没有告警会响,只有某天写不进去。此后关掉 13 条(在
> [`roadmap-done.md`](roadmap-done.md)),最后一条「容量配额」于 2026-08-06 **拍板不再扩,
> 整条删除**,理由进了那次提交的信息。留着标题是因为这是一个真实存在的分类 ——
> 空是个**状态**,不是这一档不存在。

## 编辑器与功能广度

- 🟡 **表格**(2026-07-22 复核:原描述大幅失实)—— 实测:**富行内单元格**(粗体/斜体/行内代码/链接,cell 存可重解析 md 源码、两端渲染+编辑,`cellDisplaySpan`/`CellEditController`)与**矩形/行列选区**(跨格拖选、点行/列把手选整行列、Ctrl+C/X 复制为 TSV+HTML、Delete 清空、Esc 清除)**本来就能用**;本轮仅补 **Shift+点击扩展选区**。**合并单元格有意不做**——8 家同类(Notion/AFFiNE/AppFlowy/Outline/siyuan/Joplin/logseq/anytype)调研定论:合并与「Markdown 权威 + round-trip 不变量」在 GFM 下**架构级互斥**(siyuan 能合并因它放弃了 md 权威;Joplin 同约束只能冻单向 HTML;Logseq/Notion 干脆不做)。要做只能另开 HTML 逃生舱块退出 round-trip,是独立决策。块级单元格/列宽 GFM 表达不了,同样不做。
- 🟡 **页面属性/标签**(**M1 已完成**,2026-07-22)—— 走 front matter 权威路(调研定论:同类 md 权威系均如此,见 `docs/page-properties.md`)。**M1 全部落地**:① 数据/权威层——Rust `crates/markdown/src/properties.rs`(解析扁平子集 + 类型推断 + 外科式写回,round-trip 不变量经用户批准从字节保真降为规范化子集稳定)+ Dart 镜像 `properties.dart`,两端逐条测试一致(Rust 9 / Dart 10 全绿);② 页头属性面板 `property_panel.dart`(读 root 块 `data['front_matter']` → 类型化编辑:文本/数字/日期文本框、勾选、tags chips 增删 + 增/删属性 → 编辑经 `onApplyOperations` 单入口自分派写回 root 块,local/cloud-CRDT/cloud-REST 三模式通用,无需穿层新回调);flutter build windows 通过。tags = `tags:` list 属性。**Obsidian-lite 闭环已完成**:增删改属性(类型 text/number/checkbox/date/list)、tags chips、**可搜**(属性值折进 content_text,list 值以 `#值` 存)、**tag 点击精确跳页**(搜 `#值` 只命中真正带该标签的页,ce13cef)、**默认隐藏在页头 ⓘ 图标后**(不占版)+ **AppFlowy 式面包屑路径**(579272f)、AFFiNE 式紧凑面板(7379444)。**故意不做/另立项**:① 数据库视图级「按属性筛选/排序/看板」——是 Notion 数据库那套,与 markdown 权威+round-trip 架构互斥(要豁免 md 权威,AFFiNE/siyuan 路),独立大决策;~~② 存量页要下次编辑才索引属性~~ ✅ **v0.13.14 顺带关掉,不是单独做的**:migration 0019 把每行 `link_targets` 置 NULL,而回填条件是 `content_text = '' OR link_targets IS NULL` —— 于是**每一行的 content_text 都被重新推导**,其中就含 front matter 的属性值。生产快照上做过哨兵验证(把某行 content_text 改成哨兵、link_targets 置 NULL,跑真回填后哨兵被覆盖)。存量页无需逐页编辑即可搜属性;③ 日期选择器 UI(现文本输入)。**数据库视图(带类型列/筛选/relation)另立项**——与 markdown 权威+round-trip 架构互斥,要么破双表示红线要么豁免 md 权威(AFFiNE/siyuan 路),是独立大决策。(L) `[需后端]`
- 🆕 **评论 Phase 2 + 建议(suggest mode)**(2026-08-03 立,Phase 1 整条已归档)—— Phase 1 已闭环并
  经真机验收(评论栏、跨块高亮、锚点随文字位移),整条见 `roadmap-done.md`。**没做的三件**:
  ① **orphan 模糊重锚** —— 锚定文字被删后 thread 只剩 `quote`,现在是死的,按 quote 模糊重锚未做;
  ② **@提及 / 通知** —— 需要通知底座(现在没有),不是评论本身的活;
  ③ **建议(suggest mode)** —— **刻意另立项**:建议是正文内 insert/delete overlay,与评论
  (side-store、正文一字不动)是两个问题,`comments-plan.md` 明写「别共用存储设计」。
  (①S / ②M `[需后端]` / ③L)
- 🟡 **结构块 callout/toggle/embed** —— **callout 已做**(GFM alert `> [!TYPE]` 5 类型,复用 quote 扁平模型、round-trip 干净、记分牌未降,e7ff038)。**残留/定论**(2026-07-22 调研):① **toggle** —— **已拍板(2026-07-29):分两步,先做渲染层折叠 UI**。关键区分:`<details><summary>` 是**合法的 CommonMark/GFM raw-HTML block**(GitHub 官方推荐写法),属于"我们的解析器没接住",**不是**像合并单元格那样"标准 md 表达不了"——所以**不适用**那条的「红线不做」先例。现状比参照产品还弱一档:AppFlowy(原生 `toggle_list` block)和 AFFiNE(`collapsed` 做成 list/heading 通用属性)**编辑器里都有可点的折叠 UI**,只在导出 md 时降级;我们是连折叠 UI 都没有,`<details>` 源码当代码块摆着。~~**第一步(S)**~~ ✅ **已做(2026-08-03)**:纯渲染层折叠上线,`code_block`+`raw:true` 的解析/序列化一字未动,折叠态复用 `data.collapsed`(**实测 round-trip 字节不变**:点开折叠只写 `data.collapsed`,服务端读回的 `<details>` 仍无 `open`)。走 `AtomicBlockRenderer` 注册表 —— **顺带关掉了 P3-1**:注册表原本是 kind→renderer 的 Map,而 `code_block` 已被 Mermaid 占住,第二个 renderer 会**静默替换**它、无编译错误;现在是 kind→List,按注册顺序第一个不返回 null 的胜出(`render-architecture.md` 已改)。**但覆盖面比原计划小,原因是解析形态**:`<details>` 两种形态解析结果完全不同 —— **紧凑形态**(不留空行)是**一个** raw 块,已折叠;**GitHub 文档推荐的空行形态**(留空行让正文按 Markdown 解析)因为空行终止 type-6 HTML 块,解析成**三个块**(`<details>+<summary>` / 真正的 Markdown 正文 / `</details>`)。折叠后者要隐藏的是**一段范围**,而 `_layouts[i]` 在渲染器里全按节点下标索引,跳过节点会整体错位 → 得改成「零高度隐藏布局」,牵动选区/光标/命中测试/拖拽把手多条路径 —— **那是第二步的量级,不是第一步的尾巴**。空行形态今天仍是「两段源码夹着正文」,与改动前一致、不更差。**第二步(M–L,另议)**:规范子集结构化成新 kind(summary 富文本 + body 复用 `data.li` 扁平容器),非规范形态退回现状直通;成本大头在教导入器反解析真实世界五花八门的 `<details>`。**顺带定论**:折叠状态**是文档数据不是视图状态**——AppFlowy(`updateNode` 事务)、AFFiNE(`store.updateBlock`)、以及本仓库自己的代码块折叠先例(`controller.dart` `toggleCollapsed` 走 `update_block`,但 `collapsed` **从不进 md 字节**)三方一致;`<details open>` 让我们有机会比前两家更彻底(折叠态也能 round-trip),但那意味着"点一下折叠"变成一次真实文本编辑,留到第二步再定;② embed 未做。附:render 注册表 P3-1 对这两种块**不是前置**(仅撞已有 kind 如 Graphviz 时才需)。(各 S–L)
- **无屏幕阅读器语义(a11y) / 无 RTL 双向文本** —— 自绘 RenderBox 无 Semantics;硬编码 `TextDirection.ltr`(editor-engine, `render.dart`)。
  **2026-08-03 核实:比原文写的更糟** —— 编辑器目录里 `Semantics` **0 处**(不是「少」,是没有),
  `TextDirection.ltr` **31 处**(2026-08-05 复数;原文写「10+」,08-03 写 32)。缓解:设置里有 85–140% 应用内字号(`EditorAppearance.fontScale`),覆盖低视力一部分。(各 L)
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

- **window_manager→nativeapi / Turso 观望**(各 S,已隔离在 trait 后)。

## 客户端质量与兜底 🆕

> 2026-07-22 新增小节。离线功能面做得全,但崩溃/损坏/双开几处兜底缺失会真丢数据。

> **这一节空了(2026-08-05)。** 拍板不做的条目已整条删除,理由留在那次提交的信息里。
> 留着标题是因为这是一个真实存在的分类 —— 空是个**状态**,不是这一档不存在。

## 性能

- **frb v2 热路径 FFI 基准待测** —— IME/逐字输入若过慢,热路径留 Dart(phase2 §12)。(M)

## 开发者体验 / CI / Markdown

- 🟡 **CI 的 Windows 集成测试:4 个排除项已收 1,剩 3 个卡对象存储**(2026-08-05 复核+补)——
  `flutter-integration.yml` 在 windows-latest **串行**跑离线集成测试(文件间杀
  `mica_flutter.exe`,化解单实例守卫的 debug-connection race)。
  **本轮补上 `cloud_sync_test`**(第一版是假绿,见下):新 `cloud` job 起 windows-latest 预装的 PostgreSQL
  (Windows runner 没有 `services:` 块 —— 这正是当初排除它们的原因)+ 本机 cargo 起 api,
  跑真 Windows 客户端 ↔ 真服务端的 WS/CRDT 收敛。**剩 3 个**(migration_sync /
  offline_image_reconcile / page_switch_fidelity)要走 presign→PUT→complete 传真图片字节,
  而 Windows runner 起不了 rustfs 的 Linux 容器;要做只能上 Windows 版 S3 二进制,
  为三个测试不值。
  **第一版假绿,值得记**:api 起在自己的 step 里,日志写着「api healthy on :8090」,
  而下一步的测试同时打印了「✅ 收敛」和「skipping cloud_sync_test: no server」——
  runner 不保证上一步的子进程活到下一步。且这个测试**没服务端就降级为跳过且仍退出 0**。
  两处都修:服务端与测试放进**同一个 step**,并加一条守卫 —— 输出里出现 `skipping`
  就判失败,否则绿色证明不了服务端可达。
  **顺带修了真正的缺口**(这一轮事故的根因不是缺测试):`cloud_sync_integrity_test`
  **本来就在 CI 里**、并且红了六次提交,失效的是本地闸门 —— `just test` 报「All tests
  passed」被读成了「都过了」。现在它跑完会明说自己没覆盖 integration_test,并给出
  `gh run list` 的查法;另加 `just test-integration` 让离线那批能在本机跑。(S 剩余)
- 🟡 **不可信输入解析面 fuzz:三个面都已覆盖,yrs 那面挖出三类问题、已报上游并提 PR**(2026-08-05)—— 三个吃不可信字节的面:markdown 解析、ZIP 导入、yrs 二进制更新。**前两个**(`markdown/tests/proptest_parse.rs`、`interchange/tests/proptest_zip.rs`,2026-07-23)未挖出 panic,落成快回归门。
  **第三个 2026-08-05 补上**(`mica-core/tests/proptest_yrs.rs`),而**搁置它的理由本身是错的**:原条目写「UB 要 cargo-fuzz + sanitizer 才抓得住,proptest 只抓 panic 不够」—— 实际普通 proptest **几秒就撞上了**,根本没用上 ASan。搁置的代价是这个面白空了两周。
  **挖出三类**(yrs 0.27.3,最新版):① `assert!` panic(`block.rs:92`)—— unwinding,服务端 `catch_unwind` 兜得住;② **UB** `invalid value for char` —— 非 unwinding,兜不住,release 下是静默 UB;③ **无界分配** —— **21 字节让 yrs 要 215 TB**,分配失败直接 abort,debug/release 都复现。②③ 从 `push_update` 可达 = **任何已认证客户端都能打挂 api 进程**。
  **调研结论:别人没规避掉。** y-crdt#415(2024-04 至今 open,标 bug,已指派)提交者原话是生产机器被打挂;AppFlowy 在同一 issue 下报同样问题;#373(evanw)是另一类堆损坏/segfault,同样 open。AppFlowy 的 CRDT 层用的也是 `catch_unwind`,和我们一模一样 —— **挡住的是同一类,漏掉的也是同一类**。
  **已做的处置**:③ 的复现器发到 #415;**PR y-crdt/y-crdt#644** 修 ②③ 两类(`any.rs` 的 `with_capacity` → `try_reserve`,沿用该仓库自己在 `block.rs`/`update.rs` 的既有模式;两处 `from_utf8_unchecked` → 检查版),含回归测试,yrs 全量 375+34 通过,回滚任一处补丁测试即 abort。
  **残留 = 等上游**。这一侧兜不住:限制输入大小没用(才 21 字节),预校验等于重写解码器,进程隔离业界无一家这么做、在单用户实例上不成比例。本地 `proptest_yrs.rs` 全部 `#[ignore]`(两类会 abort 测试进程,不 ignore 就是把 CI 打挂而不是报告),上游合并后去掉 ignore 即变回归门。
  **⚠️ 触发条件:开放注册前必须解决。** 今天风险低是因为注册关闭、只有一个账号;有第二个用户那天,任何普通成员都能让实例反复重启,而且不需要技巧 —— 我是随机灌字节撞出来的。(残留:等上游) `[需后端]`

- 🟡 **cli 测试 + 覆盖率度量**(2026-07-23:起步)—— 原 `crates/cli` 零测试 + 无覆盖率工具。**已做**:9 个纯逻辑单测(`url_file_name`/`slugify`/`sanitize_rel` 路径防穿越/`workspace_dir`/`mirror` 备份 reconcile 增删剪/`Config` serde),`ci.yml` 测试步补 `-p mica-cli`(进 CI),`just coverage`(`cargo llvm-cov`,不入 CI 门)。**残留**:REST `Client` 方法需活服务端未测;`config_path/load/save` 走进程级 env + 真实用户配置目录,未注入点故略(用 serde 落盘形状覆盖)。覆盖率数字化了但远非高覆盖。(S)
- 🆕 **Linux 桌面在仓库但从不在 CI 构建**(low) —— `linux/` runner + 托盘降级逻辑在库,CLAUDE.md 还为它写了约束,但 CI/release 都无 `flutter build linux` → 编译债不可见。flaky 债本身很轻(仅 2 个带理由 `#[ignore]`)。(M)
- **可选/later 基建:Redis、OTel、索引块表** —— 索引块表是搜索/反链/分析的底座(architecture.md)。(L) `[需后端]`

## 产品与公开发布合规 🆕

> 2026-07-22 新增小节。生产节点已上公网 + 开放注册,这类义务是上线后才暴露的。
>
> **2026-07-23 进度**:三项 medium 硬缺口(关注册开关 / 账号删除 / 密码找回,后者顺带
> 建起邮件底座)已全部落地并发版 0.12.16 上线端到端验证。剩下均为 low:AGPL 源码入口、
> 隐私声明·条款、OFL.txt 随附。


> **这一节空了(2026-08-05)。** 拍板不做的条目已整条删除,理由留在那次提交的信息里。
> 留着标题是因为这是一个真实存在的分类 —— 空是个**状态**,不是这一档不存在。

## 接下来最该做的(2026-08-04 重排)

> **这是 top-N,不是清单全文。** 上面正文的每一条都有真实的未做部分 —— 拍板不做的条目
> 已于 2026-08-05 全部删除(理由留在那次提交的信息里)。这一节只排「下一步最该动手的」,所以它短不等于活干完了 ——
> 2026-08-04 它缩到一项,是因为用户当天拍板删掉三项 + 归档一项,不是因为没事可做。
> **别拿这一节的长度当进度**:要看还剩什么,读上面的分区。
> 这一节短的真实含义是:**剩下的都不是缺陷、也不是底线**,该由真实需求驱动排期。

> **旧清单第 1 项那四件安全底线已关**(WS token TTL 心跳、token 出 query string、桌面
> DPAPI + web 凭据走 HttpOnly cookie、弱口令),整条搬进存档。⚠️ **这不等于「安全整档已关」**
> —— 只是剩下的都不到底线级,而 2026-08-05 拍板不做的那几条已整条删除,§安全 现在是空的。
> (compose 注册默认那条 2026-08-05 实测已到生产,整条搬走。)(2026-08-04 复读时抓到的:上一版这里就写成了「整档已关」。)
> 08-03→08-04 又关掉五件:评论 Phase 1、`<details>` 折叠第一步、web outbox 背压、本地文档
> 增量持久化、同块并发编辑的**文本**那半。
>
> **2026-08-05 又关掉七条**:注册默认已到生产、回收站 blob 备份缺口(整条为假)、写放大
> (量化后拍板暂不做)、M8 parser(拍板继续自研)、磁盘慢渗镜像累积、yrs fuzz(挖出三类、
> 已报上游并提 PR)、更新器 digest、i18n 漏网。**这一节的数字每关一条就会过期一次** ——
> 上面那句「37 条」就是隔了一天写下的,今天复核时已经是 33。
>
> **上一版这里有一句错的,已删**:它说「数据面无界增长(yrs base 无 GC + push 写放大)是
> 最该先动的」,而 **yrs base 无 GC 早已实测为假并归档** —— 那句话会把人带去修一个不存在的
> 问题。写放大那条仍然成立,但它单独不构成「无界增长」。这正是维护规矩第 3 条要求把核实
> 结论写进条目本身的原因:结论进了条目,却没同步到引用它的排序清单里。

1. **provisioning 层**(L,方案在 `docs/cd-plan.md`,**刻意不含实施**:手工走一遍还没人做过,
   没走过就写 IaC 等于把猜测固化)。
   **这是清单里仅剩的一项。** 同批出清的:M8(读侧 parser)已拍板继续自研并整条搬进存档;
   同批拍板不做而**整条删除**的:写放大、SVG 纵深、TLS 硬闸门、回收站自动清空、
   Windows 签名、机外黑盒探测、成熟度大件 —— 理由见删除它们那次提交的信息。

**清单只剩一项本身就是个信号**:剩下的既不是缺陷也不是底线,而是一个待拍的决定 + 一笔基建
欠账。产品面已经进入「稳定期」,再排期就该由真实需求驱动,不该由清单自己驱动。

**下一轮核实的重点**:**未核实的 2 条**(fuzz 覆盖面、隐私声明/ToS),以及所有 🟡 条目的
「剩下那半」—— 部分完成的条目最容易整条烂掉:它已经有了一个 ✅,看上去就像做完了。

**这份排序清单本身也要核**,而且它烂得更隐蔽:读的人以为它是结论的索引,于是不会去对条目。
两轮实例:2026-08-03 那版留了一句「yrs base 无 GC 是最该先动的」,而那条早已实测为假并归档;
08-04 重排后复读,又抓到三处 —— 「安全整档已关」(实际还剩 4 条)、「表格补齐」(实际全是有意
不做)、`/metrics` 在第 1 和第 5 项各排了一次。**规矩**:改完条目就回来对一遍这份清单,它引用
的每个结论都要能在条目里找到同样的话。
