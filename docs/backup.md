# Backup & Restore

> **Direction (2026-07): backup is moving external.** Baking a restic engine
> (`rustic_core`) into `mica-cli` was a mistake — it couples us to one backup
> tool, pulls a heavy dep tree, and re-invents what mature tools do better. The
> new model separates mechanism from policy: **Mica's job is a great, consistent
> `export`/`import` (API + CLI); backup is the job of dedicated tools** (restic,
> borg, rclone, cron) pointed at the export. Markdown exports are text, so they
> dedup and diff beautifully. See `crates/mcp-server/README.md#backup`. The
> embedded `mica-cli backup` (rustic) below is **deprecated**; retirement is
> **staged** — keep the running prod backup until the external replacement is
> verified. New setups should use `mica-cli export` + an external backup tool.

Mica ships its own backup as part of the `mica-cli` tool: `mica-cli export`
projects every workspace to a Markdown + images tree, and `mica-cli backup`
(an embedded, restic-format, encrypted/deduplicated engine — `rustic_core`)
snapshots that tree to a local repo and/or Aliyun OSS. No external
`restic`/`backrest`/`cron` binary.

```
[timer] mica-cli export --out <dir>              # content: all workspaces → md + images
        mica-cli backup snapshot --path <dir>     # encrypt + dedup + incremental → repo
        mica-cli backup forget --keep-… --prune   # retention
```

The `backup` subcommand is compiled in **by default**. It carries a large
dependency tree (`rustic_core` + OpenDAL); a plain `cargo build -p mica-cli`
includes it, and `--no-default-features` builds a light client without it. The
**api-server is a separate crate and never pulls any of this**, so the
production server binary stays lean regardless.

## What this is (and isn't)

- **Content backup** — Markdown source + referenced images for every workspace,
  portable and human-readable (restorable into Mica, or openable in Obsidian etc.).
- **Not** a full-instance disaster-recovery image. It does **not** capture users
  / passwords / memberships, nor the fine-grained CRDT edit history — restore
  re-imports the content into a fresh instance and produces fresh document state.
  If you need turnkey whole-instance DR, also take a `pg_dump` of the Mica
  Postgres alongside this (the two are complementary).

## Build & install

```bash
cargo build -p mica-cli --release          # → target/release/mica-cli (backup built in)
# light client, no backup / no rustic:   cargo build -p mica-cli --release --no-default-features
sudo install -m755 target/release/mica-cli /usr/local/bin/
sudo install -m755 deploy/mica-backup.sh   /usr/local/bin/
```

The backup subcommand needs rustc ≥ 1.88; the rest of the workspace stays at 1.85.

## Aliyun OSS setup (one-time)

1. **Create a bucket** (e.g. `mica-backups`), ideally in a **different region**
   than the Mica ECS for geographic isolation.
2. **Create a least-privilege RAM AccessKey** scoped to *only* that bucket —
   `oss:PutObject / GetObject / DeleteObject / ListObjects` + `oss:GetBucket*`.
   Do **not** reuse the app's rustfs S3 keys or an account-wide key.
3. **Enable Object Versioning / Object-Lock** on the bucket so a buggy prune or a
   compromised key can't erase history.

> **OSS gotcha:** OSS only serves virtual-host-style addressing, so
> `enable_virtual_host_style=true` is **required** — path style fails with
> "Path `config` does not exist".

## Deploy inside the Docker stack (recommended)

The production compose ships a `backup` service — mica-cli in a container that
runs the daily backup over the **internal** network (`MICA_SERVER=http://api:8080`
is hard-wired), so the token never leaves the host. It is **opt-in** behind the
`backup` profile; a plain `docker compose up -d` ignores it.

