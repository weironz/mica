# 容灾方案（DR）— 现状实测、恢复路径、以及为什么不做双活

> 2026-07-30 首版。全部数字来自当天在生产节点 `mica.cloudcele.com` 的实测，
> 每条都标了取证方式，好让下一个人能复核而不是继承信念。
>
> 本文只管**灾难恢复**（数据没了怎么找回来）。日常备份的**机制**说明在
> `docs/backup.md`，恢复 runbook 也在那里；本文管的是**策略**：备了什么、丢多少、
> 多久能起来、哪条路没人走过。

## 0. 一句话结论

**（2026-07-30 当天已改变）** 现在内容与数据库**都**每天异地一份。本文最初写下时
数据库异地一份都没有 —— 那个洞当天就补上了（5.0），过程与取证留在下面，因为它是
本文存在的理由，也是「机制写好了但没人接线」这类失败的标本。

**当前状态**：
- ✅ **数据库**：每日 `pg_dump` → rustic `label=_pgdump` → 阿里云 OSS
- ✅ **内容（正文+图片）**：每日导出 → 每工作区一个 label → 同一仓库
- 🟡 **对象字节**：仍只靠导出间接覆盖（孤儿 blob / 回收站独有图片不在），计划走
  rclone 直传（5.4）
- ⏸️ **跨账号第二份**：**已拍板不做**（用户判断云账号级损失概率足够低）
- ✅ **凭据**：`RUSTIC_PASSWORD` 已由用户单独留存于节点之外

**仍然没有答案的一件事**：RTO。恢复路径现在**存在**了，但端到端「换台机器把服务起回来」
这条路**没有人走过**，所以「多久能恢复」目前只有猜测（5.5）。

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

### 2.1 分类：两类数据源 + 一类凭据，Markdown 导出不在其中

容易搞错的一点：**Markdown 导出不是「要备份的东西」，它是当前唯一在跑的备份手段**（一个有损投影）。
真正持有状态的是节点上的卷，而其中只有两个算数据源：

| 卷 | 装什么 | 要不要备 | 异地现状 |
| --- | --- | --- | --- |
| `mica-prod-postgres` | 一切关系型状态（账号 / 关系 / CRDT / 版本 / 评论 / 分享 token） | ✅ **要** | 🔴 **没有** —— `MICA_BACKUP_PGURL` 未设 |
| `mica-prod-rustfs` | 图片字节 | ✅ **要** | 🟡 **间接**：只靠导出里的图片 |
| `mica-prod-backup` | 导出暂存区 `/var/lib/mica/export` | ❌ 派生产物 | — |

**图片那格是「间接」不是「有」**：没有任何东西直接备 `mica-prod-rustfs`，图片能回来纯粹因为导出把
它们一起写出去了。这意味着覆盖范围由**导出规则**（`is_deleted=false`）决定，而不由盘上有什么决定 ——
孤儿 blob 与回收站页面独有的图片都不在。要覆盖完整，得直接备那个卷，或者接受这个边界并写明。

### 2.2 第三类：凭据（不备这类，前两类备得再好也白搭）

节点 `/data/mica/.env`，里面有 `POSTGRES_PASSWORD`、JWT secret、S3 / RustFS keys、邮件 AK、
`MICA_BACKUP_TOKEN`，以及 **`RUSTIC_PASSWORD`**。

**它现在哪儿都没备，且与数据同一块盘。** 丢了它 = 拿到一个完好但**永远打不开**的仓库。
这一类的恢复成本是无穷大而备份成本是零（抄进密码管理器），所以它排在 WAL 归档之前（见 5.2）。

### 2.3 Traefik / ACME 证书（2026-07-30 查实）

- **Traefik 不在 mica 的 compose 里。** compose 第 218 行那个 `traefik:` 是 `networks:` 段的
  `external: true` 网络（`traefik-network`），**不是 service**。反代是独立的一套栈
  （容器 `traefik-traefik-1`），印证 roadmap 那句「Traefik 配置本体在仓库外未纳管」。
- **证书是持久化的**：`traefik_traefik-certificates/_data → /etc/traefik/acme`，重启不丢。
- **但它不在任何 mica 备份里**（属于另一套栈的卷）。份量轻：丢了可重新签发，只需注意
  Let's Encrypt 的频率限制。真正的残留仍是**配置本体没纳管**。

### 2.4 别把邻居的备份当成自己的（2026-07-30 查实）

同一台节点上还跑着一套无关的 `neostor-*` 栈，里面有 `neostor-pgbackup-1`
（`prodrigestivill/postgres-backup-local:16`，`@daily`）和 `neostor-oss-offsite-1`
（rclone `sync /backups oss-crypt:`）。**它们不覆盖 mica**：那个 pgbackup 的
`POSTGRES_DB=neostor` / `POSTGRES_USER=neostor`，连的是它自己那套栈的 postgres。

记在这里是因为「机器上有个 pgbackup 容器在跑」很容易被下一个人（或未来的我）当成
"数据库有备份"。**mica 是这台机器上唯一没有自动异地 DB 备份的应用。**
顺带一句：邻居用 `postgres-backup-local` + rclone→OSS 把同一个问题解决了，而 mica 只差
一个已经写好的开关没打开（5.1）。

## 3. RPO / RTO

分两套算，因为它们的备份状况天差地别。

