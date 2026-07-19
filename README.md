<h1 align="center">Mica</h1>

<p align="center">
  <em>Your notes are Markdown files. On your disk, or on your server. Never on ours.</em>
</p>

<p align="center">
  <a href="https://github.com/weironz/mica/releases/latest"><img alt="Release" src="https://img.shields.io/github/v/release/weironz/mica?label=download&color=2f7d6f"></a>
  <a href="https://github.com/weironz/mica/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/weironz/mica/actions/workflows/ci.yml/badge.svg"></a>
  <a href="#-license"><img alt="License" src="https://img.shields.io/badge/license-AGPL--3.0-2f7d6f"></a>
  <img alt="Platform" src="https://img.shields.io/badge/platform-Windows%20%7C%20Web-2f7d6f">
</p>

<p align="center">
  <a href="#-install">Download</a> ·
  <a href="#-self-hosting">Self-host</a> ·
  <a href="docs/">Documentation</a> ·
  <a href="docs/roadmap.md">Roadmap</a>
</p>

<p align="center">
  <b>English</b> | <a href="README.zh-CN.md">简体中文</a>
</p>

<!-- SCREENSHOT: a wide shot of the editor goes here, before anything else. -->

---

**Looking to use Mica?** Grab the [installer](#-install) — you don't need anything
else on this page.

**Looking to self-host, hack on it, or see how it works?** Read on.

> **Project status.** Actively developed and used daily by its author. Migrations
> always run forward and the export format round-trips, so real notes are safe.
> But this is pre-1.0: there is no compatibility promise across minor versions,
> and Linux/macOS desktop builds are not published yet.

## ✨ What Mica does

**Two worlds, one app.** A *local* workspace is a folder of Markdown on your own
disk — no account, no network, no sync. A *cloud* workspace lives on a Mica
server, syncs in real time, and can be shared. You switch between them
explicitly, and the app only ever shows one at a time, so there is never a
question of where a page lives.

**The editor is drawn, not composed.** It's a single Flutter `RenderBox` that
paints text, carets, selections and blocks itself, over a marks-over-plain-text
model. Not a widget tree, not a `WebView`, not a wrapped third-party editor.

- **Local-first workspaces** — plain Markdown on disk, no account required.
- **Real-time collaboration** — CRDT sync via `yrs`, with collaborator presence.
- **Offline-tolerant** — edits queue and reconcile; images upload when you reconnect.
- **Folders and pages** — drag to reorder or reparent, per-user workspace ordering.
- **Rich blocks** — tables, code with highlighting, LaTeX math, Mermaid diagrams, footnotes.
- **Version history** — automatic snapshots plus named checkpoints, with diff preview.
- **Import** — Markdown, folders, ZIP archives, and Notion exports.
- **Export** — Markdown, HTML, PDF, and ZIP; exports re-import losslessly.
- **Public links** — share a document read-only.
- **MCP server** — let Claude read, search and edit your notes.
- **Optional AI** — hidden entirely when no API key is configured.

### Markdown is the format, not a feature

CommonMark 0.31.2 is the base — **641/641 spec examples pass on the read side** —
with GFM on top (24/24), plus a small dialect for what GFM cannot express:
footnotes, front matter, Pandoc-style math. The writer emits a normalized subset,
and **round-trip stability is an enforced invariant**, pinned by a regression
floor in CI. See the [scoreboard](docs/commonmark-scoreboard.md).

## 🤔 Why another one of these

There are a lot of good notes apps. Mica exists because of a specific combination
that none of them quite had:

- **[Obsidian](https://obsidian.md)** got local-first files right, but isn't open source and has no real-time server story.
- **[Notion](https://notion.so)** got the editor right, but your data lives in their database.
- **[AFFiNE](https://github.com/toeverything/AFFiNE)** and **[AppFlowy](https://github.com/AppFlowy-IO/AppFlowy)** are both excellent and both open source — they are Mica's reference points, and their source has settled more than one architecture argument here. AFFiNE is web-first; AppFlowy runs a Rust core under Flutter.
- **[SiYuan](https://github.com/siyuan-note/siyuan)** and **[Logseq](https://github.com/logseq/logseq)** are closer to the block-model end than the document end.

Mica's bet is a **hand-written editor over a Rust data plane**, where the same
document model serves a local folder and a synced server, and where Markdown is
the storage format rather than an export target.

**What Mica is not:** it has no plugin ecosystem, no mobile apps, no whiteboard
or database views, and a fraction of the polish of any project listed above. If
you need those today, use one of those instead — they're genuinely good.

## 📦 Install

| Platform | Download |
| --- | --- |
| **Windows** | [`Mica-Setup-*.exe`](https://github.com/weironz/mica/releases/latest) — the app self-updates from the same feed |
| **Linux / macOS desktop** | Not published yet. The Flutter project targets Linux and the code is platform-agnostic, but neither is built in CI or tested — treat as unsupported. |
| **Web** | Self-hosted only, see below |

**CLI** — `mica-cli` binaries for Windows, Linux and macOS are attached to every
[release](https://github.com/weironz/mica/releases/latest). It drives the same
API and hosts the MCP server (`mica-cli mcp`). See [docs/cli.md](docs/cli.md).

## 🏗 Self-hosting

Two compose files ship in `deploy/`. The single-server one is **self-contained** —
PostgreSQL and RustFS (S3-compatible storage) run alongside the app, so you don't
need to bring your own.

<details>
<summary><b>Single server — nginx on port 80, no Traefik</b></summary>

```sh
cp deploy/.env.prod.example .env.prod
vi .env.prod          # SERVER_IP, plus a strong JWT_SECRET and passwords
                      #   openssl rand -hex 32
./deploy/deploy.sh    # builds the Flutter bundle + API image, starts everything
```

Then open `http://<SERVER_IP>/`. Public ports are **80** (app) and **9000**
(RustFS, because the browser presigns straight against it — so `S3_ENDPOINT`
must be browser-reachable). Postgres stays inside the compose network.

`./deploy/deploy.sh --web-only` rebuilds just the Flutter bundle; nginx serves it
live, no restart needed.

</details>

<details>
<summary><b>Behind an existing Traefik — HTTPS, no host ports</b></summary>

`deploy/docker-compose.yml` is the canonical stack: label routing plus Let's
Encrypt, pulling published images rather than building. Set `MICA_VERSION` to pin
a release. See [Deployment](docs/deploy.md#behind-traefik-the-canonical-production-stack).

</details>

**Migrations are embedded in the API binary and run at startup** — there is no
separate migration step, and rolling forward is the only supported direction.

Full notes: [Deployment](docs/deploy.md) · [Backup](docs/backup.md) · [Release process](docs/release.md)

## 🧩 How it's built

**Rust** for anything that parses, walks archives, hashes, or talks to storage.
**Dart/Flutter** for painting, caret/selection, hit-testing, and the editor's
latency-critical paths. `yrs` (the Rust port of Yjs) for CRDT sync — the Web
client speaks the same wire format via Yjs itself, so one engine serves every
platform.

```
crates/
  api-server     Axum HTTP + WebSocket; migrations; the only thing that
                 talks to Postgres and S3
  app-core       document operations, sync, snapshot/yrs bridging
  mica-core      the CRDT document (yrs) — from_blocks / to_blocks
  markdown       the Markdown engine: block model, parser, renderer
                 (authoritative; the Dart side mirrors it)
  interchange    archive-level import/export planning — pure, no I/O
  mcp-server     MCP tool surface over the REST API
  cli            mica-cli
  infra          shared plumbing
clients/mica_flutter/
  lib/editor     the hand-written editor (render.dart is the canvas)
  lib/local      the local-vault store (Rust via flutter_rust_bridge)
  rust           the client-side Rust core (FFI)
docs/            design documents
migrations/      Postgres schema, applied automatically at API startup
```

## 🛠 Development

Requires Rust (stable, see `rust-toolchain.toml`), Flutter, and Docker.
[`just`](https://github.com/casey/just) drives everything — `just --list` shows
every recipe.

```sh
cp .env.example .env
just dev-up          # Postgres + MinIO in Docker
just dev-api         # cargo run -p mica-api-server (migrations run on start)
just app             # Flutter desktop client
just app chrome      # Flutter web client
just test            # cargo test + flutter test
just check           # fmt + clippy + analyze
```

With `S3_*` unset the file endpoints return `503` and everything else works. Same
for `ANTHROPIC_API_KEY` and the AI endpoints.

**Before changing anything structural, read [`CLAUDE.md`](CLAUDE.md)** — project
principles, the invariants that have been broken before, and the release process
— and [`docs/lessons.md`](docs/lessons.md), which records what those invariants
cost when they were broken.

Three rules that will otherwise bite you:

1. **The Markdown grammar is duplicated on purpose** (Rust engine + Dart mirror,
   because input rules can't take a network round trip per keystroke). Shared
   conformance fixtures pin the two together — change one, run both suites.
2. **Every bug fix ships with a regression test**, and the commit message states
   the root cause rather than a changelog.
3. **New render capabilities get a mechanism**, not an `if` branch in
   `render.dart`. See [render architecture](docs/render-architecture.md).

## 📚 Documentation

| | |
| --- | --- |
| [Architecture](docs/architecture.md) | System shape and the decisions behind it |
| [Lessons](docs/lessons.md) | Bugs that cost the most, and why |
| [Editor design](docs/editor.md) · [engine](docs/editor-engine.md) · [render](docs/render-architecture.md) | The editor's principles and internals |
| [Sync and API](docs/sync-and-api.md) | REST surface and the WebSocket envelope |
| [Export / Import](docs/export-import.md) | Archive format, Notion adaptation, round-trip rules |
| [CommonMark scoreboard](docs/commonmark-scoreboard.md) | Spec conformance, tracked per release |
| [Local-first](docs/local-first-plan.md) · [Vault mode](docs/vault-mode.md) | The local world |
| [MCP server](docs/mcp-server.md) · [Connecting a client](docs/mcp-connect.md) | AI tool access |
| [Deployment](docs/deploy.md) · [Release](docs/release.md) · [Backup](docs/backup.md) | Running it |
| [Shortcuts](docs/shortcuts.md) | Authoritative keyboard shortcut list |
| [Roadmap](docs/roadmap.md) | What's next |

Some design documents predate later decisions — notably `architecture.md`, which
was written when the project was cloud-only. `CLAUDE.md` and the documents it
points at are the current word.

## 🤝 Contributing

Issues and pull requests are welcome. For anything beyond a small fix, please
open an issue first — a fair amount of this codebase is load-bearing in
non-obvious ways, and `docs/lessons.md` exists because of it.

## 📄 License

[AGPL-3.0-or-later](LICENSE).
