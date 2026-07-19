# Mica

A Markdown workspace with a hand-written editor, a Rust data plane, and two
places to keep your notes: a local folder on your disk, or a synced cloud
workspace. Desktop (Windows) and Web, from one codebase.

> Status: actively developed, used daily by its author. The API and storage
> formats are stable enough to trust with real notes (every release runs
> migrations forward and the export format round-trips), but this is not a
> 1.0 product and there is no compatibility promise across major versions yet.

## What it is

**Two worlds, one app.** A *local* workspace is a folder of Markdown on your
own disk — no account, no network, the files stay yours. A *cloud* workspace
lives on a Mica server you or someone else runs, syncs in real time, and can be
shared with other people. You switch between them explicitly; the app only ever
shows one world at a time, so there is never a question of where a page lives.

**The editor is drawn, not composed.** It is a single Flutter `RenderBox` that
paints text, carets, selections and blocks itself, over a marks-over-plain-text
model — not a tree of widgets, not a `WebView`, not a wrapped third-party
editor. That is why the caret behaves the same on every platform, why IME
(Chinese/Japanese/Korean) composition works, and why large documents stay
responsive.

**Markdown is the format, not a feature.** CommonMark 0.31.2 is the base
(641/641 spec examples on the read side), GFM on top (24/24), plus a small
dialect for what GFM cannot express — footnotes, front matter, Pandoc-style
math. The writer emits a normalized subset, and round-trip stability is an
enforced invariant, not an aspiration. See
[the scoreboard](docs/commonmark-scoreboard.md).

### Features

- **Editing** — headings, lists, tables, code blocks with syntax highlighting,
  quotes, footnotes, task lists, LaTeX math (inline and block),
  Mermaid diagrams, images with paste/drag-drop upload.
- **Page tree** — folders and pages, drag to reorder or reparent, per-user
  workspace ordering. Folders contain; pages are leaves.
- **Real-time sync** (cloud) — CRDT-based (`yrs`, the Rust port of Yjs) with
  collaborator presence. The Web client speaks the same wire format via Yjs.
- **Offline-first** (cloud) — edits queue locally and reconcile on reconnect;
  images inserted offline upload when the network returns.
- **History** — automatic snapshots plus manual checkpoints, with restore.
- **Import** — Markdown files, folders, and ZIP archives, including Notion
  exports (IDs stripped, duplicate H1 titles removed, nested `Part-N.zip`
  expanded). See [Export / Import](docs/export-import.md).
- **Export** — Markdown, HTML, PDF, and ZIP archives of a page, a folder, a
  workspace, or everything at once. Exports re-import losslessly.
- **Sharing** (cloud) — public read-only links per document.
- **AI** (optional) — `/`-menu and global "Ask AI" backed by the Anthropic
  Messages API. Disabled and hidden when no key is configured.
- **MCP server** — expose your workspace to Claude or any MCP client so it can
  read, search, create and edit pages. See [MCP](docs/mcp-connect.md).

## Install

**Desktop (Windows)** — download `Mica-Setup-<version>.exe` from
[Releases](https://github.com/weironz/mica/releases/latest). The app updates
itself from the same feed.

**CLI** — `mica-cli` binaries for Windows / Linux / macOS are attached to every
release. It drives the same API and hosts the MCP server (`mica-cli mcp`). See
[CLI](docs/cli.md).

**Server** — the API and Web bundle publish as container images per release.
See [Deployment](docs/deploy.md) and [Release process](docs/release.md).

Linux and macOS desktop builds are not published yet. The Flutter project
targets Linux and the code is platform-agnostic, but neither is built in CI or
tested, so treat them as unsupported.

## Repository layout

```
crates/
  api-server     Axum HTTP + WebSocket server; migrations; the only thing
                 that talks to Postgres and S3
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
docs/            design documents — see the index below
migrations/      Postgres schema, applied automatically at API startup
```

## Development

Requires Rust (stable, see `rust-toolchain.toml`), Flutter, and Docker for
Postgres. [`just`](https://github.com/casey/just) drives everything;
`just --list` shows all recipes.

```sh
cp .env.example .env
just dev-up          # Postgres (+ MinIO) in Docker
just dev-api         # cargo run -p mica-api-server (runs migrations on start)
just app             # Flutter desktop client
just app chrome      # Flutter web client
just test            # cargo test + flutter test
just check           # fmt + clippy + analyze
```

The API serves `http://127.0.0.1:8080`; check it with
`curl http://127.0.0.1:8080/api/health`.

File uploads need S3-compatible storage (`just dev-up` starts MinIO). With the
`S3_*` variables unset, the file endpoints return `503` and the rest of the app
works. Same for `ANTHROPIC_API_KEY` and the AI endpoints.

### Working on this codebase

- **`CLAUDE.md` is the operating manual** — project principles, invariants that
  have been broken before, and the release process. Read it before changing
  anything structural.
- **The data plane is Rust.** Anything that parses, walks archives, hashes, or
  talks to storage belongs in a crate, not in Dart. Dart owns painting,
  caret/selection, hit-testing, and the editor's latency-critical paths.
- **The Markdown grammar is duplicated on purpose** (Rust engine + Dart mirror,
  because input rules cannot take a network round trip per keystroke). The two
  are pinned together by shared conformance fixtures — change one, run both
  test suites.
- **Every bug fix ships with a regression test.** Commit messages state the
  root cause, not a changelog.

## Documentation

| | |
| --- | --- |
| [Architecture](docs/architecture.md) | System shape and the decisions behind it |
| [Editor design](docs/editor.md) · [Editor engine](docs/editor-engine.md) | The editor's principles and internals |
| [Render architecture](docs/render-architecture.md) | How new block types are added (registry, not `if`-branches) |
| [Sync and API](docs/sync-and-api.md) | REST surface and the WebSocket envelope |
| [Export / Import](docs/export-import.md) | Archive format, Notion adaptation, round-trip rules |
| [CommonMark scoreboard](docs/commonmark-scoreboard.md) | Spec conformance, tracked per release |
| [Local-first plan](docs/local-first-plan.md) · [Vault mode](docs/vault-mode.md) | The local world |
| [MCP server](docs/mcp-server.md) · [Connecting a client](docs/mcp-connect.md) | AI tool access |
| [Deployment](docs/deploy.md) · [Release](docs/release.md) · [Backup](docs/backup.md) | Running it |
| [Keyboard shortcuts](docs/shortcuts.md) | Authoritative shortcut list |
| [Roadmap](docs/roadmap.md) | What's next |

Some design documents predate later decisions (notably `architecture.md`, which
was written when the project was cloud-only). `CLAUDE.md` and the docs it points
at are the current word.

## License

AGPL-3.0-or-later.