| | 内容（正文+图片） | 数据库（账号/关系/历史/评论） |
| --- | --- | --- |
| **RPO**（丢多少） | ≤ 24h（每日 `BACKUP_HOUR` 一次） | ≤ 24h（2026-07-30 起，同一节拍）<br>~~∞ —— 无异地副本~~ |
| 唯一副本位置 | 阿里云 OSS（异地，同账号） | 阿里云 OSS，`label=_pgdump`（同上，2026-07-30 起）<br>另有节点本地 `/data/mica/pre-*.sql.gz` 手工还原点 |
| 那份副本的新鲜度 | 每天 | 每天（本地还原点仍只在**发版前手工**落） |
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
| **节点盘损坏 / ECS 没了** | DB 与本地还原点同时没，但 OSS 里有昨天的全库 | 新实例 → 从 OSS 取 `_pgdump` 恢复 DB → 取对象字节 → 起服务 | 🟡 **路径已存在（2026-07-30 起），但端到端没人走过** |
| **云账号级损失**（欠费、封号、AK 泄露被清） | 节点与 OSS 一起没 | 无 | ⏸️ **已拍板接受**（2026-07-30，用户判断概率足够低；跨账号第二份因此不做） |
| **勒索软件 / 误删仓库** | rustic 仓库被删或加密 | 仓库不可变性未配；密码只在节点 `.env` | 🟠 见 5.2 / 5.3 |
| **`RUSTIC_PASSWORD` 丢失** | 备份在，但**永远解不开** | 无 | 🟠 需确认它存在节点之外 |

## 5. 立即行动项（按 ROI 排）

### 5.0 ✅ 已完成（2026-07-30 当天）

**`MICA_BACKUP_PGURL` 已接上，数据库第一次有了异地副本。** 节点 `.env` 加了一行
（原文件备份为 `.env.bak-<ts>`），重建 backup 容器后手动跑了一次完整备份：

- 日志出现 `pg_dump → /var/lib/mica/export/_pgdump/mica.sql.gz` 与 `snapshot pg_dump → label=_pgdump`
- `rustic snapshots --filter-label _pgdump` 从 **`total: 0`** 变成 **1 个快照**（23.7 MiB，2026-07-30 13:39:55）

于是第 3 节那张表里「DB 的 RPO = ∞」**不再成立**，改为 ≤24h（跟内容同一个节拍）。
第 4 节「节点盘损坏」那一行也从「账号/历史/评论不可恢复」降级 —— 现在可以从异地 pg_dump 恢复，
**但仍未端到端演练过**（见 5.5）。

### 5.1 ~~🔴 #0 —— 接上 `MICA_BACKUP_PGURL`~~（已完成，留档：当时的状态与做法）

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

### 5.2 ✅ ~~#1 —— 把 `RUSTIC_PASSWORD` 挪出节点~~（用户已单独留存，2026-07-30）

脚本注释自己写着 `SAVE OFF-HOST (lose it, lose the repo)`。**零代码**：确认它在密码管理器里。
节点没了而密码只在节点上，等于备份存在但解不开 —— 最便宜也最容易被跳过的一条。

### 5.3 ⏸️ ~~#2 —— 第二份仓库，跨账号或跨提供商~~（已拍板不做，2026-07-30）

原建议：rustic 仓库与 ECS 同在一个阿里云账号下，账号级事件（欠费 / 封号 / AK 被清 / 区域故障）
会同时带走两边，`rustic copy` 到另一个账号或提供商可以解开这个耦合。

**用户判断该概率足够低，决定接受这个风险。** 记在这里是为了让下一个人知道
**它被想过并被有意跳过**，而不是漏了 —— 重新提这条建议前，先确认前提有没有变
（例如账号上跑了更多业务、或出现过一次欠费告警）。

### 5.4 🟡 #2 —— `_rustfs`：rclone 直传 OSS，让对象字节不再只靠导出捞

现在图片能到异地纯属搭内容导出的便车，覆盖范围由 `is_deleted=false` 决定（见 2.1）。
**定下的做法**：rclone 从 RustFS 的 S3 接口 `copy` 到 OSS 的一个前缀，**不经 rustic**。

三个理由：① 图片是**内容寻址、不可变**的，只增不改 —— 镜像即备份，不需要时间点；
② S3 → S3 直传，**不落本地暂存**；③ content-type 等对象元数据随 rclone 一起走，
不会像「下载成文件再传回去」那样丢掉（blob 端点是 302 跳存储，content-type 决定浏览器怎么渲染）。

**用 `copy` 不用 `sync`**：`sync` 会把删除也镜像过去；`copy` 只增不删，对不可变对象正好，
顺带扛住「误删了桶里的对象」。代价是被 `blob_gc` 正常回收的孤儿字节在备份侧会永远留着 —— 体量下无所谓。

**加不加 rclone crypt：倾向不加**，以保住「任何 S3 客户端都能恢复」这个属性；
威胁只剩 AK 泄露，而那种情况下 rustic 那两条腿一样暴露（AK 能删仓库）。

**待做**：`Dockerfile.cli` 加 rclone、`mica-backup.sh` 加这一段。**要发版**（脚本烤在 cli 镜像里）。

**顺带一并改**：现在是 `pg_dump | gzip`，而 **gzip 会把内容相似性打散，rustic 的 dedup 因此几乎失效**。
应改成不压缩、交给 rustic 自己压 —— 同一批每日全库快照会从「每天一份 24 MiB」变成「增量」。

### 5.5 🟡 #3 —— 端到端恢复演练（RTO 至今没有答案）

`just restore-drill` 验的是「把一份 dump 恢复进一次性库并断言可读」，**不是**节点重建。
要给出真 RTO，得在一台干净机器上从 OSS 拉 `_pgdump` + 对象字节，把服务起到 `/api/ready` 绿，记录耗时。
在走过一次之前，第 3 节的 RTO 那格只能空着。

### 5.6 🟡 #4 —— WAL 归档，把 DB 的 RPO 压到分钟级

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
