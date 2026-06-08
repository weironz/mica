**Mica v0.1.1** — adds a native **Windows desktop** app and **offline-first CRDT** sync on top of the v0.1 cloud workspace. Rust backend (Axum + PostgreSQL + S3-compatible storage), one Flutter codebase across **Web + Desktop**, custom canvas editor.

## What's new since v0.1.0

**Native Windows desktop**
- Flutter desktop build: window size/position memory + min-size, app-level keyboard shortcuts
- **Chinese IME** verified against the custom canvas editor (composing region + candidate-window positioning via TextInputClient)
- Native file dialogs (open/save/dir), rich clipboard (paste HTML/images, copy images)
- **Mermaid renders fully offline** — pure-Rust `merman` engine over FFI → SVG → flutter_svg raster (no webview/JS/browser)
- Distributed as a per-user **Inno Setup installer** (`Mica-Setup-0.1.1.exe`, unsigned — SmartScreen → More info → Run anyway)

**Offline-first + CRDT (Phase 2)**
- yrs-based CRDT document model; one authoritative engine across platforms (desktop via Rust FFI, web via byte-compatible yjs)
- **Local-offline mode** — full workspace on-device (SQLite store), no account/network; field-level CRDT block props
- Cloud sync over WebSocket: per-workspace monotonic stream + state-vector fallback; large-doc stream truncation with client re-bootstrap
- **Object storage, dual path** — on-device content-addressed blob CAS (offline images) + cloud S3; cloud images mirror locally for offline read
- **Local → cloud migration** — copy a local workspace up in place (local data preserved); content-addressed blob reconciliation, no orphaned tree
- Collaborative remote cursors (awareness); on-device checkpoint rollback UI

## Docker images

```bash
docker pull willdockerhub/mica-api:v0.1.1
docker pull willdockerhub/mica-web:v0.1.1
```

Single-server stack: `deploy/docker-compose.prod.yml` (nginx serves the web bundle, proxies `/api` + `/ws`; only ports 80 and 9000 public). `latest` also points at v0.1.1.

## Assets

- `Mica-Setup-0.1.1.exe` — Windows desktop installer (per-user, no admin)
- `mica-api-server-v0.1.1-linux-x86_64.tar.gz` — backend binary (migrations embedded, run on boot)
- `mica-web-v0.1.1.tar.gz` — prebuilt Flutter web bundle (serve statically; same-origin API resolution)
