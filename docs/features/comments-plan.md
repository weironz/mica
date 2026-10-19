# 评论/建议锚点设计

> 2026-07-23。调研 Yjs/yrs relative-position、AFFiNE/BlockSuite、ProseMirror、Google Docs、
> Notion、Confluence、Automerge、Lexical 后定稿。答复 roadmap「评论/建议未建 … marks 模型本
> 为 range 锚点预留」。**这是设计与计划,落地留新会话。**

## 落地进度(2026-07-26)——服务端 + 渲染 + API 已闭环,剩面板 UI

| 层 | 状态 | 出处 |
|---|---|---|
| 数据表(`comment_threads` / `comments`) | ✅ | migration `0014_comments.sql` |
| yrs sticky 锚点原语(`sticky_for_range` / `resolve_range` / `is_empty`) | ✅ 8 单测 | `mica-core/doc.rs`、`tests/comment_anchor.rs`(2672201) |
| store 层 CRUD + `load_doc` | ✅ | `app-core/comments.rs`(354f946) |
| 5 个端点(建/列/回复/resolve/删,gated on `commenter`) | ✅ | `api-server/routes/comments.rs`(354f946) |
| **Postgres 集成测试(8 项,CI 真跑)** | ✅ **CI 绿** | `app-core/tests/comments_pg.rs`(736639c) |
| 客户端 API 层(DTO + 5 方法) | ✅ 6 单测 | `api/models.dart`、`api/client.dart`(af03743) |
| **渲染期高亮(纯 paint,不影响布局)** | ✅ 5 widget 测 | `render.dart` `commentHighlights`(08f221d) |
| **评论面板 + 选区→「添加评论」入口 + main.dart 接线** | ✅ 9 widget 测(**观感待真机**) | `ui/comment_panel.dart`、`editor.dart` 右键项、`main.dart`(f3bf181) |
| **orphan 模糊重锚(Phase 2 ①)** | ✅ 13 单测 + 5 单测 + 3 PG 集成测 | `mica-core/quote_match.rs`、`app-core/comments.rs` `anchor_state`/`rematch`/`reanchor` |
| 建议(suggest mode) | ⏸️ 有意不做(独立立项,见文末) | — |

**两个实测修正了本文档的原设计假设**(照原文写会埋 bug):
1. **orphan 不能只判 `None`**。原文写"任一端解成 None → orphaned";实测 **yrs 保留 tombstone**,
   删掉锚定文字后两端**仍能解析**、只是范围**塌缩成零长**。故加 `CommentRange::is_empty()`,
   **"解不出"与"塌缩"必须同等视为 orphan**——只信 None 会漏掉最常见的删除情形。
2. **表名用复数**(`comment_threads`/`comments`)贴仓库惯例,原文写的是单数。

### Phase 1 已闭环(f3bf181)——四步都落地
1. **接线**:`onReady` 里 `listComments` → `_commentThreads` → 过滤 `isHighlightable` →
   `CommentHighlight` → `MicaEditor`/`DocumentSurface`。**任何变更后重新拉取**(服务端是
   锚点与 orphan 的唯一真相源);加载失败只是没有评论,**文档照常打开**。
2. **入口**:右键菜单选区内加「添加评论」,offset 用 **UTF-16**(与渲染器/服务端同单位),
   跨块直接支持;**只由 `onAddComment != null` 决定,不看 `canEdit`**(commenter 能评论
   不能改正文)。quote 截 300 字。
3. **面板**(`ui/comment_panel.dart`):列 thread、回复、resolve/重新打开、删除;
   orphaned **照样可读**(删除线 + 说明),未解决排前;面板**从不算位置**。
4. 锚不上 → 按 400 提示「这段选区已不在文档中」,不抛原始错误。

### ✅ 真机确认完毕(2026-08-03)
五条全部跑过,两条结论与原文相反 —— 详见各条:
- [x] **面板形态 —— 已改成右侧常驻栏**(2026-08-03,`d345b6a`)。结论不是"380 太窄",是
  **模态错了**:Dialog 盖住了评论所指的正文,而对着正文读评论就是这件事的全部。扒了 AFFiNE
  (`components/comment/sidebar/index.tsx` + `watchForPendingComments` 自动展开),照它做了栏 +
  本地用户新建时 auto-open。AppFlowy 不是参照系 —— 它编辑器里没有评论 UI。
