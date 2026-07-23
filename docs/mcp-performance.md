# mica MCP 性能与 token 优化(权威)

面向"AI agent 大量实时读写 mica 文档(读/写/插入/追加)"的场景。目标:尽量逼近本地文件读写的手感与流畅,同时省 token。本文是**评估 + 落地清单 + 实现状态**;三份调研(MCP 规范、同类 MCP 真源码、token/延迟/缓存范式)的结论已并入。

## 一句话

**mica MCP 已经站在同类前列**(Markdown 线格式 + 增量写 + 精简 ack + 连接池,多数文件/文本 MCP 连这些都没有)。真正的优化空间是**读侧**(范围读 + 缓存)和**往返数**(粗粒度工具 + 就近部署),而**服务端并发/线程池是收益最低的角度**。物理地板:每操作 = 1 次到后端的往返,想破它只有"进程内常驻 CRDT"或"就近/本地部署 API"。

## 关键事实(有据,别再走弯路)

- **MCP 已删掉批量请求**(spec 2025-06-18 起)。省往返不能在协议层做,只能在**工具设计层**做——一个 `apply_edits(ops[])` 顶 N 次调用。参见 spec `2025-11-25/basic`。
- **host(Claude)串行发工具调用**,规范也不指望 pipelining → **服务端并发/线程池几乎没用**(除非走"常驻模型 + 异步刷")。
- **stdio 是对的**(最低延迟、规范首选),别换 Streamable HTTP,除非要远程/多客户端。
- **异步 Tasks 是编辑热路径的反模式**(反而加"create→poll→result"往返)。
- **resources/订阅不天然省 token**(payload 才是成本),且依赖 host 是否消费——当可选优化,别当主路径。
- **编辑格式**:id 锚定的 **search/replace**(Anthropic `str_replace` 那套)同时打败"全文重发"和"原始 udiff"(aider + Diff-XYZ benchmark;带行号标记的 udiff 反而更差)。
- **缓存一致性**用 CRDT **state-vector**:发 vector → 后端只回"自某版本以来的 delta",合并无冲突、不用因并发重拉。mica 写回包里已有的 `seq` 是它的轻量雏形。
- **同类横评**:filesystem / text-editor(tumf)/ Anthropic / Obsidian / Notion / postgres MCP。多数无状态、无缓存、无冲突检测;只有 tumf 做 hash 乐观并发,只有 Notion/mica 用 Markdown 当 token 高效底座。

## mica 现状:已经做对的(别白优化)

| 已做对 | 位置 |
|---|---|
| Markdown 当读写线格式,服务端 derive block ops(block JSON 重 5–10×) | `update_document_markdown` |
| 增量写(append/insert_at/find_replace 只发片段) | `mica_update_document` |
| 回包精简(`write_ack` 只回 id+count+seq,不吐内容) | `write_ack` |
| 连接池复用(单 `reqwest::Client`) | `MicaMcp::new` |
| outline 轻量导航(只标题+block id) | `mica_get_outline` |
| 坏 LaTeX / 大图 / 错误体 都有上限保护 | `reject_mangled_latex` / `MAX_INLINE_IMAGE_BYTES` |

## Tier 1 落地清单(按收益排序,MCP 层为主,ship 在 mica-cli 二进制里、不需动 prod)

> MCP 层改动**不需要发版**:用户把 MCP 指向新版 `mica-cli` 并重连即生效(见 CLAUDE.md「分层生效」)。

1. **范围读**(token 最大头)——`mica_read_document` 加 `offset`/`limit`(按行窗口)+ `section`(按标题段)。MCP 拉全文后切片,只把窗口回给模型。回包带「showing lines X–Y of N」。
2. **str_replace 唯一匹配纪律**——`find_replace` 先探测命中数:0 → 可读「no match」;>1 且请求 `unique` → 「found N matches, add context」让 agent 自纠;默认仍 replace-all(不破坏既有语义),但 ack 回**实际替换了几处**。
3. **富 ack**——写回包带**改动/新增的 block-id**(+ 计数 + seq),agent 写完**不用重读**去拿锚点。
4. **乐观并发**(需 api-server 改动 + 发版才生效)——读回 `seq`;写可带 `expected_seq`,过期写由服务端 409 拒绝并内联当前 seq。MCP 侧先把 `seq` 透出、写侧透传 `expected_seq`;**服务端 enforcement 待发版**。
5. **瘦身 `tools/list`**——精简冗长工具描述(每轮常驻占 token),schema 走默认 JSON Schema 2020-12(省 `$schema`)。
6. **语义锚点扩展**——`insert_at` 支持用**标题文本**当锚(MCP 经 outline 把标题解析成 block-id,再 insert_at),比行号稳(插入不失效)。

