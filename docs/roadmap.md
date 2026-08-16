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

- 🟡 **备份恢复演练:已有脚本 + 已实跑一次,仍未自动化**(2026-07-30)—— ~~纯手动、无脚本承载~~ ✅:`deploy/restore-drill.sh` + `just restore-drill <basename>`,一条命令恢复进一次性库 → 断言 → DROP(不碰 `mica`、不重启容器),并顺带跑 `rustic check`。三条硬门槛:恢复错误 0、`documents` > 0、**可读页数 > 0**(走每次读都要走的 `views→documents→document_yrs_base` join,要求 `length(state)>0 AND content_text<>''`)—— 因为一次产出空 `state` blob 的恢复能通过所有「表在不在」式断言。**首次实跑(这条路径此前从未被走过)**:错误 0、`_sqlx_migrations`=15、S5 删掉的三张表都回来了、行数与备份时记录逐项一致、32 FK + 19 PK、3331 可读页;`rustic check` 170 snapshot 全过。**残留 = 自动化,而它被一条刻意的安全边界挡着**:CI 那把 key 不是 shell key,`~mica-deploy/.ssh/authorized_keys` 用 `restrict,command=/usr/local/sbin/mica-deploy` 钉死,只能执行 `deploy <version> <sha>`;要让 Actions 定时跑演练,得在节点上装一条新的 pinned 命令 + 一把新 key —— 那是生产侧的凭据/策略变更(deploy.yml 自己写着 CI「may READ the fence that limits it, never install it」),**该由用户决定并执行**,不该由 agent 代办。在那之前:发版落还原点后手动 `just restore-drill` 一次,以及 `rustic check` 进每周节拍,每季度恢复一个 workspace diff 并记日期。(S)`[等用户]`
- 🆕 **provisioning 层不存在:「给台新机器就能起全套」今天做不到**(2026-08-02 写下方案,未实施)——
  仓库里只有 `deploy/docker-compose.yml`;Traefik、`/data/mica` 目录、`.env`、受限部署账号与
  `/usr/local/sbin/mica-deploy`、ACR 登录全是当年手工装的,没有一条能重放的路径。
  **Traefik 那一片的具体形状**(2026-08-06 从原「证书过期无人看守」条并入,该条已删):配置
  本体在仓库外未纳管;ACME **平时自动续期,不需要盯** —— 唯一已知的卡死场景是**改过 DNS 之后**
  进 issuance backoff、一直挂着 TRAEFIK DEFAULT CERT,处置是重启 Traefik
  (`docs/deploy.md` 的 “Behind Traefik” 一节)。原条目标题写成「证书过期无人看守」,
  读起来像随时会炸,**实际触发窗口只在你主动改 DNS 时存在**,那时人本来就在盯 —— 这是它被
  删掉的原因。同源的第二个症状:**一个 `vX.Y.Z` tag 焊住三条节奏不同的发布线** ——
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

## 导入 / 批量整理(0.13.21 之后剩下的)

- 🟡 **本地模式的 zip 导入入口。** 0.13.21 把它从「空实现、点了没反应」改成了**隐藏**,
  因为客户端没有 zip 读侧(只有 `buildStoreZip` 打包侧,解包在服务端 Rust 里)。要放出来
  得新增一个解包 FFI;资产那半已经就绪(`from_markdown_with_assets` + 本地 CAS),
  剩下的只是「把 zip 解成 `List<ArchiveFile>`」这一步。
- 🟡 **`_selectedMarkdown` 恒为 null。** 0.13.21 删掉了唯一给它赋值的孤儿方法,状态本身
  和依赖它的显示还留着 —— 拆要动外壳 widget,单独做。


- 🔴 **导入 job 落库。** 现在 `state.import_jobs` 是内存里的 `RwLock<HashMap>`:重启 api
  会杀掉任务**并丢掉记录**,而已写入的页面留着 —— 工作区握着半个归档,没有任何东西说明
  这件事。落库一次解决三件:重启不丢、重开应用接得回进度、设置页能有导入历史(Notion /
  Outline / Slack 都把长导入放在 `Settings → Import`,理由是进度必须住在用户能**再次找到**
  的地方)。这也是 0.13.19 只解锁界面、没做"接回进度"的原因:那只在下次部署之前有效。
