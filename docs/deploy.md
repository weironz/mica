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
mkdir -p /data/mica && cd /data/mica

# The two files the server needs. Pinned to a RELEASE TAG, not `main`: the
# compose file and the images it pulls (MICA_VERSION) have to be the same
# generation, and `main` can be ahead of the newest release.
curl -fsSLO https://raw.githubusercontent.com/weironz/mica/v0.13.15/deploy/docker-compose.single.yml
curl -fsSL  https://raw.githubusercontent.com/weironz/mica/v0.13.15/deploy/.env.prod.example -o .env.prod

vi .env.prod          # Two lines to edit by hand:
                      #   SERVER_IP     — UNCOMMENT it; the address BROWSERS use
                      #   MICA_VERSION  — ships EMPTY on purpose; pick a release
                      #                   from github.com/weironz/mica/releases
                      # `compose config` refuses to resolve until both are set.

docker compose --env-file .env.prod -f docker-compose.single.yml up -d
```

> **The object-store credentials default to a value published in this
> repository**, and `:9000` is internet-facing. That is what makes the block
> above two lines instead of five — but on any node strangers can reach, set
> `S3_ACCESS_KEY` and `S3_SECRET_KEY` before the first start, or anyone who
> reads the repo can read and write your files. The api warns about it in the
> log on every production start; see
> [Secrets](#secrets-what-you-generate-and-what-generates-itself).

Already have the repo checked out on the server? Then it is just
`cp deploy/.env.prod.example .env.prod` and point `-f` at
`deploy/docker-compose.single.yml` — but a checkout is not required, and that is
the point: the server needs Docker and nothing else.

Nothing is built: `mica-api` and `mica-web` are pulled, and the web image
already carries nginx, its config and the Flutter bundle. The server needs
Docker and nothing else — no Rust toolchain, no Flutter SDK.

Then open `http://<SERVER_IP>/`. Database migrations are embedded in the
binary and run automatically at startup.

> Local dev (host-run api on :8080, python http.server on :8090) collides with
> the prod stack on :9000 — stop it first on a shared machine:
> `pkill -x mica-api-server; docker compose down`.

## Upgrades

```bash
cd /data/mica
vi .env.prod          # bump MICA_VERSION to the release you want
docker compose --env-file .env.prod -f docker-compose.single.yml up -d --pull always
```

No `git pull` — the compose file is the only repo file this stack needs, and
the version it runs is the tag in `.env.prod`, not whatever your checkout is at.

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
- **`S3_INTERNAL_ENDPOINT` is the same store as the API sees it**
  (`http://rustfs:9000`). Both compose files set it. It exists because the two
  questions have different answers: the browser needs a public address, while
  reaching that same public address from inside the api container may mean
  hairpin NAT, or DNS plus TLS plus a round trip back through Traefik — for a
  service one hop away on the compose network. The server-side callers (bucket
  provisioning, blob GC) use it; browsers never see it. Unset falls back to
  `S3_ENDPOINT`, so an existing deployment behaves as before.
- **`client_max_body_size 1g`** on nginx matches the server-side import's
  body limit (whole-workspace ZIP uploads in one request).
- **WebSocket upgrade headers** on `/ws/` are required for realtime
  collaboration; without them rooms silently fall back to errors.
- RustFS CORS is pinned to the app origin (`http://SERVER_IP`), no longer
  `*` as in dev.
- **The API creates the bucket, over the S3 API.** Nothing else does: RustFS is
  filesystem-backed and creates no bucket on its own, and without one the stack
  comes up entirely healthy and then 404s every upload — the worst shape a
  missing step can have, since nothing looks wrong until someone pastes an
  image. It used to be a manual `docker exec … mkdir` documented only in the
  Traefik section, so a quickstart reader never saw it; then briefly a
  `rustfs-init` compose service, which worked only because RustFS happens to
  store buckets as directories. Both were replaced by `bucket::ensure_bucket`,
  which speaks only S3 and therefore also works against MinIO, Alibaba OSS or
  AWS. It **never fails startup** — see
  [Bucket provisioning](bucket-provisioning-plan.md) for the branch-by-branch
  reasoning and the survey it came from.

## Secrets: what you generate, and what generates itself

