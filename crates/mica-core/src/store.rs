//! On-device local store + identity (P2-M2), SQLite via rusqlite.
//!
//! Feature-gated `store` — desktop client only; the server never pulls SQLite in.
//! M2 persists each document as a full base snapshot (a yrs v1 update). The
//! incremental update queue + squash + sync cursor (§4/§5) arrive with
//! incremental saves and cloud sync (P2-M4).

use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};

use crate::doc::{DocError, MicaDoc};

#[derive(thiserror::Error, Debug)]
pub enum StoreError {
  #[error("sqlite error: {0}")]
  Sqlite(#[from] rusqlite::Error),
  #[error(transparent)]
  Doc(#[from] DocError),
  /// The store was written by a newer app version (§10): refuse to open rather
  /// than silently corrupt or drop data on a downgrade.
  #[error("store schema v{found} is newer than supported v{supported}; upgrade the app")]
  SchemaTooNew { found: i64, supported: i64 },
  /// A stored CRDT blob is unusable: it failed its recorded checksum
  /// ([`verify_blob_crc`]) or made yrs panic while integrating it
  /// ([`contain_yrs_panic`]). Recoverable — the local store is a cache and the
  /// cloud holds the authoritative copy, so the caller drops it and re-seeds.
  #[error("corrupt local CRDT state for {doc_id} (checksum or yrs integration failed)")]
  CorruptDoc { doc_id: String },
  /// A view write that would break the page tree: a page as a parent, or a
  /// parent cycle. The local twin of the server's
  /// `ensure_parent_accepts_children` (400 there, refused here).
  #[error("view {view_id} cannot live under {parent_id}: {reason}")]
  InvalidParent {
    view_id: String,
    parent_id: String,
    reason: &'static str,
  },
}

/// The page-link graph of the local workspace: nodes that participate in at
/// least one link, the edges between them, and a COUNT of everything else.
#[derive(Debug, Clone)]
pub struct LocalGraph {
  pub nodes: Vec<LocalGraphNode>,
  pub edges: Vec<LocalGraphEdge>,
  /// Live documents that no link touches. A count, not nodes — see
  /// [`LocalStore::graph_local`].
  pub unlinked: i64,
}

#[derive(Debug, Clone)]
pub struct LocalGraphNode {
  pub view_id: String,
  pub name: String,
  /// Edges touching this node (in + out), so the client can size hubs without
  /// counting edges itself.
  pub degree: i64,
}

#[derive(Debug, Clone)]
pub struct LocalGraphEdge {
  pub source: String,
  pub target: String,
}

/// One page that links TO the page being viewed. Mirrors the cloud `Backlink`
/// so the panel renders both worlds from the same shape.
#[derive(Debug, Clone)]
pub struct LocalBacklink {
  pub view_id: String,
  pub object_id: String,
  pub name: String,
}

/// One local search result. Mirrors the cloud's hit shape so the Dart side has
/// one model, not two.
#[derive(Debug, Clone)]
pub struct LocalSearchHit {
  pub view_id: String,
  pub object_id: String,
  pub name: String,
  pub parent_id: Option<String>,
  pub is_folder: bool,
  /// Matched on the title rather than the body — the row says so, the same way
  /// the server's `title_match` does, so a bodyless hit does not read as noise.
  pub title_match: bool,
  /// A window of body text around the match. Empty when the hit was on the
  /// title only.
  pub snippet: String,
}

/// Both index projections of a stored state from ONE decode + ONE flatten:
/// `content_text` (plain body text, for search — the same projection the cloud
/// keeps in `document_yrs_base.content_text`, so a page reads the same either
/// side) and `link_targets` (the pages it links TO, as a JSON array — derived
/// by the SAME [`crate::page_link_targets`] so the two worlds agree on what a
/// backlink is). They used to decode the blob independently — two full CRDT
/// decodes (and two flattens) per save, on top of the caller's own.
///
/// `link_targets` is JSON rather than a child table because `upsert_snapshot`
/// is ONE statement: a second table would need a transaction around it and
/// could then drift from `state`, which is exactly the property this store
/// gets for free. SQLite reads it back with `json_each`, so it is still
/// queryable, not an opaque blob. Sorted + deduplicated: a page linked from
/// five blocks is one edge.
///
/// Each is `None` — never an error — when it cannot be derived. See
/// [`LocalStore::upsert_snapshot`]: the index is allowed to miss a document,
/// the save is not allowed to fail because of it.
fn derive_projections(doc_id: &str, state: &[u8]) -> (Option<String>, Option<String>) {
  let doc = match contain_yrs_panic(doc_id, || Ok(MicaDoc::from_update(state)?)) {
    Ok(doc) => doc,
    Err(_) => return (None, None),
  };
  derive_projections_from_doc(&doc)
}

/// [`derive_projections`] over an already-decoded doc. The compaction paths
/// ([`LocalStore::squash`] / [`LocalStore::compact_local`]) hold the doc they
/// just folded; decoding the fresh state again inside `upsert_snapshot` would
/// be a second full CRDT decode of the same content.
fn derive_projections_from_doc(doc: &MicaDoc) -> (Option<String>, Option<String>) {
  let blocks = doc.to_blocks();
  let content_text = blocks
    .iter()
    .map(|b| b.text.trim_end_matches('\n'))
    .filter(|t| !t.is_empty())
    .collect::<Vec<_>>()
    .join(
      "
",
    );
  let mut targets: Vec<String> = blocks
    .iter()
    .flat_map(|b| crate::page_link_targets(&b.data))
    .collect();
  targets.sort_unstable();
  targets.dedup();
  (Some(content_text), serde_json::to_string(&targets).ok())
}

/// Does `table` (a live, opened table) have a column named `col`?
///
/// The migrations in [`LocalStore::open`] are probe-driven — they ask the
/// schema itself instead of trusting a stored version number, because a store
/// may have been opened (and partly migrated) by any older build, so columns
/// genuinely arrive independently. Every probe used to carry this same
/// query_row boilerplate; now each reads as one line. `false` on a SQLite
/// error, the same tolerance the inline probes had.
fn has_column(conn: &Connection, table: &str, col: &str) -> bool {
  conn
    .query_row(
      &format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name='{col}'"),
      [],
      |r| r.get::<_, i64>(0),
    )
    .map(|c| c > 0)
    .unwrap_or(false)
}

/// Does the database contain `table` at all? Read-only (`sqlite_master`), so it
/// is safe to ask BEFORE any DDL — which is exactly where [`LocalStore::open`]'s
/// too-new gate asks it. `false` on a SQLite error, same tolerance as
/// [`has_column`].
fn has_table(conn: &Connection, table: &str) -> bool {
  conn
    .query_row(
      "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
      params![table],
      |r| r.get::<_, i64>(0),
    )
    .map(|c| c > 0)
    .unwrap_or(false)
}

/// Run yrs decode/integrate work, turning a panic into [`StoreError::CorruptDoc`].
///
/// yrs integrates an update by INDEXING into its block store, and on malformed
/// bytes it panics instead of returning `Err`. Seen in the wild as:
///
/// ```text
/// panicked at yrs-0.27.0/src/block_store.rs:60:46:
/// index out of bounds: the len is 732 but the index is 715827364
/// ```
///
/// Note `Update::decode_v1` had already SUCCEEDED there — the blow-up is during
/// integration — so the `?` on these calls never had a chance to contain it.
///
/// Containing it matters far beyond one bad load. Callers hold a mutex across
/// this (the desktop FFI's `Mutex<LocalStore>`), so the panic poisoned the lock
/// and every later `list_views` failed too: the page tree went blank and STAYED
/// blank until the app restarted, on every launch, because cloud seeding calls
/// `load_doc` at startup. A corrupt local blob is recoverable — this store is a
/// cache and the cloud holds the document — so it must degrade to an error the
/// caller can re-seed from, not take the process down.
fn contain_yrs_panic<T>(
  doc_id: &str,
  work: impl FnOnce() -> Result<T, StoreError>,
) -> Result<T, StoreError> {
  // The default hook still prints the panic, which is what makes the
  // underlying corruption diagnosable instead of silently swallowed.
  match std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)) {
    Ok(result) => result,
    Err(_) => Err(StoreError::CorruptDoc {
      doc_id: doc_id.to_string(),
    }),
  }
}

/// CRC-32 (IEEE 802.3 / zlib: reflected, poly 0xEDB88320, init & xorout
/// 0xFFFFFFFF) lookup table, built at COMPILE time — no runtime init, no
/// `OnceLock`, no dependency. Principle #1: ~20 lines of textbook algorithm
/// beats pulling `crc32fast` for a border-layer need.
const CRC32_TABLE: [u32; 256] = {
  let mut table = [0u32; 256];
  let mut i = 0;
  while i < 256 {
    let mut crc = i as u32;
    let mut j = 0;
    while j < 8 {
      crc = if crc & 1 == 1 {
        (crc >> 1) ^ 0xEDB8_8320
      } else {
        crc >> 1
      };
      j += 1;
    }
    table[i] = crc;
    i += 1;
  }
  table
};

/// CRC-32 (IEEE) of `bytes`. An ACCIDENTAL-corruption detector (bit-rot, torn
/// writes) — deliberately NOT a security primitive. The threat model for a
/// local on-device cache is a damaged disk, not an attacker, so a cryptographic
/// hash would be over-reach; CRC-32 catches every single-bit / short-burst /
/// truncation error a storage fault produces.
fn crc32(bytes: &[u8]) -> u32 {
  let mut crc = 0xFFFF_FFFFu32;
  for &b in bytes {
    crc = (crc >> 8) ^ CRC32_TABLE[((crc ^ b as u32) & 0xFF) as usize];
  }
  crc ^ 0xFFFF_FFFF
}

/// The checksum to persist beside a freshly-authored blob (SQLite stores it as
/// a signed `INTEGER`; a `u32` widens losslessly and stays non-negative).
fn blob_crc(bytes: &[u8]) -> i64 {
  crc32(bytes) as i64
}

/// Verify a stored blob against its recorded checksum BEFORE it reaches yrs.
///
/// A damaged blob may still be a valid, decodable update with different content.
/// Decode errors and `contain_yrs_panic` cannot detect that. The checksum is an
/// integrity check independent of yrs's malformed-input fixes in 0.28.0.
///
/// `stored_crc == None` ⇒ a legacy row written before checksums existed (or by
/// an older app): nothing to check, so PASS and let the bytes fall through to
/// the existing `contain_yrs_panic` guard. Such rows gain integrity protection
/// when next rewritten; the checksum does not retroactively bless old data.
fn verify_blob_crc(doc_id: &str, bytes: &[u8], stored_crc: Option<i64>) -> Result<(), StoreError> {
  if let Some(expected) = stored_crc {
    if blob_crc(bytes) != expected {
      return Err(StoreError::CorruptDoc {
        doc_id: doc_id.to_string(),
      });
    }
  }
  Ok(())
}

/// On-disk schema version (§10 — explicit version + downgrade guard). Bump when
/// the table layout changes and add the migration in [`LocalStore::open`].
/// v7 = origin in every document-level primary key (issue #102).
const SCHEMA_VERSION: i64 = 7;

/// Auto version cadence — collapse a burst of saves to one version per 10 min
/// (AFFiNE's min-interval). Retention — auto versions live 30 days; named ones
/// forever.
const VERSION_AUTO_INTERVAL_MS: i64 = 10 * 60 * 1000;
const VERSION_AUTO_RETENTION_MS: i64 = 30 * 24 * 60 * 60 * 1000;

/// One entry in a local page's version timeline. `label` is None for an auto
/// snapshot, Some for a named checkpoint. `created_at` is unix millis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalVersion {
  pub id: String,
  pub label: Option<String>,
  pub created_at: i64,
}

