# 容灾方案（DR）— 现状实测、恢复路径、以及为什么不做双活

> 2026-07-30 首版。全部数字来自当天在生产节点 `mica.cloudcele.com` 的实测，
> 每条都标了取证方式，好让下一个人能复核而不是继承信念。
>
> 本文只管**灾难恢复**（数据没了怎么找回来）。日常备份的**机制**说明在
> `docs/backup.md`，恢复 runbook 也在那里；本文管的是**策略**：备了什么、丢多少、
> 多久能起来、哪条路没人走过。

## 0. 一句话结论

**内容（正文 + 图片）每天异地一份，数据库异地一份都没有。** 节点盘一坏，账号、
成员关系、评论、CRDT 编辑历史、分享链接**永久消失**；能救回来的只有 22 个工作区的
Markdown 和图片，形态是「新实例 + 重新导入 + 所有人重新注册」。

**双活不做**（第 6 节存档理由）。要提可用性，正确的下一档是 warm standby。

## 1. 今天实测到的事实

| 事实 | 取证 |
| --- | --- |
| 每日备份**跳过了 pg_dump** | 备份容器日志原文：`WARN: MICA_BACKUP_PGURL unset — skipping pg_dump (content-only backup, no DB disaster recovery)` |
| 异地仓库里**从来没有过** DB 快照 | `rustic snapshots --filter-label _pgdump` → `total: 0 snapshot(s)`（全仓 170 个快照） |
| 节点 `.env` 缺 `MICA_BACKUP_PGURL` | `grep -E '^(MICA_BACKUP|OSS_|BACKUP_)' /data/mica/.env` —— 有 TOKEN / PASSWORD / OSS_* / BACKUP_HOUR，**无 PGURL** |
| 内容备份是好的 | 同一次运行 `snapshotted 22 workspace(s)`，导出含 **Markdown + 图片** |
| DB 的唯一副本在**节点自己的盘上** | `/data/mica/pre-*.sql.gz` 共 5 个手工还原点，最新 `pre-0.13.5-20260730-164652.sql.gz` |
| 异地仓库与节点**同一个云账号** | rustic → `opendal:s3` → 阿里云 OSS；ECS 也在阿里云 |
| 仓库自身完整性 OK | `rustic check` 170 snapshot 全过（2026-07-30） |
| 恢复演练走过一次 | `just restore-drill`：错误 0、`_sqlx_migrations`=15、**3331 可读页**、32 FK / 19 PK |

### 1.1 为什么这个洞能活这么久

**死人开关对它是绿的。** `mica-backup-loop.sh` 在运行**成功**时 ping healthchecks.io ——
而「跳过 pg_dump」是一次**成功的运行**：脚本按设计降级、打一条 WARN、继续把内容备完、
正常退出。监控能抓住「备份没跑」，抓不住「备份少备了一半」。没人读备份日志，WARN 就等于没写。

同一形状在 `lessons.md` 里反复出现：**降级路径必须让人看见，否则它就是静默失败**。

## 2. 备了什么 / 没备什么

内容导出（`mica-cli export`，每日 → OSS）覆盖的：

- 22 个工作区的页面正文（Markdown）
- 页面里的图片（导出即含，**不是**只有 DB 里的引用）

**不覆盖**的（这些只存在于 Postgres，而 Postgres 没有异地副本）：

- 账号与口令 hash、邮箱验证状态
- workspace 成员关系与角色（谁能看谁的东西）
- **CRDT 编辑历史**（`document_yrs_base` 的 state）—— 导出是当前正文的投影，不含历史
- 版本检查点（`document_yrs_versions`，含用户命名的检查点）
- **评论**（`comment_threads` / `comments`，锚点存在独立表，正文里一个字都没有）
- 分享 token（公开链接会全部失效）
- refresh tokens（所有人被登出，可接受）
- 回收站内容（导出过滤 `is_deleted=false`）

一句话：**导出救得回「文字和图片」，救不回「这是谁的、谁改过、谁评论过」。**

