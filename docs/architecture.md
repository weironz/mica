# Cloud-first Markdown Workspace Architecture

## Goal

Build a cloud-first collaborative Markdown workspace aligned with AppFlowy's core ideas, but without AppFlowy's local-first client runtime. Data lives in the cloud. Clients use REST and WebSocket APIs. The backend is pure Rust.

The first-class product surface is Web. Desktop and mobile should reuse the same backend protocol and document model.

## Key Decisions

- Cloud is the source of truth.
- No local-first storage, no RocksDB client core, no embedded Rust core through Dart FFI.
- Frontend can be Flutter, but it should communicate with the backend over network APIs.
- Internal document state is a block model, not raw Markdown.
- Markdown is an import/export and authoring format.
- Real-time sync is operation/update based.
- History is implemented with snapshots plus append-only updates.
- Infinite canvas is out of scope.
- **Rust by default for the data plane.** Anything that parses files, walks
  archives, batches, hashes or talks to storage belongs in the Rust backend
  (see `crates/interchange` + `docs/export-import.md` for the reference
  shape: a pure engine crate, side effects in api-server, the client only
  uploads and polls). Dart/Flutter owns what only a client can own: painting,
  caret/selection, hit-testing, and the editor's latency-critical hot paths.
- **Editor hot paths stay client-side by design.** Live input rules
  (`**b**` as you type), paste-to-blocks, copy-as-markdown (clipboard APIs
  must produce data inside the user gesture) cannot take a network round
  trip per keystroke. This means a small, deliberate duplication of the
  markdown/inline-marks grammar in Dart (`lib/editor/marks.dart`,
  `markdown.dart`) mirroring the Rust engine (`crates/markdown`). The two
  are pinned together by shared conformance fixtures
  (`crates/markdown/tests/fixtures/conformance` — gold `.blocks.json` files
  asserted by both `cargo test -p mica-markdown` and the Dart
  `markdown_conformance_test.dart`; regenerate with `GEN_GOLD=1` after an
  intentional grammar change). The long-term in-house path to a single
  implementation is compiling the Rust engine to WASM for the client.
