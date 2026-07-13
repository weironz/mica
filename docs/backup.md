# Backup & Restore

**Mica does not do backups — on purpose (mechanism vs policy).** Mica's job is a
great, consistent **`export`**; backup — encryption, dedup, retention, remote
targets — is the job of a dedicated tool. The production stack ships a small
container that wires the two together: `mica-cli export` (a thin API client, no
backup engine) + **`rustic`** (external, restic-format) snapshotting to Aliyun
OSS. `mica-cli` itself has no `backup` command anymore.

## Topology: one repo, one bucket, one lineage per workspace

```
[daily + on start]
  mica-cli export --out /export            # every workspace → Markdown + images (mirrored) + manifest.json
  per workspace (read from manifest.json):
    rustic backup /export/<dir> --label <workspace-id> --tag ws=<name> --tag mica
  rustic forget --group-by label --keep-daily 7 --keep-weekly 4 --keep-monthly 6 --prune
```

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
- **Not** a full-instance disaster-recovery image: it does not capture users /
  passwords / memberships, nor the fine-grained CRDT edit history. For turnkey
  whole-instance DR, also take a `pg_dump` of the Mica Postgres alongside this
  (the two are complementary).

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
   # optional: BACKUP_HOUR=3  KEEP_DAILY=7  KEEP_WEEKLY=4  KEEP_MONTHLY=6
   ```
3. **Start the service, initialise the repo once, then trigger the first run.**
   On start the container renders `/etc/rustic/rustic.toml` from the `OSS_*` env,
   so plain `rustic` works via `docker exec` (no wrapper). `--no-deps` avoids
   recreating the `api` container.
   ```bash
   docker compose --profile backup up -d --no-deps backup   # starts + renders the rustic config
   docker exec mica-backup-1 rustic init                     # create the repo (once)
   docker exec mica-backup-1 mica-backup.sh                  # first backup now (else it waits for BACKUP_HOUR)
   docker logs -f mica-backup-1
   ```
   > Sharing a bucket with an existing repo? Give this one its own prefix via
   > `OSS_ROOT=<prefix>` in `.env` (else init fails, "config file already exists").

The container runs `mica-backup.sh` (export → per-workspace snapshot → forget
--prune) at `${BACKUP_HOUR}:00` daily and once on (re)start; the staging tree
lives in the `mica-prod-backup` volume.

## Daily operations (init / backup / inspect / restore)

Run against the **running** backup container with `docker exec` — it already has
the repo config (rendered from env on start), so plain `rustic` works. (Don't use
`docker compose run`; its `depends_on` would recreate the `api` container.)

```bash
# Initialize the repo (ONE time, right after first `up -d`):
docker exec mica-backup-1 rustic init

# Run a backup right now (what the daily loop calls; export → snapshot → forget):
docker exec mica-backup-1 mica-backup.sh

# List snapshots grouped by workspace (label = workspace id, tag ws=<name>):
docker exec mica-backup-1 rustic snapshots --group-by label

# Verify repository integrity:
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
