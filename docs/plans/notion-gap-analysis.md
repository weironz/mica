# Notion 对标分析（2026-10-18 专项 · 2026-10-19 第三轮复核）

**一句话结论**：在「单人/小团队写作与知识沉淀」这条窄道上，Mica 的**工程纪律**（测试、发布、回归）确实处在 Notion 所属量级；但上一版用来支撑「本地优先反超」和「下一步做什么」的两条论据，在第三轮复核里一条**被推翻**、一条**已过期**。修正后，差异要从「有 / 无」改说「深 / 浅」，策略结论不变。

> **信息来源分级（2026-10-19 第三轮后）**
>
> - ✅ **联网核实**：有可访问的原文 URL + 日期 / 标题。
> - ⚠️ **只到标题层**：本机把 `notion.com`、`developers.notion.com`、`github.com` 解析成
>   非公网地址，`web_fetch` 全部被拦。第三轮拿到的证据是**搜索结果里的标题、URL、日期**，
>   不是正文原句；引文级复核仍以 10-18 轮为准，并逐条标明。
> - ❓ **转述待验**：二手报道或训练知识，未实测。
> - 🧪 **仓库实测**：本轮亲自跑过，命令与结果见 §5。
>
> 把「✅」的门槛写死，是因为上一轮的教训：一条没注明**信源窗口**的否定式断言，
> 被标成了 ✅（见 §1）。

分析基线：**v0.13.56**（2026-10-19 复核；`pubspec.yaml`、`main.dart` 的 `kAppVersion`、
根 `Cargo.toml` 三处一致）。Mica 侧数字全部为本轮实测。

## 0. 第三轮结论速览

| # | 10-18 / 10-19 版的说法 | 第三轮结论 | 依据 |
|---|---|---|---|
| 1 | Notion 无专门的离线支持；help 中心无离线文档页 | **推翻** | Notion 2.53（2025-08-19）发版即「offline mode」；help 有 `use-pages-offline`；TechCrunch / 9to5Mac 2025-08-20 报道（§1） |
| 2 | 当前最值的下一步 = 多标签页，可开工 | **过期** | 主体 2026-08-12 随 v0.13.18 发版并真机验过；只剩三条尾巴（§2） |
| 3 | Notion 已有官方 MCP、53 个工具（10-18 核实） | **本轮未能复核** | `developers.notion.com` 不可达；沿用 10-18 数字并标 ⚠️（§3） |
| 4 | 两轮净新增 P1 共 3 条 | **应为 2 条** | `purge_view` 在 `roadmap.md:67-68` 是已知的、明确留待拍板的 S 项，不是评审新发现（§8） |
| 5 | 协作 undo：`reconcile()` 合入远端时「不更新快照」 | **机制描述不准，结论不变** | `controller.dart:184` 会 `_present = _snapshot()`；保留的是历史栈（§8） |
| 6 | 641/641、34 工具、940 行、v0.13.56 | **复核通过** | 见 §5 |

## 1. 被推翻的那条：「Notion 没有离线支持」

上一版的核心论据是「Notion 无专门的离线支持」，并由此推出「本地优先是架构差异 / 反超」。
这条**是错的**，而且错了不止一年：

| 信源 | 标题 / 事实 | 日期 |
|---|---|---|
| 官方发版页（西 / 法 / 日 / 韩 / 葡 五语种同题） | 「Notion 2.53: Offline mode」 — https://www.notion.com/releases/2025-08-19 | 2025-08-19 |
| 官方帮助中心 | 存在 `use-pages-offline` 页，含 `#who-can-use-pages-offline`、`#manage-downloaded-pages` 锚点 — https://www.notion.com/help/use-pages-offline | — |
| TechCrunch | 「Finally, Notion now works without an internet connection」 — https://techcrunch.com/2025/08/20/finally-notion-now-works-without-an-internet-connection/ | 2025-08-20 |
| 9to5Mac | 「Notion gains offline mode, but it's not quite automatic」 — https://9to5mac.com/2025/08/20/notion-offline-mode/ | 2025-08-20 |
| Yahoo / Mynavi | 「You'll Need to Manually Enable It」 — https://tech.yahoo.com/apps/articles/notion-finally-allows-offline-editing-163000253.html ；「接続なしでも編集・作成が可能に」 — https://news.mynavi.jp/article/20250820-3410476/ | 2025-08-20 |