/// Stable on-device identity. `client_id` is the yrs actor id used for every doc
/// on this device — minted once and reused forever, never random per launch (§6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
  pub device_id: String,
  pub client_id: u64,
}

/// A document's sync high-water marks against the cloud (P2-M4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SyncCursor {
  /// Highest cloud stream id (`rid`) this device has pulled and applied.
  pub last_synced_rid: i64,
  /// Highest local update `clock` this device has pushed to the cloud.
  pub pushed_clock: i64,
}

/// A local workspace (P2-M3): a named container for a page tree. The on-device
/// store can hold several, mirroring the cloud workspace list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalWorkspace {
  pub id: String,
  pub name: String,
  /// Zero-padded ordering among workspaces.
  pub position: String,
  /// Provenance: `"local"` for on-device workspaces, or a server URL for a
  /// cloud workspace mirrored for offline navigation (P2 option C). Scopes a
  /// store that holds both.
  pub origin: String,
  /// The user's membership role in this workspace (`"owner"`/`"admin"`/
  /// `"editor"`/`"viewer"`), mirrored from the server so an offline (re)start
  /// knows whether editing is allowed (P2d). Local workspaces are the user's
  /// own; the client treats them as owner regardless.
  pub role: String,
}

/// A page-tree node (P2-M3): the local mirror of the client's `DocumentView`.
/// Scoped to a `workspace_id`. Each view points at one document (`object_id` =
/// its `doc_id` in `doc_snapshot`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalView {
  pub id: String,
  pub workspace_id: String,
  pub parent_id: Option<String>,
  pub object_id: String,
  pub name: String,
  /// Zero-padded sibling ordering (same convention as the cloud `position`).
  pub position: String,
  /// Soft-deleted into the trash; purge removes the row.
  pub trashed: bool,
  /// Provenance: `"local"` or a server URL — same scoping as [`LocalWorkspace`].
  pub origin: String,
  /// `"document"` (default) or `"folder"` (a pure container, no content).
  /// Mirrored so the offline nav can render/behave correctly without a server.
  pub object_type: String,
}

/// A local document store backed by one SQLite file.
pub struct LocalStore {
  /// `pub(crate)` so sibling modules ([`crate::view_ops`] — the transactional
  /// page-tree subtree operations, issue #67) can run their own SQL against
  /// the same connection instead of re-listing every table through the
  /// public whole-row API.
  pub(crate) conn: Connection,
}

