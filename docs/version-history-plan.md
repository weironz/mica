# Page Version History — Design & Implementation Plan

**Status: IMPLEMENTED (2026-07-18).** Built as designed (Option B, yrs-native),
**cloud AND local** in one pass. Auto snapshots on a 10-min cadence + named
checkpoints; restore is a forward update (never a log rewrite). Research
(AFFiNE/AppFlowy/Outline/Notion/SiYuan) confirmed the design and set the
defaults: **10-min min-interval, 30-day auto retention, named versions kept
forever** — see [[export-html-pdf-word]]-style memory notes and the commits.

What shipped vs. this doc:
- **Cloud (Phases 0–2):** migration `0009_document_yrs_versions`; auto-capture +
  retention in `sync::push_update`; `store::{list,fetch_state,create_named}_yrs_version`
  + `yrs_state_to_payload`; **restore via `MicaDoc::set_blocks`** (the doc's
  "apply-as-update can't revert a CRDT" is exactly why); `history.rs` repointed
  entirely to yrs (op-model writers no longer called); client dialog shows
  auto+named interleaved.
- **Local (Phase 4):** `doc_version` table (additive, no SCHEMA_VERSION bump);
  auto-capture in `save_doc`; FFI `list/create/restore_local_version`; restore =
  hard reset (single-device, the `rollback_doc` shape) with the pre-restore state
  preserved as an auto version. Wired to the same dialog for local pages.
- **Deferred:** rich read-only preview in the editor (the `GET /versions/{id}`
  endpoint returns blocks and is ready; the client shows the timeline + restore,
  restore being non-destructive/undoable). Retire the dead op-model history
  store fns (`create_named_version`/`restore_snapshot`) — left in place, unused.

The original plan follows for reference.

---

## 1. The reality today: there is no usable history

A version-history feature sounds like "just expose what we already store," but
Mica stores **only the latest state**, and the one history API that exists is
**dead for real documents**.

- **Two server representations of a cloud doc:**
  - Op-model `document_snapshots` (jsonb blocks, one row per `version_seq`) +
    `document_updates` op log — `migrations/0001_initial.sql:66-94`.
  - yrs CRDT `document_yrs_base` (`state bytea`, **PK = document_id → one row,
    overwritten every push**) + the per-workspace `workspace_updates` stream —
    `migrations/0003_workspace_yrs_sync.sql:13-31`.
- **Live editing goes through yrs only.** `sync::push_update`
  (`crates/app-core/src/sync.rs:158`) folds each update onto the base and
  **upserts `document_yrs_base` (overwrite)** — it never writes
  `document_snapshots`. Since **P4①b** dropped the periodic snapshot fold, the
  op-model snapshot **freezes at the seed** for any normally-edited page. This
  is the documented "snapshot/yrs split-brain" (`store.rs:244-257` comment;
  `store::current_payload:258-277` papers over it on reads by overriding
  blocks from yrs).
- **The existing history API is wired but broken for yrs docs.** `/history`,
  `POST /versions`, `GET /versions/{id}`, `POST /restore`
  (`crates/api-server/src/routes/history.rs`, registered
  `routes/mod.rs:148-163`) + `document_versions` table
  (`0001_initial.sql:87-94`) all operate on the **frozen** op-model:
  `create_named_version` pins `latest_snapshot` (the seed); `restore_snapshot`
  writes `document_snapshots` and **never touches `document_yrs_base`**, then
  broadcasts an op-model event yrs clients don't apply. Exposed as-is it would
  **blank the page or silently no-op.** The Flutter client calls none of it.
- **The yrs stream is pruned.** `workspace_updates` keeps only a rolling
  ~64-update tail (`sync.rs:26-28` `STREAM_KEEP_MARGIN=64` /
  `STREAM_PRUNE_EVERY=32`; delete at `sync.rs:203-209`) — it's for catch-up,
  not history.
- **Local docs have no timeline either.** `crates/mica-core/src/store.rs`:
  `doc_snapshot(doc_id PK)` overwritten by `save_doc`; a **single-slot**
  `doc_snapshot_backup` (`store.rs:145`) via `checkpoint_doc`/`rollback_doc`
  (one-level undo, not versioning); `squash` (`store.rs:624`) **deletes** the
  update logs. FFI exposes no version methods.
