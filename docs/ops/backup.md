# Backup & Restore

**Mica does not do off-site backup policy — on purpose (mechanism vs policy).** Mica's
job is a consistent, incremental **`export`** (Markdown + `.mica.json` + images);
encryption, deduplication, retention and remote OSS upload belong to dedicated backup
services:

| Data layer | Owner | Off-site target (Aliyun OSS) | Off-host recovery secret |
| --- | --- | --- | --- |
| **PostgreSQL**（账号、权限、CRDT、评论、分享） | **Databasus** | `mica-backup/mica_pg/` | `secret.key` |
| **RustFS**（图片与附件字节） | **Zerobyte** (restic) | `mica-backup/mica_s3/` | `restic.pass` |
| **Markdown 镜像导出**（脱离 Mica 可读兜底） | **`mica-backup` 容器**（`mica-cli backup daemon`）→ **Zerobyte** 接力 | `mica-backup/mica-md/` | `restic.pass` |

> **2026-11-21 瘦身说明**：`deploy/Dockerfile.cli` 已移除 `rustic`、`rclone` 与
> `postgresql-client-18`，仅保留纯 Rust `mica-cli` 与 `ca-certificates`。常驻的
> `backup` profile 容器只负责按 `BACKUP_HOUR` 增量导出 Markdown + 图片到卷目录
> （`/var/lib/mica/export`），校验 `manifest.json` 覆盖完整性并上报 `HEALTHCHECK_URL`，
> 随后由 Zerobyte 只读挂载该目录增量备份到 OSS `mica-backup/mica-md/`。

## Topology: export mirror → Zerobyte relay

```text
[daily at BACKUP_HOUR (or on start unless BACKUP_ON_START=0)]
  mica-backup container (mica-cli backup daemon):
    1. acquire exclusive OS lock (/tmp/mica-backup.lock)
    2. mica-cli export --out /var/lib/mica/export   # incremental mirror + manifest.json
    3. verify every workspace in manifest.json exists on disk
    4. ping HEALTHCHECK_URL (or HEALTHCHECK_URL/fail on error)

[Zerobyte scheduled job (after BACKUP_HOUR)]
  mount export directory read-only → restic incremental backup → OSS mica-backup/mica-md/
```

**Coverage is checked, not assumed.** Every workspace announced in `manifest.json` must
have its mirrored directory on disk; otherwise the run fails and pings
`${HEALTHCHECK_URL}/fail`.

> Note what this does NOT check: the export only ever covers **the workspaces the backup
> token's own account can see**. Other accounts' content and full relational state
> (accounts, memberships, CRDT history, comments, share tokens) are captured by
> **Databasus** (`mica-backup/mica_pg/`) + **Zerobyte** (`mica-backup/mica_s3/`).

`mica-cli export` maintains a complete incremental Markdown/image mirror using
read-scoped API access (`MICA_PAT`) over the internal Compose network
(`MICA_SERVER=http://api:8080`). Unchanged pages and assets are reused from the local
state cache; deleted pages and legacy `_pgdump/` files are pruned automatically so
Zerobyte only uploads live Markdown content.

## 并发、超时与容器配置

- 导出轮次持有 `/tmp/mica-backup.lock` 的操作系统独占锁，正常退出或进程被杀时由系统
  释放。**不要通过删除锁文件解除运行中任务的锁**。
- 子进程 `mica-cli export` 总执行时限为 **2 小时**；超时会终止整个 Unix 进程组并回收
  子进程，失败上报 `${HEALTHCHECK_URL}/fail`。
- `backup` 容器在 `deploy/docker-compose.yml` 中固定 `hostname: mica-backup`、启用
  `init: true`、限制内存上限 `MICA_BACKUP_MEMORY_LIMIT`（默认 `768m`），通过
  `${MICA_EXPORT_HOST_DIR:-mica-prod-backup}:/var/lib/mica/export` 暴露导出目录：
  - 默认使用命名卷 `mica-prod-backup`（宿主机路径通常为
    `/var/lib/docker/volumes/mica_mica-prod-backup/_data`，Zerobyte 可直接以外部卷或
    只读 bind mount 接入）；
  - 如希望固定宿主机目录（例如 `/data/mica/export`），在节点 `.env` 设置
    `MICA_EXPORT_HOST_DIR=/data/mica/export` 即可。