impl LocalStore {
  /// Open (creating if needed) the store at `path` and ensure the schema.
  pub fn open(path: &str) -> Result<Self, StoreError> {
    let conn = Connection::open(path)?;
    // Schema version gate (§10) — FIRST, before any DDL or PRAGMA touches the
    // file. It used to run after the CREATE/PRAGMA batch, which was already too
    // late: `PRAGMA journal_mode=WAL` rewrites the database header and journal
    // mode, and the CREATE batch adds tables a newer app may not expect — so a
    // user who downgrades would have their NEWER store mutated before being
    // told the version is unsupported. Nothing above this point may write.
    //
    // A store with no `local_meta` at all (a fresh file, or one older than that
    // table) has no version to guard; the probe is read-only either way.
    let stored: i64 = if has_table(&conn, "local_meta") {
      conn
        .query_row(
          "SELECT value FROM local_meta WHERE key='schema_version'",
          [],
          |r| r.get::<_, String>(0),
        )
        .optional()?
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
    } else {
      0
    };
    if stored > SCHEMA_VERSION {
      return Err(StoreError::SchemaTooNew {
        found: stored,
        supported: SCHEMA_VERSION,
      });
    }
    conn.execute_batch(
      "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
            -- Document-level rows are keyed by (origin, doc_id) — issue #102. The
            -- page tree is already origin-scoped (local_view/local_workspace), but a
            -- backup restore, a server migration or a cloned environment keeps
            -- document UUIDs: two origins can then legitimately hold the SAME doc id.
            -- Keyed by a bare doc_id they would share one body, one sync cursor and
            -- one offline outbox, and clearing one origin's mirror would delete the
            -- other origin's text. `origin` carries the same value as the owning
            -- view's ('local' for on-device pages, a server URL for cloud mirrors).
            CREATE TABLE IF NOT EXISTS doc_snapshot(
                origin       TEXT NOT NULL,
                doc_id       TEXT NOT NULL,
                state        BLOB NOT NULL,
                updated_at   INTEGER NOT NULL,
                state_crc    INTEGER,
                -- Plain text of `state` PLUS its update log, for workspace search.
                -- A pure projection: `state` is the authority, this is derived from
                -- it and never read back into a document. NULL = not derived yet (a
                -- row written by an older build, a doc whose decode failed) —
                -- searchable-or-not, never wrong.
                content_text TEXT,
                -- The pages this document links TO, as a JSON array (backlinks /
                -- graph). Same projection contract as `content_text`.
                link_targets TEXT,
                -- 1 = the projections above are behind the update log: an
                -- incremental append landed since they were derived. Readers that
                -- need the index rebuild it instead of answering from stale text
                -- (issue #104); every projection write clears it. Deliberately NOT
                -- folded into `content_text IS NULL`: not-indexed-yet is the
                -- backfill's queue, indexed-then-edited is a read's job, and one
                -- flag for both would make a broken legacy row re-decode on every
                -- search.
                projection_stale INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY(origin, doc_id)
            );
            CREATE TABLE IF NOT EXISTS local_meta(
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS local_view(
                origin       TEXT NOT NULL DEFAULT 'local',
                id           TEXT NOT NULL,
                workspace_id TEXT NOT NULL DEFAULT 'local',
                parent_id    TEXT,
                object_id    TEXT NOT NULL,
                name         TEXT NOT NULL,
                position     TEXT NOT NULL,
                trashed      INTEGER NOT NULL DEFAULT 0,
                object_type  TEXT NOT NULL DEFAULT 'document',
                PRIMARY KEY(origin, id)
            );
            CREATE TABLE IF NOT EXISTS local_workspace(
                origin   TEXT NOT NULL DEFAULT 'local',
                id       TEXT NOT NULL,
                name     TEXT NOT NULL,
                position TEXT NOT NULL,
                role     TEXT NOT NULL DEFAULT 'viewer',
                PRIMARY KEY(origin, id)
            );
            CREATE TABLE IF NOT EXISTS doc_update(
                origin      TEXT NOT NULL,
                doc_id      TEXT NOT NULL,
                clock       INTEGER NOT NULL,
                payload     BLOB NOT NULL,
                payload_crc INTEGER,
                PRIMARY KEY(origin, doc_id, clock)
            );
            CREATE TABLE IF NOT EXISTS doc_remote_update(
                origin      TEXT NOT NULL,
                doc_id      TEXT NOT NULL,
                rid         INTEGER NOT NULL,
                payload     BLOB NOT NULL,
                payload_crc INTEGER,
                PRIMARY KEY(origin, doc_id, rid)
            );
            CREATE TABLE IF NOT EXISTS sync_cursor(
                origin          TEXT NOT NULL,
                doc_id          TEXT NOT NULL,
                last_synced_rid INTEGER NOT NULL DEFAULT 0,
                pushed_clock    INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY(origin, doc_id)
            );
            CREATE TABLE IF NOT EXISTS doc_snapshot_backup(
                origin     TEXT NOT NULL,
                doc_id     TEXT NOT NULL,
                state      BLOB NOT NULL,
                updated_at INTEGER NOT NULL,
                state_crc  INTEGER,
                PRIMARY KEY(origin, doc_id)
            );
            -- Local page version history (docs/features/version-history-plan.md §6). One
            -- row = one full yrs state. `label` NULL = an auto snapshot (captured
            -- on a cadence by save_doc, pruned after retention); set = a named
            -- checkpoint, kept forever. Keyed by (origin, id): version ids are this
            -- device's own, but the ROW belongs to one origin's document.
            CREATE TABLE IF NOT EXISTS doc_version(
                origin     TEXT NOT NULL,
                id         TEXT NOT NULL,
                doc_id     TEXT NOT NULL,
                label      TEXT,
                created_at INTEGER NOT NULL,
                state      BLOB NOT NULL,
                state_crc  INTEGER,
                PRIMARY KEY(origin, id)
            );",
    )?;
    // (The too-new gate already ran, before the first write of any kind; the
    // migrations below are probe-driven — column/PK shape, not version — and
    // the v4/v7 rebuilds are destructive, which is exactly why the gate could
    // not stay here: run against a future schema they would silently strip
    // columns the newer app added. The stamp happens at the end, after every
    // migration succeeded.)

    // Migrate pre-multi-workspace stores: add the workspace_id column and a
    // default workspace that existing views attach to.
    if !has_column(&conn, "local_view", "workspace_id") {
      conn.execute(
        "ALTER TABLE local_view ADD COLUMN workspace_id TEXT NOT NULL DEFAULT 'local'",
        [],
      )?;
    }
    // `content_text` on an existing store. Additive and nullable, so this is the
    // whole migration: no rewrite, no version bump, and an older build that
    // opens the file afterwards simply never selects the column.
    if !has_column(&conn, "doc_snapshot", "content_text") {
      conn.execute("ALTER TABLE doc_snapshot ADD COLUMN content_text TEXT", [])?;
    }
    // Same additive shape for `link_targets` (the backlink index). Two
    // separate PRAGMA checks rather than one version bump: a store may have
    // been opened by a build that had content_text but not this, so the two
    // columns genuinely arrive independently.
    if !has_column(&conn, "doc_snapshot", "link_targets") {
      conn.execute("ALTER TABLE doc_snapshot ADD COLUMN link_targets TEXT", [])?;
    }
    // Always have a default workspace so migrated views resolve and a fresh
    // store starts usable.
    conn.execute(
            "INSERT OR IGNORE INTO local_workspace(id,name,position) VALUES('local','本地工作区','0000000010')",
            [],
        )?;

    // v2 (P2 option C): add `origin` so a cloud page tree can be mirrored into
    // the same store as local-only workspaces, scoped by server URL ('local'
    // for on-device content). Existing rows default to 'local' — the
    // default-only ALTER preserves them. Table names are compile-time literals.
    for table in ["local_view", "local_workspace"] {
      if !has_column(&conn, table, "origin") {
        conn.execute(
          &format!("ALTER TABLE {table} ADD COLUMN origin TEXT NOT NULL DEFAULT 'local'"),
          [],
        )?;
      }
    }

    // v3 (P2d): persist each mirrored cloud workspace's `role` so an offline
    // (re)start knows whether this user may edit it — otherwise the offline
    // nav forces read-only. Local workspaces are the user's own (the UI treats
    // them as owner); the default only affects rows until they're re-mirrored.
    if !has_column(&conn, "local_workspace", "role") {
      conn.execute(
        "ALTER TABLE local_workspace ADD COLUMN role TEXT NOT NULL DEFAULT 'viewer'",
        [],
      )?;
    }

    // v4 (P3a): promote the (origin, id) pair to the PRIMARY KEY so the
    // local/cloud namespace isolation becomes a constraint, not a convention
    // — a bare-id PK lets `ON CONFLICT(id)` upsert and `purge_view(id)`
    // reach across origins, which P3's detach ("same id under 'local' AND a
    // server URL") would turn from a theoretical hazard into a certainty.
    // SQLite can't ALTER a primary key, so this is the rebuild-table dance,
    // atomic in one transaction, with a one-time pre-migration snapshot of
    // both tables (recovery hatch, doc_snapshot_backup-style). Runs after
    // the column ALTERs above so every SELECTed column exists; detected via
    // the PK column count (bare id = 1, composite = 2).
    let view_pk_cols: i64 = conn.query_row(
      "SELECT COUNT(*) FROM pragma_table_info('local_view') WHERE pk>0",
      [],
      |r| r.get(0),
    )?;
    if view_pk_cols < 2 {
      conn.execute_batch(
                "BEGIN;
                 CREATE TABLE IF NOT EXISTS local_view_v3_backup AS SELECT * FROM local_view;
                 DROP TABLE IF EXISTS local_view_v4;
                 CREATE TABLE local_view_v4(
                     origin       TEXT NOT NULL DEFAULT 'local',
                     id           TEXT NOT NULL,
                     workspace_id TEXT NOT NULL DEFAULT 'local',
                     parent_id    TEXT,
                     object_id    TEXT NOT NULL,
                     name         TEXT NOT NULL,
                     position     TEXT NOT NULL,
                     trashed      INTEGER NOT NULL DEFAULT 0,
                     PRIMARY KEY(origin, id)
                 );
                 INSERT INTO local_view_v4(origin,id,workspace_id,parent_id,object_id,name,position,trashed)
                     SELECT origin,id,workspace_id,parent_id,object_id,name,position,trashed FROM local_view;
                 DROP TABLE local_view;
                 ALTER TABLE local_view_v4 RENAME TO local_view;
                 COMMIT;",
            )?;
    }
    let ws_pk_cols: i64 = conn.query_row(
      "SELECT COUNT(*) FROM pragma_table_info('local_workspace') WHERE pk>0",
      [],
      |r| r.get(0),
    )?;
    if ws_pk_cols < 2 {
      conn.execute_batch(
                "BEGIN;
                 CREATE TABLE IF NOT EXISTS local_workspace_v3_backup AS SELECT * FROM local_workspace;
                 DROP TABLE IF EXISTS local_workspace_v4;
                 CREATE TABLE local_workspace_v4(
                     origin   TEXT NOT NULL DEFAULT 'local',
                     id       TEXT NOT NULL,
                     name     TEXT NOT NULL,
                     position TEXT NOT NULL,
                     role     TEXT NOT NULL DEFAULT 'viewer',
                     PRIMARY KEY(origin, id)
                 );
                 INSERT INTO local_workspace_v4(origin,id,name,position,role)
                     SELECT origin,id,name,position,role FROM local_workspace;
                 DROP TABLE local_workspace;
                 ALTER TABLE local_workspace_v4 RENAME TO local_workspace;
                 COMMIT;",
            )?;
    }

    // v6 (folder type): mirror a view's `object_type` so a folder (pure
    // container) survives offline — otherwise the offline nav can't tell a
    // folder from a document. Additive ALTER (default preserves old rows);
    // runs after the v4 rebuild so the rebuilt table (whose hard-coded
    // column list predates this) gets the column too.
    if !has_column(&conn, "local_view", "object_type") {
      conn.execute(
        "ALTER TABLE local_view ADD COLUMN object_type TEXT NOT NULL DEFAULT 'document'",
        [],
      )?;
    }

    // Per-blob checksum columns (docs/reviews/code-review-2026-07-20.md P0-0). ADDITIVE
    // and NULLABLE, so — like the doc_version table before it — no SCHEMA_VERSION
    // bump: every read/write uses an explicit column list (never SELECT * / bare
    // INSERT), so an older app silently ignores the column and a NULL means "no
    // checksum recorded" (verify_blob_crc passes it through). Bumping would trip
    // the too-new gate and lock every v6 app out of its own cache — grossly
    // disproportionate to an optional integrity column. These blob tables have no
    // destructive rebuild path (unlike local_view/local_workspace), so the column
    // survives untouched. Probe-driven + idempotent, same shape as the ALTERs above.
    //
    // Downgrade caveat (deliberately not defended with a trigger): an OLD app that
    // rewrites doc_snapshot.state leaves state_crc STALE (it can't update a column
    // it doesn't know). A later new app then reads new-bytes-vs-old-crc and returns
    // CorruptDoc — which for load_doc is a harmless re-seed from the authoritative
    // cloud (the store is a cache), and for restore/rollback a spurious, retryable
    // error. Harmless enough, and the rollout direction is UPGRADE into this fix,
    // not downgrade, so a hidden reset-trigger on the hot save path isn't worth it.
    for (table, col) in [
      ("doc_snapshot", "state_crc"),
      ("doc_version", "state_crc"),
      ("doc_snapshot_backup", "state_crc"),
      ("doc_update", "payload_crc"),
      ("doc_remote_update", "payload_crc"),
    ] {
      if !has_column(&conn, table, col) {
        conn.execute(&format!("ALTER TABLE {table} ADD COLUMN {col} INTEGER"), [])?;
      }
    }

    // v7 (issue #102): promote `origin` into the primary key of every
    // document-level table. The page tree has been origin-scoped since v2/v4
    // (`local_view.origin`), but documents, their logs, sync cursors and
    // version history were keyed by a bare `doc_id`. Two origins that hold the
    // SAME document UUID — a restored backup, a migrated or cloned server —
    // then shared one body, one cursor and one outbox, and deleting one
    // origin's mirror deleted the other's text. A bare-id primary key is the
    // bug: no amount of call-site discipline fixes it, so the key has to
    // change.
    //
    // Probe: no released store has an `origin` column on these tables, and any
    // store that does has been rebuilt around the composite key (the rebuild is
    // one transaction, so the column and the key always arrive together). One
    // `has_column` per table therefore detects exactly the legacy shape — a PK
    // count would not: `doc_update`'s legacy key was already two columns.
    // Each rebuild runs in ONE transaction, so a crash leaves the old table whole.
    //
    // Which origin does an existing row get? The origin of the view that
    // references its document, which is the only thing that can answer: a
    // pre-v7 store had a single document row per id, so it belonged to
    // whichever page tree pointed at it. A doc referenced from several origins
    // (possible: 'local' plus a cloud mirror sharing a UUID) is COPIED to each
    // — the old store cannot say which copy was "the" one, and duplicating is
    // the only lossless answer. A doc NO view references (an orphan left by a
    // crashed purge, or one seeded before its view) stays under 'local', the
    // origin every pre-multi-origin store was.
    if !has_column(&conn, "doc_snapshot", "origin") {
      conn.execute_batch(
        "BEGIN;
         CREATE TABLE doc_snapshot_v7(
             origin      TEXT NOT NULL,
             doc_id      TEXT NOT NULL,
             state       BLOB NOT NULL,
             updated_at  INTEGER NOT NULL,
             state_crc   INTEGER,
            content_text TEXT,
            link_targets TEXT,
            projection_stale INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY(origin, doc_id));
        -- `projection_stale` is DERIVED, not reset to 0: a pre-v7 row may have
        -- an un-folded log (the projections describe the base as of the last
        -- fold). Hard-writing 0 would tell every reader the index is current,
        -- so the appended edits would stay invisible forever (the very bug
        -- #104 fixed). The log tables are still in their legacy shape here,
        -- keyed by bare doc_id — which is exactly what we correlate on.
        INSERT INTO doc_snapshot_v7(origin,doc_id,state,updated_at,state_crc,content_text,link_targets,projection_stale)
            SELECT v.origin,d.doc_id,d.state,d.updated_at,d.state_crc,d.content_text,d.link_targets,
                   CASE WHEN EXISTS (SELECT 1 FROM doc_update u WHERE u.doc_id=d.doc_id)
                          OR EXISTS (SELECT 1 FROM doc_remote_update r WHERE r.doc_id=d.doc_id)
                        THEN 1 ELSE 0 END
            FROM doc_snapshot d
            JOIN (SELECT DISTINCT origin FROM local_view) v
            WHERE EXISTS (SELECT 1 FROM local_view lv
                           WHERE lv.object_id=d.doc_id AND lv.origin=v.origin);
        INSERT INTO doc_snapshot_v7(origin,doc_id,state,updated_at,state_crc,content_text,link_targets,projection_stale)
            SELECT 'local',d.doc_id,d.state,d.updated_at,d.state_crc,d.content_text,d.link_targets,
                   CASE WHEN EXISTS (SELECT 1 FROM doc_update u WHERE u.doc_id=d.doc_id)
                          OR EXISTS (SELECT 1 FROM doc_remote_update r WHERE r.doc_id=d.doc_id)
                        THEN 1 ELSE 0 END
            FROM doc_snapshot d
            WHERE NOT EXISTS (SELECT 1 FROM local_view lv WHERE lv.object_id=d.doc_id);
         DROP TABLE doc_snapshot;
         ALTER TABLE doc_snapshot_v7 RENAME TO doc_snapshot;
         COMMIT;",
      )?;
    }
    if !has_column(&conn, "doc_update", "origin") {
      conn.execute_batch(
        "BEGIN;
         CREATE TABLE doc_update_v7(
             origin      TEXT NOT NULL,
             doc_id      TEXT NOT NULL,
             clock       INTEGER NOT NULL,
             payload     BLOB NOT NULL,
             payload_crc INTEGER,
             PRIMARY KEY(origin, doc_id, clock));
         INSERT INTO doc_update_v7(origin,doc_id,clock,payload,payload_crc)
             SELECT v.origin,d.doc_id,d.clock,d.payload,d.payload_crc
             FROM doc_update d
             JOIN (SELECT DISTINCT origin FROM local_view) v
             WHERE EXISTS (SELECT 1 FROM local_view lv
                            WHERE lv.object_id=d.doc_id AND lv.origin=v.origin);
         INSERT INTO doc_update_v7(origin,doc_id,clock,payload,payload_crc)
             SELECT 'local',d.doc_id,d.clock,d.payload,d.payload_crc
             FROM doc_update d
             WHERE NOT EXISTS (SELECT 1 FROM local_view lv WHERE lv.object_id=d.doc_id);
         DROP TABLE doc_update;
         ALTER TABLE doc_update_v7 RENAME TO doc_update;
         COMMIT;",
      )?;
    }
    if !has_column(&conn, "doc_remote_update", "origin") {
      conn.execute_batch(
        "BEGIN;
         CREATE TABLE doc_remote_update_v7(
             origin      TEXT NOT NULL,
             doc_id      TEXT NOT NULL,
             rid         INTEGER NOT NULL,
             payload     BLOB NOT NULL,
             payload_crc INTEGER,
             PRIMARY KEY(origin, doc_id, rid));
         INSERT INTO doc_remote_update_v7(origin,doc_id,rid,payload,payload_crc)
             SELECT v.origin,d.doc_id,d.rid,d.payload,d.payload_crc
             FROM doc_remote_update d
             JOIN (SELECT DISTINCT origin FROM local_view) v
             WHERE EXISTS (SELECT 1 FROM local_view lv
                            WHERE lv.object_id=d.doc_id AND lv.origin=v.origin);
         INSERT INTO doc_remote_update_v7(origin,doc_id,rid,payload,payload_crc)
             SELECT 'local',d.doc_id,d.rid,d.payload,d.payload_crc
             FROM doc_remote_update d
             WHERE NOT EXISTS (SELECT 1 FROM local_view lv WHERE lv.object_id=d.doc_id);
         DROP TABLE doc_remote_update;
         ALTER TABLE doc_remote_update_v7 RENAME TO doc_remote_update;
         COMMIT;",
      )?;
    }
    if !has_column(&conn, "sync_cursor", "origin") {
      conn.execute_batch(
        "BEGIN;
         CREATE TABLE sync_cursor_v7(
             origin          TEXT NOT NULL,
             doc_id          TEXT NOT NULL,
             last_synced_rid INTEGER NOT NULL DEFAULT 0,
             pushed_clock    INTEGER NOT NULL DEFAULT 0,
             PRIMARY KEY(origin, doc_id));
         INSERT INTO sync_cursor_v7(origin,doc_id,last_synced_rid,pushed_clock)
             SELECT v.origin,d.doc_id,d.last_synced_rid,d.pushed_clock
             FROM sync_cursor d
             JOIN (SELECT DISTINCT origin FROM local_view) v
             WHERE EXISTS (SELECT 1 FROM local_view lv
                            WHERE lv.object_id=d.doc_id AND lv.origin=v.origin);
         INSERT INTO sync_cursor_v7(origin,doc_id,last_synced_rid,pushed_clock)
             SELECT 'local',d.doc_id,d.last_synced_rid,d.pushed_clock
             FROM sync_cursor d
             WHERE NOT EXISTS (SELECT 1 FROM local_view lv WHERE lv.object_id=d.doc_id);
         DROP TABLE sync_cursor;
         ALTER TABLE sync_cursor_v7 RENAME TO sync_cursor;
         COMMIT;",
      )?;
    }
    if !has_column(&conn, "doc_snapshot_backup", "origin") {
      conn.execute_batch(
        "BEGIN;
         CREATE TABLE doc_snapshot_backup_v7(
             origin     TEXT NOT NULL,
             doc_id     TEXT NOT NULL,
             state      BLOB NOT NULL,
             updated_at INTEGER NOT NULL,
             state_crc  INTEGER,
             PRIMARY KEY(origin, doc_id));
         INSERT INTO doc_snapshot_backup_v7(origin,doc_id,state,updated_at,state_crc)
             SELECT v.origin,d.doc_id,d.state,d.updated_at,d.state_crc
             FROM doc_snapshot_backup d
             JOIN (SELECT DISTINCT origin FROM local_view) v
             WHERE EXISTS (SELECT 1 FROM local_view lv
                            WHERE lv.object_id=d.doc_id AND lv.origin=v.origin);
         INSERT INTO doc_snapshot_backup_v7(origin,doc_id,state,updated_at,state_crc)
             SELECT 'local',d.doc_id,d.state,d.updated_at,d.state_crc
             FROM doc_snapshot_backup d
             WHERE NOT EXISTS (SELECT 1 FROM local_view lv WHERE lv.object_id=d.doc_id);
         DROP TABLE doc_snapshot_backup;
         ALTER TABLE doc_snapshot_backup_v7 RENAME TO doc_snapshot_backup;
         COMMIT;",
      )?;
    }
    if !has_column(&conn, "doc_version", "origin") {
      conn.execute_batch(
        "BEGIN;
         CREATE TABLE doc_version_v7(
             origin     TEXT NOT NULL,
             id         TEXT NOT NULL,
             doc_id     TEXT NOT NULL,
             label      TEXT,
             created_at INTEGER NOT NULL,
             state      BLOB NOT NULL,
             state_crc  INTEGER,
             PRIMARY KEY(origin, id));
         INSERT INTO doc_version_v7(origin,id,doc_id,label,created_at,state,state_crc)
             SELECT v.origin,d.id,d.doc_id,d.label,d.created_at,d.state,d.state_crc
             FROM doc_version d
             JOIN (SELECT DISTINCT origin FROM local_view) v
             WHERE EXISTS (SELECT 1 FROM local_view lv
                            WHERE lv.object_id=d.doc_id AND lv.origin=v.origin);
         INSERT INTO doc_version_v7(origin,id,doc_id,label,created_at,state,state_crc)
             SELECT 'local',d.id,d.doc_id,d.label,d.created_at,d.state,d.state_crc
             FROM doc_version d
             WHERE NOT EXISTS (SELECT 1 FROM local_view lv WHERE lv.object_id=d.doc_id);
         DROP TABLE doc_version;
         ALTER TABLE doc_version_v7 RENAME TO doc_version;
         CREATE INDEX doc_version_doc_created_idx
             ON doc_version(origin, doc_id, created_at DESC);
         COMMIT;",
      )?;
    }
    // The version-timeline index now leads with `origin` (v7), so it can only
    // be created once every table above has been rebuilt — on a pre-v7 store
    // the column it indexes does not exist when the base DDL runs. Idempotent:
    // the rebuild path above already created it.
    conn.execute(
      "CREATE INDEX IF NOT EXISTS doc_version_doc_created_idx
             ON doc_version(origin, doc_id, created_at DESC)",
      [],
    )?;

    // Every migration succeeded — stamp the version this app writes. (The
    // too-new gate ran up top, before anything could mutate the store.)
    conn.execute(
      "INSERT INTO local_meta(key,value) VALUES('schema_version',?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
      params![SCHEMA_VERSION.to_string()],
    )?;

    Ok(LocalStore { conn })
  }

  /// An in-memory store (tests).
  pub fn open_in_memory() -> Result<Self, StoreError> {
    Self::open(":memory:")
  }

  fn meta_get(&self, key: &str) -> Result<Option<String>, StoreError> {
    Ok(
      self
        .conn
        .query_row(
          "SELECT value FROM local_meta WHERE key=?1",
          params![key],
          |r| r.get(0),
        )
        .optional()?,
    )
  }

  fn meta_set(&self, key: &str, value: &str) -> Result<(), StoreError> {
    self.conn.execute(
      "INSERT INTO local_meta(key,value) VALUES(?1,?2)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
      params![key, value],
    )?;
    Ok(())
  }

  /// The persisted identity, minting + storing one on first use.
  pub fn identity(&self) -> Result<Identity, StoreError> {
    if let (Some(device_id), Some(cid)) = (self.meta_get("device_id")?, self.meta_get("client_id")?)
    {
      if let Ok(client_id) = cid.parse::<u64>() {
        return Ok(Identity {
          device_id,
          client_id,
        });
      }
    }
    let device_id = uuid::Uuid::new_v4().to_string();
    let raw = uuid::Uuid::new_v4();
    // yrs ClientID is 53-bit (Yjs-compatible) — mask the random u64 to fit so
    // it survives ClientID::new unchanged.
    let mut client_id =
      u64::from_le_bytes(raw.as_bytes()[0..8].try_into().unwrap()) & ((1u64 << 53) - 1);
    if client_id == 0 {
      client_id = 1;
    }
    self.meta_set("device_id", &device_id)?;
    self.meta_set("client_id", &client_id.to_string())?;
    Ok(Identity {
      device_id,
      client_id,
    })
  }

  /// Persist a document as its full snapshot (upsert).
  /// The ONLY place `doc_snapshot` is written.
  ///
  /// Four call sites used to carry the same UPSERT, and `content_text` has to
  /// move with `state` or search starts answering about a document that no
  /// longer exists in that shape. The cloud side keeps that invariant by
  /// co-writing the projection in the same statement at each of its write
  /// paths; here it is stronger and cheaper — there is one statement, so the
  /// two columns *cannot* drift, and a fifth write path would have to go out of
  /// its way to bypass this.
  ///
  /// **Deriving the text must never fail a save.** A document whose text cannot
  /// be derived stores NULL and is simply not searchable: the note is the
  /// user's, the index is ours, and losing an edit to protect a search feature
  /// is the wrong trade. That is also why the derive is contained rather than
  /// propagated.
  fn upsert_snapshot(
    &self,
    origin: &str,
    doc_id: &str,
    state: &[u8],
    crc: Option<i64>,
  ) -> Result<(), StoreError> {
    // ONE decode behind both projections — see [derive_projections].
    self.upsert_snapshot_with(
      origin,
      doc_id,
      state,
      crc,
      derive_projections(doc_id, state),
    )
  }

  /// [`Self::upsert_snapshot`] with the projections computed by the caller —
  /// for the compaction paths, which already hold the folded doc in memory
  /// and would otherwise pay a second full decode of the same state.
  fn upsert_snapshot_with(
    &self,
    origin: &str,
    doc_id: &str,
    state: &[u8],
    crc: Option<i64>,
    projections: (Option<String>, Option<String>),
  ) -> Result<(), StoreError> {
    let (content_text, link_targets) = projections;
    self.conn.execute(
      "INSERT INTO doc_snapshot(origin,doc_id,state,updated_at,state_crc,content_text,link_targets,projection_stale)
             VALUES(?1,?2,?3,?4,?5,?6,?7,0)
             ON CONFLICT(origin,doc_id) DO UPDATE SET
                 state=excluded.state, updated_at=excluded.updated_at,
                 state_crc=excluded.state_crc, content_text=excluded.content_text,
                 link_targets=excluded.link_targets, projection_stale=0",
      params![origin, doc_id, state, now_millis(), crc, content_text, link_targets],
    )?;
    Ok(())
  }

  /// Workspace search over local documents: title and body, one SQL statement.
  ///
  /// The shape the cloud arrived at the hard way — it used to decode every
  /// stored document per query, N full CRDT decodes for one search, and the fix
  /// was exactly this projection column. Doing the same here means a local
  /// workspace answers the same question the same way instead of growing a
  /// second, slower search.
  ///
  /// `LIKE` with the wildcards escaped: a query containing `%` must look for a
  /// percent sign, not match everything. CJK works because this is a substring
  /// match with no tokenizer — the same deliberate choice as the server.
  ///
  /// Rows whose `content_text` is NULL (written before the column existed, or
  /// undecodable) still match on their NAME. Not being indexed costs a page its
  /// body matches, never its existence.
  ///
  /// Scoped to ONE local workspace (`origin='local' AND workspace_id=?`): this
  /// store also holds the other local workspaces and the cloud mirrors
  /// (origin = server URL), and the cloud's search is per-workspace too. The
  /// same scope applies to [`Self::backlinks_local`] / [`Self::graph_local`].
  pub fn search_local(
    &self,
    workspace_id: &str,
    query: &str,
    limit: usize,
  ) -> Result<Vec<LocalSearchHit>, StoreError> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
      return Ok(Vec::new());
    }
    let escaped = trimmed
      .replace('\\', "\\\\")
      .replace('%', "\\%")
      .replace('_', "\\_");
    let pattern = format!("%{escaped}%");
    // An incremental edit leaves the index one fold behind (issue #104) — so
    // rebuild the workspace's stale rows BEFORE answering from them. One
    // decode per edited page per read, not per keystroke.
    self.refresh_workspace_projections("local", workspace_id)?;

    let mut stmt = self.conn.prepare(
      "SELECT v.id, v.object_id, v.name, v.parent_id, v.object_type,
                    COALESCE(s.content_text, '')
             FROM local_view v
             LEFT JOIN doc_snapshot s
                    ON s.doc_id = v.object_id AND s.origin = v.origin
             WHERE v.trashed = 0
               AND v.origin = 'local' AND v.workspace_id = ?3
               AND (v.name LIKE ?1 ESCAPE '\\' OR s.content_text LIKE ?1 ESCAPE '\\')
             ORDER BY (v.name LIKE ?1 ESCAPE '\\') DESC, v.name
             LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![pattern, limit as i64, workspace_id], |r| {
      Ok((
        r.get::<_, String>(0)?,
        r.get::<_, String>(1)?,
        r.get::<_, String>(2)?,
        r.get::<_, Option<String>>(3)?,
        r.get::<_, String>(4)?,
        r.get::<_, String>(5)?,
      ))
    })?;

    let needle = trimmed.to_lowercase();
    let mut hits = Vec::new();
    for row in rows {
      let (view_id, object_id, name, parent_id, object_type, body) = row?;
      let title_match = name.to_lowercase().contains(&needle);
      hits.push(LocalSearchHit {
        view_id,
        object_id,
        // The shared one-line window (`search_text`): the same text the
        // cloud shows for the same page. The copy that lived here sliced
        // the original by an offset taken from a lowercased copy — a
        // panic inside the store's mutex for `İ`/`ẞ` (review #3).
        snippet: crate::search_text::snippet(&body, trimmed).unwrap_or_default(),
        name,
        parent_id,
        is_folder: object_type == "folder",
        title_match,
      });
    }
    Ok(hits)
  }

  /// The pages that link TO `view_id` — the local backlinks panel.
  ///
  /// One statement over the maintained `link_targets`, the same way
  /// `search_local` reads `content_text`. The cloud learned this the expensive
  /// way: its panel used to decode every document per open (measured 690 ms on
  /// a real 798-document workspace) before migration 0019 gave it the same
  /// index. Doing it here from the start means the local world does not repeat
  /// that, and — more to the point — that both worlds answer the same question
  /// the same way instead of growing two different backlink features.
  ///
  /// `json_each` over the stored array, so a row that predates the column
  /// (NULL) simply contributes nothing rather than erroring. A page linking to
  /// itself is not a backlink. Trashed sources are excluded, matching search.
  pub fn backlinks_local(
    &self,
    workspace_id: &str,
    view_id: &str,
  ) -> Result<Vec<LocalBacklink>, StoreError> {
    // Same staleness contract as `search_local` (issue #104): a backlink the
    // log introduced must show up even if no search ran first — the panel is
    // opened on its own, and a reader must never depend on another reader
    // having refreshed the rows.
    self.refresh_workspace_projections("local", workspace_id)?;
    let mut stmt = self.conn.prepare(
      "SELECT v.id, v.object_id, v.name
             FROM local_view v
             JOIN doc_snapshot s ON s.doc_id = v.object_id AND s.origin = v.origin
             WHERE v.trashed = 0
               AND v.origin = 'local' AND v.workspace_id = ?2
               AND v.object_type = 'document'
               AND v.id <> ?1
               AND s.link_targets IS NOT NULL
               AND EXISTS (SELECT 1 FROM json_each(s.link_targets) WHERE value = ?1)
             ORDER BY v.name, v.id",
    )?;
    let rows = stmt.query_map(params![view_id, workspace_id], |r| {
      Ok(LocalBacklink {
        view_id: r.get(0)?,
        object_id: r.get(1)?,
        name: r.get(2)?,
      })
    })?;
    rows
      .collect::<Result<Vec<_>, _>>()
      .map_err(StoreError::from)
  }

  /// The page-link graph for the graph view — the local mirror of the cloud's
  /// `GET /workspaces/{id}/graph`, off the same maintained `link_targets`.
  ///
  /// Only pages that participate in a link become nodes; the rest are returned
  /// as `unlinked`. On a real library most pages link to nothing (a production
  /// snapshot: 798 documents, 136 with any link at all), and a canvas of
  /// hundreds of disconnected dots hides the structure the view exists to show.
  /// The count keeps that omission visible rather than silent.
  ///
  /// An edge survives only if BOTH ends are live documents here: a link to a
  /// trashed or foreign page is a dangling reference, and drawing it would
  /// invent a node that is not in the workspace.
  pub fn graph_local(&self, workspace_id: &str) -> Result<LocalGraph, StoreError> {
    // Same staleness contract as `search_local` (issue #104).
    self.refresh_workspace_projections("local", workspace_id)?;
    // Both ends scoped: the target join is by bare id, and ids are only
    // unique per origin — a cloud mirror page with the same id would
    // otherwise add a second copy of every edge into it.
    let mut stmt = self.conn.prepare(
      "SELECT src.id, tgt.id
             FROM local_view src
             JOIN doc_snapshot s ON s.doc_id = src.object_id AND s.origin = src.origin
             JOIN json_each(s.link_targets) j
             JOIN local_view tgt ON tgt.id = j.value AND tgt.trashed = 0
                 AND tgt.origin = 'local' AND tgt.workspace_id = ?1
             WHERE src.trashed = 0
               AND src.origin = 'local' AND src.workspace_id = ?1
               AND src.object_type = 'document'
               AND s.link_targets IS NOT NULL
               AND src.id <> j.value",
    )?;
    let edges: Vec<LocalGraphEdge> = stmt
      .query_map(params![workspace_id], |r| {
        Ok(LocalGraphEdge {
          source: r.get(0)?,
          target: r.get(1)?,
        })
      })?
      .collect::<Result<_, _>>()?;

    let mut degree: std::collections::HashMap<String, i64> = Default::default();
    for e in &edges {
      *degree.entry(e.source.clone()).or_default() += 1;
      *degree.entry(e.target.clone()).or_default() += 1;
    }

    let mut stmt = self.conn.prepare(
      "SELECT id, name FROM local_view
             WHERE trashed = 0 AND object_type = 'document'
               AND origin = 'local' AND workspace_id = ?1",
    )?;
    let named: Vec<(String, String)> = stmt
      .query_map(params![workspace_id], |r| Ok((r.get(0)?, r.get(1)?)))?
      .collect::<Result<_, _>>()?;

    let mut nodes = Vec::new();
    let mut unlinked = 0i64;
    for (view_id, name) in named {
      match degree.get(&view_id) {
        Some(&d) => nodes.push(LocalGraphNode {
          view_id,
          name,
          degree: d,
        }),
        None => unlinked += 1,
      }
    }
    nodes.sort_by(|a, b| b.degree.cmp(&a.degree).then(a.view_id.cmp(&b.view_id)));
    Ok(LocalGraph {
      nodes,
      edges,
      unlinked,
    })
  }

  /// Fill in `content_text`/`link_targets` for documents stored before those
  /// columns existed.
  ///
  /// Returns how many rows it filled. Bounded per call and driven by the
  /// caller: a store with thousands of documents must not turn the first launch
  /// after an update into a stall, and a search that is briefly incomplete is a
  /// far smaller problem than an app that appears hung.
  pub fn backfill_content_text(&self, limit: usize) -> Result<usize, StoreError> {
    let mut stmt = self.conn.prepare(
      "SELECT origin, doc_id FROM doc_snapshot
             WHERE content_text IS NULL OR link_targets IS NULL LIMIT ?1",
    )?;
    let pending: Vec<(String, String)> = stmt
      .query_map(params![limit as i64], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
      })?
      .collect::<Result<_, _>>()?;

    let mut filled = 0usize;
    for (origin, doc_id) in pending {
      self.reindex_doc(&origin, &doc_id)?;
      filled += 1;
    }
    Ok(filled)
  }

