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
  any IP — no per-server `--dart-define`, and no CORS in production.
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