## 3. RPO / RTO

分两套算，因为它们的备份状况天差地别。

| | 内容（正文+图片） | 数据库（账号/关系/历史/评论） |
| --- | --- | --- |
| **RPO**（丢多少） | ≤ 24h（每日 `BACKUP_HOUR` 一次） | **∞ —— 无异地副本** |
| 唯一副本位置 | 阿里云 OSS（异地，同账号） | 节点本地盘 `/data/mica/pre-*.sql.gz` |
| 那份副本的新鲜度 | 每天 | 只在**发版前手工**落，所以是「上次发版」而非「上次编辑」 |
| **RTO**（多久起来） | 未实测（见下） | 未实测端到端 |

**RTO 诚实说明**：`just restore-drill` 实测过的是「把一份 `pg_dump` 恢复进一次性库并断言可读」
这一段，**不是**端到端的节点重建。完整的「换台机器把服务起回来」这条路**没有人走过**，
所以任何 RTO 数字都会是编的。要给出真数字，得做一次真演练（第 7 节）。

## 4. 威胁模型 → 恢复路径

| 威胁 | 今天会发生什么 | 恢复路径 | 状态 |
| --- | --- | --- | --- |
| **误删页面** | 进回收站，用户自己能恢复 | 回收站三个出口（恢复 / 永久删除 / 清空） | ✅ 够用 |
| **迁移写坏 schema** | api 起不来 | `backup.md` 回滚 runbook：停 api → drop/create → `zcat pre-*.sql.gz \| psql` → 钉旧 tag | ✅ 有本地还原点，前提是发版前落了 |
| **单表 / 单文档写坏** | 局部数据错 | 同上，或从还原点里只捞目标表 | ✅ |
| **节点盘损坏 / ECS 没了** | **DB 全丢**（还原点与 DB 同盘） | 只能：新实例 → 从 OSS 拉内容导出 → 重新导入 → 所有人重新注册 | 🔴 **账号 / 历史 / 评论 / 分享链接不可恢复** |
| **云账号级损失**（欠费、封号、AK 泄露被清） | 节点与 OSS 一起没 | 无 | 🔴 **无任何恢复路径** |
| **勒索软件 / 误删仓库** | rustic 仓库被删或加密 | 仓库不可变性未配；密码只在节点 `.env` | 🟠 见 5.2 / 5.3 |
| **`RUSTIC_PASSWORD` 丢失** | 备份在，但**永远解不开** | 无 | 🟠 需确认它存在节点之外 |

## 5. 立即行动项（按 ROI 排）

### 5.1 🔴 #0 —— 接上 `MICA_BACKUP_PGURL`（把最大的洞堵上）

机制早就写好了，只是没人设这个变量。节点 `.env` 加一行，重启 backup 容器：

```
MICA_BACKUP_PGURL=postgres://mica:<POSTGRES_PASSWORD>@postgres:5432/mica
```

backup 容器与 postgres 共享默认网络，所以主机名就是 `postgres`。加完后
`docker compose up -d --no-deps backup`，再手动跑一次，确认日志出现 `snapshot pg_dump → label=_pgdump`
且 `rustic snapshots --filter-label _pgdump` 不再是 0。
**这一步不需要发版**（改的是节点 `.env`，与 compose 指纹无关），也不改任何代码。

**配套（更小但该做）**：让「跳过 pg_dump」不再算一次成功的运行 —— DB 被跳过时死人开关
应该 ping `/fail`，或至少走一条不同的信号。否则同一个洞会以另一个变量名再来一次。

### 5.2 🟠 #1 —— 把 `RUSTIC_PASSWORD` 挪出节点

脚本注释自己写着 `SAVE OFF-HOST (lose it, lose the repo)`。**零代码**：确认它在密码管理器里。
节点没了而密码只在节点上，等于备份存在但解不开 —— 最便宜也最容易被跳过的一条。

### 5.3 🟠 #2 —— 第二份仓库，跨账号或跨提供商

