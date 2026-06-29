**Mica v0.1.4** — a desktop reliability patch on top of v0.1.3.

## What's new since v0.1.3

- **Stay logged in across restarts.** The desktop client no longer forces a re-login on every launch in cloud / self-hosted mode. The session token is now persisted and restored on startup — validated against the server, with an expired/revoked token cleanly falling back to the login screen. (Tokens last 24h; a refresh-token for longer sessions is a future enhancement. The token is stored in plaintext prefs, same as other settings — DPAPI encryption is a noted hardening follow-up.)

No backend/API changes since v0.1.3 — the `mica-api` / `mica-web` images are rebuilt at v0.1.4 for version parity.

## Docker images

```bash
docker pull willdockerhub/mica-api:v0.1.4
docker pull willdockerhub/mica-web:v0.1.4
```

Single-server stack: `deploy/docker-compose.prod.yml`. `latest` also points at v0.1.4.

## Assets

- `Mica-Setup-0.1.4.exe` — Windows desktop installer (per-user, no admin)
- `mica-api-server-v0.1.4-linux-x86_64.tar.gz` — backend binary (migrations embedded, run on boot)
- `mica-web-v0.1.4.tar.gz` — prebuilt Flutter web bundle (serve statically; same-origin API resolution)
