# Deployment

Two compose files ship in `deploy/`:

- **`docker-compose.yml`** — the canonical production stack, behind an existing
  **Traefik** (label routing + Let's Encrypt, HTTPS, no host ports). This is what
  runs at mica.cloudcele.com; see [Behind Traefik](#behind-traefik-the-canonical-production-stack).
- **`docker-compose.single.yml`** — a simpler **single-server** variant (nginx on
  port 80, no Traefik), documented in the next section.

## Single server (IP + port 80)

nginx serves the Flutter bundle and reverse-proxies `/api` + `/ws` to the Rust
backend; PostgreSQL and RustFS run alongside. Ships via
`deploy/docker-compose.single.yml`.

```
browser ── :80 nginx ──┬── /            static Flutter bundle (deploy/web)
                       ├── /api/...     → api:8080 (REST)
                       └── /ws/...      → api:8080 (WebSocket, upgraded)
        ── :9000 RustFS  (presigned PUT/GET straight from the browser)
```

Public ports: **80** (app) and **9000** (S3 presigned access). Postgres is
reachable only inside the compose network.

## First deployment

```bash
cp deploy/.env.prod.example .env.prod
vi .env.prod          # SERVER_IP + strong JWT_SECRET / passwords (openssl rand -hex 32)
./deploy/deploy.sh    # builds web bundle + api image, starts the stack
```

Then open `http://<SERVER_IP>/`. Database migrations are embedded in the
binary and run automatically at startup.

> Local dev (host-run api on :8080, python http.server on :8090) collides with
> the prod stack on :9000 — stop it first on a shared machine:
> `pkill -x mica-api-server; docker compose down`.

## Upgrades

```bash
git pull
./deploy/deploy.sh              # full: web + api image + restart
./deploy/deploy.sh --web-only   # frontend-only change: atomic bundle swap, no restart
```

`index.html` / the service worker are served with `no-cache`, so a plain
reload picks up new releases (asset files are content-hashed).

## Why it's wired this way

- **Same-origin API.** The web bundle auto-targets the page's own origin
  when served from port 80/443 (`_resolveBaseUri`), so one build works on
  any IP — no per-server `--dart-define`, and no cross-origin request in
  production. The server therefore denies cross-origin browser reads by
  default (production); set `CORS_ALLOWED_ORIGINS` (comma-separated) only if a
  separate origin must reach the API.
- **`S3_ENDPOINT` must be browser-reachable** (`http://<SERVER_IP>:9000`):
  presigned URLs embed that host; an internal hostname would break every
  image. `S3_PUBLIC_BASE_URL` stays unset — the bucket is private and GETs
  are signed (see docs/export-import.md).
- **`client_max_body_size 1g`** on nginx matches the server-side import's
  body limit (whole-workspace ZIP uploads in one request).
- **WebSocket upgrade headers** on `/ws/` are required for realtime
  collaboration; without them rooms silently fall back to errors.
- RustFS CORS is pinned to the app origin (`http://SERVER_IP`), no longer
  `*` as in dev.

## Data & backups

| Data | Where | Backup |
|---|---|---|
| Documents, users, files index | volume `mica-prod-postgres` | `docker compose -f deploy/docker-compose.single.yml exec postgres pg_dump -U mica mica > backup.sql` |
| Image bytes | volume `mica-prod-rustfs` | snapshot the volume directory |

The canonical stack also ships an off-site, encrypted, deduplicated backup of
both content AND a `pg_dump` of Postgres — see [`backup.md`](backup.md).

## Deleting a user account (cascade order — memo, not implemented)

There is **no "delete account" today**, and it will not be a one-liner: `users` is
pinned by `ON DELETE RESTRICT` (plus one `NO ACTION`) foreign keys across the
schema, so a bare `DELETE FROM users` aborts on the first referencing row. Whoever
builds it must tear the references down in order. This memo is the FK map so that
work doesn't start by re-deriving it (sources: `migrations/0001_initial.sql`,
`0003`, `0004`, `0006`, `0008`).

**Blocks the delete — must be reassigned or removed first:**

| Table.column | On delete | Why it pins the user |
|---|---|---|
| `workspaces.owner_id` | RESTRICT | user owns the workspace |
| `documents.created_by` | RESTRICT | user created the doc |
| `views.created_by` | RESTRICT | user created the page/folder view |
| `files.uploaded_by` | RESTRICT | user uploaded the blob |
| `document_versions.created_by` | RESTRICT | user named a version |
| `document_shares.created_by` | RESTRICT | user made the public link |
| `document_updates.actor_id` | RESTRICT | user authored an op (op model) |
| `workspace_updates.actor_id` | NO ACTION | user authored a yrs update (CRDT model) — no `ON DELETE` clause, still blocks |

**Auto-cleared by CASCADE when the user row goes:** `workspace_members.user_id`,
`api_tokens.user_id`, `refresh_tokens.user_id`.

**Not a FK at all:** `document_yrs_versions.created_by` is a plain `uuid` (NULL for
system/auto) — neither blocks nor cascades; leave it or scrub it to NULL.

### Order to actually delete

1. **Workspaces the user OWNS.** Either **transfer ownership**
   (`UPDATE workspaces SET owner_id = <existing-member>`) or **delete the whole
   workspace**. Deleting a workspace CASCADEs everything scoped by `workspace_id` —
   documents, views, files, document_updates, workspace_updates, document_shares,
   and (via documents) snapshots/versions — in one shot, clearing most RESTRICT
   rows above **for that workspace**.
2. **⚠️ Deleting a workspace leaks S3/RustFS objects.** The `files` rows cascade in
   the DB, but the **object bytes** they key (`files.object_key`) do **not** —
   there is no DB→S3 hook. Enumerate the workspace's `object_key`s and delete them
   from the bucket before/after the workspace delete, else they orphan forever: the
   blob-GC sweep (`migrations/0007`) only reclaims blobs unreferenced by a *live*
   document, and a hard-deleted workspace's files never get swept.
3. **Rows the user created in OTHER people's workspaces.** A member who created a
   doc/view/upload/share/op/version in a workspace they don't own still pins `users`
   via those RESTRICT rows. Reassign `created_by`/`actor_id` to a **tombstone
   ("deleted user") account** (keeps history/attribution, least destructive) or to
   the workspace owner, or delete those specific rows. Choosing the tombstone
   policy is the real design decision here.
4. **Then the user row.** With every RESTRICT/NO ACTION reference cleared,
   `DELETE FROM users WHERE id = <uid>` succeeds and cascades `workspace_members` /
   `api_tokens` / `refresh_tokens` automatically.

Do it in **one transaction** and dry-run the final `DELETE FROM users` on a scratch
db first (`docs/lessons.md` §"迁移怎么验") — if any RESTRICT row was missed it
aborts there, rather than leaving a half-deleted account.

## PostgreSQL major-version upgrades

**Never bump the `postgres:` tag in place** (e.g. `16-alpine` → `17-alpine` on the
same volume). A new major refuses to start on the old major's data directory —
it exits with `database files are incompatible with server` and the container
crash-loops. There is no `restart: unless-stopped` around that; the fix is a
dump-and-load onto a fresh volume:

```bash
cd /data/mica
# 0) Take a restore point FIRST (deploy-prod does NOT back up before migrating):
docker exec mica-postgres-1 pg_dump -U mica -d mica | gzip > pg16.sql.gz
gzip -t pg16.sql.gz && zcat pg16.sql.gz | grep -c '^COPY public.' # sanity: tables present

# 1) Stop writers, keep old postgres up to read from.
docker compose stop api

# 2) Point the postgres service at a NEW image tag AND a NEW named volume
#    (edit docker-compose.yml: image: postgres:17-alpine, and rename the volume
#    e.g. mica-prod-postgres17 so the old data dir is untouched), then:
docker compose up -d --no-deps postgres          # new empty PG17 volume boots clean

# 3) Load the dump into the new server.
zcat pg16.sql.gz | docker exec -i mica-postgres-1 psql -q -U mica -d mica

# 4) Bring the api back and verify DB-backed readiness.
docker compose up -d --no-deps api
curl -fsS https://mica.cloudcele.com/api/ready
```

Keep the old `mica-prod-postgres` volume until the new one is proven — that
un-renamed volume IS your rollback (revert the compose edit to fall back). Only
`docker volume rm` it once `/api/ready` is green and content spot-checks pass.

There's no rush: **PG16 gets upstream support until ~2028** (five-year cycle), so
this is a deliberate maintenance task, never a casual `image:` tag bump.

## Behind Traefik (the canonical production stack)

`deploy/docker-compose.yml` — used for mica.cloudcele.com. No host
ports; Traefik (label routing, `letsencrypt` certresolver) terminates TLS
for both the app (`DOMAIN`) and RustFS (`S3_DOMAIN`, e.g.
`s3.mica.cloudcele.com` — needs its own DNS A record; presigned URLs embed
it and SigV4 survives the proxy because Traefik forwards Host unchanged).
Ship images by `docker save | scp | docker load` when the server can't
reach Docker Hub. First boot: create the bucket once
(`docker exec mica-rustfs-1 mkdir -p /data/<bucket>` + restart) — RustFS is
filesystem-backed. If the ACME cert stays on TRAEFIK DEFAULT CERT after a
DNS change, restart Traefik to clear its issuance backoff. The API must
bind `HTTP_ADDR=0.0.0.0:8080` in containers (compose files set it).

## Moving to a domain + HTTPS later

1. Point DNS at the server; put Caddy (auto-TLS) or nginx+certbot in front
   of port 80/443.
2. `.env.prod`: `SERVER_IP=app.example.com` and switch the two `http://`
   references for S3/CORS to `https://` (compose file).
3. Rebuild nothing client-side — same-origin resolution adapts.