## Tier 2(重架构,才是"打字般跟手"的真答案;唯一能破 WAN 地板)

- **MCP 进程常驻 yrs 文档**:编辑本地 apply(µs 立即返回)+ CRDT delta 异步 journal-flush + append debounce ~100ms。MCP 从无状态代理变有状态:内嵌 yrs、崩溃安全(先落盘再 ack)、per-doc 串行、背压。
- **state-vector 缓存一致性**:vector 相等 → 零网络读;否则只拉 delta `apply_update`。需 API 提供 state-vector sync 端点(mica 的 yrs WS 协议已有 base64 update 底子)。
- **就近/本地部署 API**(或 embed core):把 WAN 换成 loopback/LAN——常比内嵌 CRDT 划算。

## 实现状态

- Tier 1 #1/#2/#3/#5/#6:MCP 层实现(见 `crates/mcp-server/src/lib.rs`),ship 在 mica-cli,无需发版。
- Tier 1 #4:**服务端已实现**——`outline` 返回 `seq`(`document_outline` + `DocumentOutlineResponse`),写路径在 `apply_derived_operations` 行锁内比对 `expected_seq` vs `current_seq`,不符回 409(`ApiError::Conflict`);配 DB-gated 回归 `a_stale_expected_seq_is_a_conflict_and_the_current_one_passes`。MCP 侧 outline 描述已提示回传 seq、`expected_seq` 透传。**待一次发版**才在 prod 生效(改动已提交、本地编译+clippy 通过,DB 测试在 CI 跑)。
- **`apply_edits` 批量工具**(Tier 1 之外新增):一次调用吃一组编辑(append/insert_at/find_replace/replace_all 混搭),按序应用、后者见前者效果。agent 的**串行工具调用从 N 降到 1**——研究里"MCP 批量已删,只能在工具层收往返"的落地。纯 MCP 层循环调现有 PATCH,不发版;非原子(失败停下报"N 中 M 成",可续)。`write_markdown` 抽成 mica_update_document 与它的共用 seam。
- **"workspace 当本地目录"(Phase 1,批量读)**:用户澄清 agent 独立干活、人只事后复核(无并发共编)→ 消掉了完整 Tier 2(warm-yrs 异步 flush)最贵的"和并发的人保持一致"那部分(那需要无头 WS 同步客户端 ~2 周),且异步 flush 反而会拖慢"别处观看者"的实时性(写完立即 `broadcast_applied_update` 走 WS 推,现状近实时 ~100–300ms)。真需求是"快速读结构/扫内容 + 省 token,像本地磁盘"。Phase 1 做了 **`mica_read_documents`(一次读多篇,mode=outline 省 token 扫结构 / full 取正文,逐篇错误内联)**;发现 **`mica_list_pages` 本就一次返回整棵树**,故 `read_tree` 冗余、只把描述讲清。Phase 2(warm 进程读缓存,无并发下安全)可选、未做。完整 warm-yrs 异步 flush **不做**(对 agent 是噪音 + 拖慢观看者)。
- Tier 2(原始 warm-yrs 版):不做——理由见上(agent 瓶颈是模型回合+往返数不是 per-op 延迟;并发共编不会发生)。

## 参照

- MCP spec 2025-11-25:`modelcontextprotocol.io/specification/2025-11-25`(transports / tools / resources / changelog)。
- 编辑格式:aider `edit-formats`、Diff-XYZ(arXiv 2510.12487)、Anthropic `str_replace_based_edit_tool`。
- 缓存/CRDT:Yjs document-updates / state vectors、yrs docs.rs。
- reqwest 连接池:`reqwest::Client` 即连接池,复用即省 TLS/TCP 握手。
