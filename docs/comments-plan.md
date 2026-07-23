# 评论/建议锚点设计

> 2026-07-23。调研 Yjs/yrs relative-position、AFFiNE/BlockSuite、ProseMirror、Google Docs、
> Notion、Confluence、Automerge、Lexical 后定稿。答复 roadmap「评论/建议未建 … marks 模型本
> 为 range 锚点预留」。**这是设计与计划,落地留新会话。**

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
3. **任一端解成 None** → 该 range 被删:`status='orphaned'`,不画高亮,面板里带 `quote` 显示该 thread
   (比 BlockSuite 硬删更软、保住讨论;可选按 `quote` 模糊重锚)。
4. resolve = `status='resolved'`,**不删锚点**,"显示已解决"能重新高亮。

Rust 锚点辅助放 `doc.rs` 旁(它已管 `TextRef` 访问):建时 `sticky_for_range(doc,block,start,end)->(bytes,bytes)`,
读时 `resolve_range(doc,thread)->Option<LiveRange>`。**Dart 端只收解好的 `(block,startOffset,endOffset)`
去画,永远不碰 CRDT 内部。**

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
- **Phase 2**:orphan 模糊重锚、评论面板 UX、@提及/通知(需通知底座)。
- **建议**:独立立项(见上)。

## 参考
- Yjs RelativePosition:https://docs.yjs.dev/api/relative-positions ・ yrs StickyIndex:https://docs.rs/yrs/latest/yrs/struct.StickyIndex.html
- BlockSuite 评论(side-store + RelativePosition + quote + orphan):`toeverything/AFFiNE` `blocksuite/playground/apps/comment/comment-manager.ts`(注:playground 参考实现,非确证 = 线上 AFFiNE-Cloud)
- ProseMirror 评论作者指引:https://discuss.prosemirror.net/t/how-to-track-comment-positions/4500
- Google Docs suggestions:https://developers.google.com/docs/api/how-tos/suggestions ・ prosemirror-suggest-changes:https://github.com/handlewithcarecollective/prosemirror-suggest-changes
- Notion comments API / Automerge Cursor / Confluence storage format(见调研)
- mica:`crates/mica-core/src/doc.rs:408`(块正文=TextRef)、`marks.rs`、`documents.rs` `permissions_for_role`
