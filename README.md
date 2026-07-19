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

Mica is a Markdown workspace. Notes are stored as Markdown — in a folder on your
own disk, or on a Mica server you run — and the app is built around keeping that
format faithful in both directions.

## ✨ Highlights

**Two worlds, one app.** A *local* workspace is a folder of Markdown on your own
disk: no account, no network, no sync. A *cloud* workspace lives on a Mica
server, syncs in real time, and can be shared. You switch between them
explicitly, and the app shows one at a time, so where a page lives is always
unambiguous.

**The editor is drawn, not composed.** It's a single Flutter `RenderBox` that
paints text, carets, selections and blocks itself, over a
marks-over-plain-text model — not a widget tree, not a `WebView`, not a wrapped
third-party editor. Caret and selection behave identically on every platform,
IME composition (Chinese, Japanese, Korean) works natively, and long documents
stay responsive because painting is clipped to the viewport.

**One CRDT engine everywhere.** Sync runs on `yrs`, the Rust port of Yjs. The Web
client uses Yjs itself, and the two are byte-compatible at the update,
state-vector and lib0 encoding layers — so every platform speaks one authoritative
format rather than a translation of one.

**A Rust data plane.** Everything that parses files, walks archives, hashes bytes
or talks to storage runs in Rust. Dart owns painting, caret and selection,
hit-testing, and the editor's latency-critical paths.

### Editing

- **Blocks** — headings, lists, tables, quotes, task lists, footnotes.
- **Code** — fenced blocks with syntax highlighting.
- **Math** — LaTeX, inline and block. Inline formulas typeset on the baseline and scale with font size.
- **Diagrams** — Mermaid, rendered offline by a pure-Rust engine (no browser, no Node).
- **Images** — paste or drag to upload, content-addressed and deduplicated by SHA-256.
- **Keyboard-first** — live input rules as you type, paste-to-blocks, copy-as-Markdown.

### Workspace

- **Folders and pages** — drag to reorder or reparent; workspace order is per user.
- **Real-time collaboration** — collaborator presence, live cursors.
- **Offline-tolerant** — edits queue and reconcile on reconnect; images inserted offline upload when the network returns.
- **Version history** — automatic snapshots plus named checkpoints, with a read-only preview and block-level diff.
- **Public links** — share a document read-only.
- **Full-text search** across a workspace.

### Data in and out

- **Import** — Markdown files, folders, ZIP archives, and Notion exports (IDs stripped, duplicated H1 titles removed, nested `Part-N.zip` expanded).
- **Export** — Markdown, HTML, PDF, and ZIP archives of a page, a folder, a workspace, or every workspace at once.
- **Lossless round-trip** — exporting and re-importing restores the tree, the names, the assets and the links.
- **MCP server** — let Claude or any MCP client read, search, create and edit pages.
- **Optional AI** — an `/` command and a global composer, backed by the Anthropic Messages API. Hidden entirely when no key is configured.

### Markdown fidelity

CommonMark 0.31.2 is the base — **641/641 spec examples pass on the read side** —
with GFM on top (**24/24**), plus a small dialect for what GFM cannot express:
footnotes, front matter, Pandoc-style math. The writer emits a normalized subset,
and **round-trip stability is an enforced invariant** with a regression floor in
CI. See the [scoreboard](docs/commonmark-scoreboard.md).

Where a feature has no GFM representation, the rule is: serialize to valid GFM
that renders acceptably in any foreign viewer — never invent syntax others would
misrender — and carry the lossless form out-of-band, so re-importing our own
export restores what GFM dropped.

## 📦 Install

**Desktop** — download `Mica-Setup-*.exe` from
[Releases](https://github.com/weironz/mica/releases/latest). The app updates
itself from the same feed.

**CLI** — `mica-cli` binaries for Windows, Linux and macOS are attached to every
release. It drives the same API and hosts the MCP server (`mica-cli mcp`). See
[docs/cli.md](docs/cli.md).

**Web** — self-hosted, see below.

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
separate migration step.

Full notes: [Deployment](docs/deploy.md) · [Backup](docs/backup.md) · [Release process](docs/release.md)

## 🧩 Repository layout

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
principles, invariants, and the release process — and
[`docs/lessons.md`](docs/lessons.md), which records what those invariants cost
when they were broken.

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

`CLAUDE.md` and the documents it points at are the current word where design
documents disagree.

## 🤝 Contributing

Issues and pull requests are welcome. For anything beyond a small fix, please
open an issue first — a fair amount of this codebase is load-bearing in
non-obvious ways, and `docs/lessons.md` exists because of it.

## 📄 License

[AGPL-3.0-or-later](LICENSE).