- **The only working history is external:** `mica-cli export` (live content via
  `current_payload`) + user-run restic/rustic → a coarse, whole-workspace,
  Markdown-lossy, per-run mirror with no in-editor restore.

**Conclusion:** history must be built **yrs-first**. We can reuse the existing
`/history` HTTP shape, but every read/write must route through yrs, never the
frozen op-model.

## 2. Recommended architecture — Option B (yrs-native snapshots)

Mirrors how **AFFiNE** ("Page History") and **AppFlowy** (collab snapshot
plugin) do it: store **periodic full-state yjs/yrs snapshots** in a dedicated
table, retained by policy; restore applies a snapshot as a new update. Both
deliberately avoid yjs/yrs *native* time-travel (`Snapshot` +
`encode_state_from_snapshot`) because it needs GC disabled (state bloat); Mica
also uses default GC (`doc.rs:195` `encode_state_as_update_v1`), so we store
full-state blobs too.

### Data model (new table)

```sql
-- migrations/000X_document_yrs_versions.sql
CREATE TABLE document_yrs_versions (
  id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  document_id  uuid NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
  rid          bigint,            -- workspace_updates rid at capture (ordering)
  label        text,             -- NULL = auto snapshot; set = named version
  created_by   uuid,             -- user id, NULL for system
  created_at   timestamptz NOT NULL DEFAULT now(),
  state        bytea NOT NULL     -- yrs encode_state_as_update_v1 (full state)
);
CREATE INDEX ON document_yrs_versions (document_id, created_at DESC);
```

- **Snapshot = full yrs state** (`MicaDoc::encode_state`, `doc.rs:195`). Compact
  under v1 encoding; add retention (§5) to bound growth.
- **Key on `created_at` / `rid`, NOT `version_seq`** — the op-model `version_seq`
  is decoupled from real edits (`documents.rs:679-688`).

### Triggers (auto + manual — the chosen strategy)

- **Auto:** in `sync::push_update`, reuse the existing `rid % K` prune hook
  (`sync.rs:203`) to also snapshot every **K pushes** AND/OR when
  `now - last_snapshot > T` (e.g. K=50 pushes or T=10 min, whichever first).
  Cheap: the folded base state is already in hand.
- **Manual:** `POST /versions {label}` snapshots the current base immediately
  with the label set.

### Operations (all yrs, reuse `doc.rs` primitives)

- **List:** metadata query (no blobs) → timeline.
- **Preview:** decode the blob into a **throwaway** `MicaDoc::from_update`
  (`doc.rs`) → `to_blocks()`; render read-only. Never touches the live base.
- **Restore:** load target `state`, compute a diff against the current base
  (`sync::diff_from_base` / `MicaDoc::encode_diff`, `doc.rs:213`) OR apply the
  target state as a normal update, fold via `push_update`, and **broadcast it
  as a yrs `sync` update**. Append-only ⇒ converges on every connected client;
  restore is "a new update on top," not a hard reset (safe under concurrency).

## 3. Server changes

1. **Migration** — the table above.
2. **`sync::push_update`** (`sync.rs:158`) — after the fold, on the cadence
   condition, `INSERT` an auto version row (state = the just-folded base).
3. **`crates/app-core/src/store.rs`** — new functions, all yrs-based:
   `list_yrs_versions(document_id)`, `fetch_yrs_version(id) -> bytea`,
   `create_named_yrs_version(document_id, label, user)`,
   `restore_yrs_version(id) -> update-to-broadcast`.
4. **`routes/history.rs`** — repoint `get_history` / `get_version` /
   `create_version` / `restore` at the yrs functions. `get_version` returns
   blocks (decode → `to_blocks`) for preview. `restore` folds onto
   `document_yrs_base` and broadcasts a yrs update (NOT `broadcast_applied_update`
   op event).
5. **Retire the op-model history writers** (`store::create_named_version`,
   `restore_snapshot` on `document_snapshots`) or guard them off — they're the
   split-brain trap.

