# Notion 对标分析（2026-10-18 专项）

**一句话结论**：在「单人/小团队写作与知识沉淀」这条窄道上，Mica 的工程质量已达到 Notion 的水平（部分维度反超：本地优先、自托管、导出完整性）；差距集中在协作生态与功能广度——那是十年、几百工程师的护城河，不是 bug 债。

> **信息来源分级（2026-10-18 核实后）**：本文 Notion 侧断言分两类——✅ 已联网核实（官方 help / developers 文档原文，标注了 URL 与引文）；❓ 转述待验（训练知识，未实测，文中已标）。Mica 侧全部为仓库实测数字。

分析基线：v0.13.56（解析器分解完成后），基于代码库实测（CommonMark 计分板、MCP 工具数、roadmap 一手记录）而非印象。

## 已追平甚至反超的层（差距 0 或为负）

**数据可靠性与本地优先**——Mica 的立身之本，且做了 Notion 没做的：

- CRDT 实时协作（yrs/yjs 双引擎跨端互操作，CI 有真实跨引擎一致性门禁）+ **本地优先**：桌面端离线完整可用，on-device 镜像 + outbox 断网重放（`adv-*` 对抗测试钉死这些路径）。
  - ⚠ 2026-10-18 评审：REST/MCP 写路径 `set_blocks` 全量重建会吞掉并发/离线编辑并杀死评论锚点（A3，已在 `claude/eager-johnson-blcmyx` 修为按块 id 差量）；2026-10-19 评审：**协作模式下 Undo 用本地全域快照回滚/删除协作者的编辑（净新增，未修，需结构方案）**。「实时协作反超」在这项落地前应降半格表述。
- Notion 无专门的离线支持（2026-10-18 联网核实：help 中心无离线文档页，2026-07~10 的 releases 无任何 offline/on-device 条目，产品发布节奏全部假设在线）。Mica 的本地优先在此维度仍是架构差异；但「基本残废」是训练知识里的社区印象、非实测——**待有人真的断网用一次 Notion 桌面端再定措辞**。
- 服务器自托管 + 一键导出 Markdown/HTML/ZIP（增量导出、资产重写）。Notion 导出（官方 help 实查 2026-10-18）：数据库只能导出**当前视图**为 CSV（"Exporting all views at once isn't supported"），关系/rollup 不在保留清单，官方明说**不能靠导出重建工作区**（"You can't instantly recreate your workspace by reuploading your exported content"），大工作区最长 30 小时、链接 7 天过期，Windows 260 字符路径会炸 ZIP。Mica 的导出是往返设计（round-trip 有测试钉住）——这条差距属实，但表述以官方原文为准，不用「出了名的半残」这类转述。
- 生产级运维：pg_dump 还原点、备份 sidecar、带彩排的原子部署——940 行的 roadmap-done 记录了这些是怎么一个个踩出来的。

**Markdown/数据主权**：自研引擎 CommonMark 0.31.2 计分 **641/641（100%，11 条 GFM 方言豁免除外）**，比 Notion 的私有块模型开放得多。§8 分解后金样本 741 输入字节级守卫。

**AI 集成面**：Mica 34 个 MCP 工具 + 桌面端 AI 改写。**勘误（2026-10-18 联网核实）**：Notion 已有官方 remote MCP server（developers.notion.com/docs/mcp——OAuth 授权、内容按成员权限限定、企业版可管理客户端连接），工具面更宽：**53 个工具**，覆盖搜索/读写/评论全 CRUD、文件上传下载、skills 上传下载、agent 会话编排（spawn/wait/stop session）。此前「Notion 没有 MCP」是过期信息，更正。Mica 在此维度的真实位置：**存在且自托管**（Notion 的 MCP 托管在它的云上，自托管场景用不上）；工具数与编排能力落后。追平路径清晰：CRUD/评论面 Mica 大半已有对应 handler，差距是文件与 agent-session 族。

## 实质性差距层（按代价从小到大）