- 生产通过 `BACKUP_ON_START=0` 避免白天发版时立即触发导出；日常按 `BACKUP_HOUR`（默认
  `3`，配合 `TZ=Asia/Shanghai`）运行，必要时可手工触发：
  ```bash
  docker exec mica-backup-1 mica-cli backup run
  ```

## Deploy inside the Docker stack

The production compose ships a slim `backup` service behind the opt-in `backup` profile;
a plain `docker compose up -d` ignores it.

1. **Mint a read-scoped Mica token** (PAT) — in the app (*Settings → API Tokens*), or
   with the CLI:
   ```bash
   mica-cli auth login --email you@example.com
   mica-cli auth token create --name backup --scope read   # secret printed ONCE
   ```
2. **Configure `.env` / `.env.secrets`**:
   ```bash
   # .env.secrets
   MICA_BACKUP_TOKEN=mica_pat_…

   # .env (optional overrides)
   TZ=Asia/Shanghai
   BACKUP_HOUR=3
   BACKUP_ON_START=0
   # MICA_EXPORT_HOST_DIR=/data/mica/export   # optional host bind mount for Zerobyte
   # HEALTHCHECK_URL=https://hc-ping.com/<uuid>
   ```
3. **Start the export sidecar & test one pass**:
   ```bash
   docker compose --profile backup up -d --no-deps backup
   docker exec mica-backup-1 mica-cli backup run
   docker logs -f mica-backup-1
   ```
4. **Configure Zerobyte relay（生产已按以下参数配置并验证，2026-11-21）**:
   - 在 `/data/backup-services/compose.yaml` 中为 `zerobyte` 以只读方式挂载宿主机导出目录：
     ```yaml
     - /data/mica/export:/data/mica-export:ro
     ```
   - 生产节点 `.env`（及 [ansible/host_vars/mica-prod.yml](../../ansible/host_vars/mica-prod.yml)）配置：
     ```bash
     MICA_EXPORT_HOST_DIR=/data/mica/export
     ```
   - 在 Zerobyte（`https://zerobyte.cloudcele.com`）中创建三项：
     - **Volume**：`Mica Markdown`（Type `directory`，Path `/data/mica-export`）
     - **Repository**：`Mica Markdown OSS`（Type `rclone`，Remote `aliyun-backups`，Path `mica-backup/mica-md`，复用 `/root/.zerobyte-recovery/restic.pass`，`compressionMode=auto`，开启 `autoCheck`）
     - **Schedule**：`Mica Markdown daily`，Cron `0 4 * * *`（注意 Zerobyte 容器时区为 `TZ=Asia/Bangkok`（UTC+7），即曼谷 04:00 = **北京 05:00**，错开 `Mica RustFS daily` 的 03:40 且稳在 `mica-backup` 北京 03:00 导出完成之后），保留策略 `keepLast: 30`，排除 `.mica-export` 与 `_pgdump`（不上传导出增量状态锁文件）。
   - **首跑实测（2026-11-21）**：快照 `d3b05528`，扫描 32,271 文件 / 2,662 目录，原始 1.24 GiB，经 restic 去重压缩写入 OSS `mica-backup/mica-md` 仅 740 MiB，耗时 2 分 41 秒。

> **凭据备份提醒**：只恢复业务数据时，**Databasus 用 `secret.key`，Zerobyte 用
> `restic.pass`**，两边均需能读取 OSS。`/data/mica/.env.secrets` 及 Databasus /
> Zerobyte 自身配置目录需按下方「恢复与安全」章节单独留存。

### The archive's two halves (manifest v2, 2026-08-30)