An operator can start the stack without setting any credential. Two of the three
are safe that way for reasons that hold on their own; the third is a deliberate
trade, and it is the one to read.

| Credential | Default | Safe to leave? |
| --- | --- | --- |
| `JWT_SECRET` | **none — the server mints its own** | Yes. There is no published value to leak; every install's key differs. |
| `POSTGRES_PASSWORD` | `mica` | Yes. Neither compose file publishes a postgres port, so the database is reachable only from the other containers on the stack's own network. |
| `S3_ACCESS_KEY` / `S3_SECRET_KEY` | `mica` / `mica-default-not-a-secret` | **No, not on a public node.** RustFS serves `:9000` to the internet on purpose — browsers presign against it directly — so anyone who reads this repository can read and write the files of any install that kept the default. |

The last row is the honest cost of a two-line quickstart: it removes the last
thing an operator had to generate, and in exchange a node left alone has a
bucket the internet can write. The api therefore warns on every production
start while the default is in use — a risk that lives only in documentation is
one nobody meets until it matters:

```
S3_SECRET_KEY is the published default — anyone can read and write this
instance's files over :9000. Set S3_ACCESS_KEY and S3_SECRET_KEY in
.env.prod (openssl rand -hex 32) and restart.
```

Set both before the **first** start if you are going to set them. Changing them
afterwards means reconciling what RustFS already stored under the old
credentials, which is far easier on an empty volume.

### How the signing key mints itself

On startup the api runs `ensure_jwt_secret` (`crates/infra/src/db.rs`) inside a
transaction against `server_secrets`, a one-row table added by migration 0020:

1. `JWT_SECRET` set in the environment → validated as before (32+ characters, no
   template placeholder) and used as-is. Nothing is written to the database.
2. Not set, no row yet → generate 32 random bytes, store the hex, use it.
3. Not set, row exists → reuse it.

Step 3 is why sessions survive `docker compose up -d --pull always`: the key
lives with the data, not with the process.

**Why the database and not a file.** Two constraints, both from the deployment
shape rather than from taste. The api container mounts no volume, so a file
would be recreated on every image upgrade — silently logging out every user.
And the Traefik stack can run more than one api replica; two replicas writing
two files would each reject the other's tokens. One row is shared by
construction.

**Why this is not the `change-me` mistake again.** No default is published.
The key is generated on your machine, differs on every install, and cannot be
looked up in this repository. The old hole was shipping a working constant; the
fix is shipping nothing at all. Gitea, Vaultwarden and AFFiNE all treat the
signing key as the program's responsibility for the same reason — though Gitea
shows the cost of getting there late: with no `SECRET_KEY` set it still falls
back to a hardcoded constant, because it cannot rotate without breaking
existing installs.

**Rotation.** Clear the stored row and restart; every existing access token
stops verifying, which is the point.

```sh
docker exec mica-postgres-1 psql -U mica -d mica -c "TRUNCATE server_secrets;"
docker compose --env-file .env.prod -f docker-compose.single.yml restart api
```

**Upgrading an existing install.** Nothing to do — a `JWT_SECRET` already in
your `.env.prod` keeps working unchanged. Do **not** remove `POSTGRES_PASSWORD`
from an `.env.prod` that already has one: the volume keeps whatever password
`initdb` ran with, so dropping the line points the api at `mica` while the
database still expects the old value. Changing it later requires an
`ALTER USER` by hand.

## Data & backups

| Data | Where | Backup |
|---|---|---|
| Documents, users, files index | volume `mica-prod-postgres` | `docker compose -f docker-compose.single.yml exec postgres pg_dump -U mica mica > backup.sql` |
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
reach Docker Hub. (Creating the bucket is no longer a manual first-boot step —
the API creates it over the S3 API on startup; see below.) If the ACME cert stays on
TRAEFIK DEFAULT CERT after a
DNS change, restart Traefik to clear its issuance backoff. The API must
bind `HTTP_ADDR=0.0.0.0:8080` in containers (compose files set it).

## Moving to a domain + HTTPS later

1. Point DNS at the server; put Caddy (auto-TLS) or nginx+certbot in front
   of port 80/443.
2. `.env.prod`: `SERVER_IP=app.example.com` and switch the two `http://`
   references for S3/CORS to `https://` (compose file).
3. Rebuild nothing client-side — same-origin resolution adapts.
