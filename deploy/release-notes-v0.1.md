First public cut of **Mica** — a cloud-first collaborative Markdown workspace. Rust backend (Axum + PostgreSQL + S3-compatible storage), Flutter Web client with a custom canvas editor.

## Highlights

**Editor**
- Single-canvas block editor: paragraphs, headings, lists, to-dos, quotes, code blocks (language picker, wrap/copy/scroll), tables, dividers, images
- Inline marks (bold/italic/code/strike/links) with live `**markdown**` input rules, slash menu, `[[` page-link picker, link hover toolbar, floating selection toolbar with multi-line block conversion
- Undo/redo, full keyboard navigation incl. title ⇄ body, drag selection with auto-scroll
- Optional formatting toolbar (Settings → Appearance); all client settings persist

**Tables**
- Cell overlay editing, row/column menus (insert/move/delete), drag-resize columns and overall table width, edge dot handles

**Images**
- Content-addressed uploads (sha256 dedup) to S3/RustFS, paste/picker/URL re-hosting, permanent share links, hover + context menus, resize/align

**Import / Export (Rust engine: `mica-interchange`)**
- Workspace ⇄ hierarchical Markdown ZIP with `manifest.json` ordering, assets, and standard relative page links
- Server-side import: one upload, job progress; handles ZIP64, data descriptors, GBK entry names, nested `Part-N.zip`, wrapper folders
- Notion "Markdown & CSV" import (ID stripping, folder↔page matching); folder and multi-file import into existing workspaces
- Markdown engine (`mica-markdown`) with cross-implementation conformance fixtures (Rust + Dart pinned to the same golds)

**Realtime & platform**
- WebSocket collaboration rooms with presence, snapshot history, JWT auth, workspace members/roles, full-text page search, AI assistant hooks

## Deployment

```bash
docker pull willdockerhub/mica-api:v0.1
docker pull willdockerhub/mica-web:v0.1
```

Single-server stack: see `docker-compose.prod.yml` + `docs/deploy.md` (nginx serves the web bundle and proxies `/api` + `/ws`; only ports 80 and 9000 are public).

## Assets

- `mica-api-server-v0.1-linux-x86_64.tar.gz` — backend binary (migrations embedded, run on boot)
- `mica-web-v0.1.tar.gz` — prebuilt Flutter web bundle (serve statically; same-origin API resolution)