Every workspace/folder export zip now carries both:

```
<page>.md          human-readable Markdown, images as relative links
<page>.mica.json   {schema_version, root_block_id, blocks[], assets{file_id → path},
                    md_fingerprint, comments?}
assets/            the image bytes, de-duplicated
manifest.json      {version: 3, generator: "mica",
                    pages: [{path, title, type, id, parent_id?, position, icon?}]}
```

v1 recorded the tree by **path** alone — enough to rebuild a shape, not enough to
say "this is the same page", so nothing pointing at a page survived a round trip.
v2 adds identity. Readers must keep accepting v1: archives already downloaded do
not get re-issued.

The two halves deliberately **disagree on one thing**: an internal page link is a
relative `.md` path in the Markdown (so a human or an editor can follow it) and
stays `mica://page/<viewId>` in the JSON (so an importer can remap it through the
manifest's ids). Rewriting the JSON too would throw away the identity the JSON
exists to carry.

Importing one of our own archives (2026-11-01) builds each page from its
`.mica.json` instead of re-parsing the Markdown, so block ids and settings that
Markdown cannot carry (image width, …) come back as exported; images are
re-uploaded from `assets/` and internal links are pointed at the new pages.
That happens only while the `.md` is byte-for-byte what was exported
(`md_fingerprint`): edit a page's Markdown in an exported folder and that edit
wins. Archives exported before 2026-11-01 carry no fingerprint and import
through the Markdown as before.

Manifest v3 (2026-11-01) adds a page's comment threads to its `.mica.json`
(`comments`: status, quote, times, the anchor as block id + offsets, and each
comment's author display name, body and times; never account ids or emails).
Page properties already travel in the root block's `front_matter`. Importing
recreates the threads on the new page, anchored on the same text; the
importing user is the author of record, and a comment by anyone else opens
with an attribution line. Comments are server-side only (the local store has
none), and they never enter the `.md`. A page's export revision covers its
comments (`mica_comments_revision`, migration 0037), so the CLI mirror
re-fetches a page whose discussion changed.

The app offers the two halves as two choices (N3, 2026-11-01): **Backup**
downloads `<name>.mica.zip` with the `.mica.json` sidecars and is what restores
a workspace losslessly; **Export as Markdown** downloads `<name>.zip` with the
same `.md` files, assets and manifest but no sidecars, for other tools. The
server's tree exports take `?format=backup|markdown`; no parameter means
`backup`, so the CLI mirror and older clients are unchanged, and any other
value is a 400. Either archive imports back; the Markdown one goes through the
Markdown path, so block ids and comments are not carried.

### What this restores, and what it does not

**The content export is not an instance restore.** Putting an instance back is the
database plus the object bytes, both below — that path is exercised end to end by
`.github/workflows/dr-drill.yml`. The content export exists for two other things:

1. **Your notes are readable without Mica.** Verified 2026-08-30: restoring one
   workspace lineage out of the repository yields the original directory tree,
   nesting and non-ASCII filenames intact, with plain readable Markdown in it.
2. **A last resort if the database is unrecoverable** — rebuild the TEXT into a
   fresh instance and accept what that loses.

To do (2): restore the tree, **zip it**, and POST that to
`/api/workspaces/import` (or use the app's Import).

⚠️ **The endpoint takes a ZIP, not a directory** (`read_zip(&body)`), up to
**256 MiB** per request — a larger workspace has to be split, and the parts can
go into the same workspace with `workspace_id`. An earlier version of this
document said "re-import each workspace's tree", which reads as though you could
point the importer at the restored directory. You cannot.

⚠️ **What re-importing brings back, and what it does not.** From a backup
(`.mica.json` sidecars present, `.md` untouched) each page comes back from its
native blocks: block ids, image settings, comment threads (manifest v3) and —
since N4, 2026-11-01 — **its original view id**, whenever no view on this
instance still holds it (the workspace was deleted, or this is a fresh
instance), so `mica://page/<id>` links from other workspaces, bookmarks and open
tabs find the page again. An id still in use (the original beside the restore,
the same backup imported twice) gets a fresh one and the import's own links
are pointed at it. Never restored: CRDT edit history, share links (they belong
to the document, which is always new), users and memberships, comment authors
as accounts (the importer is the author of record, with an attribution line).
Those live only in the database. A Markdown export, or a page whose `.md` was
edited, imports through the Markdown path: new pages with the same text.

**Its status, stated exactly** (decided 2026-08-30): the importer itself is a
used, working feature — the operator has imported archives by hand. What has
never been run as one exercise is the CHAIN: restore a lineage out of the
repository, zip it, import it, check the result against the original. That is
deliberately not automated, because **recovery is the database plus the objects**
— the path the drill covers. This leg is the fallback for when that path is gone,
and the reason your notes do not need Mica to be readable.

## Restore overview (Databasus + Zerobyte)

全量恢复一台 Mica 实例分为四步，顺序固定：**先就位实例凭据 `.env.secrets`，再起栈，再分别恢复 PostgreSQL 与 RustFS**。

### 1. 恢复实例凭据（`.env.secrets`）

**必须在首次起栈前就位**（`POSTGRES_PASSWORD` 在 Postgres `initdb` 时生效，起栈后再补凭据会导致密码不匹配）：

- 从密码库（或 Zerobyte 配置备份）恢复 `/data/mica/.env.secrets`，权限设为 `0600`：
  ```bash
  install -m 600 <restored-env-secrets> /data/mica/.env.secrets
  ```
- 非敏感配置由 `ansible/deploy.yml` 从 [ansible/host_vars/mica-prod.yml](../../ansible/host_vars/mica-prod.yml) 自动渲染到 `/data/mica/.env`。

### 2. 恢复 PostgreSQL（Databasus → `mica-backup/mica_pg/`）

- 所需异机凭据：
  1. Databasus 解密密钥 **`secret.key`**（宿主机原路径 `/data/backup-services/databasus-data/secret.key`，系统盘副本 `/root/.databasus-recovery/secret.key`，须异机保存）；
  2. 可读 `mica-backup/mica_pg/` 的阿里云 OSS 凭据（或通过阿里云 RAM 临时签发）。
- **通过 Databasus 恢复**：在 Databasus Web 界面（或重建 Databasus 并加载原 `secret.key`）选择目标时间点恢复到目标库。
- **恢复到生产库前的标准动作**（与下文回滚一致）：
  1. 先在临时库预演验证（`just restore-drill` 或新建临时库导入后检查 `documents`、`views` 与可读页数）；
  2. 停止 `api` 容器（`docker compose stop api`），确保无并发写入；
  3. 恢复后确认 `.env` 中的 `MICA_VERSION` 不低于该备份对应的 schema 版本（`sqlx::migrate!` 仅支持前向迁移），再拉起 `api` 并校验 `https://mica.cloudcele.com/api/ready`。

### 3. 恢复 RustFS 对象与 Markdown 镜像（Zerobyte / restic）

- 所需异机凭据：
  1. Zerobyte / restic 仓库口令 **`restic.pass`**（宿主机路径 `/root/.zerobyte-recovery/restic.pass`，同一把密钥同时打开 `mica_s3/`、`mica-md/` 与 `assay_s3/`，须异机保存）；
  2. 可读 `mica-backup/mica_s3/` 与 `mica-backup/mica-md/` 的阿里云 OSS 凭据。
- **恢复 RustFS 图片/附件（`mica-backup/mica_s3/`）**：
  - 通过 Zerobyte Web 界面将 `Mica RustFS OSS` 最新快照恢复到目标 RustFS 卷/桶；
  - 或在任意装有 `restic` + `rclone` 的机器上直接凭 `restic.pass` 与 OSS 凭据执行 `restic -r rclone:aliyun-backups:mica-backup/mica_s3 restore latest --target <dir>`，再同步回 RustFS `mica` 桶。
- **恢复脱离 Mica 可读的 Markdown 镜像（`mica-backup/mica-md/`）**：
  - 通过 Zerobyte Web 界面（`Mica Markdown OSS`）或 `restic -r rclone:aliyun-backups:mica-backup/mica-md restore latest --target <dir>` 取出完整目录树与图片。
- **如果要连同 Databasus / Zerobyte 管理界面、任务与历史一并恢复**：
  - Databasus 需配套保留 `databasus-data/` + 原 `secret.key`；
  - Zerobyte 需配套保留 `/data/backup-services/zerobyte-secrets/app-secret`、`/data/backup-services/zerobyte-data/`，继续运行备份任务时还需恢复或重建 `/data/backup-services/zerobyte-secrets/rclone.conf`。

### Roll back a bad migration (the `pre-*.sql.gz` restore point)

与灾难恢复不同，发版包含 SQL 迁移时，`ansible/deploy.yml` 会在迁移前自动在节点本地落一份经内容校验的还原点 `/data/mica/pre-<version>-<ts>.sql.gz`（最多保留最近 5 份；手工取点命令如下）：

```bash
# BEFORE deploying a data migration (on the node, in /data/mica):
ts=$(date +%Y%m%d-%H%M%S)
docker exec mica-postgres-1 pg_dump -U mica -d mica | gzip > pre-<migration>-$ts.sql.gz
gzip -t pre-<migration>-$ts.sql.gz
zcat pre-<migration>-$ts.sql.gz | grep -c '^COPY public.<affected-table>'
```

如果迁移后线上异常，需**同时回滚数据库与镜像版本**：

```bash
cd /data/mica
# 1) Stop the api so nothing writes into the half-migrated schema.
docker compose stop api

# 2) Restore the pre-migration dump OVER prod.
docker exec -i mica-postgres-1 psql -U mica -d postgres \
  -c 'DROP DATABASE mica' -c 'CREATE DATABASE mica'
zcat pre-<migration>-<ts>.sql.gz | docker exec -i mica-postgres-1 psql -q -U mica -d mica

# 3) Pin MICA_VERSION back to the previous tag and restart api.
sed -i -E 's|^MICA_VERSION=.*|MICA_VERSION=v<OLD>.<Y>.<Z>|' .env
docker compose up -d --no-deps api
curl -fsS https://mica.cloudcele.com/api/health
curl -fsS https://mica.cloudcele.com/api/ready
```

若节点无 `pre-*.sql.gz`，回退到 Databasus 异地库 `mica-backup/mica_pg/` 的最近一份全库备份。

## Restore test (do this — a backup you've never restored is a guess)

定期验证两件事：
1. 用异机保存的 `secret.key` + OSS 凭据验证能从 `mica-backup/mica_pg/` 还原到临时库并通过表/可读页断言；
2. 用异机保存的 `restic.pass` + OSS 凭据验证 `mica-backup/mica_s3/` 与 `mica-backup/mica-md/` 快照可读且 `Auto Check` 通过。

## Security

- **只恢复业务数据的最小异机密钥集**：Databasus `secret.key`、Zerobyte `restic.pass`、以及可访问对应 OSS 前缀的阿里云 RAM 凭据（或主账号重新签发能力）。丢失 `secret.key` 或 `restic.pass` 将导致对应加密备份永久不可读。
- **管理面与实例凭据**：Databasus / Zerobyte 管理员密码、`/data/mica/.env.secrets` 与 `zerobyte-secrets/rclone.conf` 须加密存放于密码库，不得提交至 Git 仓库。
- **导出目录权限边界**：`/data/mica/export` 存放明文 Markdown 与图片（与数据库同信任边界）；Zerobyte 以 `:ro` 只读挂载该目录，上传 OSS 前由 restic 完成端到端加密。
- 定期轮换 RAM AccessKey，并在 OSS 备份桶开启版本控制 / 防误删策略。
