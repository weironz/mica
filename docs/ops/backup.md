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
4. **Configure Zerobyte relay**:
   - 将导出的卷目录（`mica_mica-prod-backup` 或 `MICA_EXPORT_HOST_DIR`）以只读方式挂载进
     Zerobyte；
   - 新建备份任务，目标指向阿里云 OSS 路径 `mica-backup/mica-md/`（复用现有
     `restic.pass` 与 OSS 凭据）；
   - 调度时间错开在 `BACKUP_HOUR` 之后（例如 `BACKUP_HOUR=3`，Zerobyte 在 `04:00` 接力
     增量上传）。

> **凭据备份提醒**：只恢复业务数据时，Databasus 用 `secret.key`，Zerobyte 用
> `restic.pass`，两边均需能读取 OSS。建议将 `/data/mica/.env.secrets` 也纳入 Zerobyte
> 备份或密码库异机保存。

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

## Restore the instance's credentials

Do this FIRST in a rebuild — the steps below need them.

A fresh machine has this image and nothing else, so the restore cannot depend on
a running stack — it runs straight from the image:

```bash
docker run --rm -v /data/mica:/restore   --entrypoint /usr/local/bin/mica-cli   -e RUSTIC_PASSWORD -e OSS_BUCKET -e OSS_ENDPOINT -e OSS_REGION -e OSS_ROOT   -e OSS_ACCESS_KEY_ID -e OSS_SECRET_ACCESS_KEY   <registry>/mica-cli:<version> backup restore-config
install -m 600 /data/mica/etc/mica/env.secrets /data/mica/.env.secrets
```

`--entrypoint` is needed on v0.13.41, whose entrypoint is the whole
`mica-cli backup daemon` — without it the arguments append to that and you get
`backup daemon backup restore-config`. From the next release the entrypoint is
just the binary and the flag is optional (harmless either way).

`RUSTIC_PASSWORD` and the `OSS_*` pair are what you just used to reach and open
the repository, so they come from wherever you keep them, not from here.

**Do this before starting the stack**, not after: `POSTGRES_PASSWORD` lives in
this file and Postgres uses it at initdb. Restore it late and the database was
created with one password while the config names another.

## Restore the database from a `pg_dump` (full-instance DR)

Use this when the Postgres volume is lost/corrupt, or to stand up a clone — it
brings back users / memberships / CRDT history the content re-import cannot. The
DB dump rides its own rustic lineage (`--label _pgdump`), so pull it the same way
as a workspace.

**Rehearse on a scratch DB first** — the same discipline as a migration
(`docs/dev/lessons.md` §"迁移怎么验"): a restore you've never run is a guess, and a
half-restored prod DB is worse than a down one.

```bash
# 1) Pull the latest DB dump out of the repo (its own label):
docker exec mica-backup-1 rustic restore latest /tmp/pg --filter-label _pgdump
docker cp mica-backup-1:/tmp/pg/mica.sql ./mica.sql
# The _pgdump lineage is UNCOMPRESSED on purpose (rustic dedups plain SQL well,
# gzip defeats it — see crates/cli/src/backup.rs). So there is no `gzip -t` here; the
# integrity guarantee comes from rustic itself, which verifies chunk hashes on
# restore. Sanity-check it looks like a dump instead:
head -3 ./mica.sql && grep -c '^COPY public\.' ./mica.sql   # header + table count

# 2) REHEARSE into a throwaway DB on the SAME server (no prod password leaves it).
#    `just restore-drill <basename>` is this, with assertions — prefer it:
just restore-points                                     # what's on the node
just restore-drill pre-0.13.4-20260730-100443.sql.gz    # restore → assert → drop
```

### 恢复演练(`just restore-drill`)

`deploy/restore-drill.sh`,一条命令跑完:恢复进一次性库 → 断言 → `DROP`(trap 兜底),
`mica` 生产库全程不碰、不重启任何容器,输出只有计数(不打印正文/用户字段)。
顺带跑 `rustic check`(对象存储那半的静默损坏,只能靠问才发现)。

`\dt` 那种「表在不在」的断言不够 —— 一次产出空 `state` blob 的恢复能通过所有计数。
所以它有三条**硬门槛**,不满足就非零退出:恢复错误数 = 0、`documents` > 0、
**可读页数 > 0**(走 `views → documents → document_yrs_base` 那个每次读都要走的 join,
且要求 `length(state) > 0 AND content_text <> ''`)。

**首次实跑(2026-07-30,`pre-0.13.4-20260730-100443.sql.gz`)**:恢复错误 0、
`_sqlx_migrations` = 15(正确的 pre-0016 世界)、被 S5 删掉的三张表都回来了、行数与备份时
记录的一致(snapshots 3975 / updates 239 / versions 3 / docs 3736 / bases 3736 /
views 4572 / users 9)、32 个 FK + 19 个 PK、**3331 个可读页**;`rustic check` 170 个
snapshot 全部通过。**在这之前,这条恢复路径从未被真正走过一次。**

> 为什么突然重要:S5(migration 0016)是本 schema 第一个不可逆迁移,它删掉了 op 模型
> 三张表 —— 对早于它的数据,这个 dump 就是唯一的回头路。`gzip -t` 只证明文件没截断。

**Restore over prod** (destructive — the rehearsal must have passed):