**从标题层可以推断的边界**（⚠️ 正文未取，属推断不是核实）：Notion 的离线是
「**手动启用 + 按页下载**」的形态，支持离线编辑与新建；不是「整库本地副本」。
「谁可用」「数据库 / 评论 / AI 在离线时是否可用」这些正文细节，**本轮没取到，不下结论**。

**上一轮为什么错**：勘误提交（`b3b74d5`）写的是「help 无文档、近四月 releases 无条目」——
功能 2025-08-19 就发了，而检索窗口是 2026-07~10。**在一个从功能发布 11 个月后才开始的
窗口里找「没有」，能找到的只有「最近没再提」。** 这是否定式断言的经典失败：
它证明的是「我没看见」，不是「不存在」。

**影响**：

- 「反超」这个词必须撤。反超的前提是对方没有，而对方有。
- 差异仍然存在，但要改成**深度**表述：Mica 是「整个工作区 on-device 库 + outbox 断网重放 +
  yrs/yjs 双引擎收敛」，Notion 是「选定页的离线缓存 + 编辑」。**「更彻底」目前是架构层推断，
  没有实测对照**——要下这个结论，得拿两边的离线边界各跑一遍（Notion 侧需要真机断网）。
- 迁移理由改写：不是「Notion 离线残废」，而是「**自托管 + 数据主权 + 可往返导出**」这三条
  里，**自托管是 Notion 结构上给不了的**；离线深度是待验证的加分项，不是已证的护城河。

## 2. 过期的那条：多标签页不是「下一步」，是两个月前就发版的东西

上一版把「多标签页」列进差距表（代价：中），并在文末写「**当前最值的下一步：多标签页——
结构分析已完成，可开工**」。roadmap 的同一节里，紧接着的记录是：

| 时间 | 事件 | 位置 |
|---|---|---|
| 2026-08-08 | 扒 AppFlowy，回答结构问题①② | `roadmap.md:80-117` |
| 2026-08-12 | 扒 AFFiNE，推翻「N 标签 = N 条 WS」这个共同前提 | `roadmap.md:119-149` |
| 2026-08-12 | 用户拍板第三种：全连但上限 ≤3，超出 LRU 降级 | `roadmap.md:151-152` |
| 2026-08-12 | 落地：`DocTab`、标签栏、右键新开、切换 / 关闭 | `roadmap.md:164-165` |
| 2026-08-12 | 发版前真机验过（v0.13.18） | `roadmap.md:174-176` |

**失败形状**：只读了同一条目里 08-08 的分析段，没读到 08-12 的落地段——信息就在**同一个
小节的下半部分**。文档写下时（10-18），主体功能已经上线两个月。

**剩下的才是真的**（`roadmap.md:167-172`、未关闭表 #8）：键盘快捷键（Ctrl+T / W / Tab，
要四处同步）、窄壳退化、标签持久化（**未拍板**）。这三条值 M，不是「可开工的大工程」。
〔2026-10-30 三条都已做完（`dbd1a254`，v0.14.2 上线）：持久化按 AFFiNE 的懒加载模型，见 `roadmap-done.md`。〕

## 3. 新增：Notion 3.0 agents（2025-09-18）

- 官方发版页「Notion 3.0: les agents」— https://www.notion.com/releases/2025-09-18
- 官方博客「Introducing Notion 3.0」— https://www.notion.com/blog/introducing-notion-3-0

⚠️ 只有标题层证据，**具体能力未核实**。但它足以说明一件事：**「AI 原生」不再是 Mica 的
独占差异**，Notion 也在把 agent 做成产品面。Mica 真正剩下的 AI 差异是「**自托管 + 自带
模型与 key + MCP 指向自己的实例**」——这是部署形态的差异，不是能力有无的差异。

## 4. Notion MCP 工具面（沿用 10-18，本轮未能复核）

10-18 轮核实：官方 remote MCP server（`developers.notion.com/docs/mcp`），OAuth 授权、
内容按成员权限限定、企业版可管理客户端连接，**约 53 个工具**，覆盖搜索 / 读写 / 评论全 CRUD、
文件上传下载、skills 上传下载、agent 会话编排（spawn / wait / stop session）。

本轮 `developers.notion.com` 不可达，**这个数字没能复核**。旁证：第三方文章
「Custom Agent session tools in Notion MCP」说明 agent 会话族确实存在（标题层）。
Mica 侧对照：**34 个工具**（本轮实测，见 §5；2026-10-30 补上评论 / 成员 / 文件删除后为 39 个），读工具只读、写工具受 PAT 与工作区权限约束。

