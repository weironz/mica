# Deployment

Two compose files ship in `deploy/`:

- **`docker-compose.yml`** — the canonical production stack, behind an existing
  **Traefik** (label routing + Let's Encrypt, HTTPS, no host ports). This is what
  runs at mica.cloudcele.com; see [Behind Traefik](#behind-traefik-the-canonical-production-stack).
- **`docker-compose.single.yml`** — a simpler **single-server** variant (nginx on
  port 80, no Traefik), documented in the next section.

## Working-set protection on a shared Linux host

On systemd hosts using cgroup v2, Mica can use its own root-level slice so
ordinary navigation does not lose its working set while other workloads are
reclaiming memory. This is optional and does not change the images, database
format, memory hard limits, or host VM settings.

The Compose files expose `MICA_CGROUP_PARENT` and soft reservations for API
(192 MiB), PostgreSQL (256 MiB), RustFS (256 MiB), and Web (16 MiB). The supplied
`deploy/mica.slice` protects up to 720 MiB in aggregate. This is best effort
reclaim protection, not an allocation. A zero-protection ancestor can limit
effective protection even when the container's own `memory.low` is nonzero;
see the [kernel's cgroup v2 documentation](https://www.kernel.org/doc/html/latest/admin-guide/cgroup-v2.html#memory-interface-files).

From a checkout, install and start the Mica-only unit before opting in:

```sh
sudo install -m 644 deploy/mica.slice /etc/systemd/system/mica.slice
sudo systemctl daemon-reload
sudo systemctl enable --now mica.slice
```

Set `MICA_CGROUP_PARENT=mica.slice` in the environment file used by the stack.
On the managed production stack, operator overrides belong in `.env.secrets`
because Ansible rewrites `.env` during deployment. Validate with `./dc config
--quiet`, then recreate only `postgres rustfs api web` using `./dc up -d
--no-deps postgres rustfs api web`. On a quickstart stack, use its normal
`docker compose --env-file ... -f ...` prefix instead. Changing cgroup parent
requires recreation, so schedule the Mica interruption; volumes and image
versions must remain unchanged. The backup sidecar stays outside the protected
slice so a batch export cannot displace the interactive services within it.

Verify the actual container `HostConfig.CgroupParent`, each leaf's `memory.low`,
and `/sys/fs/cgroup/mica.slice/memory.low`, then test create, directory loads,
document opens, and search after idle. Merely setting `mem_reservation` does
not prove effective residency. Keep the slice's `MemoryLow` consistent with
the intended aggregate protection when tuning individual reservations.

Record existing runtime-only hard limits before recreation: Docker does not
carry a manual `docker update --memory` setting into a new container. Persist
PostgreSQL/RustFS caps as `MICA_POSTGRES_MEMORY_LIMIT` and
`MICA_RUSTFS_MEMORY_LIMIT`; both default to `0` (unlimited, matching previous
Compose defaults). A nonzero cap must be at least that service's reservation.
两份 Compose 当前对 PostgreSQL / RustFS 固定使用 YAML 整数
`memswap_limit: -1`（允许无限使用宿主机可用 swap），**没有**
`MICA_*_MEMORYSWAP_LIMIT` 环境变量入口；旧版生产 Compose 不接受插值后的字符串
`"-1"`。这是 2026-10-23 排查时新增的策略，改前没有采集原 `MemorySwap`，不能说
前后策略相同。调整前应记录实际硬上限、swap 上限与占用，不要仅凭 swap 已用量归因。

To undo, restore the previous environment and Compose configuration, recreate
only the same four Mica services, then disable the empty `mica.slice` unit.
Back up configuration first and preserve existing secrets and hard limits.
The [2026-10-23 investigation](../performance/general-latency-2026-10-23.md)
records the production measurements and limitations.

## 数据库连接与容器 DNS

生产事故证据与处理记录见 [database-dns-2026-10-24.md](database-dns-2026-10-24.md)。

`MICA_DATABASE_HOST` 只用于两份 Compose 中 API 的 `DATABASE_URL` 主机部分，默认仍为
`postgres`；不改变凭据、端口或数据库。共享宿主机 Docker 内置 DNS 失效时，API 新建
数据库连接可能出现 `Temporary failure in name resolution`，从而让切换、搜索、跨工作区
移动后的刷新都报数据库错误。这不等于 SQLite 损坏或移动事务一定失败，先核对页面位置。

2026-10-24 在生产备份后，仅为当时的 Mica API 加了数据库 hosts 映射，无需重启即可
恢复只读查询。生产现在使用 `ansible/host_vars/mica-prod.yml` 中的 `postgres` 服务名，
由 Docker 服务发现跟随数据库容器地址变化；Ansible 在写文件前核对运行状态和服务别名。
需要临时绕过 DNS 时，仍可显式配置经过核实的 IP，或使用部署时解析地址的 `auto`。
显式 IP 与当前容器地址不匹配会拒绝部署。普通部署不重建 PostgreSQL、不调整全局
Docker DNS，也不修改其他项目。其他自托管环境默认继续使用 Compose 服务发现。

0.13.67 的部署失败已在独立数据库上确认是 SQL 文件 CRLF/LF 校验和差异，服务名和 IP
两种连接均在迁移校验阶段失败；见 [故障记录](release-0.13.67-failure.md)。

连接 I/O 故障或连接池不可用返回脱敏的 HTTP 503，真正的 SQL/迁移错误仍是 HTTP 500。
不自动重试写操作：连接中断后要先刷新核对结果，避免重复移动或重复写入。

`scripts/test-compose-api-env.py` 检查两份 Compose 的默认/指定主机与凭据不变；在
Ansible 控制端运行 `python ansible/tests/test_database_host_pin.py`，可验证实际部署的
预检任务接受正确地址，拒绝陈旧地址、停止的数据库和空地址，不连接生产。

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

# Pick ONE release and use it for everything below. The compose file's
# `environment:` block is an explicit ALLOWLIST, so pairing a months-old compose
# file with a newer image means a variable the image expects never reaches the
# process — a setting that silently does nothing. Naming the version once is
# what keeps them the same generation.
RELEASE=0.13.69

# Pinned to a RELEASE TAG, not `main`: `main` can be ahead of the newest release.
curl -fsSLO "https://raw.githubusercontent.com/weironz/mica/v$RELEASE/deploy/docker-compose.single.yml"
curl -fsSL  "https://raw.githubusercontent.com/weironz/mica/v$RELEASE/deploy/.env.prod.example" -o .env.prod

# MICA_VERSION must be the SAME $RELEASE.
sed -i "s/^MICA_VERSION=.*/MICA_VERSION=$RELEASE/" .env.prod
vi .env.prod          # SERVER_IP — the address BROWSERS use.
                      # `compose config` refuses to resolve until it is set.

docker compose --env-file .env.prod -f docker-compose.single.yml up -d
```

> **The object-store credentials default to a value published in this
> repository**, and `:9000` is internet-facing. That is what makes the block
> above two lines instead of five — but on any node strangers can reach, set
> `S3_ACCESS_KEY_ID` and `S3_SECRET_ACCESS_KEY` before the first start, or anyone who
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
vi .env.prod        # bump MICA_VERSION to the release you want
docker compose --env-file .env.prod -f docker-compose.single.yml up -d --pull always
```

No `git pull` — the compose file is the only repo file this stack needs, and
the version it runs is the tag in `.env.prod`, not whatever your checkout is at.

> **Check for a running workspace import first.** Import jobs live in the api's
> memory, so restarting it kills the job AND its record, while leaving the pages
> already written in place — a workspace holding half an archive, with nothing
> saying so and no job id left to ask. An import of a few hundred pages can run
> for a while, especially when the archive links to images this server cannot
> reach. Let it finish, or cancel it deliberately (a cancel at least reports how
> much landed). See [`api.md`](../dev/api.md).

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
  are signed (see docs/features/export-import.md).
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

## Every credential, and who chose its name

Six pairs, and they used to be spelled four different ways, which is a problem
exactly when it is least affordable: during a recovery, deciding which key opens
which thing. As of 2026-08-29 the four that are OURS all read
`*_ACCESS_KEY_ID` / `*_SECRET_ACCESS_KEY`.

| Pair | What it opens | Name chosen by |
| --- | --- | --- |
| `S3_ACCESS_KEY_ID` / `S3_SECRET_ACCESS_KEY` | this instance's own object store (RustFS) | us |
| `RUSTFS_S3_ACCESS_KEY_ID` / `RUSTFS_S3_SECRET_ACCESS_KEY` | the same store, as rclone's *source* when mirroring off-site. Defaults to the pair above — there is only one RustFS and it accepts one pair | us |
| `OSS_ACCESS_KEY_ID` / `OSS_SECRET_ACCESS_KEY` | the off-site **backup bucket** (read the rustic repo, write the object mirror) | us |
| `MICA_MAIL_ACCESS_KEY_ID` / `MICA_MAIL_SECRET_ACCESS_KEY` | Aliyun DirectMail | us |
| `RUSTFS_ACCESS_KEY` / `RUSTFS_SECRET_KEY` | what RustFS itself is configured WITH — compose sets these from `S3_*`; you never set them | **the RustFS image** |
| `ALICLOUD_ACCESS_KEY` / `ALICLOUD_SECRET_KEY` | creating machines/DNS with OpenTofu (`dr/aliyun/`). Never on the node — only on a laptop or in CI | **the OpenTofu provider** |

The last two keep their spelling because renaming them would mean nothing reads
them. That is the whole reason the set is not uniform, and it is worth knowing
rather than rediscovering.

> ## ⚠️ Upgrading from v0.13.41 or earlier: rename three keys FIRST
>
> ```
> S3_ACCESS_KEY               -> S3_ACCESS_KEY_ID
> S3_SECRET_KEY               -> S3_SECRET_ACCESS_KEY
> MICA_MAIL_ACCESS_KEY_SECRET -> MICA_MAIL_SECRET_ACCESS_KEY
> ```
>
> v0.13.42 accepted both spellings; **v0.13.43 reads only the new ones.** An
> instance that still uses the old names does not fail — it falls through to the
> defaults published in `docker-compose.yml`, and RustFS serves `:9000` to the
> internet. **That makes the object store world-readable and world-writable**,
> with nothing but a `default_s3_secret_in_use` line in the api log to say so.
>
> Renaming is safe at any time while on v0.13.42, and safe before upgrading from
> anything older. Values do not change — only the key names.

## Secrets: what you generate, and what generates itself

An operator can start the stack without setting any credential. Two of the three
are safe that way for reasons that hold on their own; the third is a deliberate
trade, and it is the one to read.

| Credential | Default | Safe to leave? |
| --- | --- | --- |
| `JWT_SECRET` | **none — the server mints its own** | Yes. There is no published value to leak; every install's key differs. |
| `POSTGRES_PASSWORD` | `mica` | Yes. Neither compose file publishes a postgres port, so the database is reachable only from the other containers on the stack's own network. |
| `S3_ACCESS_KEY_ID` / `S3_SECRET_ACCESS_KEY` | `mica` / `mica-default-not-a-secret` | **No, not on a public node.** RustFS serves `:9000` to the internet on purpose — browsers presign against it directly — so anyone who reads this repository can read and write the files of any install that kept the default. |

The last row is the honest cost of a two-line quickstart: it removes the last
thing an operator had to generate, and in exchange a node left alone has a
bucket the internet can write. The api therefore warns on every production
start while the default is in use — a risk that lives only in documentation is
one nobody meets until it matters:

```
S3_SECRET_ACCESS_KEY is the published default — anyone can read and write this
instance's files over :9000. Set S3_ACCESS_KEY_ID and S3_SECRET_ACCESS_KEY in the
env file this stack was started with (openssl rand -hex 32) and restart.
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
your env file keeps working unchanged. Do **not** remove `POSTGRES_PASSWORD`
from an env file that already has one: the volume keeps whatever password
`initdb` ran with, so dropping the line points the api at `mica` while the
database still expects the old value. Changing it later requires an
`ALTER USER` by hand.

## Who administers the instance

The AI provider settings (endpoint, model, API key) are **instance-wide** — one
config shared by everyone, holding the operator's provider key — so only an
admin may change them. Everyone else sees the section read-only.

* **A fresh instance needs nothing.** The first account to sign up gets the flag
  inside the same `INSERT` that creates it, so the person who stands the
  instance up can configure it.
* **An instance that already had accounts** when the flag was introduced
  (migration 0021) got a *guess*: the oldest account. That guess is documented
  here because it was wrong the first time it ran against real data — the oldest
  account was `demo@mica.dev`, a dev seed that had been loaded at some point,
  while the operator had signed up three days later. The operator opened the
  settings dialog and found every field greyed out, with nothing on screen to
  suggest the fix lived in a database row.

  Nothing can infer this reliably. "Owns workspaces" would not have separated
  those two accounts either (both did). So set it explicitly:

  ```
  MICA_ADMIN_EMAIL=you@example.com
  ```

  Applied at every boot, idempotent, and additive — it never revokes anyone, so
  it is safe to leave in `.env`. If it names an address with no account, the
  server logs a warning and carries on.

* **No admin at all?** The server warns at boot (`no account on this instance is
  an admin`). Set `MICA_ADMIN_EMAIL` and restart.

Remember that `deploy/docker-compose.yml`'s `environment:` block is an explicit
allowlist — a variable set in `.env` but absent from that block never reaches
the process.

## Data & backups

| Data | Where | Backup |
|---|---|---|
| Documents, users, files index | volume `mica-prod-postgres` | `docker compose -f docker-compose.single.yml exec postgres pg_dump -U mica mica > backup.sql` |
| Image bytes | volume `mica-prod-rustfs` | snapshot the volume directory |

The canonical stack also ships an off-site, encrypted, deduplicated backup of
both content AND a `pg_dump` of Postgres — see [`backup.md`](backup.md).

## Deleting a user account

`DELETE /api/auth/me` (password-gated) — `auth::delete_user_and_owned`, one
transaction:

1. **Workspaces the user OWNS are deleted**, with everything scoped to them
   (documents, views, files, streams, shares, comments). Their object keys are
   staged in `pending_file_uploads` first and deleted from storage after commit;
   blob GC finishes the job if that fails.
2. **Shares the user created elsewhere are deleted**, and so are their comments
   (0014's "a deleted account takes its comments with it").
3. **The `users` row becomes a tombstone** (migration 0031) instead of being
   deleted: `deleted_at` set, email → `deleted+<id>@deleted.invalid` (so the real
   address can sign up again), empty name and password hash, no avatar, admin
   flag cleared; memberships, API/refresh/reset/verification tokens, import jobs
   and settings removed. Pages, views, uploads and stream updates the user
   authored in OTHER people's workspaces stay, attributed to the tombstone —
   they are the shared work of that workspace, and their `ON DELETE RESTRICT`
   references are why a hard delete used to fail with 409 for anyone who had
   ever collaborated (C4, `docs/reviews/code-review-2026-10-18.md`). Same approach as
   Outline.

Every "who is a user" question excludes tombstones: login, refresh, `/me`, the
existence check AI calls and workspace creation make (an access token outlives
the account by its TTL), the first-account exception, and the user metrics.
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
#
#    !! ON THE MAINTAINER'S PRODUCTION NODE, MAKE THIS EDIT IN THE REPO AND CUT
#    A RELEASE — do NOT hand-edit the file on the server. Since 2026-08-25 every
#    deploy SHIPS docker-compose.yaml from the release tag, so a server-side edit
#    survives exactly until the next deploy and is then silently reverted. That
#    is the flip side of "compose drift cannot exist": the node is not allowed an
#    opinion of its own. Self-hosters who do not deploy via ansible/deploy.yml
#    are unaffected — edit away.
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

共享入口的最终资源配置、根因、部署保护与验收步骤见
[Traefik 工作集保护](traefik-working-set.md)：**上限 256 MiB、独立根 slice 与容器
两层保护各 128 MiB**。仓库 Compose 和 provisioning 均保存此配置；普通 Mica 发版
不管理 Traefik。已有生产节点已应用，**不要仅为同步文档再次重建共享入口**。

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