- 🟡 **导入完成发邮件。** 扒过的四家(Notion / AppFlowy / Slack / GitHub)全都有,而且是
  "人已经关掉应用"这个场景**唯一**被共同验证过的手段,没有替代品。仓库里有 `mail.rs`,
  不是从零开始。
- 🟡 **web/桌面端批量移动(多选)。** 服务端 `batch-move` 已在 0.13.19 落地,CLI 和 MCP 都能
  用了,**客户端一行没接**。真正的工作量不是调接口,是多选交互:选中集合与"当前打开页"
  两个概念要共存、进入多选的手势在 web 和桌面不一样(同一套代码两端跑)、现有 `Draggable`
  携带的是单个 view 要改成携带集合。**动手前先扒 AppFlowy / AFFiNE**(同为页树 + 两端同构),
  重点看有没有谁**刻意不做**多选。建议先用 CLI 跑一轮真实的批量整理,再据此定 UI。
- 🟡 **批量端点的端到端行为测试。** 0.13.19 的保证是"SQL 对真库有效 + 类型正确 + 路由表
  无冲突",**不是**"320 个 id 进去真的删对了"。需要建工作区/页面的完整 fixture。

## 数据生命周期与增长

> **这一节空了(2026-08-06)。** 建档(2026-07-22)时的问题是**多处**「删除不真删」+「无界追加」,
> 单节点小盘上会慢慢暴雷 —— 这类问题没有告警会响,只有某天写不进去。此后关掉 13 条(在
> [`roadmap-done.md`](roadmap-done.md)),最后一条「容量配额」于 2026-08-06 **拍板不再扩,
> 整条删除**,理由进了那次提交的信息。留着标题是因为这是一个真实存在的分类 ——
> 空是个**状态**,不是这一档不存在。

## 编辑器与功能广度