真实位置：Mica 的 MCP **存在且自托管**（Notion 的托管在它的云上，自托管场景用不上）；
工具数与编排能力落后，差距集中在文件族与 agent 会话族。

## 5. Mica 侧本轮实测

| 项 | 上一版 | 本轮实测 | 命令 / 依据 |
|---|---|---|---|
| 版本 | 0.13.56 | **0.13.56** | `pubspec.yaml` / `kAppVersion` / `Cargo.toml` 三处一致 |
| CommonMark | 641/641 | **641/641（100%）** | `cargo test -p mica-markdown --test commonmark_scoreboard -- --nocapture` |
| 金样本 | 741 输入字节级守卫 | **741，测试通过** | `golden_master` 通过；`block_handlers.rs:14` 注明 741 corpus inputs |
| MCP 工具 | 34 | **34** | `#[tool(` 计数：organize 11 / trash 7 / read 6 / write 6 / admin 4 |
| roadmap-done | 940 行 | **940 行** | `(Get-Content docs/plans/roadmap-done.md).Count` |
| roadmap 未关闭 | 7 条 | **7 条**（2026-10-16 复核） | `roadmap.md:285-295` |

「641/641」的口径没变：11 条 GFM 方言豁免（裸 autolink、tagfilter）不计入。

## 6. 差距表（2026-10-19 修订）

| 缺口 | 代价 | 现状 |
|---|---|---|
| ~~多标签页~~ | — | **已发版**（v0.13.18，2026-08-12）；剩余快捷键 / 窄壳 / 持久化于 2026-10-30 做完（v0.14.2），整条已归档 |
| 移动端 | 大 | Flutter 壳已有，触屏 / IME 是独立工程量。**roadmap 与 roadmap-done 里都没有这条**：`平台覆盖` 节于 2026-08-06 清空，理由记在 `7ead554`——「无触屏选择手势…需要移动端。**没有移动端**」。即移动端本身从未作为条目排期；CLAUDE.md 的平台现状同样写「无 macOS / 移动端」 |
| **数据库（表格 / 看板 / 日历视图）** | **极大** | Notion 真正的护城河。roadmap-done:488 已判定「与 markdown 权威 + round-trip 架构互斥，是独立大决策」——**不追**。Mica 只有简单表格 |
| 团队协作 | 大 | 评论已有；owner / admin / editor 已有；细粒度权限、评论 @、分享页协作编辑、搜索权限过滤没有。协作 undo 仍有 P1（§8） |
| 生态（API / webhook / 模板市场 / 嵌入） | 大 | 基本没有（有 MCP，方向不同） |
| 搜索 | 中 | 有 FTS + BodyIndex + CJK；无跨库 AI 问答 |

## 7. 定性判断（修订）

差距**不是工程质量**，而是**产品定位重叠度**：

- Notion = 「团队工作操作系统」：数据库为核、协作生态、云端 SaaS、agent。
- Mica = 「个人 / 小团队知识库 + 本地优先 + 自托管 + MCP」。

**上一版的表述要收回一半。** 「Mica 的工程质量已达到 Notion 水平（部分维度反超）」这句话里，
「反超」所依赖的离线论据已被推翻；准确的表述是：**在同一量级的功能面上，Mica 的
测试 / 发布 / 回归纪律不低于 Notion 所属组织；产品广度差距是结构性的，不是 bug 债。**
工程质量的主张仍然成立——641/641、761 测试、CI 门禁、940 行复盘都撑得住——但它**不能**
再用「对方没有离线」来论证。

**策略结论不变**：追赶 Notion 的功能广度是死路（10 年 + 数亿美金的产物）；窄道是
「本地优先 + 数据主权 + 自托管 + MCP/AI 原生」。但窄道的卖点要按第三轮修正后的证据重排：
**自托管（结构差异）> 导出往返（已证）> 离线深度（待验）**。

## 8. 工程质量校验（2026-10-18 / 10-19 两轮评审 + 第三轮复核）

前三点是 10-19 的结论，第四、五点是第三轮新增：

1. **单轮评审不足以穷举 P1**。10-18（4 个域切片）与 10-19（9 个域切片 + clippy 全 workspace
   零警告）两轮重叠约 14 项，各有一片对方没看到的区域。10-19 报告列了 3 条 P1，其中 1 条是
   路线图已知项（见第 4 点），**实际净新增 2 条**。
