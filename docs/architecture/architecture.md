# Mica Architecture

> Rewritten 2026-10-23 (issue #90) to describe what IS, replacing the original
> inception-era planning document (Tantivy / Redis / "no embedded Rust core
> through Dart FFI" / separate auth-workspace-folder-collab-history crates /
> comrak / op-model tables — none of which survived contact with the product).
> The detailed designs live in the sibling documents; this page is the map.

## What Mica is

A collaborative Markdown workspace with a **Rust backend and a Flutter client**
(web + Windows desktop). It started cloud-only; the shipped shape is **hybrid**:

- The cloud (PostgreSQL + S3-compatible storage) is the collaboration plane:
  accounts, workspaces, page trees, real-time co-editing, history, search.
- The desktop app additionally runs an **offline local mode** through Dart FFI
  to the same Rust data plane (`mica-core`) — no backend involved. Web cannot
  do this (no FFI there) and stays cloud-only. A unification of the two worlds
  into one local-first storage with an optional sync layer is designed in
  [`local-first-plan.md`](local-first-plan.md); what is merged so far is noted
  there.

The editor is **entirely self-painted** (one `RenderBox`, no contenteditable, no
third-party editor package) — see [`editor.md`](editor.md) and
[`render-architecture.md`](render-architecture.md) for the principles and the
renderer registry red line.

## Key decisions that still bind

- **Rust by default for the data plane.** Anything that parses files, walks
  archives, batches, hashes or talks to storage belongs in Rust; Dart/Flutter
  owns what only a client can own: painting, caret/selection, hit-testing,
  and the editor's latency-critical hot paths (a network round trip per
  keystroke is not a latency budget).
- **The internal document state is a block model, not raw Markdown**;
  Markdown is the import/export/authoring format, and **round-trip
  stability** (our export re-imports losslessly) is the invariant.
- **CommonMark 0.31.2 is the parsing base, GFM the target dialect**, and
  anything beyond degrades gracefully: serialize to valid GFM any foreign
  viewer renders acceptably, carry the lossless form out-of-band, never
  invent syntax others would misrender.
- **Minimal third-party dependencies** (in-house first for the core data
  plane); every exception is registered in CLAUDE.md's dependency exemption
  list with its reason — there is no "customary exemption".
- **Infinite canvas is out of scope.**

## The shape

```text
Flutter client  (clients/mica_flutter)
  web:     yjs (JS) + IndexedDB, no FFI
  windows: flutter_rust_bridge FFI -> rust_lib_mica_flutter -> mica-core (SQLite store)
        |
        | REST (JWT) + WebSocket document rooms
        v
Rust backend  (crates/)
  api-server   Axum HTTP + WS entrypoint, routes per domain
  app-core     document/sync domain + AppState (BodyIndex search, sync, store)
  mica-core    storage/transport-agnostic data plane (block DTO, marks, yrs, SQLite)
  markdown     the CommonMark+GFM engine (block model, MD/HTML, render)
  interchange  archive-level import/export (serde_json only)
  infra        config, db pool, errors, mailer
  mcp-server   MCP tool surface for AI clients (REST + PAT, no direct DB)
  cli          backup / export / admin CLI (same REST API)
        |
        +-- PostgreSQL        document_yrs_base (CRDT state), views, auth, …
        +-- S3-compatible     rustfs (files, exports); MinIO/OSS would work too
        +-- in-process memory BodyIndex full-text search (NO Tantivy, NO Redis)
```

## Storage model

- **Document content lives in `document_yrs_base`** as yrs (the Rust `yrs`
  CRDT)
  state — the op-model tables (`document_snapshots` / `document_updates` /
  `document_versions`) were retired in S0–S5 (migration 0016); the split-brain
  streams live in `workspace_updates` (rid-ordered, folded). Stream retention
  runs on EVERY write path (WebSocket `push_update_checked` and the REST/MCP
  op write) via one shared helper: it fires per document once that document's
  stream grows `prune_every` rows past the keep margin, then deletes rows
  below the last `keep_margin` **global** rids — the window is measured in
  global rids, not "this document's last N updates".
- **The page tree is `views`**: folders are the only containers, pages are
  leaves. The invariant is enforced in the API guard
  (`ensure_parent_accepts_children`) AND a DB trigger
  (`views_parent_must_be_folder`, migration 0011, tightened by 0033: parent
  must be a folder, in the same workspace, and not in the bin when the child
  is live).
- **History is yrs-native** (see `routes/history.rs`): auto-snapshots captured
  inside every write (`sync::archive_auto_version_tx`), expired auto rows
  pruned by the same shared retention pass on both the WebSocket and REST/MCP
  write paths; endpoints list / preview / pin a named version / restore.
  Restore is a NEW forward update broadcast as a yrs `sync.update`, never a
  rewrite of the log.