- 🆕 **多标签页(new tab):同时开多个页面、来回切换**(2026-08-08 用户提)——
  AppFlowy 与 Notion 都有:点页面可「在新标签页打开」,顶部一排标签,面包屑跟着当前标签走。
  **参照系明确,按 CLAUDE.md #6 先扒 AppFlowy 的实现再动手**(它是 Flutter 同构,约束最接近)。
  重点看它**刻意没做什么**,以及标签状态存在哪一层。
  **这不是 UI 贴一排按钮,三个已识别的结构问题**:
  ① **导航模型是单数的** —— `_selectedView` / `_selectedBootstrap` 全局各一份,标签要求每个
  标签各自持有一份(含滚动位置、编辑器状态、未提交的改名等)。
  ② **CRDT 会话的生命周期** —— 现在一次只为「当前文档」开一条 WS(`_reconcileSync` 会 dispose
  上一条)。N 个标签是要 N 条连接,还是只有前台标签连、后台标签退成只读快照?**这是最贵的一个
  决定**,直接关系内存与服务端连接数,别在写 UI 的过程中顺手选掉。
  ③ **格式工具栏会被挤**(用户当场就点出来了)—— 标签行与工具栏抢同一条横向空间。多半要给
  工具栏单开一行,那又要重新算编辑器的可用高度。
  **另需想清**:标签要不要持久化(重启后恢复)?窄壳(`kNarrowShellWidth` 以下)怎么退化?
  与刚做完的「定位/新建落点」规则如何共存(在新标签打开是否改变定位)?(L)

  **2026-08-08 已扒 AppFlowy(commit `5cf3a36`),两个问题有答案了**:

  - **结构(答①)**:`workspace/application/tabs/tabs_bloc.dart` 里 `TabsBloc` 持
    `List<PageManager>` + `currentPageManager`;`PageManager`(在
    `presentation/home/home_stack.dart`)**每标签一个**,内部装一个 `plugin`。UI 是
    `home/tabs/tabs_manager.dart`(栏) + `flowy_tab.dart`(单个),关闭走
    `TabsEvent.closeTab(pageManager.plugin.id)`。顺带:它**支持固定标签**
    (`indexOfFirstUnpinnedTab` 的插入逻辑)。
    **关键差异**:它的 per-tab 状态不是「一个 view id」,而是一整个可实例化的
    `PageManager`。Mica 的 `_selectedView`/`_selectedBootstrap` 是**全局单数字段** ——
    所以加标签对 AppFlowy 是自然的,对 Mica 不是,**真正的工作量在把这两个字段收成一个
    可实例化对象**,不在标签栏本身。

  - **生命周期(答②的一半)**:`PageManager.setPlugin` 里
    `if (newPlugin.id != plugin.id && disposeExisting) _plugin.dispose();` ——
    dispose **只发生在「这个标签换了内容」**。切换标签不走这条路,只改
    `currentPageManager`,其余 `PageManager` 与其 plugin **原地留着**。
    即 **N 个标签 = N 个活着的实例,后台不降级不断开**。

  **⚠️ 但这一条不能照抄。** AppFlowy 敢这么做是因为同步走本地 Rust 后端,多开一个文档
  几乎没有远端成本;**Mica 的云工作区每个文档是一条真 WebSocket**,照搬就是「开 8 个标签
  = 8 条常驻连接 + 8 份 CRDT 内存」。这是 Mica 特有的约束,AppFlowy 没有 ——
  **CLAUDE.md #6 说的「相同约束下」在这里不成立,所以它的答案只能参考不能采纳。**

  **2026-08-12 又扒了 AFFiNE(commit `26c515e`),它的答案跟 AppFlowy 完全不同**:

  - **标签不在 app 里,在 Electron 主进程里** ——
    `apps/electron/src/main/windows-manager/tab-views.ts` 的 `WebContentViewsManager`,
    **每个标签一个 `WebContentsView`**(独立渲染进程,各自 `loadURL` 到
    `/workspace/<id>/<docId>`)。一个标签 = 一整个 app 实例。
  - **web 版压根没有标签**:`desktop/components/app-container/index.tsx` 里
    `LayoutComponent = BUILD_CONFIG.isElectron ? DesktopLayout : BrowserLayout`,
    浏览器版直接用浏览器自己的标签页。
  - **标签持久化(答上一轮"仍未扒"之一):有。** `tabViewsMetaSchema`(zod,
    `main/shared-state-schema.ts`)写进 `globalStateStorage`,重启恢复。
  - **恢复是懒加载的**:启动只 `showTab(activeWorkbenchId)`,其余标签**只有 meta 没有 view**
    (`tabsStatus$` 里 `loaded: views.has(w.id)`),点到才 `loadTab`。但一旦加载**就不再卸载**
    —— 只有 `setBackgroundThrottling(true)`(Chromium 后台节流,停 timer/rAF,**不断连接**)。
  - **关标签(答另一半)**:不允许关最后一个;延迟 500ms 再 dispose「避免闪烁」;dispose 是逐级
    升压的 `close(waitForBeforeUnload:true)` → 等 1s → `forcefullyCrashRenderer()` →
    `close(false)`;有 `closedWorkbenches` 栈支持**撤销关闭标签**;固定标签不会被隐式关掉。
  - **一个标签里还能再分屏**:`WorkbenchMeta.views: WorkbenchViewMeta[]`,`separateView`
    把分屏里的一个 view 拆成新标签。

  **⚠️ 最重要的一条 —— 上一轮那张表的共同前提是错的。**
  AFFiNE 的同步**不是每文档一条连接**:`common/nbstore/src/impls/cloud/socket.ts` 里 socket 是
  **按 endpoint 缓存的进程内单例 + 引用计数**(`SOCKET_MANAGER_CACHE` / `refCount`),
  `space:join` 只带 `spaceType`/`spaceId`,**`docId` 是消息里的一个字段**
  (`space:push-doc-update` / `space:broadcast-doc-update`)。一个渲染进程里开多少文档
  **都只有一条连接**。
  Mica 是 `/ws/workspaces/{workspace_id}/documents/{document_id}`(`routes/mod.rs:28`),
  `document_id` 在 upgrade 时绑死、一路传进 `run_connection`(`ws.rs:70`)——
  **「N 个标签 = N 条 WS」是 Mica 自己的连接粒度造成的,不是多标签的固有代价。两个参照系都没有
  这个约束。** 这正是 CLAUDE.md #6 点名的那类错误:给「几选一」之前先问这些选项的**共同前提**
  验证过没有。所以表加第四行:

  **✅ 2026-08-12 用户拍板:第三种(全连但设上限 ≤3,超出按 LRU 降级)。** 已实现
  (`kMaxLiveSyncTabs` / `tabsToPark`,`lib/doc_tab.dart`)。第四种仍是长期正解,见表末行。

  | 方案 | 代价 |
  | --- | --- |
  | 全连(照抄 AppFlowy) | 简单、切换零延迟;连接数与内存随标签线性增长 |
  | 只前台连、后台留快照 | 连接恒为 1;切回要重连 + 重新 bootstrap,有可感知延迟且后台内容是旧的 |
  | 全连但设上限(如 ≤3,超出按 LRU 降级) | 兼顾,但多一套 LRU 要维护 |
  | **改连接粒度:一条工作区连接多路复用文档**(AFFiNE 的做法) | 连接恒为 1、无降级、无 LRU、切换零延迟 —— **长期正解**;但要动协议两端(路由、`run_connection` 里的文档态、服务端广播分组)并升 `WS_PROTOCOL_VERSION`(该机制已就位:`check_protocol` / `ws_min_protocol`)。**明显比标签栏本身贵** |

  倾向:**先按第三种做标签,第四种日后单独改** —— 粒度换掉时标签这层不用重写,所以不必为了标签
  先做完协议改造。但如果打算做第四种,**别把 LRU 做复杂**,它是过渡件。

  **已落地(2026-08-12,commit `9d97ce7` / `20b458f` + 本轮)**:`DocTab` 模型、标签栏、
  右键「在新标签页打开」、切换/关闭、≤3 全连 + LRU 降级。

  **本条剩下的**:
  - 键盘快捷键(Ctrl+T / Ctrl+W / Ctrl+Tab)—— 加了要**四处同步**(见本文件"快捷键"约定)。
  - 窄壳(`kNarrowShellWidth` 以下)标签栏怎么退化,尚未处理。
  - **标签是否持久化仍未拍板**。AFFiNE 有(zod schema 进 globalStateStorage,且恢复是懒加载);
    AppFlowy 未确认。
  - 标签拖拽排序、固定标签、撤销关闭 —— 两家都有,我们**都没做,也没需求**,先不做。

  ✅ **已真机验过(2026-08-12,v0.13.18 发版前)**:常驻标签栏、右键「在新标签页打开」、
  `+` 菜单(新建 / 挑一个已有页面)、切换、关闭。原先这里写着「未在真机实测」——
  那种否定式条目只会静默变假,所以发版时当场核掉。

  **答③(格式工具栏被挤)—— 两家做法相反,但都不是「给工具栏单开一行」**:
  - AFFiNE:标签行是**整宽一行,压在侧边栏上面**(JSX 结构
    `desktopAppViewContainer` → `desktopTabsHeader` + `desktopAppViewMain`(sidebar + main))。
    而且这行**不是纯增量** —— 侧边栏开关与前进后退按钮搬进了标签行的 `left` 槽
    (`<AppTabsHeader left={<SidebarSwitch/><NavigationButtons/>}/>`),把自己占的高度赚回来一部分。
  - AppFlowy:`TabsManager` 在 `HomeStack` **内容列内部**(侧边栏右边,带 `menuSpacing` 左内边距),
    侧边栏保持满高。
  - 另:AppFlowy 切标签时**主动 unfocus 编辑器以收起浮动选择工具栏**
    (`home_stack.dart` 的 `onIndexChanged` 注释原话)—— 这条与横向空间无关,对我们直接适用。

  **所以③的前提也要修正**:标签行是**纵向**多占一行,**不与格式工具栏抢横向空间**,
  用户当时担心的「估计要新开一行放格式工具栏」按两家的做法并不需要;真正要重算的是编辑器可用**高度**。