  /// Rebuild one document's search/link projections from its CURRENT state —
  /// the base snapshot plus both update logs — and store them.
  ///
  /// This is the read-side half of issue #104. `append_update` records an edit
  /// without touching the base (that is what makes typing cheap), so the
  /// projection columns describe the page as of the last fold. A reader that
  /// answers from those columns must therefore refresh what it is about to
  /// read. Refreshing here rather than on every append keeps the cost off the
  /// keystroke path: N edits between two searches cost ONE decode.
  ///
  /// Never fails a read for a bad row: an undecodable document is written as
  /// empty projections (`""`, `"[]"`), the same "not searchable, never wrong"
  /// contract [`Self::backfill_content_text`] documents — and the same reason,
  /// so a broken row is not re-decoded on every query.
  fn reindex_doc(&self, origin: &str, doc_id: &str) -> Result<(), StoreError> {
    let state: Option<Vec<u8>> = self
      .conn
      .query_row(
        "SELECT state FROM doc_snapshot WHERE origin=?1 AND doc_id=?2",
        params![origin, doc_id],
        |r| r.get(0),
      )
      .optional()?;
    let Some(state) = state else {
      return Ok(());
    };
    // Base + logs when they decode; otherwise fall back to the base alone — a
    // rotted log row must not cost the page the body it still has.
    let mut projections = derive_projections(doc_id, &state);
    let folded = contain_yrs_panic(doc_id, || {
      let mut doc = MicaDoc::from_update(&state)?;
      for (_rid, update) in self.doc_remote_updates(origin, doc_id)? {
        doc.apply_update(&update)?;
      }
      for (_clock, update) in self.doc_updates(origin, doc_id)? {
        doc.apply_update(&update)?;
      }
      Ok(derive_projections_from_doc(&doc))
    });
    if let Ok(folded) = folded {
      projections = folded;
    }
    let text = projections.0.unwrap_or_default();
    let links = projections.1.unwrap_or_else(|| "[]".to_string());
    self.conn.execute(
      "UPDATE doc_snapshot SET content_text=?3, link_targets=?4, projection_stale=0
             WHERE origin=?1 AND doc_id=?2",
      params![origin, doc_id, text, links],
    )?;
    Ok(())
  }