```bash
cd /data/mica
# a) STOP the api so nothing writes mid-restore (the DB must be quiescent).
#    Leave postgres up — you're restoring INTO it.
docker compose up -d --no-deps --scale api=0 api || docker compose stop api

# b) Drop + recreate the database, then load the dump.
docker exec -i mica-postgres-1 psql -U mica -d postgres \
  -c 'DROP DATABASE mica' -c 'CREATE DATABASE mica'
docker exec -i mica-postgres-1 psql -q -U mica -d mica < ./mica.sql

# c) Pin MICA_VERSION to the tag the dump's SCHEMA matches, THEN bring api up.
#    sqlx::migrate! is forward-only: a NEWER api meeting an OLDER restored schema
#    just re-applies the missing migrations (fine), but an OLDER api meeting a
#    NEWER schema will not start. If unsure, roll to the tag that was live when
#    the dump was taken:
sed -i -E 's|^MICA_VERSION=.*|MICA_VERSION=vX.Y.Z|' .env
docker compose up -d --no-deps api
curl -fsS https://mica.cloudcele.com/api/ready       # DB-backed readiness = restore worked
```

**The recovery-window gap.** The dump is a point-in-time snapshot (last daily run
+ any manual one). Writes between that instant and the outage are **not** in it
and are lost on restore — there is no WAL archive here, this is dump-based DR, not
PITR. Two mitigations before you drop prod: (1) if the old volume is still
readable, take a fresh `pg_dump` off it first and restore THAT instead; (2)
announce the restore point so users know which edits to redo. Widen the window by
lowering `BACKUP_HOUR` cadence or adding manual `docker exec mica-backup-1
mica-cli backup run` runs before risky changes.

### Roll back a bad migration (the `pre-*.sql.gz` restore point)

Different trigger from the DR case above: not a lost volume, but a **deploy whose
migration went wrong**. `just deploy-prod` runs the new api's migrations the moment
it comes up and does **not** back up first (see the ⚠️ in [`deploy.md`](deploy.md)),
so a data-touching migration must be preceded by a hand-taken restore point, per the
CLAUDE.md discipline:

```bash
# BEFORE deploying a data migration (on the node, in /data/mica):
ts=$(date +%Y%m%d-%H%M%S)
docker exec mica-postgres-1 pg_dump -U mica -d mica | gzip > pre-<migration>-$ts.sql.gz
gzip -t pre-<migration>-$ts.sql.gz                                    # prove it's not truncated
zcat pre-<migration>-$ts.sql.gz | grep -c '^COPY public.<affected-table>'   # target table present
```

If the migration lands and prod is wrong (api won't start, data mangled, a spot
check fails), roll back **both** the schema and the code:

```bash
cd /data/mica
# 1) Stop the api so nothing writes into the half-migrated schema.
docker compose up -d --no-deps --scale api=0 api || docker compose stop api

# 2) Restore the pre-migration dump OVER prod. Same drop/create/load as
#    "Restore over prod" above — the only difference is the source is your
#    hand-taken pre-*.sql.gz, not the rustic _pgdump lineage.
docker exec -i mica-postgres-1 psql -U mica -d postgres \
  -c 'DROP DATABASE mica' -c 'CREATE DATABASE mica'
zcat pre-<migration>-<ts>.sql.gz | docker exec -i mica-postgres-1 psql -q -U mica -d mica

# 3) Roll the DEPLOY back to the tag that was live before this one. The restored
#    schema matches the OLD tag; bringing the NEW api back up would just re-run
#    the same bad migration (sqlx::migrate! is forward-only), so you MUST pin the
#    old image, not the new one.
sed -i -E 's|^MICA_VERSION=.*|MICA_VERSION=v<OLD>.<Y>.<Z>|' .env
docker compose up -d --no-deps api
curl -fsS https://mica.cloudcele.com/api/health   # confirm it reports v<OLD>.<Y>.<Z>
curl -fsS https://mica.cloudcele.com/api/ready    # DB-backed readiness = schema + code back in sync
```

**No `pre-*.sql.gz`?** You skipped the restore point. Fall back to the off-site
`_pgdump` lineage (the section above) — it's the last daily snapshot, so you lose
everything written since that night's run, but it's a real restore point.

**Lost writes in the recovery window.** Any edit accepted between the bad deploy
going live and the restore is **not** in `pre-*.sql.gz` and is gone on restore —
the same point-in-time gap as the DR section. Two mitigations before you drop the
corrupt DB: (1) if it's still readable, `pg_dump` it FIRST and diff/cherry-pick the
rows written after the cutoff; (2) announce the restore point so users know which
edits to redo. **Rehearse the restore into a scratch db** first
(`docs/dev/lessons.md` §"迁移怎么验": `CREATE DATABASE mica_restest` → `zcat … | psql`
→ assert → `DROP DATABASE`) — a restore you've never run is a guess.

## Restore test (do this — a backup you've never restored is a guess)

Periodically restore a workspace into a throwaway target and assert the Markdown
is there and byte-intact, then `check`. A repo you can't restore isn't a backup.

## Security

- The **repo password** and the **RAM AccessKey** are the only unrecoverable
  secrets — store them **off the host** (password manager + a sealed copy). Lose
  the password and the encrypted repo is permanently unreadable.
- The password lives in `RUSTIC_PASSWORD` (env) and is **never** written to the
  rendered `/etc/rustic/rustic.toml` (which holds only the OSS backend options).
- The staging tree (`/var/lib/mica/export` in the `mica-prod-backup` volume)
  holds plaintext content — same trust boundary as the DB. OSS is the real
  off-site copy; rustic encrypts before upload.
- Rotate the RAM key periodically; keep OSS Versioning/Object-Lock on.
