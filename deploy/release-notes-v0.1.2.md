**Mica v0.1.2** — a small desktop polish release on top of v0.1.1.

## What's new since v0.1.1

- **Page width slider fixed** — its range now maps to the editor's actually-reachable column width, so the whole slider changes the layout instead of the upper half being a dead no-op once the page already filled the window. Full-bleed is the far right; the middle is a genuine medium width.
- **About** — moved into **Settings → About**; opens a dialog showing the current app version (v0.1.2), with a "View licenses" link.

No backend/API changes since v0.1.1 — the `mica-api` / `mica-web` images are rebuilt at v0.1.2 for version parity.

## Docker images

```bash
docker pull willdockerhub/mica-api:v0.1.2
docker pull willdockerhub/mica-web:v0.1.2
```

Single-server stack: `deploy/docker-compose.prod.yml`. `latest` also points at v0.1.2.

## Assets

- `Mica-Setup-0.1.2.exe` — Windows desktop installer (per-user, no admin)
- `mica-api-server-v0.1.2-linux-x86_64.tar.gz` — backend binary (migrations embedded, run on boot)
- `mica-web-v0.1.2.tar.gz` — prebuilt Flutter web bundle (serve statically; same-origin API resolution)