- [x] 图标 + 未解决计数摆位 **合适**(实测:加评论后计数变 1);按钮已随侧栏改造变成
  **toggle**,加了 active 态 —— 看不出自己开着的开关只是个"再点一次试试"的按钮。
- [x] 琥珀高亮浓度 **可以** —— 淡而可辨,且**与选区不会混淆**(选区灰、评论琥珀)。
- [x] 添加评论链路 **顺** —— 右键菜单里「添加评论」排在「复制」之后,输入即出高亮。
- [x] **跨块选区的高亮 —— 2026-08-03 真机实测,结论与原文相反**:高亮**横跨所有块都画了**
  (标题里被选中的那半 + 整段正文,都上了色),不是"只画起始块"。跨块评论也能正常创建,
  引用完整收录三个块的文字。
  **过程中我先得出过一个错误结论**:第一次测时右键菜单只剩「粘贴」,我据此说"跨块选区
  做不出来、跨块评论根本创建不了"。**真实原因是右键点落在了行间空白**,而
  「复制/添加评论/剪切」三项统一门控在 `insideSelection`(`editor.dart`)——点在选区外
  它们就整体消失。右键点**在被选中的字上**,五项齐全。
  教训:菜单少了几项时,先问"是不是我点在了选区外",再问"是不是功能没做"。

## 决策速览
- **评论锚点 = yrs sticky index(`StickyIndex` + `Assoc`),存在独立的 Postgres 评论表**,
  文档正文/markdown **一个字不动** → round-trip 红线保住。marks 模型**只**复用于渲染期的
  "临时高亮",**绝不**把评论存成文档里的 mark。
- **建议(suggest mode)是相反的问题**:建议本身就是"对正文的提议改动",同类一律存**在文档内**
  (insert/delete 特殊 mark,接受/拒绝时整体解析)。**第一期不做**,且**不与评论共用存储**。

## 为什么是 side-store + sticky index(证据支撑)

### 锚点表示:CRDT relative/sticky position 是行业标准(硬证据)
- **Yjs `RelativePosition`**:官方文档明说它"attaches positions to specific elements rather than
  indices",`assoc` 决定绑到 index 前/后的字符,解析失败或目标被删则**返回 null**——并明确这
  就是给**评论和光标**用的机制。