> **这一节空了(2026-08-06)。** 做完的搬进 [`roadmap-done.md`](roadmap-done.md),拍板不做的整条删除(理由留在那几次提交的信息里)。
> 留着标题是因为这是一个真实存在的分类 —— 空是个**状态**,不是这一档不存在。

## 平台覆盖

> **这一节空了(2026-08-06)。** 拍板不做的条目已整条删除,理由留在那次提交的信息里。
> 留着标题是因为这是一个真实存在的分类 —— 空是个**状态**,不是这一档不存在。

## 客户端质量与兜底

> **这一节空了(2026-08-07)。** 建档 2026-07-22,离线兜底那批已全部关闭并归档;08-05 清空过
> 一次(拍板不做的整条删除);08-07 进的两条(换头像不同步、页内查找吞按键,都是 v0.13.16
> 上线冒烟测**实测**发现的,不是盘点想出来的)当天修完并搬进
> [`roadmap-done.md`](roadmap-done.md)。留着标题是因为这是一个真实存在的分类 ——
> 空是个**状态**,不是这一档不存在。

- 🆕 **跟随画布的浮层里没有 tooltip 了**(2026-08-12,`_followCanvas`)——
  格式条 / 链接条 / 斜杠菜单 / 单元格编辑器都由 `CompositedTransformFollower` 定位,
  而本版 Flutter 的 `Tooltip` 是 `OverlayPortal`,它的 `_OverlayChildLayoutBuilder`
  **在布局期**就向锚点要 paint transform —— `RenderFollowerLayer` 那时还没有,于是每帧一次
  断言(用户拖选时刷红框,真机栈点名 `editor.dart` 的 `IconButton`)。
  现在整棵子树 `TooltipVisibility(visible: false)`,**代价是这些浮层的悬停提示全没了**。
  Flutter 官方给的正解是把 follower 换成 `OverlayPortal.overlayChildLayoutBuilder`,
  两者都能保住 —— 但那要重写这些条如何跟随画布(`777c07c` / `9c367b2` 正是为了让它们跟随),
  所以单独算一件事。**未拍板**。(M)