## 4. Client changes (Flutter)

- **History panel** — a timeline list (created_at, label, author) from
  `GET /history`. Entry point: page menu / a "History" affordance near the
  outline panel.
- **Preview** — load a version's blocks into the editor **read-only** (reuse the
  bootstrap path with `canEdit=false`); a banner "viewing version <time>" with
  Restore / Close.
- **Restore** — `POST /restore`; the shell applies the broadcast yrs update via
  the existing pull path → the live editor updates. No client-side merge logic.
- New API-client methods in `lib/api/client.dart` (the endpoints already exist
  server-side once repointed).

## 5. Retention (bound storage growth)

- Thin auto snapshots: keep **all for 1h → hourly for 1d → daily for 1w →
  weekly beyond**. **Named versions are never auto-pruned.**
- Run the thin pass in `push_update` (alongside the existing prune) or a
  periodic task.

## 6. Local / offline docs — Phase 2 (deferred per the chosen scope)

Cloud-first MVP means local docs have **no** history until this phase.

- Generalize the single-slot `doc_snapshot_backup` → a multi-row
  `doc_version(doc_id, seq, state, created_at, label)` in
  `crates/mica-core/src/store.rs`.
- Expose list/fetch/create/restore over FFI
  (`clients/mica_flutter/rust/src/api/store.rs`); regenerate the bridge.
- Make `squash` (`store.rs:624`) checkpoint a version **before** collapsing.

## 7. Staged implementation plan

- **Phase 0 — storage + capture (no UI).** Migration + `store.rs` functions +
  auto-snapshot trigger in `push_update`. Verify snapshots accumulate on edit;
  retention thins them. Rust tests.
- **Phase 1 — endpoints repointed to yrs.** `history.rs` list/preview/restore on
  the yrs table; restore folds + broadcasts. Tests: restore converges across two
  sessions; preview is read-only; named version round-trips.
- **Phase 2 — client UI.** History panel + read-only preview + restore action.
- **Phase 3 — retention/thinning.**
- **Phase 4 (optional) — local docs.** FFI + `doc_version` storage + squash
  checkpoint.

## 8. Risks

1. **Split-brain (highest).** Everything must route through yrs; never write
   `document_snapshots` for this feature; retire the frozen op-model history
   writers. (`store.rs:467`, `store.rs:512-557`, `history.rs:158`.)
2. **Storage growth** — retention policy is not optional.
3. **`version_seq` is unusable** as a history key for yrs docs — key on
   `rid`/`created_at`.
4. **No native yrs time-travel** without disabling GC (a costly global change)
   — use stored full-state blobs.
5. **Restore under concurrency** — apply-as-update converges but merges with
   in-flight edits; frame restore as "restore = a new revision," not a reset.

## 9. Prior art

- **AFFiNE — Page History:** server stores periodic full yjs snapshots
  (binary `encodeStateAsUpdate`) with timestamps, retained by plan tier; restore
  applies a snapshot as a new update. Closest match — mirror it.
- **AppFlowy:** `collab` snapshot plugin persists periodic full-state blobs keyed
  by object id + timestamp, separate from the live doc. Same shape.

## 10. Key files (for whoever implements)

- `crates/app-core/src/store.rs` — `current_payload:258` (+ comment `244-257`),
  `restore_snapshot:499`, `create_named_version:459`, `apply_derived_operations:114`.
- `crates/app-core/src/sync.rs` — `push_update:158`, prune `203-209`,
  `diff_from_base:149`, `document_base`.
- `crates/api-server/src/routes/history.rs`; `routes/mod.rs:148-163`;
  `routes/ws.rs:311,416` (op vs yrs write paths).
- `crates/mica-core/src/store.rs` — `doc_snapshot_backup:145`, `squash:624`,
  `checkpoint_doc:725`; `crates/mica-core/src/doc.rs:195-215` (yrs primitives).
- `clients/mica_flutter/rust/src/api/store.rs:179-358` (FFI); `lib/api/client.dart`.
- `migrations/0001_initial.sql:66-94`; `migrations/0003_workspace_yrs_sync.sql:13-31`.
