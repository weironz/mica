# Backup & Restore

**Mica does not do backups — on purpose (mechanism vs policy).** Mica's job is a
great, consistent **`export`**; backup — encryption, dedup, retention, remote
targets — is the job of a dedicated tool. The production stack ships a small
container that wires the two together: `mica-cli export` (a thin API client, no
backup engine — see [`cli.md`](cli.md)) + **`rustic`** (external, restic-format)
snapshotting to Aliyun OSS. `mica-cli` itself has no `backup` command anymore.

## Topology: one repo, one bucket, one lineage per workspace

```
[daily + on start]
  mica-cli export --out /export            # every workspace → Markdown + images (mirrored) + manifest.json
  pg_dump $MICA_BACKUP_PGURL > /export/_pgdump/mica.sql    # off-site DB image (if MICA_BACKUP_PGURL set)
  rclone copy rustfs:<bucket> ossblob:<bucket>/mica-blobs  # object bytes, S3→S3, NOT through rustic
  per workspace (read from manifest.json):
    rustic backup /export/<dir> --label <workspace-id> --tag ws=<name> --tag mica
  rustic backup /export/_pgdump --label _pgdump --tag pgdump --tag mica   # DB dump as its own lineage
  rustic forget --group-by label --keep-daily 7 --keep-weekly 4 --keep-monthly 6 --prune
```

**Coverage is checked, not assumed.** Every workspace the export announces in
`manifest.json` must reach the repository; one that does not fails the run and
names itself in the reason. This used to be a log line and nothing else — the
run still succeeded and still pinged the healthcheck, so a workspace could stop
being backed up while every signal you had said the backups were fine.

> Note what this does NOT check: the export only ever covers **the workspaces
> the backup token's own account can see**. That is by design, and reconciling
> against `mica-cli ws list` would be comparing the manifest with itself. Other
> accounts' content is captured by the `pg_dump` leg (the whole database), just
> not as portable Markdown.

The daily loop also drives a **dead man's switch**: on a successful run it pings
`${HEALTHCHECK_URL}`, on a failed one `${HEALTHCHECK_URL}/fail`. Point it at a
monitor (e.g. healthchecks.io) and you get paged when the *expected* success
ping stops arriving — a wedged loop or a dead container that a stderr line
reaches no one about. Unset → no ping; the ping itself is best-effort and never
fails the backup.

**One rustic repo** (one bucket, one key, one dedup pool). Each workspace is its
own **retention lineage**, keyed on the **stable workspace id** via rustic's
`--label`, so a rename never splits history; the mutable (and possibly non-ASCII)
name rides along as a readable `ws=<name>` tag. `forget --group-by label` applies
the keep policy **per workspace**. Restore, list, and prune are all
per-workspace by label.

`mica-cli export` unpacks each workspace's `export.zip` into a **mirrored tree**
(not a zip), so rustic dedups properly and prunes upstream-deleted files. It is
all GETs — a **read-scoped** token suffices.

## What this is (and isn't)

- **Content backup** — Markdown source + referenced images per workspace,
  portable and human-readable (restorable into Mica, or openable in Obsidian etc.).
- **Full-instance DR** — the `pg_dump` step (below) captures what the content
  export cannot: users / passwords / memberships and the fine-grained CRDT edit
  history. The two are complementary and now ride the SAME rustic repo off-site,
  so a single restore point covers both. The DB dump is **only taken when
  `MICA_BACKUP_PGURL` is set** — unset, the run logs a `WARN` and produces a
  content-only backup (it never silently drops the DB image).
- **The instance's own credentials** — `.env.secrets`, mounted read-only into
  the backup container and snapshotted under its own lineage (`--label
  _config`). Without it a rebuilt machine reaches a running, EMPTY stack and
  stops there: putting the data back needs credentials that otherwise exist
  only in somebody's password manager. This leg is **skipped, and a skipped leg
  fails the run**, when the file is not mounted.

  Three things it cannot do, by construction: `RUSTIC_PASSWORD` opens this
  repository, and `OSS_ACCESS_KEY_ID` / `OSS_SECRET_ACCESS_KEY` are what reach
  it at all — and all three are inside that file. A copy in the repository
  cannot help you open the repository. **Those three still have to be kept
  somewhere else**; they are the only ones that do.

## Aliyun OSS setup (one-time)

1. **Create a bucket** (e.g. `mica-backups`), ideally in a **different region**
   than the Mica ECS for geographic isolation.
2. **Create a least-privilege RAM AccessKey** scoped to *only* that bucket —
   `oss:PutObject / GetObject / DeleteObject / ListObjects` + `oss:GetBucket*`.
   Do **not** reuse the app's rustfs S3 keys or an account-wide key.
3. **Enable Object Versioning / Object-Lock** on the bucket so a buggy prune or a
   compromised key can't erase history.

> **OSS gotcha:** OSS only serves virtual-host-style addressing, so the backup
> config forces `enable_virtual_host_style = true`. Path style fails with
> "Path `config` does not exist".

## Deploy inside the Docker stack

The production compose ships a `backup` service — the container above running the
daily loop over the **internal** network (`MICA_SERVER=http://api:8080` is
hard-wired), so the token never leaves the host. It is **opt-in** behind the
`backup` profile; a plain `docker compose up -d` ignores it.