- **The Markdown engine is a named crate.** `mica-markdown` owns the block
  model ("AST") and Markdown/HTML parsing+rendering (like SiYuan's lute);
  `mica-app-core` keeps document *operations* and re-exports the engine;
  `mica-interchange` builds archive-level import/export on top.

## AppFlowy Alignment

AppFlowy uses:

- Flutter UI.
- Rust local core.
- Dart FFI event dispatch.
- Protobuf-generated event wrappers.
- AppFlowy-Collab / Yrs for collaborative objects.
- Local persistence through RocksDB/SQLite.
- AppFlowy Cloud as a sync provider.

This project should reuse the architectural ideas, not the exact client architecture.

Keep:

- Workspace / folder / document / database separation.
- Block-based document model.
- Action-based edit pipeline.
- Collab object type system.
- Snapshot-based history.
- Awareness/presence model.
- Flutter editor transaction adapter pattern.

Do not copy:

- Dart FFI event bus.
- Client-side Rust `AppFlowyCore`.
- Local mode.
- RocksDB/SQLite as primary client state.
- LocalServer/AppFlowyCloud dual-provider complexity.

## Target Architecture

```text
Flutter Web / Desktop / Mobile
  Editor UI
  Workspace UI
  REST client
  WebSocket sync client
        |
        | HTTPS + WSS
        v
Rust Cloud Backend
  Axum API
  Auth
  Workspace service
  Page/folder service
  Document service
  Sync service
  History service
  Search service
  Storage service
        |
        +-- PostgreSQL
        +-- Redis optional: presence, pub/sub, rate limits
        +-- S3/MinIO: files and exports
        +-- Tantivy: full-text search
```

## Backend Workspace Layout

Recommended Rust workspace:

```text
crates/
  api-server/        Axum HTTP/WebSocket entrypoint
  app-core/          service composition and shared application state
  auth/              auth/session/user model
  workspace/         workspace, membership, permissions
  folder/            page tree and navigation model
  document/          block document model and operations
  collab/            real-time room, update validation, broadcast
  history/           snapshots, versions, restore
  storage/           file uploads and object storage
  search/            indexing and query
  markdown/          import/export
  infra/             config, tracing, errors, database pool
apps/
  flutter_app/
docs/
```

Use AppFlowy-style module boundaries, but expose network APIs instead of FFI events.

## Backend Stack

- Rust async runtime: Tokio
- HTTP/WebSocket: Axum
- SQL: SQLx
- Database: PostgreSQL
- Object storage: S3-compatible API
- Serialization: Serde JSON for public APIs, optional protobuf/bincode internally
- Search: Tantivy
- Markdown: `comrak` or `markdown-rs`
- Observability: tracing, OpenTelemetry later

## Frontend Stack

Recommended:

- Flutter for product UI if cross-platform consistency is the priority.
- A document editor based on AppFlowy's `appflowy_editor` ideas.
- Client state management: Bloc or Riverpod; choose one and keep it consistent.

Risk note:

Flutter Web can work, but the editor is the highest-risk area. If Web editing quality becomes a blocker, isolate the editor behind an adapter so the Web client can switch to a ProseMirror/Tiptap editor without changing backend contracts.

## Domain Model

### Workspace

```text
Workspace
  id
  name
  owner_id
  created_at
  updated_at
```

### Membership

```text
WorkspaceMember
  workspace_id
  user_id
  role: owner | admin | editor | commenter | viewer
  joined_at
```

### Page/View

AppFlowy uses "views" for objects in the page tree. Keep that idea.

```text
View
  id
  workspace_id
  parent_view_id
  object_id
  object_type: document | database
  name
  icon
  position
  is_deleted
  created_by
  created_at
  updated_at
```

### Document

The document object contains block state.

```text
Document
  id
  workspace_id
  root_block_id
  created_by
  created_at
  updated_at
```

### Block

Store blocks as structured JSON in PostgreSQL, but treat operations as the source of history.

```text
Block
  id
  document_id
  parent_id
  type
  data: jsonb
  external_id
  external_type
  position
  created_at
  updated_at
```

`external_id` is useful for AppFlowy-style long rich text deltas:

```text
block.data       -> structural attributes
external_text    -> rich text delta / text CRDT state
```

## Document Storage Strategy

MVP storage should be simple:

- Store current document state in `document_snapshots`.
- Store every accepted operation/update in `document_updates`.
- Periodically compact current state into snapshots.

Do not write every block into normalized SQL tables first unless needed for query. JSON snapshots are easier to evolve while the editor model changes.

Later, add indexed block tables for search, backlinks, and analytics.

## Operation Model

Client edits should be converted to backend operations:

```json
{
  "op_id": "uuid",
  "document_id": "uuid",
  "base_version": 42,
  "ops": [
    {
      "type": "insert_block",
      "block": {
        "id": "block_id",
        "type": "paragraph",
        "data": {},
        "external_id": "text_id",
        "external_type": "text"
      },
      "parent_id": "root",
      "previous_id": "prev_block"
    }
  ]
}
```

Initial operation types:

- `insert_block`
- `update_block`
- `delete_block`
- `move_block`
- `update_text_delta`
- `set_awareness`

## Real-time Sync

Each open document has a room:

```text
/ws/workspaces/{workspace_id}/documents/{document_id}
```

Client connects, authenticates, receives:

- Current document snapshot version.
- Missing updates since client version.
- Presence state.

Client sends:

- Document operations.
- Text delta updates.
- Cursor/selection awareness.

Server:

- Validates permissions.
- Validates operation shape.
- Applies operation to authoritative document state.
- Appends accepted update.
- Broadcasts accepted update with server sequence.
- Emits snapshot tasks.

## History

Use three concepts:

- Update: every accepted operation or collab update.
- Snapshot: compact full document state at a version.
- Named version: user-visible history checkpoint.

```text
document_updates
  id
  document_id
  seq
  actor_id
  update_kind
  payload
  created_at

document_snapshots
  id
  document_id
  version_seq
  payload
  created_at

document_versions
  id
  document_id
  snapshot_id
  name
  created_by
  created_at
```

Restore flow:

1. Load target snapshot.
2. Create a new update of kind `restore_snapshot`.
3. Broadcast restored state.
4. Keep old history immutable.

## Permissions

All write paths must enforce workspace membership and object permissions.

Minimum roles:

- owner: all permissions
- admin: workspace management except ownership transfer
- editor: create/update/delete content
- commenter: comment only
- viewer: read only

Never trust client operations just because they came through an authenticated WebSocket.

## Markdown

Markdown should be implemented as converters:

```text
Markdown -> AST -> DocumentData
DocumentData -> AST -> Markdown
DocumentData -> HTML
```

MVP support:

- headings
- paragraphs
- bold/italic/inline code
- links
- unordered/ordered lists
- task lists
- blockquote
- code block
- tables if editor supports them

Avoid making Markdown text the primary storage, or Notion/AppFlowy-style block features will become fragile.

## Risks

- Flutter Web editor quality is the largest frontend risk.
- Collaborative editing semantics are the largest backend risk.
- Markdown round-trip fidelity can consume large time if not scoped.
- History restore must be immutable and auditable.
- Permission checks must run on both REST and WebSocket paths.

## Initial Recommendation

Build the Rust backend and document protocol first. Keep frontend replaceable behind a document client adapter.

The first proof of concept should demonstrate:

- Create workspace.
- Create document.
- Open same document in two clients.
- Edit blocks in real time.
- See history entries.
- Restore a previous version.
- Export Markdown.