## 性能

- 🆕 **搜索正文靠 `ILIKE` 全表扫,没有文本索引**(2026-08-12 重新记入)——
  `document_yrs_base.content_text ILIKE '%needle%'`,前置通配符,任何 B-tree 都用不上。
  **实测(2026-08-06,生产)**:单工作区 81ms 里 **~75ms 是读 798 篇文档的正文**,
  剩下 6ms 才是行管道。
  **跨工作区搜索(2026-08-12,`GET /search`)把这条按可见文档数线性放大** ——
  工作区一多就是秒级,而根因不在那条路由。
  难点是 **CJK**:PG 自带的 `to_tsvector` 不分中文词,所以走不了标准 FTS;
  候选是 `pg_trgm` GIN(对 CJK 子串有效,索引体积换扫描)或外部分词器。
  **未拍板做不做,先把事实记下来** —— 这一条以前存在过、被整条删掉了,而删掉之后
  `crates/api-server/src/routes/documents.rs` 里两处注释仍在写「见 roadmap 的搜索条目」,
  指向一个不存在的东西。空的性能小节 + 悬空引用,合起来正好把一个已知瓶颈说成不存在。(M)

## 开发者体验 / CI / Markdown

- 🟡 **不可信输入解析面 fuzz:三个面都已覆盖,yrs 那面挖出三类问题、已报上游并提 PR**(2026-08-05)—— 三个吃不可信字节的面:markdown 解析、ZIP 导入、yrs 二进制更新。**前两个**(`markdown/tests/proptest_parse.rs`、`interchange/tests/proptest_zip.rs`,2026-07-23)未挖出 panic,落成快回归门。
  **第三个 2026-08-05 补上**(`mica-core/tests/proptest_yrs.rs`),而**搁置它的理由本身是错的**:原条目写「UB 要 cargo-fuzz + sanitizer 才抓得住,proptest 只抓 panic 不够」—— 实际普通 proptest **几秒就撞上了**,根本没用上 ASan。搁置的代价是这个面白空了两周。
  **挖出三类**(yrs 0.27.3,最新版):① `assert!` panic(`block.rs:92`)—— unwinding,服务端 `catch_unwind` 兜得住;② **UB** `invalid value for char` —— 非 unwinding,兜不住,release 下是静默 UB;③ **无界分配** —— **21 字节让 yrs 要 215 TB**,分配失败直接 abort,debug/release 都复现。②③ 从 `push_update` 可达 = **任何已认证客户端都能打挂 api 进程**。
  **调研结论:别人没规避掉。** y-crdt#415(2024-04 至今 open,标 bug,已指派)提交者原话是生产机器被打挂;AppFlowy 在同一 issue 下报同样问题;#373(evanw)是另一类堆损坏/segfault,同样 open。AppFlowy 的 CRDT 层用的也是 `catch_unwind`,和我们一模一样 —— **挡住的是同一类,漏掉的也是同一类**。
  **已做的处置**:③ 的复现器发到 #415;**PR y-crdt/y-crdt#644** 修 ②③ 两类(`any.rs` 的 `with_capacity` → `try_reserve`,沿用该仓库自己在 `block.rs`/`update.rs` 的既有模式;两处 `from_utf8_unchecked` → 检查版),含回归测试,yrs 全量 375+34 通过,回滚任一处补丁测试即 abort。
  **残留 = 等上游**。这一侧兜不住:限制输入大小没用(才 21 字节),预校验等于重写解码器,进程隔离业界无一家这么做、在单用户实例上不成比例。本地 `proptest_yrs.rs` 全部 `#[ignore]`(两类会 abort 测试进程,不 ignore 就是把 CI 打挂而不是报告),上游合并后去掉 ignore 即变回归门。
  **⚠️ 触发条件:开放注册前必须解决。** 今天风险低是因为注册关闭、只有一个账号;有第二个用户那天,任何普通成员都能让实例反复重启,而且不需要技巧 —— 我是随机灌字节撞出来的。(残留:等上游) `[需后端]`