1. **Build & load the image** (the node can't pull from Docker Hub):
   ```bash
   docker build -f deploy/Dockerfile.cli -t willdockerhub/mica-cli:v0.3 . --provenance=false --sbom=false
   docker save willdockerhub/mica-cli:v0.3 | gzip | ssh root@<node> 'gunzip | docker load'
   ```
2. **Mint a read-scoped Mica token** (PAT) — in the app (*Settings → API
   Tokens*), or with the CLI:
   ```bash
   mica-cli auth login --email you@example.com          # once, to authenticate
   mica-cli auth token create --name backup --scope read   # secret printed ONCE
   ```
   Add it plus the repo/OSS config to the node's `.env` (next to `MICA_VERSION`).
   Each value stands alone so the AccessKey rotates without touching the rest:
   ```
   MICA_BACKUP_TOKEN=mica_pat_…
   MICA_BACKUP_PASSWORD=…                 # SAVE THIS OFF-HOST — lose it, lose the repo
   OSS_BUCKET=my-mica-backups
   OSS_ENDPOINT=https://oss-cn-hangzhou.aliyuncs.com
   OSS_REGION=oss-cn-hangzhou
   OSS_ACCESS_KEY_ID=…
   OSS_SECRET_ACCESS_KEY=…
   OSS_ROOT=mica                          # object-key prefix; use a fresh one to share a bucket
   # Off-site DB image (full-instance DR). The backup container shares the
   # default network with postgres, so use the INTERNAL host `postgres:5432`.
   # Reuse the same password as POSTGRES_PASSWORD. Unset → content-only backup.
   MICA_BACKUP_PGURL=postgres://mica:<POSTGRES_PASSWORD>@postgres:5432/mica
   # Dead man's switch (optional): pinged on success, <URL>/fail on failure.
   HEALTHCHECK_URL=https://hc-ping.com/<uuid>
   # optional: BACKUP_HOUR=3  KEEP_DAILY=7  KEEP_WEEKLY=4  KEEP_MONTHLY=6
   #   BACKUP_HOUR takes `3`, `03`, `3:00` or `03:30` — a wall-clock time in the
   #   CONTAINER's zone. Set `TZ` with it (compose defaults to Asia/Shanghai):
   #   without a TZ the container has no local time, `chrono::Local` degrades to
   #   UTC, and `BACKUP_HOUR=3` silently means 11:00 in Beijing. That was live
   #   until 2026-08-29 — the config said "nightly" and it ran mid-workday.
   #   An unreadable value logs a line and falls back to 03:00; it does not
   #   refuse to start, because no backups is worse than backups at the default.
   ```
3. **Start the service, initialise the repo once, then trigger the first run.**
   On start the container renders `/etc/rustic/rustic.toml` from the `OSS_*` env,
   so plain `rustic` works via `docker exec` (no wrapper). `--no-deps` avoids
   recreating the `api` container.
   ```bash
   docker compose --profile backup up -d --no-deps backup   # starts + renders the rustic config
   docker exec mica-backup-1 rustic init                     # create the repo (once)
   docker exec mica-backup-1 mica-cli backup run                  # first backup now (else it waits for BACKUP_HOUR)
   docker logs -f mica-backup-1
   ```
   > Sharing a bucket with an existing repo? Give this one its own prefix via
   > `OSS_ROOT=<prefix>` in `.env` (else init fails, "config file already exists").

The container runs `mica-cli backup daemon` (export → per-workspace snapshot → forget
--prune) at `BACKUP_HOUR` daily and once on (re)start; the staging tree
lives in the `mica-prod-backup` volume.

## Daily operations (init / backup / inspect / restore)

Run against the **running** backup container with `docker exec` — it already has
the repo config (rendered from env on start), so plain `rustic` works. (Don't use
`docker compose run`; its `depends_on` would recreate the `api` container.)

```bash
# Initialize the repo (ONE time, right after first `up -d`):
docker exec mica-backup-1 rustic init

# Run a backup right now (what the daily loop calls; export → snapshot → forget):
docker exec mica-backup-1 mica-cli backup run

# List snapshots grouped by workspace (label = workspace id, tag ws=<name>):
docker exec mica-backup-1 rustic snapshots --group-by label

# Verify repository integrity (also run by `just restore-drill`, see 恢复演练 below):
docker exec mica-backup-1 rustic check

# Restore one workspace's latest snapshot. SNAPSHOT + DESTINATION are POSITIONAL
# (not --target); --filter-label narrows "latest" to that workspace's lineage:
docker exec mica-backup-1 rustic restore latest /tmp/restore --filter-label <workspace-id>
docker cp mica-backup-1:/tmp/restore ./restore     # then: docker exec mica-backup-1 rm -rf /tmp/restore
```

Find a workspace's id (the `--label`/`--filter-label` value) in `rustic snapshots
--group-by label` (the `Label` column, tagged `ws=<name>`), or in the export's
`manifest.json`.

To put content back into a Mica instance, re-import each workspace's tree (the
app's Import / the `/api/workspaces/import` endpoint). Recreate user accounts
separately — they are not part of a content backup.

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
(`docs/lessons.md` §"迁移怎么验"): a restore you've never run is a guess, and a
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
(`docs/lessons.md` §"迁移怎么验": `CREATE DATABASE mica_restest` → `zcat … | psql`
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
