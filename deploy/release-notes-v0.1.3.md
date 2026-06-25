**Mica v0.1.3** — a fix release for a cloud data-loss/display bug.

## Fixed

- **Cloud: content appeared lost after switching pages.** When a page was edited via the CRDT (yrs) path, its content was saved to the yrs base but the op-snapshot — which the page loader reads on a switch — stayed stale, and the editor never reconciled the real content back in (it only re-read on a *version* change, which the reconcile didn't bump). Switching away and back showed the empty snapshot. Now the editor reconciles the CRDT content on (re)open, so it's displayed; and the page switch flushes the editor's last (debounced) edits and drains the sync session before tearing it down, so nothing in flight is dropped. (Your existing content was on the server in the yrs base the whole time — this release makes it display again.)

No backend/API behavior changes since v0.1.2 — the `mica-api` image is rebuilt at v0.1.3 for version parity; the fix ships in the desktop installer and the `mica-web` bundle.

## Docker images

```bash
docker pull willdockerhub/mica-api:v0.1.3
docker pull willdockerhub/mica-web:v0.1.3
```

`latest` also points at v0.1.3.

## Assets

- `Mica-Setup-0.1.3.exe` — Windows desktop installer (per-user, no admin)
- `mica-api-server-v0.1.3-linux-x86_64.tar.gz` — backend binary (migrations embedded, run on boot)
- `mica-web-v0.1.3.tar.gz` — prebuilt Flutter web bundle (serve statically; same-origin API resolution)