  /// Bring every STALE projection in one workspace up to date, before a reader
  /// queries it. Scoped to the workspace being read, so the cost is bounded by
  /// what that view can show rather than by the whole store — and rows marked
  /// NULL by an older build are deliberately NOT picked up here (that is the
  /// backfill's queue; a read must not turn a broken legacy row into a decode
  /// on every query).
  fn refresh_workspace_projections(
    &self,
    origin: &str,
    workspace_id: &str,
  ) -> Result<(), StoreError> {
    let mut stmt = self.conn.prepare(
      "SELECT DISTINCT s.origin, s.doc_id
             FROM local_view v
             JOIN doc_snapshot s ON s.doc_id = v.object_id AND s.origin = v.origin
             WHERE v.origin = ?1 AND v.workspace_id = ?2 AND s.projection_stale <> 0",
    )?;
    let stale: Vec<(String, String)> = stmt
      .query_map(params![origin, workspace_id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
      })?
      .collect::<Result<_, _>>()?;
    drop(stmt);
    for (row_origin, doc_id) in stale {
      self.reindex_doc(&row_origin, &doc_id)?;
    }
    Ok(())
  }

  pub fn save_doc(&self, origin: &str, doc_id: &str, doc: &MicaDoc) -> Result<(), StoreError> {
    let state = doc.encode_state();
    // Fresh checksum over the freshly-encoded bytes. `state_crc=excluded.state_crc`
    // in the SET clause is load-bearing: without it the FIRST save records a crc
    // and every LATER save (this is an UPSERT — every debounced edit) updates
    // `state` but leaves the crc stale → a false CorruptDoc on a healthy doc.
    let crc = blob_crc(&state);
    // Projections from the doc in hand, not a second decode of the state
    // just encoded from it — this runs on every debounced save.
    let projections = derive_projections_from_doc(doc);
    self.upsert_snapshot_with(origin, doc_id, state.as_ref(), Some(crc), projections)?;
    // Version history: archive this folded state as an AUTO snapshot, but only
    // if none was captured in the last VERSION_AUTO_INTERVAL — so a burst of
    // debounced saves converges to one version per window (AFFiNE min-interval).
    self.capture_auto_version(origin, doc_id, &state, crc)?;
    Ok(())
  }

  /// Capture `state` (with its already-computed checksum) as an auto version if
  /// the cadence allows, then prune auto versions past retention. Named versions
  /// (label set) are never touched.
  fn capture_auto_version(
    &self,
    origin: &str,
    doc_id: &str,
    state: &[u8],
    crc: i64,
  ) -> Result<(), StoreError> {
    let now = now_millis();
    let last: Option<i64> = self
      .conn
      .query_row(
        "SELECT created_at FROM doc_version
                 WHERE origin=?1 AND doc_id=?2 AND label IS NULL
                 ORDER BY created_at DESC LIMIT 1",
        params![origin, doc_id],
        |r| r.get(0),
      )
      .optional()?;
    if last.is_some_and(|t| now - t < VERSION_AUTO_INTERVAL_MS) {
      return Ok(());
    }
    self.conn.execute(
            "INSERT INTO doc_version(origin,id,doc_id,label,created_at,state,state_crc) VALUES(?1,?2,?3,NULL,?4,?5,?6)",
            params![origin, uuid::Uuid::new_v4().to_string(), doc_id, now, state, crc],
        )?;
    self.conn.execute(
      "DELETE FROM doc_version
             WHERE origin=?1 AND doc_id=?2 AND label IS NULL AND created_at < ?3",
      params![origin, doc_id, now - VERSION_AUTO_RETENTION_MS],
    )?;
    Ok(())
  }