- **Search** is an in-process, lockable-memory substring index over document
  bodies (`BodyIndex`) — index proposes candidates, SQL disposes (workspace
  scope, deleted, ordering). Why not Postgres text indexes or Tantivy:
  [`search.md`](search.md).

## Sync and collaboration

WebSocket document rooms. The client sends yrs updates; the server validates
permissions **per message** (re-read from the DB, not cached at connect),
applies, appends, and broadcasts. The reconnect protocol is rid-based:
`sync.bootstrap → sync.base`, `sync.pull{since_rid} → sync.updates{head}`,
`sync.push → sync.ack{rid}`. Presence/awareness rides the same socket.

The web client runs the **JS yjs engine** against the server's **Rust yrs**;
byte-compatibility of the update encoding is pinned by the cross-engine
conformance gate `mica-core/tests/web_interop.rs` (runs the committed web
bundle headlessly against the Rust side). The two ends of that gate are
deliberately the two engines real users run.

## Markdown

One Rust engine: `crates/markdown` (CommonMark 0.31.2 641/641 + GFM 24/24 +
dialect extensions: footnotes, front matter, Pandoc math; scoreboard in
[`commonmark-scoreboard.md`](commonmark-scoreboard.md)). It powers the server
(HTML render, export) and, via the FFI crate's `render` feature (merman +
ratex), the desktop editor's live mermaid/math preview — one engine
everywhere except web, which uses its JS counterparts.

The editor's **latency-critical hot paths stay in Dart** (input rules, paste,
clipboard) with a deliberate mirror of the inline-marks grammar
(`lib/editor/marks.dart`), pinned to the Rust engine by shared conformance
fixtures. Round-trip stability (export → re-import loses nothing) is the
invariant. Where the boundary sits and why:
[`markdown-boundary-comparison.md`](../single-source/history/markdown-boundary-comparison.md).

**Being replaced (2026-10-26):** the [single-source project](../single-source/README.md)
removes the Dart mirror — one Rust engine (`crates/md-core`) on every platform
(FFI on desktop, wasm on web), with the editor calling it only at its
boundaries (paste, copy, import, export, AI). Until its S4 lands, the paragraph
above still describes the code; the project docs win where they disagree.

## Auth

Email/password → short-lived JWT access token + single-use rotating refresh
token (reuse detection). Registration is **closed by default**; opening it
still requires email verification, and workspace membership for new users
arrives via invitations (migration 0032). The first account on a fresh
install is auto-verified (otherwise self-hosting cannot bootstrap). The
desktop local mode has no auth layer at all — it talks straight to the local
SQLite store over FFI.

## Engineering gates worth knowing

- **Structure gate** (`scripts/check-file-size.sh`, = CI `structure` job =
  `just check-structure`): per-unit line budget — Dart libraries are measured
  as root+parts, Rust per file (FFI crate included); absolute limits + a
  shrink-only ratchet + a delta gate.
- **rustfmt / clippy / analyze** run in CI (`ci.yml`) and locally
  (`just ci`, `scripts/release-check.sh`) with the same flags.
- **Web e2e** against a real stack (nginx + api + rustfs) and the Windows
  **integration suite** (real FFI) run in their own workflows.
- **MCP conformance**: `mica-cli mcp` is driven by the official MCP
  inspector; a conforming client is the only thing that reads what we
  actually declare.

## Deployment

Single node (docker compose: postgres, rustfs, api, web behind nginx), images
published from CI to Aliyun ACR + Docker Hub. `Deploy` is the only path that
touches prod — it rehearses against the real node first (`--check --diff`
via ansible), snapshots pre-migration, and restores `MICA_VERSION` on failure.
Off-site backups + a restore drill (`just restore-drill`) are part of the
loop. Details: [`../ops/release.md`](../ops/release.md),
[`../ops/deploy.md`](../ops/deploy.md).

## Where the details live

| Topic | Document |
| --- | --- |
| Editor UX principles | [`editor.md`](editor.md) |
| Editor engine (self-painted canvas, marks model) | [`editor-engine.md`](editor-engine.md) |
| Block rendering registry (the red line) | [`render-architecture.md`](render-architecture.md) |
| Sync / history design notes | [`sync-and-api.md`](sync-and-api.md) (endpoints: `../dev/api.md`) |
| Full-text search | [`search.md`](search.md) |
| Markdown engine boundary | [`single-source/`](../single-source/README.md) (single-implementation project; history in `single-source/history/`) |
| CommonMark scoreboard | [`commonmark-scoreboard.md`](commonmark-scoreboard.md) |
| Local-first unification plan | [`local-first-plan.md`](local-first-plan.md) |
| Version history (yrs-native) | [`../features/version-history-plan.md`](../features/version-history-plan.md) |
| Release / deploy / DR | [`../ops/`](../ops/) |