1. **Build & load the image** (the node can't pull from Docker Hub):
   ```bash
   docker build -f deploy/Dockerfile.cli -t willdockerhub/mica-cli:v0.3 .
   docker save willdockerhub/mica-cli:v0.3 | gzip | ssh root@<node> 'gunzip | docker load'
   ```
2. **Mint a read-scoped token** — app *Settings → API Tokens*, or
   `mica-cli auth token create --name backup --scope read` — and add it plus the
   repo/OSS config to the node's `.env` (next to `MICA_VERSION`). The compose file
   maps each `OSS_*` var to one `MICA_OPT_<KEY>` backend option, so every value
   lives on its own line (rotate the AccessKey without touching the rest):
   ```
   MICA_BACKUP_TOKEN=mica_pat_…
   MICA_BACKUP_REPO=opendal:s3:/mica
   MICA_BACKUP_PASSWORD=…                 # SAVE THIS OFF-HOST — lose it, lose the repo
   OSS_BUCKET=my-mica-backups
   OSS_ENDPOINT=https://oss-cn-hangzhou.aliyuncs.com
   OSS_REGION=oss-cn-hangzhou
   OSS_ACCESS_KEY_ID=…
   OSS_SECRET_ACCESS_KEY=…
   OSS_ROOT=mica                          # object-key prefix; use a fresh one to share a bucket
   # optional: BACKUP_HOUR=3  KEEP_DAILY=14  KEEP_WEEKLY=8  KEEP_MONTHLY=6
   ```
   (`enable_virtual_host_style=true`, required for OSS, is a literal in the
   compose. The systemd path below takes the same `MICA_OPT_<KEY>` vars — or a
   single packed `MICA_BACKUP_OPTS` string if you prefer.)
3. **Initialise the repo once, then start the service:**
   ```bash
   # One-off mica-cli commands must override the daily-loop ENTRYPOINT:
   docker compose --profile backup run --rm --entrypoint mica-cli backup backup init
   docker compose --profile backup up -d backup
   docker compose logs -f backup      # BACKUP_ON_START=1 → the first run happens now
   # inspect later: … run --rm --entrypoint mica-cli backup backup snapshots
   ```
   > Sharing a bucket with an existing repo? Give this one its own prefix via
   > `OSS_ROOT=<prefix>` in `.env` (else init fails with "Config file already
   > exists").

The container runs `mica-backup.sh` (export → snapshot → forget --prune) at
`${BACKUP_HOUR}:00` daily and once on (re)start; the staging dir lives in the
`mica-prod-backup` volume. A **read** token suffices — the export is all GETs.

The systemd-timer setup below is the alternative for hosts **not** running the
Docker stack.

## Configure (systemd host, alternative)

Copy `deploy/backup.env.example` → `/etc/mica/backup.env`, fill it in, lock it down:

```bash
sudo mkdir -p /etc/mica /var/lib/mica
sudo cp deploy/backup.env.example /etc/mica/backup.env
sudo vi /etc/mica/backup.env      # MICA_TOKEN, MICA_BACKUP_PASSWORD, MICA_BACKUP_OPTS (OSS creds)
sudo chmod 600 /etc/mica/backup.env
```

All backend config — including the OSS AccessKey — goes in `MICA_BACKUP_OPTS`
(read from the env), so credentials never appear on the command line / in `ps`.

## Initialize the repo (one-time)

```bash
set -a; . /etc/mica/backup.env; set +a
mica-cli backup init              # creates the encrypted repo in the bucket
```

## Schedule it

```bash
sudo cp deploy/mica-backup.service deploy/mica-backup.timer /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now mica-backup.timer
systemctl list-timers mica-backup.timer     # confirm next run
sudo systemctl start mica-backup.service     # run once now; check `journalctl -u mica-backup`
```

The timer runs `mica-backup.sh` daily at 03:00: export → snapshot → forget+prune.

## Manual use / inspection

```bash
set -a; . /etc/mica/backup.env; set +a
mica-cli backup snapshots              # list (add --json for scripts/agents)
mica-cli backup check                  # verify repository integrity
mica-cli backup snapshot --path /var/lib/mica/export --tag adhoc
```

## Restore

```bash
set -a; . /etc/mica/backup.env; set +a
mica-cli backup restore --snapshot latest --target /tmp/mica-restore
# → /tmp/mica-restore/<...>/<workspace>/<page>.md  + assets/  + manifest.json
```

To put the content back into a Mica instance, re-import each workspace's tree
(the `/api/workspaces/import` endpoint / the app's Import). Recreate user
accounts separately — they are not part of a content backup.

## Restore test (do this — a backup you've never restored is a guess)

Periodically restore `latest` into a throwaway dir and assert the content is
there and byte-intact:

```bash
mica-cli backup restore --snapshot latest --target /tmp/rt && \
  find /tmp/rt -name '*.md' | head && \
  mica-cli backup check --json
rm -rf /tmp/rt
```

## Security

- The **repo password** and the **RAM AccessKey** are the only unrecoverable
  secrets — store them **off this host** (password manager + a sealed copy).
  Lose the password and the encrypted repo is permanently unreadable.
- Keep `/etc/mica/backup.env` root-600. Rotate the RAM key periodically.
- The local staging dir (`/var/lib/mica/export`) and, if used, a local repo hold
  plaintext content — they share the host's trust boundary (same as the DB).
  OSS is the real off-site copy; the engine encrypts before upload.