  /// Pin the current saved state as a NAMED version (a manual checkpoint, never
  /// auto-pruned). Returns None if the document has no saved snapshot yet.
  pub fn create_local_version(
    &self,
    origin: &str,
    doc_id: &str,
    label: &str,
  ) -> Result<Option<LocalVersion>, StoreError> {
    // Carry the source row's checksum forward rather than recompute it, so the
    // crc keeps validating against the ORIGINAL author's bytes end-to-end. A
    // recompute-on-copy would launder in-place rot into a fresh "valid" crc.
    let row: Option<(Vec<u8>, Option<i64>)> = self
      .conn
      .query_row(
        "SELECT state, state_crc FROM doc_snapshot WHERE origin=?1 AND doc_id=?2",
        params![origin, doc_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
      )
      .optional()?;
    let Some((bytes, crc)) = row else {
      return Ok(None);
    };
    let now = now_millis();
    let id = uuid::Uuid::new_v4().to_string();
    self.conn.execute(
            "INSERT INTO doc_version(origin,id,doc_id,label,created_at,state,state_crc) VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![origin, id, doc_id, label, now, bytes, crc],
        )?;
    Ok(Some(LocalVersion {
      id,
      label: Some(label.to_string()),
      created_at: now,
    }))
  }

  /// The document's version timeline, newest first (metadata only, no blobs).
  pub fn list_local_versions(
    &self,
    origin: &str,
    doc_id: &str,
  ) -> Result<Vec<LocalVersion>, StoreError> {
    let mut stmt = self.conn.prepare(
      "SELECT id, label, created_at FROM doc_version
             WHERE origin=?1 AND doc_id=?2 ORDER BY created_at DESC",
    )?;
    let rows = stmt
      .query_map(params![origin, doc_id], |r| {
        Ok(LocalVersion {
          id: r.get(0)?,
          label: r.get(1)?,
          created_at: r.get(2)?,
        })
      })?
      .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
  }

  /// A version's stored yrs state blob (for a read-only preview — decode it
  /// into a throwaway MicaDoc, never the live one). None if not found.
  pub fn local_version_state(
    &self,
    origin: &str,
    doc_id: &str,
    version_id: &str,
  ) -> Result<Option<Vec<u8>>, StoreError> {
    let row: Option<(Vec<u8>, Option<i64>)> = self
      .conn
      .query_row(
        "SELECT state, state_crc FROM doc_version WHERE origin=?1 AND id=?2 AND doc_id=?3",
        params![origin, version_id, doc_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
      )
      .optional()?;
    let Some((bytes, crc)) = row else {
      return Ok(None);
    };
    // The caller decodes this into a throwaway MicaDoc OUTSIDE store.rs, so no
    // contain_yrs_panic covers it — verifying here is the only local defense.
    verify_blob_crc(doc_id, &bytes, crc)?;
    Ok(Some(bytes))
  }

  /// Restore a document to a stored version, making it the live base and
  /// dropping the incremental log (a local page is single-device, so this is a
  /// clean reset — the same shape as [`Self::rollback_doc`]). The pre-restore
  /// state is force-captured as an auto version first, so a restore is itself
  /// undoable. Returns the restored doc, or None if the version isn't found.
  pub fn restore_local_version(
    &self,
    origin: &str,
    doc_id: &str,
    version_id: &str,
    client_id: u64,
  ) -> Result<Option<MicaDoc>, StoreError> {
    let target: Option<(Vec<u8>, Option<i64>)> = self
      .conn
      .query_row(
        "SELECT state, state_crc FROM doc_version WHERE origin=?1 AND id=?2 AND doc_id=?3",
        params![origin, version_id, doc_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
      )
      .optional()?;
    let Some((bytes, crc)) = target else {
      return Ok(None);
    };
    // Verify the version blob BEFORE it becomes the live base or reaches yrs —
    // this was one of two recovery paths that previously decoded straight into
    // yrs with no guard at all (docs/reviews/code-review-2026-07-20.md P0-0).
    verify_blob_crc(doc_id, &bytes, crc)?;
    // Decode BEFORE anything is written: a crc only proves the bytes are the
    // ones stored, not that yrs can read them. Writing first made a bad
    // version the live base (with the log already dropped) and only then
    // failed — the page went from fine to unopenable. Unwind guard too, not
    // just the crc: a legacy version blob (crc NULL) still reaches yrs here.
    let doc = contain_yrs_panic(doc_id, || {
      Ok(MicaDoc::from_update_with_client_id(
        &bytes,
        Some(client_id),
      )?)
    })?;
    // Preserve the current state so the restore can be undone — the FULL
    // current state, base + logs. The base row alone misses every edit still
    // in the log, and those are exactly what the restore is about to drop.
    let archive: Option<(Vec<u8>, Option<i64>)> = match self.load_doc(origin, doc_id, client_id) {
      Ok(Some(cur)) => {
        let state = cur.encode_state();
        let crc = blob_crc(&state);
        Some((state, Some(crc)))
      }
      Ok(None) => None,
      // A current state that will not load is the usual reason to restore
      // at all; it must not block the way out. Keep its raw base (crc
      // carried forward, copy path) — the best record of it there is.
      Err(StoreError::CorruptDoc { .. } | StoreError::Doc(_)) => self
        .conn
        .query_row(
          "SELECT state, state_crc FROM doc_snapshot WHERE origin=?1 AND doc_id=?2",
          params![origin, doc_id],
          |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?,
      Err(e) => return Err(e),
    };
    // Archive + base swap + log drop commit together (10-19 P2-12). Split, a
    // crash between the swap and the drop left the old log to replay over
    // the restored base on the next open — the discarded edits came back.
    let tx = self.conn.unchecked_transaction()?;
    if let Some((cur, cur_crc)) = archive {
      tx.execute(
                "INSERT INTO doc_version(origin,id,doc_id,label,created_at,state,state_crc) VALUES(?1,?2,?3,NULL,?4,?5,?6)",
                params![origin, uuid::Uuid::new_v4().to_string(), doc_id, now_millis(), cur, cur_crc],
            )?;
    }
    self.upsert_snapshot(origin, doc_id, bytes.as_ref(), crc)?;
    tx.execute(
      "DELETE FROM doc_update WHERE origin=?1 AND doc_id=?2",
      params![origin, doc_id],
    )?;
    // The restore drops the LOCAL log but deliberately keeps the remote one
    // (the server already owns those updates), while the projections above came
    // from the version blob alone — so `load_doc`, which replays the remote
    // rows, would show more than the index. Flag it and let the next reader
    // rebuild (issue #104 follow-up). No remote rows → the flag stays 0.
    tx.execute(
      "UPDATE doc_snapshot SET projection_stale=1
             WHERE origin=?1 AND doc_id=?2
               AND EXISTS (SELECT 1 FROM doc_remote_update
                            WHERE origin=?1 AND doc_id=?2)",
      params![origin, doc_id],
    )?;
    tx.commit()?;
    Ok(Some(doc))
  }

  /// Load a document by id, decoding it with `client_id` (the device's actor)
  /// and replaying any incremental updates on top of the base snapshot (P2-M4).
  /// Returns `None` if there's no such document.
  pub fn load_doc(
    &self,
    origin: &str,
    doc_id: &str,
    client_id: u64,
  ) -> Result<Option<MicaDoc>, StoreError> {
    let row: Option<(Vec<u8>, Option<i64>)> = self
      .conn
      .query_row(
        "SELECT state, state_crc FROM doc_snapshot WHERE origin=?1 AND doc_id=?2",
        params![origin, doc_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
      )
      .optional()?;
    let (bytes, crc) = match row {
      Some(pair) => pair,
      None => return Ok(None),
    };
    // Verify storage integrity BEFORE decoding: even a valid update may
    // contain silently corrupted content that yrs cannot identify.
    verify_blob_crc(doc_id, &bytes, crc)?;
    // Read both logs BEFORE entering the panic guard: these are SQLite
    // errors, which are not corruption and must keep their own message. The
    // helpers verify each row's checksum, so a rotted log row surfaces as
    // CorruptDoc here (via `?`) and never reaches apply_update.
    let remote = self.doc_remote_updates(origin, doc_id)?;
    let local = self.doc_updates(origin, doc_id)?;

    // Everything that hands bytes to yrs lives inside the guard. Any of the
    // three can panic on a corrupt blob — the base snapshot or either log.
    let doc = contain_yrs_panic(doc_id, || {
      let mut doc = MicaDoc::from_update_with_client_id(&bytes, Some(client_id))?;
      // Replay both logs (P4-1): the remote stream tail first, then the
      // local outbox. yrs updates commute and are idempotent, so relative
      // order and overlap with the base are both safe.
      for (_rid, update) in remote {
        doc.apply_update(&update)?;
      }
      for (_clock, update) in local {
        doc.apply_update(&update)?;
      }
      Ok(doc)
    })?;
    Ok(Some(doc))
  }

  // ── incremental update log + sync cursor — P2-M4 ─────────────────────────

  /// Append an incremental update to a document's log, returning its `clock`
  /// (a per-doc monotonic local sequence). The base snapshot is left untouched;
  /// [`Self::squash`] folds the log back into the base later.
  pub fn append_update(
    &self,
    origin: &str,
    doc_id: &str,
    update: &[u8],
  ) -> Result<i64, StoreError> {
    let max_log: i64 = self.conn.query_row(
      "SELECT COALESCE(MAX(clock),0) FROM doc_update WHERE origin=?1 AND doc_id=?2",
      params![origin, doc_id],
      |r| r.get(0),
    )?;
    // The clock must be strictly monotonic for the doc's whole lifetime —
    // even after `trim_updates_through` deletes acked rows — because the cloud
    // outbox is `updates_after(pushed_clock)`. Deriving it from `MAX(clock)` of
    // the remaining rows alone would reset to 1 after a full trim, minting a
    // clock ≤ `pushed_clock` that would never be pushed (silent divergence).
    // So the high-water is `max(MAX(clock), pushed_clock)`: `pushed_clock`
    // covers every already-acked-and-trimmed clock (all ≤ it), the remaining
    // rows cover the un-pushed tail.
    let next = max_log.max(self.sync_cursor(origin, doc_id)?.pushed_clock) + 1;
    // The index now describes the pre-edit page: `content_text`/`link_targets`
    // are derived from the BASE, and this row holds only the log entry until
    // something folds it (issue #104). Flagging is one UPDATE on a cache row —
    // the alternative, deriving the projections here, would put a CRDT decode
    // on every keystroke. Readers rebuild what they are about to read; see
    // `refresh_workspace_projections`.
    //
    // Flag BEFORE the insert: a crash between the two then costs an unneeded
    // rebuild, while the other order would leave a log entry whose text the
    // index never learns about.
    self.conn.execute(
      "UPDATE doc_snapshot SET projection_stale=1 WHERE origin=?1 AND doc_id=?2",
      params![origin, doc_id],
    )?;
    self.conn.execute(
      "INSERT INTO doc_update(origin,doc_id,clock,payload,payload_crc) VALUES(?1,?2,?3,?4,?5)",
      params![origin, doc_id, next, update, blob_crc(update)],
    )?;
    Ok(next)
  }

  /// Durably append a REMOTE update (P4-1) and advance `last_synced_rid` in
  /// the same transaction — so the persisted cursor can never claim a stream
  /// position whose update isn't on disk. Idempotent per `(doc_id, rid)`
  /// (re-pulls replay the same rids). Remote updates get their own log —
  /// never `doc_update`, whose rows are the PURE-LOCAL outbox (P2c invariant:
  /// a remote row there would corrupt `pushed_clock` semantics).
  pub fn append_remote_update(
    &self,
    origin: &str,
    doc_id: &str,
    rid: i64,
    update: &[u8],
  ) -> Result<(), StoreError> {
    self.append_remote_updates(origin, doc_id, &[(rid, update.to_vec())])
  }

  /// Batch variant of [`Self::append_remote_update`]: all rows + the cursor
  /// advance in ONE transaction (one commit), so a 1000-row catch-up pull
  /// costs one journal sync instead of a thousand (the reconnect storm the
  /// P4-1 review flagged).
  pub fn append_remote_updates(
    &self,
    origin: &str,
    doc_id: &str,
    items: &[(i64, Vec<u8>)],
  ) -> Result<(), StoreError> {
    if items.is_empty() {
      return Ok(());
    }
    let tx = self.conn.unchecked_transaction()?;
    // A remote update is content too: the projections describe the base, and
    // this row only becomes visible when something folds or re-derives it. The
    // same flag `append_update` sets, in the same transaction as the rows, so a
    // reader can never see the log without seeing the staleness (issue #104).
    tx.execute(
      "UPDATE doc_snapshot SET projection_stale=1 WHERE origin=?1 AND doc_id=?2",
      params![origin, doc_id],
    )?;
    let mut max_rid = i64::MIN;
    for (rid, update) in items {
      tx.execute(
                "INSERT OR IGNORE INTO doc_remote_update(origin,doc_id,rid,payload,payload_crc) VALUES(?1,?2,?3,?4,?5)",
                params![origin, doc_id, rid, update, blob_crc(update)],
            )?;
      max_rid = max_rid.max(*rid);
    }
    tx.execute(
      "INSERT INTO sync_cursor(origin,doc_id,last_synced_rid,pushed_clock) VALUES(?1,?2,?3,0)
             ON CONFLICT(origin,doc_id) DO UPDATE SET
                 last_synced_rid=MAX(last_synced_rid, excluded.last_synced_rid)",
      params![origin, doc_id, max_rid],
    )?;
    tx.commit()?;
    Ok(())
  }

  /// A document's persisted remote-stream tail, ordered by `rid`. Each row's
  /// checksum is verified before it is returned, so a rotted remote update
  /// surfaces as `CorruptDoc` instead of being replayed into yrs.
  pub fn doc_remote_updates(
    &self,
    origin: &str,
    doc_id: &str,
  ) -> Result<Vec<(i64, Vec<u8>)>, StoreError> {
    let mut stmt = self.conn.prepare(
      "SELECT rid,payload,payload_crc FROM doc_remote_update
             WHERE origin=?1 AND doc_id=?2 ORDER BY rid",
    )?;
    let rows = stmt
      .query_map(params![origin, doc_id], |r| {
        Ok((
          r.get::<_, i64>(0)?,
          r.get::<_, Vec<u8>>(1)?,
          r.get::<_, Option<i64>>(2)?,
        ))
      })?
      .collect::<Result<Vec<_>, _>>()?;
    let mut out = Vec::with_capacity(rows.len());
    for (rid, payload, crc) in rows {
      verify_blob_crc(doc_id, &payload, crc)?;
      out.push((rid, payload));
    }
    Ok(out)
  }

  /// (local outbox rows, remote log rows) — the compaction trigger reads this
  /// to keep both logs bounded without timers.
  pub fn log_sizes(&self, origin: &str, doc_id: &str) -> Result<(i64, i64), StoreError> {
    let local: i64 = self.conn.query_row(
      "SELECT COUNT(*) FROM doc_update WHERE origin=?1 AND doc_id=?2",
      params![origin, doc_id],
      |r| r.get(0),
    )?;
    let remote: i64 = self.conn.query_row(
      "SELECT COUNT(*) FROM doc_remote_update WHERE origin=?1 AND doc_id=?2",
      params![origin, doc_id],
      |r| r.get(0),
    )?;
    Ok((local, remote))
  }

  /// All of a document's incremental updates, ordered by `clock`. Each row's
  /// checksum is verified before it is returned (same rationale as
  /// [`Self::doc_remote_updates`]).
  pub fn doc_updates(&self, origin: &str, doc_id: &str) -> Result<Vec<(i64, Vec<u8>)>, StoreError> {
    self.updates_after(origin, doc_id, i64::MIN)
  }

  /// Updates with `clock > after` — the queue still to push to the cloud.
  ///
  /// Checksums verified here, for [`Self::doc_updates`] too (one reader, so
  /// the two cannot disagree again — this one used to skip the check). It is
  /// the outbox: a rotted row pushed upstream would be integrated by the
  /// server and every peer, the one place a local bit-flip becomes
  /// everyone's problem.
  pub fn updates_after(
    &self,
    origin: &str,
    doc_id: &str,
    after: i64,
  ) -> Result<Vec<(i64, Vec<u8>)>, StoreError> {
    let mut stmt = self.conn.prepare(
      "SELECT clock,payload,payload_crc FROM doc_update
             WHERE origin=?1 AND doc_id=?2 AND clock>?3 ORDER BY clock",
    )?;
    let rows = stmt
      .query_map(params![origin, doc_id, after], |r| {
        Ok((
          r.get::<_, i64>(0)?,
          r.get::<_, Vec<u8>>(1)?,
          r.get::<_, Option<i64>>(2)?,
        ))
      })?
      .collect::<Result<Vec<_>, _>>()?;
    let mut out = Vec::with_capacity(rows.len());
    for (clock, payload, crc) in rows {
      verify_blob_crc(doc_id, &payload, crc)?;
      out.push((clock, payload));
    }
    Ok(out)
  }

  /// Drop acked log entries (`clock ≤ up_to_clock`) to bound the append-log,
  /// leaving the un-pushed outbox (`clock > pushed_clock`) intact — callers pass
  /// `up_to_clock = pushed_clock`. Safe: the base snapshot already folds these
  /// updates, so `load_doc` is unchanged; and `append_update` stays monotonic
  /// across the trim (its high-water includes `pushed_clock`), so no future
  /// clock is ever reused below the trimmed range.
  pub fn trim_updates_through(
    &self,
    origin: &str,
    doc_id: &str,
    up_to_clock: i64,
  ) -> Result<(), StoreError> {
    // Clamp to pushed_clock so an over-eager caller can never delete un-pushed
    // outbox entries (safe-by-construction, not by caller discipline).
    let up_to = up_to_clock.min(self.sync_cursor(origin, doc_id)?.pushed_clock);
    self.conn.execute(
      "DELETE FROM doc_update WHERE origin=?1 AND doc_id=?2 AND clock<=?3",
      params![origin, doc_id, up_to],
    )?;
    Ok(())
  }

  /// Fold the base snapshot + all logged updates into a single new base, then
  /// drop the **acked** log entries (`clock ≤ pushed_clock`). Coalesces history
  /// growth (§4). Caller passes the device `client_id` so the squashed doc keeps
  /// a consistent actor.
  ///
  /// The un-pushed outbox tail (`clock > pushed_clock`) is deliberately kept:
  /// deleting it would drop edits the cloud never received (they'd live only in
  /// the local base, never syncing out) and reset the clock below `pushed_clock`
  /// — the same silent-divergence hazard [`Self::append_update`] guards against.
  /// The new base already folds the kept tail, so replaying it on load is
  /// idempotent. For a pure-local doc (`pushed_clock = 0`, and the log is
  /// unused — local edits write full snapshots via [`Self::save_doc`]) this
  /// deletes nothing and is a harmless re-baseline.
  pub fn squash(&self, origin: &str, doc_id: &str, client_id: u64) -> Result<(), StoreError> {
    let doc = match self.load_doc(origin, doc_id, client_id)? {
      Some(d) => d,
      None => return Ok(()),
    };
    // Projections come from this in-memory doc — decoding the fresh state
    // again in `upsert_snapshot` would be a second decode of the same content.
    let projections = derive_projections_from_doc(&doc);
    let state = doc.encode_state();
    let crc = blob_crc(&state);
    let pushed = self.sync_cursor(origin, doc_id)?.pushed_clock;
    // One transaction: fold + trim + clear commit together (crash mid-way
    // was already safe via replay idempotency, but structural atomicity is
    // cheaper to reason about than a CRDT argument).
    let tx = self.conn.unchecked_transaction()?;
    self.upsert_snapshot_with(origin, doc_id, state.as_ref(), Some(crc), projections)?;
    tx.execute(
      "DELETE FROM doc_update WHERE origin=?1 AND doc_id=?2 AND clock<=?3",
      params![origin, doc_id, pushed],
    )?;
    // The remote log is fully folded into the new base (it has no
    // un-pushed semantics — the server already owns these updates).
    tx.execute(
      "DELETE FROM doc_remote_update WHERE origin=?1 AND doc_id=?2",
      params![origin, doc_id],
    )?;
    tx.commit()?;
    Ok(())
  }

  /// Fold every logged update into the base and clear the log — the
  /// local-only counterpart of [`Self::squash`].
  ///
  /// [`Self::squash`] deliberately KEEPS the un-pushed outbox tail
  /// (`clock > pushed_clock`), because those updates still owe the server, and
  /// [`Self::trim_updates_through`] clamps to the same mark. A document with no
  /// cloud counterpart owes nothing and its `pushed_clock` stays 0 forever — so
  /// neither of them can ever bound its log. Without this, moving the local
  /// backend from "re-snapshot on a debounce" to "append" would trade a bounded
  /// full write for an UNBOUNDED log that [`Self::load_doc`] replays on every
  /// open: strictly worse, and invisible until someone's file got slow.
  ///
  /// Returns whether it compacted. It REFUSES on any document showing evidence
  /// of cloud sync — a non-zero sync cursor, or any remote-log row. Safe by
  /// construction rather than by caller discipline: dropping an un-pushed
  /// outbox entry is silent server-side data loss (red line #1), and this is
  /// the one operation in the store that deletes updates the base might not
  /// have folded.
  ///
  /// Captures an auto version like [`Self::save_doc`] does. Local version
  /// history rode on the debounced full save; without this line it would stop
  /// the moment the caller stopped calling `save_doc`, and nothing would say so.
  pub fn compact_local(
    &self,
    origin: &str,
    doc_id: &str,
    client_id: u64,
  ) -> Result<bool, StoreError> {
    let cursor = self.sync_cursor(origin, doc_id)?;
    if cursor.pushed_clock != 0 || cursor.last_synced_rid != 0 {
      return Ok(false);
    }
    let remote_rows: i64 = self.conn.query_row(
      "SELECT COUNT(*) FROM doc_remote_update WHERE origin=?1 AND doc_id=?2",
      params![origin, doc_id],
      |r| r.get(0),
    )?;
    if remote_rows != 0 {
      return Ok(false);
    }
    let doc = match self.load_doc(origin, doc_id, client_id)? {
      Some(d) => d,
      None => return Ok(false),
    };
    // Same as `squash`: projections from the doc already in memory, not a
    // second decode of the fresh state.
    let projections = derive_projections_from_doc(&doc);
    let state = doc.encode_state();
    let crc = blob_crc(&state);
    // One transaction: the fold and the clear commit together. Replay is
    // idempotent so a crash between them would be survivable anyway, but a
    // reader shouldn't have to reconstruct that argument to trust the file.
    let tx = self.conn.unchecked_transaction()?;
    self.upsert_snapshot_with(origin, doc_id, state.as_ref(), Some(crc), projections)?;
    tx.execute(
      "DELETE FROM doc_update WHERE origin=?1 AND doc_id=?2",
      params![origin, doc_id],
    )?;
    tx.commit()?;
    self.capture_auto_version(origin, doc_id, &state, crc)?;
    Ok(true)
  }

  /// A document's cloud sync high-water marks (zeroed if never synced).
  pub fn sync_cursor(&self, origin: &str, doc_id: &str) -> Result<SyncCursor, StoreError> {
    Ok(
      self
        .conn
        .query_row(
          "SELECT last_synced_rid,pushed_clock FROM sync_cursor
             WHERE origin=?1 AND doc_id=?2",
          params![origin, doc_id],
          |r| {
            Ok(SyncCursor {
              last_synced_rid: r.get(0)?,
              pushed_clock: r.get(1)?,
            })
          },
        )
        .optional()?
        .unwrap_or_default(),
    )
  }

  /// Persist a document's sync high-water marks.
  pub fn set_sync_cursor(
    &self,
    origin: &str,
    doc_id: &str,
    cursor: SyncCursor,
  ) -> Result<(), StoreError> {
    self.conn.execute(
      "INSERT INTO sync_cursor(origin,doc_id,last_synced_rid,pushed_clock) VALUES(?1,?2,?3,?4)
             ON CONFLICT(origin,doc_id) DO UPDATE SET
                 last_synced_rid=excluded.last_synced_rid, pushed_clock=excluded.pushed_clock",
      params![origin, doc_id, cursor.last_synced_rid, cursor.pushed_clock],
    )?;
    Ok(())
  }

  /// All stored document ids of one `origin` (sorted).
  pub fn list_docs(&self, origin: &str) -> Result<Vec<String>, StoreError> {
    let mut stmt = self
      .conn
      .prepare("SELECT doc_id FROM doc_snapshot WHERE origin=?1 ORDER BY doc_id")?;
    let ids = stmt
      .query_map(params![origin], |r| r.get::<_, String>(0))?
      .collect::<Result<Vec<_>, _>>()?;
    Ok(ids)
  }

  /// Delete a document of ONE origin — every row keyed by `(origin, doc_id)`,
  /// in one transaction.
  ///
  /// This is what 「永久删除」 and removing a server's mirror promise, so the
  /// list must cover every table that holds the document's CONTENT — the
  /// version history (`doc_version`, full states) was missing, leaving the
  /// whole text on disk after the page was gone. A test enumerates every
  /// table with a `doc_id` column, so a new one cannot be forgotten silently.
  /// Origin-scoped (issue #102): clearing one server's mirror must never
  /// delete the SAME document id held by another origin.
  pub fn delete_doc(&self, origin: &str, doc_id: &str) -> Result<(), StoreError> {
    let tx = self.conn.unchecked_transaction()?;
    self.delete_doc_rows(origin, doc_id)?;
    tx.commit()?;
    Ok(())
  }

  /// The per-table deletes of [`Self::delete_doc`] with NO transaction of
  /// their own, for callers that already hold one — [`crate::view_ops`]
  /// folds every document of a purged subtree into the same single commit as
  /// the view rows (statements on this connection join the open
  /// `unchecked_transaction`). Same table list as `delete_doc`; the drift
  /// test below enumerates every table with a `doc_id` column.
  pub(crate) fn delete_doc_rows(&self, origin: &str, doc_id: &str) -> Result<(), StoreError> {
    for table in [
      "doc_snapshot",
      "doc_snapshot_backup",
      "doc_update",
      "doc_remote_update",
      "sync_cursor",
      "doc_version",
    ] {
      self.conn.execute(
        &format!("DELETE FROM {table} WHERE origin=?1 AND doc_id=?2"),
        params![origin, doc_id],
      )?;
    }
    Ok(())
  }

  // ── snapshot checkpoint / rollback — §10 recovery safety net ─────────────

  /// Save the document's current base snapshot as a recovery checkpoint. Call
  /// at safe points (doc close, app pause) so a later corruption can be rolled
  /// back. No-op if the document has no base yet.
  pub fn checkpoint_doc(&self, origin: &str, doc_id: &str) -> Result<(), StoreError> {
    // SQL-only copy — the checksum rides along in the same statement, so the
    // backup keeps validating against the base's original bytes.
    self.conn.execute(
            "INSERT INTO doc_snapshot_backup(origin,doc_id,state,updated_at,state_crc)
             SELECT origin, doc_id, state, ?3, state_crc FROM doc_snapshot
             WHERE origin=?1 AND doc_id=?2
             ON CONFLICT(origin,doc_id) DO UPDATE SET
                 state=excluded.state, updated_at=excluded.updated_at, state_crc=excluded.state_crc",
            params![origin, doc_id, now_millis()],
        )?;
    Ok(())
  }

  /// Restore a document from its last [`Self::checkpoint_doc`], making it the
  /// live base and dropping the incremental log. Returns the restored doc, or
  /// `None` if there's no checkpoint.
  pub fn rollback_doc(
    &self,
    origin: &str,
    doc_id: &str,
    client_id: u64,
  ) -> Result<Option<MicaDoc>, StoreError> {
    let row: Option<(Vec<u8>, Option<i64>)> = self
      .conn
      .query_row(
        "SELECT state, state_crc FROM doc_snapshot_backup WHERE origin=?1 AND doc_id=?2",
        params![origin, doc_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
      )
      .optional()?;
    let (bytes, crc) = match row {
      Some(pair) => pair,
      None => return Ok(None),
    };
    // The other previously-unguarded decode path (P0-0): verify the backup
    // before it becomes the live base or reaches yrs — and decode it before
    // writing, then swap + drop in one transaction (same reasons as
    // `restore_local_version`).
    verify_blob_crc(doc_id, &bytes, crc)?;
    let doc = contain_yrs_panic(doc_id, || {
      Ok(MicaDoc::from_update_with_client_id(
        &bytes,
        Some(client_id),
      )?)
    })?;
    let tx = self.conn.unchecked_transaction()?;
    self.upsert_snapshot(origin, doc_id, bytes.as_ref(), crc)?;
    tx.execute(
      "DELETE FROM doc_update WHERE origin=?1 AND doc_id=?2",
      params![origin, doc_id],
    )?;
    // Same divergence as `restore_local_version`: the checkpoint drops the
    // local log, the remote log stays, and the projections came from the
    // checkpoint blob alone. Flag only when remote rows actually remain.
    tx.execute(
      "UPDATE doc_snapshot SET projection_stale=1
             WHERE origin=?1 AND doc_id=?2
               AND EXISTS (SELECT 1 FROM doc_remote_update
                            WHERE origin=?1 AND doc_id=?2)",
      params![origin, doc_id],
    )?;
    tx.commit()?;
    Ok(Some(doc))
  }

  // ── page tree (views) — P2-M3 ────────────────────────────────────────────

  /// All views across all workspaces (including trashed), ordered by
  /// `position`. The client filters by workspace + trash and builds the tree
  /// from `parent_id`.
  pub fn list_views(&self, origin: &str) -> Result<Vec<LocalView>, StoreError> {
    let mut stmt = self.conn.prepare(
      "SELECT id,workspace_id,parent_id,object_id,name,position,trashed,origin,object_type \
             FROM local_view WHERE origin=?1 ORDER BY position",
    )?;
    let rows = stmt
      .query_map(params![origin], |r| {
        Ok(LocalView {
          id: r.get(0)?,
          workspace_id: r.get(1)?,
          parent_id: r.get(2)?,
          object_id: r.get(3)?,
          name: r.get(4)?,
          position: r.get(5)?,
          trashed: r.get::<_, i64>(6)? != 0,
          origin: r.get(7)?,
          object_type: r.get(8)?,
        })
      })?
      .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
  }

  /// Upsert a view — covers create, rename, move (parent/position), and trash
  /// toggling, all by writing the desired row. Refuses a row the page tree
  /// cannot hold ([`Self::ensure_valid_parent`]).
  pub fn save_view(&self, v: &LocalView) -> Result<(), StoreError> {
    self.ensure_valid_parent(&v.origin, &v.id, v.parent_id.as_deref())?;
    self.conn.execute(
            "INSERT INTO local_view(id,workspace_id,parent_id,object_id,name,position,trashed,origin,object_type)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)
             ON CONFLICT(origin,id) DO UPDATE SET
                 workspace_id=excluded.workspace_id, parent_id=excluded.parent_id,
                 object_id=excluded.object_id, name=excluded.name,
                 position=excluded.position,
                 -- `trashed` is a trash BATCH (view_ops::set_views_trashed):
                 -- an upsert of a row that stays trashed keeps its batch.
                 trashed=CASE WHEN excluded.trashed<>0 AND local_view.trashed<>0
                              THEN local_view.trashed ELSE excluded.trashed END,
                 object_type=excluded.object_type",
            params![
                v.id,
                v.workspace_id,
                v.parent_id,
                v.object_id,
                v.name,
                v.position,
                v.trashed as i64,
                v.origin,
                v.object_type
            ],
        )?;
    Ok(())
  }

  /// Permanently remove a view row of one `origin` (the document is deleted
  /// separately via [`Self::delete_doc`]). Origin-scoped so a purge can never
  /// reach across the local/cloud namespaces (v4 composite-PK semantics).
  pub fn purge_view(&self, origin: &str, id: &str) -> Result<(), StoreError> {
    self.conn.execute(
      "DELETE FROM local_view WHERE origin=?1 AND id=?2",
      params![origin, id],
    )?;
    Ok(())
  }

  // ── workspaces — P2-M3 ───────────────────────────────────────────────────

  /// All local workspaces, ordered by `position`.
  pub fn list_workspaces(&self, origin: &str) -> Result<Vec<LocalWorkspace>, StoreError> {
    let mut stmt = self.conn.prepare(
      "SELECT id,name,position,origin,role FROM local_workspace WHERE origin=?1 ORDER BY position",
    )?;
    let rows = stmt
      .query_map(params![origin], |r| {
        Ok(LocalWorkspace {
          id: r.get(0)?,
          name: r.get(1)?,
          position: r.get(2)?,
          origin: r.get(3)?,
          role: r.get(4)?,
        })
      })?
      .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
  }

  /// Upsert a workspace (create / rename / reorder).
  pub fn save_workspace(&self, w: &LocalWorkspace) -> Result<(), StoreError> {
    self.conn.execute(
            "INSERT INTO local_workspace(id,name,position,origin,role) VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(origin,id) DO UPDATE SET name=excluded.name, position=excluded.position, role=excluded.role",
            params![w.id, w.name, w.position, w.origin, w.role],
        )?;
    Ok(())
  }

  /// Delete one `origin`'s workspace and all its views' rows. Documents are
  /// deleted separately by the caller (it knows the object ids). Origin-scoped
  /// so removing a cloud mirror can never touch a same-id local workspace
  /// (v4 composite-PK semantics).
  pub fn delete_workspace(&self, origin: &str, id: &str) -> Result<(), StoreError> {
    self.conn.execute(
      "DELETE FROM local_view WHERE origin=?1 AND workspace_id=?2",
      params![origin, id],
    )?;
    self.conn.execute(
      "DELETE FROM local_workspace WHERE origin=?1 AND id=?2",
      params![origin, id],
    )?;
    Ok(())
  }
}

fn now_millis() -> i64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|d| d.as_millis() as i64)
    .unwrap_or(0)
}

#[cfg(test)]
mod tests;
