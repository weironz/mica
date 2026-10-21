# 全文搜索(`/search`)实现

权威是代码:`crates/app-core/src/search.rs`(BodyIndex)、`crates/app-core/src/body_arena.rs`
(存储与锁定)、`crates/api-server/src/routes/documents/search.rs`(路由)。这份文档把散在
模块注释和提交信息里的**为什么**收拢到一处,并记下运维要知道的开关与观测点。

## 一次搜索怎么走

```
GET /search?q=…
  └─ search_views
       ├─ BodyIndex::refresh      按 updated_at 游标增量拉取改动过的正文(串行化,见下)
       ├─ BodyIndex::matching_ids 在内存正文里做子串扫描 → 候选 id
       ├─ SQL                     候选 id + 标题匹配;工作区范围、is_deleted、排序、LIMIT 都在这里
       └─ BodyIndex::snippets     从内存正文切单行摘要(≤50 条)
```

- **索引只给候选,不是权威。** 工作区归属、是否删除、排序全由 SQL 决定;索引里多一条或旧一条,
  代价是一个白算的候选 id,不会答错。已彻底删除的文档留在索引里直到重启,不专门追踪。
- **拉取式刷新,不挂写路径。** `content_text` 在两个 crate 的三处事务里写;每处都去失效缓存
  就是 lessons §1 的双表示漂移。刷新只有一个入口(搜索前),陈旧度由游标的安全回退
  (`REFRESH_LAP_SECONDS = 5`,跟**数据库时钟**而非数据里的最大时间戳)封顶。
- **刷新串行化**(`refresh_lock`):旧快照不会后完成、覆盖新正文;读已加载的索引不受影响。
  启动预热也走同一把锁,预热完成前到来的搜索会等它,不会再读一遍全表。
- **匹配**:ASCII 大小写不敏感。每篇只存原文 + `has_upper` 标记:正文没有大写、或搜索词不含
  ASCII 字母(所有中文查询)时,原文上的 `contains` 就是精确答案;两者都有时才逐窗口
  `eq_ignore_ascii_case`。1 个字的查询和 10 个字一样便宜。
- **摘要来自索引**(2026-10-19):以前 SQL 为摘要再 JOIN 一遍整篇 `content_text`,冷缓存下
  每个命中都要从磁盘解 TOAST,正好落在"第一次搜新词"上。现在摘要和匹配出自同一次刷新。

## 为什么不用 Postgres 文本索引

2026-08-28 调研(详见 `search.rs` 模块注释):`pg_trgm` 对 1–2 字查询无 trigram 可用、且在
C locale 下根本不收 CJK;`pg_bigm` / PGroonga 没有 Alpine 包,得维护自定义 Postgres 镜像。
同类产品:AppFlowy Cloud 只做向量检索,AFFiNE 用自己的内存索引。约 1.5 万篇、70 MB 的语料,
线性扫描是毫秒级,且与跨多少工作区无关。

## 存储:一整块可锁定内存(body_arena)

所有正文首尾相接放进**一个** `String`,`HashMap<Uuid, Slot{start,len,has_upper}>` 定位。

- 替换的正文留下死字节;只在需要扩容时整理:分配 `live + max(10%, 1 MiB)`,只拷贝存活正文。
  死字节因此不超过扩容余量,保留内存不随编辑次数增长(单测断言)。
- **整块 `mlock`**,使内核不能把它换出。在内存吃紧、长期在 swap 的节点上,"常驻内存的索引"
  并不常驻:闲置一两分钟页就被换出,下一次搜索按 4 KB 一页换回来(见下方历史)。
- **锁在填满之后**:扩容只分配不锁,`insert_all` 逐行拷贝、每行拷完即释放源串,整批完成后
  才锁。先锁后填会让峰值变成两份语料(0.13.62 回滚的原因之一)。
- **锁失败不致命**:没有 `CAP_IPC_LOCK` 或超出 memlock 限额时只告警一次,状态记为
  `not locked (原因)`,索引照常工作(可被换出)。不重试 —— 原因在运行期不会变。

## 运维

| 项 | 说明 |
| --- | --- |
| `MICA_SEARCH_INDEX_MLOCK` | 默认开;只有 `off`/`0`/`false`/`no` 关。空值、拼错都保持开(compose 的 `${VAR:-}` 会传空串)。已在两份 compose 的 `environment:` 允许清单里 |
| `cap_add: [IPC_LOCK]` | 两份 compose 的 api 服务都有;去掉它等于关锁(会告警) |
| 内存代价 | 永久占用约等于正文总量 + 10%(生产 2026-10-21:15190 篇,正文 73 MB,预留 80 MB) |
| 启动日志 | `body-text search index warmed documents=… body_mb=… reserved_mb=… memory=locked\|off\|not locked (…)` |
| 慢搜索日志 | 总耗时 ≥ 500 ms 记 `slow search`,带 refresh / scan / sql / 总耗时与候选数、行数 —— 先看慢在哪一段 |
| 预热 | bind 之后的后台任务(`main.rs`),不在启动关键路径上(lessons §13) |

查看生产现状:手动触发 `Prod diagnostics` workflow,日志里有内存 / swap / vmstat、
api 容器内存、预热日志与 `slow search` 记录。

## 历史(每一步的根因)

| 版本 | 现象 | 根因 | 改动 |
| --- | --- | --- | --- |
| FTS M1(迁移 0012) | 单工作区 81 ms,跨工作区乘以工作区数 | Postgres 对 TOAST 正文做 `ILIKE` | 维护 `content_text` 列 |
| FTS M2 | — | 同上 | 进程内 BodyIndex,增量刷新 |
| 0.13.57 | 第一次搜新词慢 | 摘要 JOIN 整篇正文,冷读 TOAST | 摘要改从索引取;`slow search` 分段日志。定位过程见 [`performance/search-latency-2026-10-16.md`](../performance/search-latency-2026-10-16.md) |
| 0.13.60 | 0.13.59 上线后 4–50 s | 摘要改造给含大写的正文多存一份小写副本,索引近乎翻倍 → 节点进 swap | 只存原文 + `has_upper` |
| 0.13.62 | 闲置后首次搜索 6–37 s,紧接着再搜 < 1 s | 索引仍允许被换出;减半没改变形状 | body_arena 整块存储 + `mlock` + 开关 |
| 0.13.63 | 0.13.62 部署 api unhealthy → 回滚 | 预热在 bind 之前,且先锁后填使峰值两份语料 | 预热移到后台;填满后再锁。上线后闲置再搜即时返回 |

教训见 [`dev/lessons.md`](../dev/lessons.md) §13(启动关键路径)、§14(内存驻留)。

## 刻意没做

- **给 `updated_at` 建索引**:刷新探针现在是 heap 顺序扫描(约 1.5 万行,十毫秒级)。建索引会让
  每次 push 失去 HOT 更新,写放大不划算。
- **前端取消过期请求、跨工作区点击先 bootstrap 无关页面**:10-16 定位时识别的独立候选,未做。
- **禁用 swap 代替 mlock**:整机生效,把其他服务可恢复的分页变成 OOM;mlock 只钉住搜索真正需要的
  那 80 MB。见 [`ops/host-pressure-2026-10-16.md`](../ops/host-pressure-2026-10-16.md)。