| 缺口 | 代价 | 现状（2026-10-18） |
|---|---|---|
| 多标签页 | 中 | roadmap 已扒完 AppFlowy（`5cf3a36`），三个结构问题有答案：①标签状态建模（TabsBloc 持 `List<PageManager>`）；②CRDT 会话生命周期——N 个标签 N 条连接还是前台连/后台只读快照，**这是最贵的决定**；③格式工具栏横向空间。是「开始施工」状态而非「要不要做」 |
| 移动端 | 大 | Flutter 壳已有，但触屏交互/IME 是独立工程量 |
| **数据库（表格/看板/日历视图）** | **极大** | Notion 的真正护城河不是笔记，是 relational database；Mica 只有简单表格。**最合理的不追判断** |
| 团队协作 | 大 | 评论已有；owner/admin/editor 角色已有；细粒度权限、评论 @、分享页协作编辑、搜索权限过滤没有。10-18/19 评审修正：评论锚点此前被每次 REST/MCP 写失效（A3，已修）；协作 undo 全域快照会吞远端编辑（未修，见文末）；owner/admin 权限有一处降级漏洞（已修 + DB 触发器兜底） |
| 生态（API/webhook/模板市场/嵌入） | 大 | 基本没有（有 MCP，方向不同） |
| 搜索 | 中 | 有 FTS+BodyIndex+CJK；无跨库 AI 问答 |

## 定性判断

差距**不是工程质量**（测试纪律、发布工程、CI 门禁严于多数创业公司），而是**产品定位重叠度**：

- Notion = 「团队工作操作系统」：数据库为核、协作生态、SaaS。
- Mica = 「个人/小团队知识库 + 本地优先 + AI 原生 + 自托管」——与 **Obsidian/Affine 的赛道更重叠**；Affine 拿了三轮融资做同样的事，工程上未必更扎实（conformance 100% + 金样本纪律在开源笔记类项目里罕见）。

**策略结论**：追赶 Notion 的功能广度是死路（10 年 + 数亿美金的产物）；窄道（本地优先 + 数据主权 + MCP/AI 原生 + 单机自托管体验）恰好是 Notion 用户中「在意数据所有权」那群人的迁移理由。

**当前最值的下一步**：多标签页——直接对齐 AppFlowy/Notion 用户的肌肉记忆，结构分析已完成（见 roadmap「编辑器与功能广度」节的三个已识别结构问题），可开工。

## 工程质量校验（2026-10-18 / 10-19 两轮独立评审更新）

「工程质量已达到甚至超过 Notion」这一主张在两天内被两轮**相互独立**的全项目代码评审检验，需要三点修正：

1. **单轮评审不足以穷举 P1**。10-18 评审（4 个按问题域切片的评审代理，见 `docs/code-review-2026-10-18.md`，修复在分支 `claude/eager-johnson-blcmyx`，761 测试全绿）与 10-19 评审（9 个按域切片 + clippy 全 workspace 零警告，见 `docs/code-review-2026-10-19.md`）重叠约 14 项，但各有一片对方没看到的区域——10-19 净新增 3 个 P1（`purge_view` 可删活页、本地搜索 snippet panic、协作 undo 吞远端编辑），10-18 独有的 A3（`set_blocks` 吞并发编辑）是两轮中最严重的一条。
2. **反复命中的形状只有两个**：「兄弟路径漏守卫」（单条 purge vs 批量 purge、api-server vs 本地 store 的同型 snippet）和「不可信输入无上界」（zip/层级深度/嵌套 label，后果都是进程级 abort 而非单请求失败）。这两条值得写进 review checklist 与 lessons，而不是逐条修。
3. **修复纪律本身是质量主张最强的论据**：10-18 轮在 24h 内闭环了绝大部分 A/B/C/D 项并全量回归；结论准确表述应是「**发现-修复-回归的速度与钉形测试纪律**达到甚至超过 Notion 所属量级的工程组织」，而非「没有 P1」。

待这项校验完全收口的标准：两轮净新增 P1 清零 + 协作 undo 有结构方案 + B4 存储域 XSS（已有方案待真机验证）落地。

## 复查锚点

下次重估时核对这几个数（都可用命令直接验）：

- CommonMark 计分：`cargo test -p mica-markdown --test commonmark_scoreboard -- --nocapture`
- MCP 工具数：`grep -c "tool(" crates/mcp-server/src/tools_*.rs` 求和（2026-10-18 = 34；2026-10-19 复核不变）
- 功能缺口消解：`docs/roadmap.md` 对应条目是否已搬去 `docs/roadmap-done.md`
- 评审收口：`docs/code-review-2026-10-18.md`（分支 `claude/eager-johnson-blcmyx`；该分支同时删除了本文档，合并时需确认保留哪份）与 `docs/code-review-2026-10-19.md`（main）的「修复进度/净新增」表是否清零