## 产品与公开发布合规 🆕

> 2026-07-22 新增小节。生产节点已上公网 + 开放注册,这类义务是上线后才暴露的。
>
> **2026-07-23 进度**:三项 medium 硬缺口(关注册开关 / 账号删除 / 密码找回,后者顺带
> 建起邮件底座)已全部落地并发版 0.12.16 上线端到端验证。剩下均为 low:AGPL 源码入口、
> 隐私声明·条款、OFL.txt 随附。


> **这一节空了(2026-08-05)。** 拍板不做的条目已整条删除,理由留在那次提交的信息里。
> 留着标题是因为这是一个真实存在的分类 —— 空是个**状态**,不是这一档不存在。

## 接下来最该做的

> **2026-08-07 重写。** 上一版这节比它索引的整个待办总账还长,而且引用的东西大半已经不存在
> ——「37 条 / 33 条」的计数、「未核实的 2 条」、对表格与 `/metrics` 的排序,指向的条目当天
> 都已归档或删除。**它自己最后一句就是规矩**:「改完条目就回来对一遍这份清单」——
> 而 08-06 那轮改了十几条,一次都没回来对过。**同一个失败,第三次。**
>
> 这一次连结构一起改:不再复述正文,因为**正文只剩 3 条,复述它没有信息量**。

> **08-07 当天复核过一次(这是上面那条规矩第一次真的被执行)。** 中途总账是 5 条:v0.13.16
> 上线冒烟测进了两个真 bug(换头像不同步、页内查找吞按键),当天都修完并搬进
> [`roadmap-done.md`](roadmap-done.md),于是又回到 3 条。**这一节的内容因此仍然成立** ——
> 但成立是核对出来的,不是默认的。

**待办总账只剩 3 条,其中 2 条卡在用户**:

1. **provisioning 层**(L,`[等用户]`)—— 唯一还需要"决定做不做"的一项,方案在
   `docs/cd-plan.md`。**刻意不含实施**:最关键的一步(手工走一遍并记下每条命令)还没人做过,
   没走过就写 IaC 等于把猜测固化。
2. **备份恢复演练自动化**(S,`[等用户]`)—— 被一条刻意的安全边界挡着,要在生产装新 pinned
   命令 + 新 key,是凭据/策略变更。
3. **yrs fuzz**(等上游)—— 修复已提 PR,这一侧兜不住,只能等合并。

**所以今天没有"下一步最该做的"** —— 三条要么等用户、要么等上游。这不是"活干完了",是
**剩下的都不是我能独立推进的**。真要再排期,该由真实需求驱动。

**这份清单为什么反复烂**(记在这里,因为它比清单内容更有用):读的人**以为它是结论的索引,
于是不会去对条目**。三轮实例:08-03「yrs base 无 GC 是最该先动的」(那条早已实测为假并归档);
08-04 复读抓到三处(「安全整档已关」实际还剩 4 条、「表格补齐」实际全是有意不做、
`/metrics` 排了两次);08-06 改完十几条后整节没同步。**规矩不变,只是显然不够**:
改完条目就回来对一遍,它引用的每个结论都要能在条目里找到同样的话。