- **yrs `StickyIndex`/`Assoc`**(mica 用的 Rust 端,同一东西):`text.sticky_index(txn,i,Assoc::After)`
  建、`sticky.get_offset(txn)` 解回当前 offset;文档明说前面插入内容会让它自动位移("stays
  before the o")。
- **为什么不用数字 offset**:offset 只在某一版本正确,任何并发的前置插/删都会错位——relative
  position 存在的全部理由。

### 存哪:side-store(强证据,且是关键分叉)
| 产品 | 评论存法 | round-trip 后果 |
|---|---|---|
| **AFFiNE/BlockSuite** | 独立 `Y.Map('comments')`,锚点 `{blockId, Y.RelativePosition}`×2 + `quote` 快照 | 正文不受评论影响(**真源码**,playground) |
| **Notion** | 评论是独立对象、`parent.block_id`,单独 API 取 | 块内容干净 |
| **ProseMirror(Outline 等)** | 作者 Marijn 明确:**别建成文档节点/mark**,存"range 引用、文档外单独追踪、每次编辑 map forward" | 导出不带评论锚 |
| **Confluence** | 唯一把标记塞进正文的(`ac:inline-comment-marker`),因它的权威格式是自带该元素的 XHTML | 编辑摩擦已知 |

**负面结论**:**没有一个把评论标记塞进"markdown 权威文档"的产品**,也**没有谁"接受评论导出丢失"
当作有意选择**——大家都靠**从不把评论放进可导出正文**来绕开。mica 跟这条走,round-trip 不变量
零改动。

### 建议 = 文档内 mark(硬证据,故与评论分开)
Google Docs(正文 run 带 `suggestedInsertionIds`/`suggestedDeletionIds` + Accept/Reject API)、
`prosemirror-suggest-changes`(`insertion`/`deletion`/`modification` 三种 mark,接受=去 insert
mark 留内容/删 delete 内容)——建议一律**存在文档里**,接受/拒绝是一次 mark 解析。这是唯一
不得不碰正文的地方,所以另立项、不与评论(side-store、非正文)混设计。

## 落地不是空谈:mica 的原语已就位
1. **每块正文本就是 yrs `TextRef`**(`crates/mica-core/src/doc.rs:408`,`write_block`),marks 是它上的
   format op(`marks.rs`)。所以 `TextRef::sticky_index` / `StickyIndex::get_offset` **直接可用**——
   和 BlockSuite `createRelativePositionFromTypeIndex(blockText.yText, i)` 一模一样,**零新 CRDT 机器**。
2. **`commenter` 角色已存在**(`documents.rs` `permissions_for_role`),鉴权只是给新端点加门,不用发明角色。

## 数据模型(side-store,Postgres,镜像现有 document_* 表)
```
comment_thread
  id                   uuid pk
  document_id          uuid fk -> documents
  anchor_start_block   text          -- block id
  anchor_start_sticky  bytea         -- yrs StickyIndex::encode_v1
  anchor_end_block     text
  anchor_end_sticky    bytea
  quote                text          -- 建 thread 时锚定文本快照(orphan 兜底 + 列表预览)
  status               text          -- 'open' | 'resolved' | 'orphaned'
  created_by           uuid
  created_at           timestamptz
  resolved_by          uuid null
  resolved_at          timestamptz null

comment
  id                   uuid pk
  thread_id            uuid fk -> comment_thread
  author_id            uuid
  body                 text          -- 评论正文(markdown)
  created_at           timestamptz
  edited_at            timestamptz null
```
要点:
- **存 `StickyIndex` 的 encode_v1 字节,不存 offset**——并发存活免费。
- **存 `quote`**(BlockSuite 做法):orphan 兜底 + 不解析锚点就能给列表预览。
- **两端各 `(block_id, sticky)`** 支持跨块选区;`Assoc`:start 用 `After`、end 用 `Before`,高亮
  "紧贴"选中文本,紧邻外的插入不被吞进来。
- 放 Postgres(非 yrs 文档内):可查询、**保证永不进导出路径**(BlockSuite 放 Y.Map 是因它整库
  一个 Yjs doc;mica 有关系型侧,一张表摩擦更小且天然隔离 markdown)。

## 锚点 → 渲染期高亮
1. 文档加载 / 应用更新后,每个 open thread:解码两个 `StickyIndex`,读 txn 里对各自块 `TextRef`
   调 `get_offset(txn)`。
2. **两端都解出** → 得 `(start_block,start_offset)`/`(end_block,end_offset)`(当前 UTF-16)。交给编辑器
   当**临时装饰**——复用 marks-over-plaintext 高亮渲染路(每帧算的合成"comment" mark,**绝不**写进块
   `data`)。这就是你设想的"marks 只复用于编辑器内高亮"。
3. **任一端解成 None(或塌缩成零长)** → 先按 `quote` 尝试重锚(下节);重锚不成才是 orphan:
   `status='orphaned'`,不画高亮,面板里带 `quote` 显示该 thread(比 BlockSuite 硬删更软、保住讨论)。
4. resolve = `status='resolved'`,**不删锚点**,"显示已解决"能重新高亮。

Rust 锚点辅助放 `doc.rs` 旁(它已管 `TextRef` 访问):建时 `sticky_for_range(doc,block,start,end)->(bytes,bytes)`,
读时 `resolve_range(doc,thread)->Option<LiveRange>`。**Dart 端只收解好的 `(block,startOffset,endOffset)`
去画,永远不碰 CRDT 内部。**

## orphan 模糊重锚(Phase 2 ①,2026-08-06 落地)

**先纠正一个前提**:原文把 orphan 归因为"锚定文字被删"。真跑下来**高发因由是另一个** ——
`MicaDoc::set_blocks` 会把每个块条目删掉重建(新的 `TextRef`),而**任何 REST/MCP 写入、任何版本恢复**
都走它。于是**正文一个字没变,整篇文档的锚点全部失效**。文字真被删是另一回事,那时确实该 orphan。
`quote` 正是用来区分这两者的:去当前正文里找它,找到就重锚,找不到就继续 orphan。

- **判定放在 Rust**(`crates/mica-core/src/quote_match.rs`),因为解析锚点本来就在这层
  (`resolve_range`);Dart 只收算好的 `(block, startOffset, endOffset)`,不知道 sticky index 是什么。
  匹配对文档**只读**,重锚只改 `comment_threads` 的 sticky 字节 —— 正文永远不动。
- **两级,且一律偏向拒绝**(错锚比没锚更糟:orphan 至少看得出是 orphan,错锚是把讨论悄悄挂到别人身上):
  ① **精确层**(忽略大小写、忽略空白种类);同一段文字出现多处时,只信**原始块**里那处,
  否则必须全文唯一才采纳;② **模糊层**(Sellers 近似匹配,编辑距离 ≤ 25%),要求 quote ≥ 8 字符,
  且**最佳位置唯一** —— 两处并列一样好就判定歧义、维持 orphan。
- **块边界**:索引把各块文本用一个边界字符连起来,与客户端用 `\n` 拼跨块选区的方式对齐,
  所以跨块 quote 同样能重锚;`…`(客户端 300 字截断标记)在匹配前剥掉,代价是超长选区重锚后会**变短**。
- **状态**:重锚成功且原状态是 `orphaned` → 回到 `open`;`resolved` **保持 resolved**(重锚是记账,
  不是重开讨论)。写库是 best-effort(下一次列举会重新推导,读请求不因它失败)。
- **成本**:quote 索引在列表端点里**按需构建**(全部锚点都还活着就一次都不建);模糊层有
  DP 单元上限(约 30k 字 × 300 字 quote),超了只跑精确层。

## 端点(gated on commenter 角色)
`POST .../documents/{doc}/comments`(建 thread+首条,body 里带锚 range)、`GET .../comments`(列 + 解析
锚点)、`POST .../comments/{thread}/reply`、`POST .../comments/{thread}/resolve`、`DELETE`。授权走
`permissions_for_role`。

## 建议(suggest mode)——明确排出第一期
按调研:建议 = 文档内 insert/delete overlay,接受/拒绝解析。对 md 权威 mica,干净解法(仿 Google Docs
语义):**建议永不进可导出 markdown;导出时对 pending 建议"全接受"或"全拒绝"(择一策略),pending overlay
存成自己的 `(anchor, op, author)` 侧结构、编辑器当视觉 overlay 叠加、接受前不改基础正文**。这是更大的独立
决策,第一期不做;记住**评论(side-store 非正文)与建议(正文内 overlay)是两个问题,别共用存储设计**。

## 分期
- **Phase 1(评论 MVP)**:上表两张表 + Rust `sticky_for_range`/`resolve_range` + 端点 + 列表/建/回复/resolve
  + 高亮渲染 + orphan 处理。round-trip 零改动(正文不动)。
- **Phase 2**:~~orphan 模糊重锚~~ ✅(2026-08-06,见上节)、评论面板 UX、@提及/通知(需通知底座)。
- **建议**:独立立项(见上)。

## 参考
- Yjs RelativePosition:https://docs.yjs.dev/api/relative-positions ・ yrs StickyIndex:https://docs.rs/yrs/latest/yrs/struct.StickyIndex.html
- BlockSuite 评论(side-store + RelativePosition + quote + orphan):`toeverything/AFFiNE` `blocksuite/playground/apps/comment/comment-manager.ts`(注:playground 参考实现,非确证 = 线上 AFFiNE-Cloud)
- ProseMirror 评论作者指引:https://discuss.prosemirror.net/t/how-to-track-comment-positions/4500
- Google Docs suggestions:https://developers.google.com/docs/api/how-tos/suggestions ・ prosemirror-suggest-changes:https://github.com/handlewithcarecollective/prosemirror-suggest-changes
- Notion comments API / Automerge Cursor / Confluence storage format(见调研)
- mica:`crates/mica-core/src/doc.rs:408`(块正文=TextRef)、`marks.rs`、`documents.rs` `permissions_for_role`