2. **反复命中的形状只有两个**：「兄弟路径漏守卫」和「不可信输入无上界」。这两条值得写进
   checklist 与 lessons，而不是逐条修。
3. **修复纪律本身是质量主张最强的论据**。10-18 轮在 24h 内闭环了绝大部分项并全量回归。
   准确表述是「**发现-修复-回归的速度与钉形测试纪律**达到甚至超过 Notion 所属量级的工程组织」，
   而不是「没有 P1」。
4. **「净新增」这个标签本身也要复核**。`purge_view` 缺 `is_deleted` 在 `roadmap.md:67-68`
   是一条已知的、明确写着「要不要收紧它是独立决定」的 S 项——风险真实（任意 editor 可永久
   删活页），但它**不是两轮评审的新发现**。所以「两轮净新增 P1 = 3」应改为 **2**
   （本地 snippet panic、协作 undo）。评审发现与路线图已知项要对照，否则会把「早就在账上
   的欠账」记成「这轮新欠的」。
5. **评审的机制描述也可能不准**。10-19 #8 写「`reconcile()` 合入远端 CRDT 时不更新快照」，
   源码里 `reconcile()` 末尾会执行 `_present = _snapshot()`（`controller.dart:184`）——
   它对齐的是**当前基线**，保留的是 `_undoStack` 里的**历史快照**。所以缺陷不是「快照没跟上
   远端」，而是「历史栈里存的是远端编辑之前的全域状态」；`_restore()` 会把它整份还原并
   diff 成 `delete_block` 推给协作者。**结论不变，但修法必须按历史栈设计**——不能在
   `reconcile()` 里简单重置，那会顺手清掉用户自己的撤销历史。

**待收口的标准**（与上一版一致，进度未变）：两轮净新增 P1 清零 + 协作 undo 有结构方案 +
B4 存储域 XSS（已有方案，待真机验证）落地。

## 9. 当前最值的下一步（修订）

按 roadmap 的 7 条未关闭项 + 本轮评审，重排如下（**roadmap 仍是待办权威，本节只是分析结论**）：

1. **协作 undo 的结构方案**（评审 P1 #8）。它直接决定「实时协作」这句主张能不能站住：
   协作者在场时，一次 undo 会回滚甚至删除对方的编辑。短期兜底（协作者在场时禁用全域 undo）、
   中期按块 id 归并、长期记录本地 op 的逆操作。
2. **多标签页三条尾巴**（roadmap #8）：快捷键要按「四处同步」的约定加；窄壳退化未处理；
   **持久化未拍板**，需要用户决定。
3. **`purge_view` 的 `is_deleted` 守卫**（roadmap #7 附注）：一行过滤 + 一条 pg 回归测试，
   顺带把「兄弟路径漏守卫」变成门禁。

**不追**（本轮不改这些状态）：数据库视图（roadmap-done:488 已拍板不追）、全生态、
移动端（从未进入排期）。

## 10. 复查锚点

下次重估时核对这几个数，**都能用命令直接验**：

- 版本三处一致：`pubspec.yaml` / `main.dart` `kAppVersion` / 根 `Cargo.toml`
- CommonMark 计分：`cargo test -p mica-markdown --test commonmark_scoreboard -- --nocapture`
  （2026-10-19 = **641/641**）
- 金样本：`cargo test -p mica-markdown --test golden_master`（741 corpus inputs）
- MCP 工具数：`#[tool(` 计数求和（2026-10-19 = **34**）
- 功能缺口消解：`docs/plans/roadmap.md` 对应条目是否已搬去 `docs/plans/roadmap-done.md`
  （当前未关闭 **7** 条，`roadmap.md:285-295`）
- 评审收口：`docs/reviews/code-review-2026-10-18.md` **只在分支** `claude/eager-johnson-blcmyx`
  上（main 没有这个文件），该分支同时**删除了本文档**，合并时需确认保留哪份；main 上的
  `docs/reviews/code-review-2026-10-19.md`（已加「第三轮复核追加」校正 #2 / #8）。两份的
  「修复进度 / 净新增」表是否清零
- Notion 侧下次复核的入口：`https://www.notion.com/releases`（**要查完整时间轴，别只看
  最近几个月**——这是 §1 的教训）、`https://www.notion.com/help/use-pages-offline`、
  `https://developers.notion.com/docs/mcp`