现在 rustic 仓库与 ECS 在同一个阿里云账号下，账号级事件同时带走两边。
`rustic copy` 就是为此存在的：加一个第二 target，另一个账号（更好是另一个提供商）。
顺带缓解勒索 / 误删：第二份可以只给 append 权限。

### 5.4 🟡 #3 —— WAL 归档，把 DB 的 RPO 压到分钟级

**先做完 5.1 再考虑这条** —— 5.1 是「从没有到每天一份」，这条是「从每天一份到分钟级」，
前者的收益大一个数量级。

实现上有个要如实说的皱褶：`postgres:16-alpine` 里没有能往 OSS 推 WAL 的工具，
`archive_command` 需要 **wal-g 或 pgBackRest**（都支持 S3 兼容后端）。这是引依赖，
但按项目原则恰好是对的：in-house 该留给**核心数据面**（CRDT / 文档模型 / 同步），
WAL 归档是平台粘合层，「粘合层自研要背三套平台原生层，用成熟包反而对」。

## 6. 双活（active-active）—— 存档结论：不做

不是「暂时不做」，是架构上不该做。四条：

1. **WS 层每文档一个内存 broadcast channel。** 两个节点各持一半连接，同一篇文档的两个
   客户端连到不同节点就**收不到对方的更新**。要修必须上跨节点 pub/sub = Redis，而 Redis
   在 roadmap 里还躺在「可选 / later 基建」没引进来。这不是配置问题，是同步模型的前提被打破。
2. **Postgres 多主本身是重活**（BDR / pgEdge，商业或重型）。而 mica 的写路径是
   `push_update` **全档 decode + encode + upsert**，两个主同时改同一篇的 base 行正好是最坏情况。
3. **新增的运维面**：负载均衡 + 会话粘滞 + 双份证书 + 第二个节点。当前连 Traefik 配置
   都还没纳管（roadmap「生产运维」小节）。
4. **为谁做？** 单管理员自托管产品，没有 SLA 承诺对象。这正是「不要过度设计」要挡的形状。

**双活买的是「零 RTO + 双倍写入吞吐」，这两样现在都不需要，而它要的前提（跨节点 pub/sub）
比它解决的问题更贵。**

### 6.1 可用性的正确下一档：warm standby

真要缩 RTO，走 Postgres 流复制只读副本 +「promote + 切 DNS」的 runbook：

- RTO 从「手工恢复几小时」降到「几分钟」
- **不引 Redis、不改一行应用代码**（应用始终只跟一个主说话）
- 代价：多一台机器的成本与运维面

但**它不替代 5.1**：副本跟着主一起坏（误删、逻辑损坏、勒索都会复制过去），
所以「异地备份」和「热备」是两件事，先有前者。

## 7. 怎么复核这份文档还成立

最容易腐烂的是第 1 节那些数字。一条命令重新取证：

```bash
ssh root@mica.cloudcele.com 'docker logs mica-backup-1 2>&1 | grep -E "pg_dump|snapshotted" | tail -5; docker exec mica-backup-1 rustic snapshots --filter-label _pgdump 2>&1 | tail -3'
```

- 若仍见 `WARN: MICA_BACKUP_PGURL unset` → 5.1 还没做，第 0 节结论原样成立
- 若 `_pgdump` 谱系有快照且新鲜 → 更新第 3 节 DB 那一列，把 RPO 从 ∞ 改成实际节奏

还有一件只能靠真演练回答的：**第 3 节的 RTO 至今是空的。** 要填上它，得在一台干净机器上
从 OSS 拉备份、把服务起到 `/api/ready` 绿，然后记下耗时。这条路没人走过，所以现在
「多久能恢复」这个问题**没有答案**，只有猜测。

⚠️ **SSH 别猛敲**：连太密会被云上游限流（一个会话十几次后清一色 `Connection closed`，
等 ~7 分钟自然恢复）。一次 ssh 里用 heredoc 跑完所有检查。
