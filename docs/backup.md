# Backup & Restore

Mica ships its own backup: `mica-cli export` projects every workspace to a
Markdown + images tree, and `mica-backup` (an embedded, restic-format,
encrypted/deduplicated engine — `rustic_core`) snapshots that tree to a local
repo and/or Aliyun OSS. No external `restic`/`backrest`/`cron` binary.

```
[timer] mica-cli export --out <dir>        # content: all workspaces → md + images
        mica-backup snapshot --path <dir>   # encrypt + dedup + incremental → repo
        mica-backup forget --keep-… --prune # retention
```

## What this is (and isn't)

- **Content backup** — Markdown source + referenced images for every workspace,
  portable and human-readable (restorable into Mica, or openable in Obsidian etc.).
- **Not** a full-instance disaster-recovery image. It does **not** capture users
  / passwords / memberships, nor the fine-grained CRDT edit history — restore
  re-imports the content into a fresh instance and produces fresh document state.
  If you need turnkey whole-instance DR, also take a `pg_dump` of the Mica
  Postgres alongside this (the two are complementary).

## Build & install

`mica-cli` is light; the `mica-backup` engine is behind the `backup` feature so
nothing else (api-server, the default `mica-cli`) pulls its dependency tree.

```bash
cargo build -p mica-cli --release                                   # → target/release/mica-cli
cargo build -p mica-cli --release --features backup --bin mica-backup  # → target/release/mica-backup
sudo install -m755 target/release/mica-cli target/release/mica-backup /usr/local/bin/
sudo install -m755 deploy/mica-backup.sh /usr/local/bin/
```

`mica-backup` needs rustc ≥ 1.88; the rest of the workspace stays at 1.85.

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

## Configure

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
mica-backup init                  # creates the encrypted repo in the bucket
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
mica-backup snapshots                 # list (add --json for scripts/agents)
mica-backup check                     # verify repository integrity
mica-backup snapshot --path /var/lib/mica/export --tag adhoc
```

## Restore

```bash
set -a; . /etc/mica/backup.env; set +a
mica-backup restore --snapshot latest --target /tmp/mica-restore
# → /tmp/mica-restore/<...>/<workspace>/<page>.md  + assets/  + manifest.json
```

To put the content back into a Mica instance, re-import each workspace's tree
(the `/api/workspaces/import` endpoint / the app's Import). Recreate user
accounts separately — they are not part of a content backup.

## Restore test (do this — a backup you've never restored is a guess)

Periodically restore `latest` into a throwaway dir and assert the content is
there and byte-intact:

```bash
mica-backup restore --snapshot latest --target /tmp/rt && \
  find /tmp/rt -name '*.md' | head && \
  mica-backup check --json
rm -rf /tmp/rt
```

## Security

- The **repo password** and the **RAM AccessKey** are the only unrecoverable
  secrets — store them **off this host** (password manager + a sealed copy).
  Lose the password and the encrypted repo is permanently unreadable.
- Keep `/etc/mica/backup.env` root-600. Rotate the RAM key periodically.
- The local staging dir (`/var/lib/mica/export`) and, if used, a local repo hold
  plaintext content — they share the host's trust boundary (same as the DB).
  OSS is the real off-site copy; restic encrypts before upload.
