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
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
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
/// This is the one thing `contain_yrs_panic` cannot do. yrs 0.27.x can reach
/// `hint::unreachable_unchecked` on malformed bytes — a NON-unwinding abort that
/// `catch_unwind` cannot contain, and plain UB in a release build (see the
/// `yrs_corrupt_input_is_unsound` test). Rejecting a bad blob here stops those
/// bytes before `from_update_with_client_id` / `apply_update` ever see them.
///
/// `stored_crc == None` ⇒ a legacy row written before checksums existed (or by
/// an older app): nothing to check, so PASS and let the bytes fall through to
/// the existing `contain_yrs_panic` guard — i.e. exactly today's behavior. Such
/// rows keep today's exposure to the upstream UB until they are next rewritten;
/// this feature protects data authored from here on, it does not retroactively
/// bless the existing corpus.
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
const SCHEMA_VERSION: i64 = 6;

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
    conn: Connection,
}

impl LocalStore {
    /// Open (creating if needed) the store at `path` and ensure the schema.
    pub fn open(path: &str) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             CREATE TABLE IF NOT EXISTS doc_snapshot(
                 doc_id     TEXT PRIMARY KEY,
                 state      BLOB NOT NULL,
                 updated_at INTEGER NOT NULL,
                 state_crc  INTEGER
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
                 doc_id      TEXT NOT NULL,
                 clock       INTEGER NOT NULL,
                 payload     BLOB NOT NULL,
                 payload_crc INTEGER,
                 PRIMARY KEY(doc_id, clock)
             );
             CREATE TABLE IF NOT EXISTS doc_remote_update(
                 doc_id      TEXT NOT NULL,
                 rid         INTEGER NOT NULL,
                 payload     BLOB NOT NULL,
                 payload_crc INTEGER,
                 PRIMARY KEY(doc_id, rid)
             );
             CREATE TABLE IF NOT EXISTS sync_cursor(
                 doc_id          TEXT PRIMARY KEY,
                 last_synced_rid INTEGER NOT NULL DEFAULT 0,
                 pushed_clock    INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE IF NOT EXISTS doc_snapshot_backup(
                 doc_id     TEXT PRIMARY KEY,
                 state      BLOB NOT NULL,
                 updated_at INTEGER NOT NULL,
                 state_crc  INTEGER
             );
             -- Local page version history (docs/version-history-plan.md §6). One
             -- row = one full yrs state. `label` NULL = an auto snapshot (captured
             -- on a cadence by save_doc, pruned after retention); set = a named
             -- checkpoint, kept forever. Purely additive → no SCHEMA_VERSION bump
             -- (an older app just ignores this table).
             CREATE TABLE IF NOT EXISTS doc_version(
                 id         TEXT PRIMARY KEY,
                 doc_id     TEXT NOT NULL,
                 label      TEXT,
                 created_at INTEGER NOT NULL,
                 state      BLOB NOT NULL,
                 state_crc  INTEGER
             );
             CREATE INDEX IF NOT EXISTS doc_version_doc_created_idx
                 ON doc_version(doc_id, created_at DESC);",
        )?;
        // Schema version gate (§10) — checked BEFORE any migration below, so a
        // store written by a newer app is refused while still untouched. The
        // migrations are probe-driven (column/PK shape, not version), and the v4
        // rebuild is destructive (DROP + recreate from a hard-coded column list):
        // run against a future schema it would silently strip columns the newer
        // app added, and only THEN would a trailing gate reject — too late. The
        // stamp still happens at the end, after every migration succeeded.
        // (Everything above this point is CREATE IF NOT EXISTS / PRAGMA — no-ops
        // on an existing store.)
        let stored: i64 = conn
            .query_row(
                "SELECT value FROM local_meta WHERE key='schema_version'",
                [],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        if stored > SCHEMA_VERSION {
            return Err(StoreError::SchemaTooNew {
                found: stored,
                supported: SCHEMA_VERSION,
            });
        }

        // Migrate pre-multi-workspace stores: add the workspace_id column and a
        // default workspace that existing views attach to.
        let has_ws_col: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('local_view') WHERE name='workspace_id'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map(|c| c > 0)
            .unwrap_or(false);
        if !has_ws_col {
            conn.execute(
                "ALTER TABLE local_view ADD COLUMN workspace_id TEXT NOT NULL DEFAULT 'local'",
                [],
            )?;
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
            let has_origin: bool = conn
                .query_row(
                    &format!(
                        "SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name='origin'"
                    ),
                    [],
                    |r| r.get::<_, i64>(0),
                )
                .map(|c| c > 0)
                .unwrap_or(false);
            if !has_origin {
                conn.execute(
                    &format!(
                        "ALTER TABLE {table} ADD COLUMN origin TEXT NOT NULL DEFAULT 'local'"
                    ),
                    [],
                )?;
            }
        }

        // v3 (P2d): persist each mirrored cloud workspace's `role` so an offline
        // (re)start knows whether this user may edit it — otherwise the offline
        // nav forces read-only. Local workspaces are the user's own (the UI treats
        // them as owner); the default only affects rows until they're re-mirrored.
        let has_role: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('local_workspace') WHERE name='role'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map(|c| c > 0)
            .unwrap_or(false);
        if !has_role {
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
        let has_object_type: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('local_view') WHERE name='object_type'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map(|c| c > 0)
            .unwrap_or(false);
        if !has_object_type {
            conn.execute(
                "ALTER TABLE local_view ADD COLUMN object_type TEXT NOT NULL DEFAULT 'document'",
                [],
            )?;
        }

        // Per-blob checksum columns (docs/code-review-2026-07-20.md P0-0). ADDITIVE
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
            let has: bool = conn
                .query_row(
                    &format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name='{col}'"),
                    [],
                    |r| r.get::<_, i64>(0),
                )
                .map(|c| c > 0)
                .unwrap_or(false);
            if !has {
                conn.execute(&format!("ALTER TABLE {table} ADD COLUMN {col} INTEGER"), [])?;
            }
        }

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
        Ok(self
            .conn
            .query_row(
                "SELECT value FROM local_meta WHERE key=?1",
                params![key],
                |r| r.get(0),
            )
            .optional()?)
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
        if let (Some(device_id), Some(cid)) =
            (self.meta_get("device_id")?, self.meta_get("client_id")?)
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
    pub fn save_doc(&self, doc_id: &str, doc: &MicaDoc) -> Result<(), StoreError> {
        let state = doc.encode_state();
        // Fresh checksum over the freshly-encoded bytes. `state_crc=excluded.state_crc`
        // in the SET clause is load-bearing: without it the FIRST save records a crc
        // and every LATER save (this is an UPSERT — every debounced edit) updates
        // `state` but leaves the crc stale → a false CorruptDoc on a healthy doc.
        let crc = blob_crc(&state);
        self.conn.execute(
            "INSERT INTO doc_snapshot(doc_id,state,updated_at,state_crc) VALUES(?1,?2,?3,?4)
             ON CONFLICT(doc_id) DO UPDATE SET
                 state=excluded.state, updated_at=excluded.updated_at, state_crc=excluded.state_crc",
            params![doc_id, state, now_millis(), crc],
        )?;
        // Version history: archive this folded state as an AUTO snapshot, but only
        // if none was captured in the last VERSION_AUTO_INTERVAL — so a burst of
        // debounced saves converges to one version per window (AFFiNE min-interval).
        self.capture_auto_version(doc_id, &state, crc)?;
        Ok(())
    }

    /// Capture `state` (with its already-computed checksum) as an auto version if
    /// the cadence allows, then prune auto versions past retention. Named versions
    /// (label set) are never touched.
    fn capture_auto_version(&self, doc_id: &str, state: &[u8], crc: i64) -> Result<(), StoreError> {
        let now = now_millis();
        let last: Option<i64> = self
            .conn
            .query_row(
                "SELECT created_at FROM doc_version
                 WHERE doc_id=?1 AND label IS NULL ORDER BY created_at DESC LIMIT 1",
                params![doc_id],
                |r| r.get(0),
            )
            .optional()?;
        if last.is_some_and(|t| now - t < VERSION_AUTO_INTERVAL_MS) {
            return Ok(());
        }
        self.conn.execute(
            "INSERT INTO doc_version(id,doc_id,label,created_at,state,state_crc) VALUES(?1,?2,NULL,?3,?4,?5)",
            params![uuid::Uuid::new_v4().to_string(), doc_id, now, state, crc],
        )?;
        self.conn.execute(
            "DELETE FROM doc_version
             WHERE doc_id=?1 AND label IS NULL AND created_at < ?2",
            params![doc_id, now - VERSION_AUTO_RETENTION_MS],
        )?;
        Ok(())
    }

    /// Pin the current saved state as a NAMED version (a manual checkpoint, never
    /// auto-pruned). Returns None if the document has no saved snapshot yet.
    pub fn create_local_version(
        &self,
        doc_id: &str,
        label: &str,
    ) -> Result<Option<LocalVersion>, StoreError> {
        // Carry the source row's checksum forward rather than recompute it, so the
        // crc keeps validating against the ORIGINAL author's bytes end-to-end. A
        // recompute-on-copy would launder in-place rot into a fresh "valid" crc.
        let row: Option<(Vec<u8>, Option<i64>)> = self
            .conn
            .query_row(
                "SELECT state, state_crc FROM doc_snapshot WHERE doc_id=?1",
                params![doc_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((bytes, crc)) = row else {
            return Ok(None);
        };
        let now = now_millis();
        let id = uuid::Uuid::new_v4().to_string();
        self.conn.execute(
            "INSERT INTO doc_version(id,doc_id,label,created_at,state,state_crc) VALUES(?1,?2,?3,?4,?5,?6)",
            params![id, doc_id, label, now, bytes, crc],
        )?;
        Ok(Some(LocalVersion {
            id,
            label: Some(label.to_string()),
            created_at: now,
        }))
    }

    /// The document's version timeline, newest first (metadata only, no blobs).
    pub fn list_local_versions(&self, doc_id: &str) -> Result<Vec<LocalVersion>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, label, created_at FROM doc_version
             WHERE doc_id=?1 ORDER BY created_at DESC",
        )?;
        let rows = stmt
            .query_map(params![doc_id], |r| {
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
        doc_id: &str,
        version_id: &str,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        let row: Option<(Vec<u8>, Option<i64>)> = self
            .conn
            .query_row(
                "SELECT state, state_crc FROM doc_version WHERE id=?1 AND doc_id=?2",
                params![version_id, doc_id],
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
        doc_id: &str,
        version_id: &str,
        client_id: u64,
    ) -> Result<Option<MicaDoc>, StoreError> {
        let target: Option<(Vec<u8>, Option<i64>)> = self
            .conn
            .query_row(
                "SELECT state, state_crc FROM doc_version WHERE id=?1 AND doc_id=?2",
                params![version_id, doc_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((bytes, crc)) = target else {
            return Ok(None);
        };
        // Verify the version blob BEFORE it becomes the live base or reaches yrs —
        // this was one of two recovery paths that previously decoded straight into
        // yrs with no guard at all (docs/code-review-2026-07-20.md P0-0).
        verify_blob_crc(doc_id, &bytes, crc)?;
        // Preserve the current state so the restore can be undone. Carry its crc
        // forward (copy path).
        if let Some((cur, cur_crc)) = self
            .conn
            .query_row(
                "SELECT state, state_crc FROM doc_snapshot WHERE doc_id=?1",
                params![doc_id],
                |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Option<i64>>(1)?)),
            )
            .optional()?
        {
            self.conn.execute(
                "INSERT INTO doc_version(id,doc_id,label,created_at,state,state_crc) VALUES(?1,?2,NULL,?3,?4,?5)",
                params![uuid::Uuid::new_v4().to_string(), doc_id, now_millis(), cur, cur_crc],
            )?;
        }
        self.conn.execute(
            "INSERT INTO doc_snapshot(doc_id,state,updated_at,state_crc) VALUES(?1,?2,?3,?4)
             ON CONFLICT(doc_id) DO UPDATE SET
                 state=excluded.state, updated_at=excluded.updated_at, state_crc=excluded.state_crc",
            params![doc_id, &bytes, now_millis(), crc],
        )?;
        self.conn
            .execute("DELETE FROM doc_update WHERE doc_id=?1", params![doc_id])?;
        // Unwind guard too, not just the crc: a legacy version blob (crc NULL) still
        // reaches yrs here, so match load_doc's containment.
        let doc = contain_yrs_panic(doc_id, || {
            Ok(MicaDoc::from_update_with_client_id(&bytes, Some(client_id))?)
        })?;
        Ok(Some(doc))
    }

    /// Load a document by id, decoding it with `client_id` (the device's actor)
    /// and replaying any incremental updates on top of the base snapshot (P2-M4).
    /// Returns `None` if there's no such document.
    pub fn load_doc(&self, doc_id: &str, client_id: u64) -> Result<Option<MicaDoc>, StoreError> {
        let row: Option<(Vec<u8>, Option<i64>)> = self
            .conn
            .query_row(
                "SELECT state, state_crc FROM doc_snapshot WHERE doc_id=?1",
                params![doc_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let (bytes, crc) = match row {
            Some(pair) => pair,
            None => return Ok(None),
        };
        // Verify the base checksum BEFORE the guard and before yrs. This is THE
        // line that defends the non-unwinding UB path: a rotted base is rejected
        // here, so its bytes never reach from_update_with_client_id.
        verify_blob_crc(doc_id, &bytes, crc)?;
        // Read both logs BEFORE entering the panic guard: these are SQLite
        // errors, which are not corruption and must keep their own message. The
        // helpers verify each row's checksum, so a rotted log row surfaces as
        // CorruptDoc here (via `?`) and never reaches apply_update.
        let remote = self.doc_remote_updates(doc_id)?;
        let local = self.doc_updates(doc_id)?;

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
    pub fn append_update(&self, doc_id: &str, update: &[u8]) -> Result<i64, StoreError> {
        let max_log: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(clock),0) FROM doc_update WHERE doc_id=?1",
            params![doc_id],
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
        let next = max_log.max(self.sync_cursor(doc_id)?.pushed_clock) + 1;
        self.conn.execute(
            "INSERT INTO doc_update(doc_id,clock,payload,payload_crc) VALUES(?1,?2,?3,?4)",
            params![doc_id, next, update, blob_crc(update)],
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
        doc_id: &str,
        rid: i64,
        update: &[u8],
    ) -> Result<(), StoreError> {
        self.conn.execute_batch("BEGIN;")?;
        let result: Result<(), StoreError> = (|| {
            self.conn.execute(
                "INSERT OR IGNORE INTO doc_remote_update(doc_id,rid,payload,payload_crc) VALUES(?1,?2,?3,?4)",
                params![doc_id, rid, update, blob_crc(update)],
            )?;
            self.conn.execute(
                "INSERT INTO sync_cursor(doc_id,last_synced_rid,pushed_clock) VALUES(?1,?2,0)
                 ON CONFLICT(doc_id) DO UPDATE SET
                     last_synced_rid=MAX(last_synced_rid, excluded.last_synced_rid)",
                params![doc_id, rid],
            )?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.conn.execute_batch("COMMIT;")?;
                Ok(())
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK;");
                Err(e)
            }
        }
    }

    /// Batch variant of [`Self::append_remote_update`]: all rows + the cursor
    /// advance in ONE transaction (one commit), so a 1000-row catch-up pull
    /// costs one journal sync instead of a thousand (the reconnect storm the
    /// P4-1 review flagged).
    pub fn append_remote_updates(
        &self,
        doc_id: &str,
        items: &[(i64, Vec<u8>)],
    ) -> Result<(), StoreError> {
        if items.is_empty() {
            return Ok(());
        }
        self.conn.execute_batch("BEGIN;")?;
        let result: Result<(), StoreError> = (|| {
            let mut max_rid = i64::MIN;
            for (rid, update) in items {
                self.conn.execute(
                    "INSERT OR IGNORE INTO doc_remote_update(doc_id,rid,payload,payload_crc) VALUES(?1,?2,?3,?4)",
                    params![doc_id, rid, update, blob_crc(update)],
                )?;
                if *rid > max_rid {
                    max_rid = *rid;
                }
            }
            self.conn.execute(
                "INSERT INTO sync_cursor(doc_id,last_synced_rid,pushed_clock) VALUES(?1,?2,0)
                 ON CONFLICT(doc_id) DO UPDATE SET
                     last_synced_rid=MAX(last_synced_rid, excluded.last_synced_rid)",
                params![doc_id, max_rid],
            )?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.conn.execute_batch("COMMIT;")?;
                Ok(())
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK;");
                Err(e)
            }
        }
    }

    /// A document's persisted remote-stream tail, ordered by `rid`. Each row's
    /// checksum is verified before it is returned, so a rotted remote update
    /// surfaces as `CorruptDoc` instead of being replayed into yrs.
    pub fn doc_remote_updates(&self, doc_id: &str) -> Result<Vec<(i64, Vec<u8>)>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT rid,payload,payload_crc FROM doc_remote_update WHERE doc_id=?1 ORDER BY rid",
        )?;
        let rows = stmt
            .query_map(params![doc_id], |r| {
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
    pub fn log_sizes(&self, doc_id: &str) -> Result<(i64, i64), StoreError> {
        let local: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM doc_update WHERE doc_id=?1",
            params![doc_id],
            |r| r.get(0),
        )?;
        let remote: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM doc_remote_update WHERE doc_id=?1",
            params![doc_id],
            |r| r.get(0),
        )?;
        Ok((local, remote))
    }

    /// All of a document's incremental updates, ordered by `clock`. Each row's
    /// checksum is verified before it is returned (same rationale as
    /// [`Self::doc_remote_updates`]).
    pub fn doc_updates(&self, doc_id: &str) -> Result<Vec<(i64, Vec<u8>)>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT clock,payload,payload_crc FROM doc_update WHERE doc_id=?1 ORDER BY clock")?;
        let rows = stmt
            .query_map(params![doc_id], |r| {
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

    /// Updates with `clock > after` — the queue still to push to the cloud.
    pub fn updates_after(&self, doc_id: &str, after: i64) -> Result<Vec<(i64, Vec<u8>)>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT clock,payload FROM doc_update WHERE doc_id=?1 AND clock>?2 ORDER BY clock",
        )?;
        let rows = stmt
            .query_map(params![doc_id, after], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Drop acked log entries (`clock ≤ up_to_clock`) to bound the append-log,
    /// leaving the un-pushed outbox (`clock > pushed_clock`) intact — callers pass
    /// `up_to_clock = pushed_clock`. Safe: the base snapshot already folds these
    /// updates, so `load_doc` is unchanged; and `append_update` stays monotonic
    /// across the trim (its high-water includes `pushed_clock`), so no future
    /// clock is ever reused below the trimmed range.
    pub fn trim_updates_through(
        &self,
        doc_id: &str,
        up_to_clock: i64,
    ) -> Result<(), StoreError> {
        // Clamp to pushed_clock so an over-eager caller can never delete un-pushed
        // outbox entries (safe-by-construction, not by caller discipline).
        let up_to = up_to_clock.min(self.sync_cursor(doc_id)?.pushed_clock);
        self.conn.execute(
            "DELETE FROM doc_update WHERE doc_id=?1 AND clock<=?2",
            params![doc_id, up_to],
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
    pub fn squash(&self, doc_id: &str, client_id: u64) -> Result<(), StoreError> {
        let doc = match self.load_doc(doc_id, client_id)? {
            Some(d) => d,
            None => return Ok(()),
        };
        let state = doc.encode_state();
        let crc = blob_crc(&state);
        let pushed = self.sync_cursor(doc_id)?.pushed_clock;
        // One transaction: fold + trim + clear commit together (crash mid-way
        // was already safe via replay idempotency, but structural atomicity is
        // cheaper to reason about than a CRDT argument).
        self.conn.execute_batch("BEGIN;")?;
        let result: Result<(), StoreError> = (|| {
            self.conn.execute(
                "INSERT INTO doc_snapshot(doc_id,state,updated_at,state_crc) VALUES(?1,?2,?3,?4)
                 ON CONFLICT(doc_id) DO UPDATE SET
                     state=excluded.state, updated_at=excluded.updated_at, state_crc=excluded.state_crc",
                params![doc_id, state, now_millis(), crc],
            )?;
            self.conn.execute(
                "DELETE FROM doc_update WHERE doc_id=?1 AND clock<=?2",
                params![doc_id, pushed],
            )?;
            // The remote log is fully folded into the new base (it has no
            // un-pushed semantics — the server already owns these updates).
            self.conn.execute(
                "DELETE FROM doc_remote_update WHERE doc_id=?1",
                params![doc_id],
            )?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.conn.execute_batch("COMMIT;")?;
                Ok(())
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK;");
                Err(e)
            }
        }
    }

    /// A document's cloud sync high-water marks (zeroed if never synced).
    pub fn sync_cursor(&self, doc_id: &str) -> Result<SyncCursor, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT last_synced_rid,pushed_clock FROM sync_cursor WHERE doc_id=?1",
                params![doc_id],
                |r| {
                    Ok(SyncCursor {
                        last_synced_rid: r.get(0)?,
                        pushed_clock: r.get(1)?,
                    })
                },
            )
            .optional()?
            .unwrap_or_default())
    }

    /// Persist a document's sync high-water marks.
    pub fn set_sync_cursor(&self, doc_id: &str, cursor: SyncCursor) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO sync_cursor(doc_id,last_synced_rid,pushed_clock) VALUES(?1,?2,?3)
             ON CONFLICT(doc_id) DO UPDATE SET
                 last_synced_rid=excluded.last_synced_rid, pushed_clock=excluded.pushed_clock",
            params![doc_id, cursor.last_synced_rid, cursor.pushed_clock],
        )?;
        Ok(())
    }

    /// All stored document ids (sorted).
    pub fn list_docs(&self) -> Result<Vec<String>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT doc_id FROM doc_snapshot ORDER BY doc_id")?;
        let ids = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ids)
    }

    /// Delete a document.
    pub fn delete_doc(&self, doc_id: &str) -> Result<(), StoreError> {
        for table in [
            "doc_snapshot",
            "doc_snapshot_backup",
            "doc_update",
            "doc_remote_update",
            "sync_cursor",
        ] {
            self.conn
                .execute(&format!("DELETE FROM {table} WHERE doc_id=?1"), params![doc_id])?;
        }
        Ok(())
    }

    // ── snapshot checkpoint / rollback — §10 recovery safety net ─────────────

    /// Save the document's current base snapshot as a recovery checkpoint. Call
    /// at safe points (doc close, app pause) so a later corruption can be rolled
    /// back. No-op if the document has no base yet.
    pub fn checkpoint_doc(&self, doc_id: &str) -> Result<(), StoreError> {
        // SQL-only copy — the checksum rides along in the same statement, so the
        // backup keeps validating against the base's original bytes.
        self.conn.execute(
            "INSERT INTO doc_snapshot_backup(doc_id,state,updated_at,state_crc)
             SELECT doc_id, state, ?2, state_crc FROM doc_snapshot WHERE doc_id=?1
             ON CONFLICT(doc_id) DO UPDATE SET
                 state=excluded.state, updated_at=excluded.updated_at, state_crc=excluded.state_crc",
            params![doc_id, now_millis()],
        )?;
        Ok(())
    }

    /// Restore a document from its last [`Self::checkpoint_doc`], making it the
    /// live base and dropping the incremental log. Returns the restored doc, or
    /// `None` if there's no checkpoint.
    pub fn rollback_doc(
        &self,
        doc_id: &str,
        client_id: u64,
    ) -> Result<Option<MicaDoc>, StoreError> {
        let row: Option<(Vec<u8>, Option<i64>)> = self
            .conn
            .query_row(
                "SELECT state, state_crc FROM doc_snapshot_backup WHERE doc_id=?1",
                params![doc_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let (bytes, crc) = match row {
            Some(pair) => pair,
            None => return Ok(None),
        };
        // The other previously-unguarded decode path (P0-0): verify the backup
        // before it becomes the live base or reaches yrs.
        verify_blob_crc(doc_id, &bytes, crc)?;
        self.conn.execute(
            "INSERT INTO doc_snapshot(doc_id,state,updated_at,state_crc) VALUES(?1,?2,?3,?4)
             ON CONFLICT(doc_id) DO UPDATE SET
                 state=excluded.state, updated_at=excluded.updated_at, state_crc=excluded.state_crc",
            params![doc_id, &bytes, now_millis(), crc],
        )?;
        self.conn
            .execute("DELETE FROM doc_update WHERE doc_id=?1", params![doc_id])?;
        let doc = contain_yrs_panic(doc_id, || {
            Ok(MicaDoc::from_update_with_client_id(&bytes, Some(client_id))?)
        })?;
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
    /// toggling, all by writing the desired row.
    pub fn save_view(&self, v: &LocalView) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO local_view(id,workspace_id,parent_id,object_id,name,position,trashed,origin,object_type)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)
             ON CONFLICT(origin,id) DO UPDATE SET
                 workspace_id=excluded.workspace_id, parent_id=excluded.parent_id,
                 object_id=excluded.object_id, name=excluded.name,
                 position=excluded.position, trashed=excluded.trashed,
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
mod tests {
    use super::*;
    use crate::Block;

    fn sample() -> (String, Vec<Block>) {
        (
            "r".to_string(),
            vec![
                Block::new("r", "page").with_children(vec!["a".into()]),
                Block::new("a", "paragraph").with_text("Hello"),
            ],
        )
    }

    fn temp_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("mica_test_{}.db", uuid::Uuid::new_v4()))
    }

    #[test]
    fn schema_version_stamped_and_downgrade_guarded() {
        let path = temp_path();
        let p = path.to_str().unwrap();
        {
            let store = LocalStore::open(p).unwrap();
            let v: String = store
                .conn
                .query_row(
                    "SELECT value FROM local_meta WHERE key='schema_version'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(v, SCHEMA_VERSION.to_string(), "current version stamped");
        }
        // A store written by a future app version must be refused, not corrupted.
        {
            let conn = Connection::open(p).unwrap();
            conn.execute(
                "UPDATE local_meta SET value='99' WHERE key='schema_version'",
                [],
            )
            .unwrap();
        }
        assert!(matches!(
            LocalStore::open(p),
            Err(StoreError::SchemaTooNew { found: 99, .. })
        ));
        std::fs::remove_file(p).ok();
    }

    #[test]
    fn too_new_store_is_refused_before_any_destructive_migration() {
        // The gate must run BEFORE the probe-driven migrations: a future schema
        // whose PK shape differs would otherwise trip the v4 rebuild, which DROPs
        // and recreates from a hard-coded column list — silently stripping the
        // newer app's columns before the trailing gate could reject. Craft a fake
        // future-versioned store (3-column PK + a v5-only column) and assert open() refuses it
        // with the structure untouched.
        let path = temp_path();
        let p = path.to_str().unwrap();
        {
            let conn = Connection::open(p).unwrap();
            conn.execute_batch(
                "CREATE TABLE local_meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 INSERT INTO local_meta VALUES('schema_version','7');
                 CREATE TABLE local_view(
                     origin TEXT NOT NULL, device TEXT NOT NULL, id TEXT NOT NULL,
                     workspace_id TEXT NOT NULL DEFAULT 'local', parent_id TEXT,
                     object_id TEXT NOT NULL, name TEXT NOT NULL, position TEXT NOT NULL,
                     trashed INTEGER NOT NULL DEFAULT 0,
                     v6_only TEXT,
                     PRIMARY KEY(origin, device, id));
                 CREATE TABLE local_workspace(
                     origin TEXT NOT NULL, device TEXT NOT NULL, id TEXT NOT NULL,
                     name TEXT NOT NULL, position TEXT NOT NULL,
                     role TEXT NOT NULL DEFAULT 'viewer',
                     v6_only TEXT,
                     PRIMARY KEY(origin, device, id));
                 INSERT INTO local_view(origin,device,id,object_id,name,position,v6_only)
                     VALUES('local','dev1','v1','d1','Future Page','0000000010','keep-me');",
            )
            .unwrap();
        }
        assert!(matches!(
            LocalStore::open(p),
            Err(StoreError::SchemaTooNew { found: 7, .. })
        ));
        // Structure untouched: the v6-only column and 3-column PK are intact, and
        // no rebuild artifacts (backup / _v4 temp tables) were created.
        {
            let conn = Connection::open(p).unwrap();
            let v6_kept: String = conn
                .query_row("SELECT v6_only FROM local_view WHERE id='v1'", [], |r| r.get(0))
                .unwrap();
            assert_eq!(v6_kept, "keep-me");
            let pk: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('local_view') WHERE pk>0",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(pk, 3, "future PK shape untouched");
            let artifacts: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE name LIKE '%_v3_backup' OR name LIKE '%_v4'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(artifacts, 0, "no migration artifacts on a refused store");
        }
        std::fs::remove_file(p).ok();
    }

    #[test]
    fn migrates_legacy_schema_without_data_loss() {
        let path = temp_path();
        let p = path.to_str().unwrap();
        // A pre-multi-workspace store: local_view has no workspace_id, no
        // local_workspace table, no schema_version.
        {
            let conn = Connection::open(p).unwrap();
            conn.execute_batch(
                "CREATE TABLE doc_snapshot(doc_id TEXT PRIMARY KEY, state BLOB NOT NULL, updated_at INTEGER NOT NULL);
                 CREATE TABLE local_meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE local_view(id TEXT PRIMARY KEY, parent_id TEXT, object_id TEXT NOT NULL, name TEXT NOT NULL, position TEXT NOT NULL, trashed INTEGER NOT NULL DEFAULT 0);
                 INSERT INTO local_view(id,object_id,name,position) VALUES('v1','doc1','Old Page','0000000010');",
            )
            .unwrap();
        }
        let store = LocalStore::open(p).unwrap();
        let views = store.list_views("local").unwrap();
        let v = views.iter().find(|x| x.id == "v1").expect("legacy view kept");
        assert_eq!(v.name, "Old Page", "legacy data preserved");
        assert_eq!(v.workspace_id, "local", "back-filled to default workspace");
        assert_eq!(v.origin, "local", "legacy rows back-filled to local origin");
        assert_eq!(v.object_type, "document", "object_type (v6) back-filled");
        let local_ws = store.list_workspaces("local").unwrap();
        let default_ws = local_ws.iter().find(|w| w.id == "local");
        assert!(default_ws.is_some(), "default workspace created");
        assert_eq!(default_ws.unwrap().role, "viewer", "role column back-filled");
        let sv: String = store
            .conn
            .query_row(
                "SELECT value FROM local_meta WHERE key='schema_version'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(sv, SCHEMA_VERSION.to_string());
        drop(store);
        std::fs::remove_file(p).ok();
    }

    #[test]
    fn checkpoint_and_rollback() {
        let store = LocalStore::open_in_memory().unwrap();
        let cid = store.identity().unwrap().client_id;
        let (root, blocks) = sample();
        store
            .save_doc("d", &MicaDoc::from_blocks_with_client_id(&root, &blocks, Some(cid)))
            .unwrap();
        store.checkpoint_doc("d").unwrap();

        // Overwrite the live base with an edit.
        let mut working = store.load_doc("d", cid).unwrap().unwrap();
        working.text_insert("a", 5, " EDIT");
        store.save_doc("d", &working).unwrap();
        let has_edit = |d: &MicaDoc| d.to_blocks().iter().any(|b| b.text.contains("EDIT"));
        assert!(has_edit(&store.load_doc("d", cid).unwrap().unwrap()));

        // Rollback restores the checkpoint (edit gone) and clears the update log.
        let restored = store.rollback_doc("d", cid).unwrap().unwrap();
        assert!(!has_edit(&restored));
        assert!(!has_edit(&store.load_doc("d", cid).unwrap().unwrap()));
        assert!(store.rollback_doc("missing", cid).unwrap().is_none());
    }

    #[test]
    fn local_version_history_captures_names_and_restores() {
        let store = LocalStore::open_in_memory().unwrap();
        let cid = store.identity().unwrap().client_id;
        let (root, blocks) = sample();

        // Several saves in a burst → one auto version (10-min convergence).
        let mut d = MicaDoc::from_blocks_with_client_id(&root, &blocks, Some(cid));
        for i in 0..4 {
            d.text_insert("a", 5, &format!("{i}"));
            store.save_doc("d", &d).unwrap();
        }
        let autos = store.list_local_versions("d").unwrap();
        assert_eq!(autos.len(), 1, "burst converges to one auto version");
        assert!(autos[0].label.is_none());

        // Pin a NAMED checkpoint at the current state.
        let named = store.create_local_version("d", "milestone").unwrap().unwrap();
        assert_eq!(named.label.as_deref(), Some("milestone"));
        assert_eq!(store.list_local_versions("d").unwrap().len(), 2);

        // Edit past the checkpoint, then restore to it.
        let mut working = store.load_doc("d", cid).unwrap().unwrap();
        working.text_insert("a", 5, "LATER");
        store.save_doc("d", &working).unwrap();
        let restored = store
            .restore_local_version("d", &named.id, cid)
            .unwrap()
            .unwrap();
        let has_later = |x: &MicaDoc| x.to_blocks().iter().any(|b| b.text.contains("LATER"));
        assert!(!has_later(&restored), "restore dropped the later edit");
        assert!(!has_later(&store.load_doc("d", cid).unwrap().unwrap()));

        // A missing version id is a clean None, not an error.
        assert!(store.restore_local_version("d", "nope", cid).unwrap().is_none());
        // No snapshot yet → naming returns None rather than an empty version.
        assert!(store.create_local_version("ghost", "x").unwrap().is_none());
    }

    #[test]
    fn load_doc_rejects_corrupt_snapshot() {
        let store = LocalStore::open_in_memory().unwrap();
        let cid = store.identity().unwrap().client_id;
        // §10: a corrupt base must surface as an error, never a silent bad state.
        store
            .conn
            .execute(
                "INSERT INTO doc_snapshot(doc_id,state,updated_at) VALUES('bad',?1,0)",
                params![vec![9u8, 9, 9, 9]],
            )
            .unwrap();
        assert!(store.load_doc("bad", cid).is_err());
    }

    #[test]
    fn save_and_load_round_trip() {
        let store = LocalStore::open_in_memory().unwrap();
        let id = store.identity().unwrap();
        let (root, blocks) = sample();
        let doc = MicaDoc::from_blocks_with_client_id(&root, &blocks, Some(id.client_id));
        store.save_doc("doc1", &doc).unwrap();

        let loaded = store.load_doc("doc1", id.client_id).unwrap().unwrap();
        assert_eq!(loaded.to_blocks(), doc.to_blocks());
        assert_eq!(loaded.client_id(), id.client_id);
    }

    #[test]
    fn edits_persist_after_resave() {
        let store = LocalStore::open_in_memory().unwrap();
        let cid = store.identity().unwrap().client_id;
        let (root, blocks) = sample();
        let mut doc = MicaDoc::from_blocks_with_client_id(&root, &blocks, Some(cid));
        store.save_doc("d", &doc).unwrap();

        doc.text_insert("a", 5, " world");
        store.save_doc("d", &doc).unwrap();

        let loaded = store.load_doc("d", cid).unwrap().unwrap();
        let a = loaded.to_blocks().into_iter().find(|b| b.id == "a").unwrap();
        assert_eq!(a.text, "Hello world");
    }

    #[test]
    fn list_and_delete() {
        let store = LocalStore::open_in_memory().unwrap();
        let cid = store.identity().unwrap().client_id;
        let (root, blocks) = sample();
        let doc = MicaDoc::from_blocks_with_client_id(&root, &blocks, Some(cid));
        store.save_doc("a", &doc).unwrap();
        store.save_doc("b", &doc).unwrap();
        assert_eq!(store.list_docs().unwrap(), vec!["a", "b"]);
        store.delete_doc("a").unwrap();
        assert_eq!(store.list_docs().unwrap(), vec!["b"]);
        assert!(store.load_doc("a", cid).unwrap().is_none());
    }

    fn view(id: &str, parent: Option<&str>, name: &str, pos: &str) -> LocalView {
        LocalView {
            id: id.into(),
            workspace_id: "local".into(),
            parent_id: parent.map(|s| s.into()),
            object_id: format!("doc-{id}"),
            name: name.into(),
            position: pos.into(),
            trashed: false,
            origin: "local".into(),
            object_type: "document".into(),
        }
    }

    #[test]
    fn views_crud_and_tree_fields() {
        let store = LocalStore::open_in_memory().unwrap();
        store.save_view(&view("v1", None, "Page 1", "0000000010")).unwrap();
        store.save_view(&view("v2", None, "Page 2", "0000000020")).unwrap();
        store.save_view(&view("v3", Some("v1"), "Child", "0000000010")).unwrap();

        let all = store.list_views("local").unwrap();
        assert_eq!(all.len(), 3);
        // ordered by position; v3 (child, pos 10) and v1 (pos 10) share pos but
        // the child carries its parent.
        let v3 = all.iter().find(|v| v.id == "v3").unwrap();
        assert_eq!(v3.parent_id.as_deref(), Some("v1"));
        assert_eq!(v3.object_id, "doc-v3");

        // Rename + move (upsert same id).
        store.save_view(&LocalView { name: "Renamed".into(), ..view("v2", None, "x", "0000000005") }).unwrap();
        let v2 = store.list_views("local").unwrap().into_iter().find(|v| v.id == "v2").unwrap();
        assert_eq!(v2.name, "Renamed");
        assert_eq!(v2.position, "0000000005");

        // Trash then purge.
        store.save_view(&LocalView { trashed: true, ..view("v1", None, "Page 1", "0000000010") }).unwrap();
        assert!(store.list_views("local").unwrap().iter().find(|v| v.id == "v1").unwrap().trashed);
        store.purge_view("local", "v1").unwrap();
        assert!(store.list_views("local").unwrap().iter().all(|v| v.id != "v1"));
    }

    #[test]
    fn view_object_type_round_trips_folder_and_document() {
        let store = LocalStore::open_in_memory().unwrap();
        // A folder (pure container) and a document, mirrored offline.
        store
            .save_view(&LocalView {
                object_type: "folder".into(),
                ..view("f1", None, "Chapter", "0000000010")
            })
            .unwrap();
        store.save_view(&view("d1", Some("f1"), "Intro", "0000000010")).unwrap();

        let all = store.list_views("local").unwrap();
        let f = all.iter().find(|v| v.id == "f1").unwrap();
        let d = all.iter().find(|v| v.id == "d1").unwrap();
        assert_eq!(f.object_type, "folder", "folder type persists");
        assert_eq!(d.object_type, "document", "document is the default");

        // Upsert preserves the type across a rename (edited via save_view).
        store
            .save_view(&LocalView {
                object_type: "folder".into(),
                name: "Renamed Chapter".into(),
                ..view("f1", None, "x", "0000000010")
            })
            .unwrap();
        let f = store.list_views("local").unwrap().into_iter().find(|v| v.id == "f1").unwrap();
        assert_eq!(f.object_type, "folder");
        assert_eq!(f.name, "Renamed Chapter");
    }

    #[test]
    fn update_log_replays_on_load_and_squash_collapses() {
        let store = LocalStore::open_in_memory().unwrap();
        let cid = store.identity().unwrap().client_id;
        let (root, blocks) = sample();
        let doc = MicaDoc::from_blocks_with_client_id(&root, &blocks, Some(cid));
        store.save_doc("d", &doc).unwrap();

        // Make two edits as incremental updates (capture each as a diff).
        let mut working = store.load_doc("d", cid).unwrap().unwrap();
        let sv0 = working.state_vector();
        working.text_insert("a", 5, " world");
        let u1 = working.encode_diff(&sv0).unwrap();
        store.append_update("d", &u1).unwrap();
        let sv1 = working.state_vector();
        working.insert_block("r", 1, &Block::new("b", "paragraph").with_text("two"));
        let u2 = working.encode_diff(&sv1).unwrap();
        let clock2 = store.append_update("d", &u2).unwrap();
        assert_eq!(clock2, 2);

        // Loading replays base + updates.
        let loaded = store.load_doc("d", cid).unwrap().unwrap();
        let a = loaded.to_blocks().into_iter().find(|b| b.id == "a").unwrap();
        assert_eq!(a.text, "Hello world");
        assert_eq!(loaded.to_blocks().iter().find(|b| b.id == "r").unwrap().children, vec!["a", "b"]);
        assert_eq!(store.doc_updates("d").unwrap().len(), 2);

        // Squash re-baselines: it folds the log into the base and drops the acked
        // prefix. With both edits acked, it clears the log; state is preserved.
        store
            .set_sync_cursor("d", SyncCursor { last_synced_rid: 0, pushed_clock: 2 })
            .unwrap();
        store.squash("d", cid).unwrap();
        assert_eq!(store.doc_updates("d").unwrap().len(), 0);
        let after = store.load_doc("d", cid).unwrap().unwrap();
        assert_eq!(after.to_blocks(), loaded.to_blocks());
    }

    #[test]
    fn squash_keeps_unpushed_tail_and_clock_monotonic() {
        // squash must NOT drop the un-pushed outbox (edits the cloud never got)
        // nor reset the clock — else those edits never sync and a later clock
        // collides. Mirror of trim_bounds_log_and_clock_stays_monotonic, for the
        // squash path (which deletes by a different criterion).
        let store = LocalStore::open_in_memory().unwrap();
        let cid = store.identity().unwrap().client_id;
        let (root, blocks) = sample();
        store
            .save_doc("d", &MicaDoc::from_blocks_with_client_id(&root, &blocks, Some(cid)))
            .unwrap();

        let mut working = store.load_doc("d", cid).unwrap().unwrap();
        let sv0 = working.state_vector();
        working.text_insert("a", 5, " one");
        store.append_update("d", &working.encode_diff(&sv0).unwrap()).unwrap(); // clock 1
        let sv1 = working.state_vector();
        working.text_insert("a", 9, " two");
        store.append_update("d", &working.encode_diff(&sv1).unwrap()).unwrap(); // clock 2

        // Clock 1 acked; clock 2 still un-pushed (the offline-edit norm).
        store
            .set_sync_cursor("d", SyncCursor { last_synced_rid: 5, pushed_clock: 1 })
            .unwrap();
        store.squash("d", cid).unwrap();

        // The un-pushed tail (clock 2) survives so it can still be pushed; the
        // base folds both edits, so content is intact either way.
        let clocks = |s: &LocalStore| {
            s.doc_updates("d").unwrap().into_iter().map(|(c, _)| c).collect::<Vec<_>>()
        };
        assert_eq!(clocks(&store), vec![2], "un-pushed clock 2 kept through squash");
        assert_eq!(
            store.load_doc("d", cid).unwrap().unwrap().to_blocks().iter()
                .find(|b| b.id == "a").unwrap().text,
            "Hello one two",
        );

        // No clock reset: a new edit continues past what was issued.
        let sv2 = working.state_vector();
        working.text_insert("a", 13, "!");
        assert_eq!(
            store.append_update("d", &working.encode_diff(&sv2).unwrap()).unwrap(),
            3,
            "no reset after squash",
        );
        assert_eq!(
            store.updates_after("d", 1).unwrap().into_iter().map(|(c, _)| c).collect::<Vec<_>>(),
            vec![2, 3],
            "outbox still holds the un-pushed edits",
        );
    }

    #[test]
    fn remote_log_replays_advances_cursor_and_squashes() {
        // P4-1: remote updates persist in their own log (never the outbox),
        // advance last_synced_rid transactionally, replay on load, and squash
        // folds BOTH logs while clearing the remote one entirely.
        let store = LocalStore::open_in_memory().unwrap();
        let cid = store.identity().unwrap().client_id;
        let (root, blocks) = sample();
        let doc = MicaDoc::from_blocks_with_client_id(&root, &blocks, Some(cid));
        store.save_doc("d", &doc).unwrap();

        // A "remote" edit: a diff minted by another actor.
        let mut remote =
            MicaDoc::from_update_with_client_id(&doc.encode_state(), Some(999)).unwrap();
        let sv = remote.state_vector();
        remote.text_insert("a", 5, " remote");
        let r_diff = remote.encode_diff(&sv).unwrap();

        store.append_remote_update("d", 7, &r_diff).unwrap();
        // Idempotent per rid (a re-pull replays the same row).
        store.append_remote_update("d", 7, &r_diff).unwrap();
        assert_eq!(store.doc_remote_updates("d").unwrap().len(), 1);
        assert_eq!(
            store.sync_cursor("d").unwrap().last_synced_rid,
            7,
            "cursor moved with the append",
        );
        // A lower rid never regresses the cursor.
        store.append_remote_update("d", 3, &r_diff).unwrap();
        assert_eq!(store.sync_cursor("d").unwrap().last_synced_rid, 7);

        // The outbox stays pure-local: remote rows never enter doc_update.
        assert!(store.doc_updates("d").unwrap().is_empty());

        // A local edit alongside; load replays base + remote + local.
        let mut working = store.load_doc("d", cid).unwrap().unwrap();
        assert!(
            working.to_blocks().iter().any(|b| b.text.contains("remote")),
            "remote log replayed",
        );
        let sv2 = working.state_vector();
        working.text_insert("a", 0, "local ");
        store.append_update("d", &working.encode_diff(&sv2).unwrap()).unwrap();
        let loaded = store.load_doc("d", cid).unwrap().unwrap();
        let a = loaded.to_blocks().into_iter().find(|b| b.id == "a").unwrap();
        assert_eq!(a.text, "local Hello remote");

        // Squash folds both logs into the base; remote log cleared, un-pushed
        // local tail kept (pushed_clock=0 so nothing trims from the outbox).
        store.squash("d", cid).unwrap();
        assert!(store.doc_remote_updates("d").unwrap().is_empty(), "remote log cleared");
        assert_eq!(store.doc_updates("d").unwrap().len(), 1, "un-pushed outbox kept");
        assert_eq!(
            store.load_doc("d", cid).unwrap().unwrap().to_blocks().iter()
                .find(|b| b.id == "a").unwrap().text,
            "local Hello remote",
            "content intact after squash",
        );
        let (local, remote_rows) = store.log_sizes("d").unwrap();
        assert_eq!((local, remote_rows), (1, 0));
    }

    #[test]
    fn batch_remote_append_and_delete_doc_cover_remote_log() {
        let store = LocalStore::open_in_memory().unwrap();
        let cid = store.identity().unwrap().client_id;
        let (root, blocks) = sample();
        let doc = MicaDoc::from_blocks_with_client_id(&root, &blocks, Some(cid));
        store.save_doc("d", &doc).unwrap();
        let mut remote =
            MicaDoc::from_update_with_client_id(&doc.encode_state(), Some(42)).unwrap();
        let mut items = Vec::new();
        for i in 0..5 {
            let sv = remote.state_vector();
            remote.text_insert("a", 5, &format!(" r{i}"));
            items.push((10 + i as i64, remote.encode_diff(&sv).unwrap()));
        }
        // One transaction: all rows land, cursor at the max rid.
        store.append_remote_updates("d", &items).unwrap();
        assert_eq!(store.doc_remote_updates("d").unwrap().len(), 5);
        assert_eq!(store.sync_cursor("d").unwrap().last_synced_rid, 14);
        // Batch is idempotent per rid too.
        store.append_remote_updates("d", &items).unwrap();
        assert_eq!(store.doc_remote_updates("d").unwrap().len(), 5);
        // delete_doc removes the remote log with everything else.
        store.delete_doc("d").unwrap();
        assert!(store.doc_remote_updates("d").unwrap().is_empty());
        assert!(store.load_doc("d", cid).unwrap().is_none());
    }

    #[test]
    fn sync_cursor_round_trip() {
        let store = LocalStore::open_in_memory().unwrap();
        assert_eq!(store.sync_cursor("d").unwrap(), SyncCursor::default());
        store
            .set_sync_cursor("d", SyncCursor { last_synced_rid: 42, pushed_clock: 7 })
            .unwrap();
        let c = store.sync_cursor("d").unwrap();
        assert_eq!(c.last_synced_rid, 42);
        assert_eq!(c.pushed_clock, 7);
        // updates_after honours the pushed cursor.
        let cid = store.identity().unwrap().client_id;
        let doc = MicaDoc::from_blocks_with_client_id("r", &[Block::new("r", "page")], Some(cid));
        store.save_doc("d", &doc).unwrap();
        // append_update is monotonic PAST pushed_clock — a fresh clock always
        // lands in the outbox (updates_after(pushed_clock)), never ≤ it.
        let c1 = store.append_update("d", &[1, 2, 3]).unwrap();
        let c2 = store.append_update("d", &[4, 5, 6]).unwrap();
        assert_eq!((c1, c2), (8, 9), "clock resumes past pushed_clock=7");
        assert_eq!(store.updates_after("d", 7).unwrap().len(), 2, "both are the outbox");
        assert_eq!(store.updates_after("d", 8).unwrap().len(), 1);
    }

    #[test]
    fn trim_bounds_log_and_clock_stays_monotonic() {
        // The P2 outbox invariant: trimming acked entries must never let a later
        // clock be reused at/below pushed_clock (it would fall outside the outbox
        // query `updates_after(pushed_clock)` and never be pushed = divergence).
        let store = LocalStore::open_in_memory().unwrap();
        let cid = store.identity().unwrap().client_id;
        let doc = MicaDoc::from_blocks_with_client_id("r", &[Block::new("r", "page")], Some(cid));
        store.save_doc("d", &doc).unwrap();
        assert_eq!(store.append_update("d", &[1]).unwrap(), 1);
        assert_eq!(store.append_update("d", &[2]).unwrap(), 2);
        assert_eq!(store.append_update("d", &[3]).unwrap(), 3);

        // Ack through clock 2, then trim the acked prefix.
        store
            .set_sync_cursor("d", SyncCursor { last_synced_rid: 5, pushed_clock: 2 })
            .unwrap();
        store.trim_updates_through("d", 2).unwrap();
        let clocks = |s: &LocalStore| {
            s.doc_updates("d").unwrap().into_iter().map(|(c, _)| c).collect::<Vec<_>>()
        };
        assert_eq!(clocks(&store), vec![3], "only the un-pushed tail survives");

        // A new edit continues past the trimmed range, not back at 1.
        assert_eq!(store.append_update("d", &[4]).unwrap(), 4, "monotonic past trim");
        assert_eq!(
            store.updates_after("d", 2).unwrap().into_iter().map(|(c, _)| c).collect::<Vec<_>>(),
            vec![3, 4],
            "outbox = unpushed tail"
        );

        // Full trim (everything acked) then append still doesn't reset to 1.
        store
            .set_sync_cursor("d", SyncCursor { last_synced_rid: 6, pushed_clock: 4 })
            .unwrap();
        store.trim_updates_through("d", 4).unwrap();
        assert!(clocks(&store).is_empty(), "empty after full trim");
        assert_eq!(store.append_update("d", &[5]).unwrap(), 5, "no reset after full trim");
    }

    #[test]
    fn workspaces_crud_and_default() {
        let store = LocalStore::open_in_memory().unwrap();
        // A default workspace always exists.
        let all = store.list_workspaces("local").unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "local");
        assert_eq!(all[0].origin, "local", "default workspace is local-origin");

        store
            .save_workspace(&LocalWorkspace {
                id: "w2".into(),
                name: "Work".into(),
                position: "0000000020".into(),
                origin: "local".into(),
                role: "owner".into(),
            })
            .unwrap();
        assert_eq!(store.list_workspaces("local").unwrap().len(), 2);
        assert_eq!(
            store.list_workspaces("local").unwrap().iter().find(|w| w.id == "w2").unwrap().role,
            "owner",
            "role round-trips",
        );

        // Views are scoped; deleting a workspace removes only its views.
        store.save_view(&LocalView { workspace_id: "w2".into(), ..view("a", None, "A", "0000000010") }).unwrap();
        store.save_view(&LocalView { workspace_id: "local".into(), ..view("b", None, "B", "0000000010") }).unwrap();
        store.delete_workspace("local", "w2").unwrap();
        let views = store.list_views("local").unwrap();
        assert!(views.iter().any(|v| v.id == "b"));
        assert!(views.iter().all(|v| v.id != "a"));
        assert!(store.list_workspaces("local").unwrap().iter().all(|w| w.id != "w2"));
    }

    #[test]
    fn origin_scopes_views_and_workspaces() {
        // One store holds both on-device data (origin "local") and a cloud
        // workspace mirrored for offline nav (origin = a server URL). Listing by
        // origin must never leak one provenance into the other.
        let store = LocalStore::open_in_memory().unwrap();
        let cloud = "https://mica.example.com";

        store
            .save_workspace(&LocalWorkspace {
                id: "cw".into(),
                name: "Cloud WS".into(),
                position: "0000000010".into(),
                origin: cloud.into(),
                role: "editor".into(),
            })
            .unwrap();
        store
            .save_view(&LocalView { origin: cloud.into(), workspace_id: "cw".into(), ..view("cv", None, "Cloud Page", "0000000010") })
            .unwrap();
        // A local view/workspace alongside it.
        store.save_view(&view("lv", None, "Local Page", "0000000010")).unwrap();

        let local_views = store.list_views("local").unwrap();
        assert!(local_views.iter().any(|v| v.id == "lv"));
        assert!(local_views.iter().all(|v| v.id != "cv"), "cloud view hidden from local");

        let cloud_views = store.list_views(cloud).unwrap();
        assert_eq!(cloud_views.len(), 1);
        assert_eq!(cloud_views[0].id, "cv");
        assert_eq!(cloud_views[0].origin, cloud);

        // Default "local" workspace exists; cloud origin sees only its own.
        assert!(store.list_workspaces("local").unwrap().iter().any(|w| w.id == "local"));
        let cloud_ws = store.list_workspaces(cloud).unwrap();
        assert_eq!(cloud_ws.len(), 1);
        assert_eq!(cloud_ws[0].id, "cw");
        assert_eq!(cloud_ws[0].role, "editor", "mirrored role available offline");
    }

    #[test]
    fn composite_pk_isolates_same_id_across_origins() {
        // P3a: the SAME id may exist under 'local' AND a server URL (the detach
        // scenario). The (origin,id) PK must let both rows coexist, upserts stay
        // in-namespace, and purge/delete touch only their own origin.
        let store = LocalStore::open_in_memory().unwrap();
        let cloud = "https://mica.example.com";

        // Same view id in both namespaces.
        store.save_view(&view("x", None, "Local X", "0000000010")).unwrap();
        store
            .save_view(&LocalView {
                origin: cloud.into(),
                name: "Cloud X".into(),
                ..view("x", None, "ignored", "0000000020")
            })
            .unwrap();
        assert_eq!(store.list_views("local").unwrap().iter().find(|v| v.id == "x").unwrap().name, "Local X");
        assert_eq!(store.list_views(cloud).unwrap().iter().find(|v| v.id == "x").unwrap().name, "Cloud X");

        // Upsert stays in its namespace (would have clobbered cross-origin
        // under the old bare-id PK).
        store.save_view(&view("x", None, "Local X2", "0000000010")).unwrap();
        assert_eq!(store.list_views(cloud).unwrap().iter().find(|v| v.id == "x").unwrap().name, "Cloud X");

        // Purge one origin's row only.
        store.purge_view("local", "x").unwrap();
        assert!(store.list_views("local").unwrap().iter().all(|v| v.id != "x"));
        assert!(store.list_views(cloud).unwrap().iter().any(|v| v.id == "x"), "cloud row untouched");

        // Same workspace id in both namespaces; delete is origin-scoped and its
        // view cascade never crosses origins.
        for (origin, role) in [("local", "owner"), (cloud, "editor")] {
            store
                .save_workspace(&LocalWorkspace {
                    id: "w".into(),
                    name: format!("{origin} W"),
                    position: "0000000010".into(),
                    origin: origin.into(),
                    role: role.into(),
                })
                .unwrap();
            store
                .save_view(&LocalView {
                    origin: origin.into(),
                    workspace_id: "w".into(),
                    ..view(&format!("v-{}", role), None, "p", "0000000010")
                })
                .unwrap();
        }
        store.delete_workspace(cloud, "w").unwrap();
        assert!(store.list_workspaces("local").unwrap().iter().any(|w| w.id == "w"), "local ws kept");
        assert!(store.list_views("local").unwrap().iter().any(|v| v.id == "v-owner"), "local view kept");
        assert!(store.list_workspaces(cloud).unwrap().iter().all(|w| w.id != "w"));
        assert!(store.list_views(cloud).unwrap().iter().all(|v| v.id != "v-editor"));
    }

    #[test]
    fn v3_bare_pk_store_rebuilds_to_composite_without_data_loss() {
        // A real v3-era store: bare-id PKs, origin/role columns present, data in
        // both namespaces. Opening must rebuild both tables to the (origin,id)
        // PK, keep every row, and leave the one-time pre-migration backups.
        let path = temp_path();
        let p = path.to_str().unwrap();
        {
            let conn = Connection::open(p).unwrap();
            conn.execute_batch(
                "CREATE TABLE doc_snapshot(doc_id TEXT PRIMARY KEY, state BLOB NOT NULL, updated_at INTEGER NOT NULL);
                 CREATE TABLE local_meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 INSERT INTO local_meta VALUES('schema_version','3');
                 CREATE TABLE local_view(
                     id TEXT PRIMARY KEY, parent_id TEXT, object_id TEXT NOT NULL,
                     name TEXT NOT NULL, position TEXT NOT NULL,
                     trashed INTEGER NOT NULL DEFAULT 0,
                     origin TEXT NOT NULL DEFAULT 'local',
                     workspace_id TEXT NOT NULL DEFAULT 'local');
                 CREATE TABLE local_workspace(
                     id TEXT PRIMARY KEY, name TEXT NOT NULL, position TEXT NOT NULL,
                     origin TEXT NOT NULL DEFAULT 'local',
                     role TEXT NOT NULL DEFAULT 'viewer');
                 INSERT INTO local_workspace(id,name,position) VALUES('local','本地工作区','0000000010');
                 INSERT INTO local_workspace(id,name,position,origin,role) VALUES('cw','Cloud','0000000020','https://s','editor');
                 INSERT INTO local_view(id,object_id,name,position) VALUES('v1','d1','Local Page','0000000010');
                 INSERT INTO local_view(id,object_id,name,position,origin,workspace_id) VALUES('v2','d2','Cloud Page','0000000010','https://s','cw');",
            )
            .unwrap();
        }
        let store = LocalStore::open(p).unwrap();
        // PKs are composite now.
        for table in ["local_view", "local_workspace"] {
            let pk: i64 = store
                .conn
                .query_row(
                    &format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE pk>0"),
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(pk, 2, "{table} has the (origin,id) composite PK");
        }
        // Every row survived, per namespace.
        assert_eq!(store.list_views("local").unwrap().iter().find(|v| v.id == "v1").unwrap().name, "Local Page");
        let cv = store.list_views("https://s").unwrap();
        assert_eq!(cv.len(), 1);
        assert_eq!((cv[0].id.as_str(), cv[0].workspace_id.as_str()), ("v2", "cw"));
        assert_eq!(
            store.list_workspaces("https://s").unwrap()[0].role,
            "editor",
            "cloud role survived the rebuild",
        );
        // The one-time pre-migration snapshots exist (recovery hatch).
        for backup in ["local_view_v3_backup", "local_workspace_v3_backup"] {
            let n: i64 = store
                .conn
                .query_row(&format!("SELECT COUNT(*) FROM {backup}"), [], |r| r.get(0))
                .unwrap();
            assert!(n >= 1, "{backup} holds the pre-v4 rows");
        }
        // Reopen is a no-op (idempotent — the PK probe skips the rebuild).
        drop(store);
        let again = LocalStore::open(p).unwrap();
        assert_eq!(again.list_views("local").unwrap().len(), 1);
        drop(again);
        std::fs::remove_file(p).ok();
    }

    /// Pre-release migration smoke: point MICA_REAL_STORE at a COPY of a real
    /// on-device store.db and run with `cargo test --features store -- --ignored`.
    /// Opens it through the full migration chain and asserts the terminal shape
    /// (v4 composite PK) plus that existing local rows are still listed.
    #[test]
    #[ignore]
    fn upgrade_real_store_smoke() {
        let p = match std::env::var("MICA_REAL_STORE") {
            Ok(p) if !p.is_empty() => p,
            _ => {
                eprintln!("MICA_REAL_STORE not set — skipping");
                return;
            }
        };
        let before_views: i64 = {
            let conn = Connection::open(&p).unwrap();
            conn.query_row("SELECT COUNT(*) FROM local_view", [], |r| r.get(0))
                .unwrap()
        };
        let store = LocalStore::open(&p).unwrap();
        for table in ["local_view", "local_workspace"] {
            let pk: i64 = store
                .conn
                .query_row(
                    &format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE pk>0"),
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(pk, 2, "{table} migrated to the (origin,id) composite PK");
        }
        let after = store.list_views("local").unwrap();
        assert_eq!(after.len() as i64, before_views, "no view rows lost in the upgrade");
        eprintln!(
            "real store upgraded OK: {} views, workspaces={:?}",
            after.len(),
            store.list_workspaces("local").unwrap().iter().map(|w| w.id.clone()).collect::<Vec<_>>(),
        );
    }

    /// The `contain_yrs_panic` guard, in ISOLATION from the checksum.
    ///
    /// Reported from a real install: `yrs-0.27.0/src/block_store.rs:60 index out
    /// of bounds: the len is 732 but the index is 715827364`, raised from
    /// `load_doc` on the cloud-seed path. The desktop FFI holds
    /// `Mutex<LocalStore>` across this call, so the panic poisoned that lock and
    /// every later `list_views` failed: the page tree rendered blank on every
    /// launch.
    ///
    /// Byte 9 flipped survives `decode_v1` and blows up later during INTEGRATION
    /// (the class `?` never saw and `catch_unwind` does) — a truncation would be
    /// rejected by `decode_v1` and prove nothing. Crucially, `state_crc` is set
    /// to NULL first: this is a LEGACY row with no checksum, so `verify_blob_crc`
    /// passes it through and the bytes actually reach yrs. Without that, the new
    /// checksum would catch the flip before the guard and this test would no
    /// longer exercise `contain_yrs_panic` at all.
    #[test]
    fn a_corrupt_snapshot_is_an_error_not_a_panic() {
        let store = LocalStore::open_in_memory().unwrap();
        let doc = MicaDoc::from_blocks(
            "r",
            &[
                crate::Block::new("r", "page").with_children(vec!["a".into()]),
                crate::Block::new("a", "paragraph").with_text("hello world".to_string()),
            ],
        );
        store.save_doc("d", &doc).unwrap();
        let good: Vec<u8> = store
            .conn
            .query_row("SELECT state FROM doc_snapshot WHERE doc_id='d'", [], |r| {
                r.get(0)
            })
            .unwrap();

        let mut bad = good.clone();
        bad[9] ^= 0xff;
        // Corrupt the bytes AND null the checksum → the crc can't mask the flip,
        // so the corruption reaches yrs and only the guard can save us.
        store
            .conn
            .execute(
                "UPDATE doc_snapshot SET state=?1, state_crc=NULL WHERE doc_id='d'",
                params![bad],
            )
            .unwrap();

        // Returning at all is the point: without the guard this line unwinds
        // out of `load_doc` and poisons the caller's mutex.
        match store.load_doc("d", 1) {
            Err(StoreError::CorruptDoc { doc_id }) => assert_eq!(doc_id, "d"),
            Ok(_) => panic!(
                "expected CorruptDoc but the load SUCCEEDED — yrs changed and the \
                 mutation no longer reaches the panicking path, so this test has \
                 stopped covering anything"
            ),
            Err(e) => panic!("expected CorruptDoc, got a different error: {e}"),
        }
    }

    /// UPSTREAM SOUNDNESS BUG, still present in yrs 0.27.3 (checked 2026-07-21).
    ///
    /// Feeding mutated bytes to `load_doc` reaches, in a debug build:
    ///
    /// ```text
    /// core/src/str/validations.rs:48: unsafe precondition(s) violated:
    ///   hint::unreachable_unchecked must never be reached
    /// thread caused non-unwinding panic. aborting.
    /// ```
    ///
    /// yrs reads unvalidated bytes as UTF-8. This is NOT containable:
    /// `catch_unwind` cannot catch a non-unwinding abort, and in a RELEASE build
    /// the check is compiled out, so the same input is plain undefined behavior.
    ///
    /// It matters well beyond the local cache: `mica-app-core`'s sync path
    /// applies client-supplied updates server-side (`sync.rs`, `apply_update`),
    /// so a crafted update is reachable remotely.
    ///
    /// `#[ignore]` because it ABORTS the test process — it cannot share a run
    /// with other tests. Run it deliberately to check whether upstream has
    /// fixed this:
    ///
    /// ```text
    /// cargo test -p mica-core --features store -- --ignored yrs_corrupt_input_is_unsound
    /// ```
    #[test]
    #[ignore = "aborts the process: upstream yrs UB on malformed input, not containable"]
    fn yrs_corrupt_input_is_unsound() {
        let store = LocalStore::open_in_memory().unwrap();
        let doc = MicaDoc::from_blocks(
            "r",
            &[
                crate::Block::new("r", "page").with_children(vec!["a".into()]),
                crate::Block::new("a", "paragraph").with_text("hello world".to_string()),
            ],
        );
        store.save_doc("d", &doc).unwrap();
        let good: Vec<u8> = store
            .conn
            .query_row("SELECT state FROM doc_snapshot WHERE doc_id='d'", [], |r| {
                r.get(0)
            })
            .unwrap();

        for i in 0..good.len() {
            for xor in [0xff_u8, 0x7f, 0x01] {
                let mut bad = good.clone();
                bad[i] ^= xor;
                store
                    .conn
                    .execute(
                        "UPDATE doc_snapshot SET state=?1 WHERE doc_id='d'",
                        params![bad],
                    )
                    .unwrap();
                let _ = store.load_doc("d", 1);
            }
        }
    }

    // ── local blob checksum (docs/code-review-2026-07-20.md P0-0) ────────────

    /// Locks the polynomial/params so a silent CRC drift can't slip through.
    /// Values are the standard CRC-32/ISO-HDLC check vectors.
    #[test]
    fn crc32_known_vectors() {
        assert_eq!(crc32(b""), 0x0000_0000);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b"The quick brown fox jumps over the lazy dog"), 0x414F_A339);
    }

    fn stored_doc() -> (LocalStore, Vec<u8>) {
        let store = LocalStore::open_in_memory().unwrap();
        let (root, blocks) = sample();
        let doc = MicaDoc::from_blocks(&root, &blocks);
        store.save_doc("d", &doc).unwrap();
        let state: Vec<u8> = store
            .conn
            .query_row("SELECT state FROM doc_snapshot WHERE doc_id='d'", [], |r| r.get(0))
            .unwrap();
        (store, state)
    }

    /// The write path stamps a correct checksum, and — critically — re-stamps it
    /// on the SECOND save (the UPSERT). A missing `state_crc=excluded.state_crc`
    /// in the SET clause would leave the crc stale here and false-flag the doc.
    #[test]
    fn save_records_and_updates_crc() {
        let (store, state) = stored_doc();
        let crc: Option<i64> = store
            .conn
            .query_row("SELECT state_crc FROM doc_snapshot WHERE doc_id='d'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(crc, Some(blob_crc(&state)), "first save records the crc");

        // Second save with changed content must advance the crc, not leave it stale.
        let (root, mut blocks) = sample();
        blocks.push(Block::new("b", "paragraph").with_text("more"));
        blocks[0] = Block::new("r", "page").with_children(vec!["a".into(), "b".into()]);
        store.save_doc("d", &MicaDoc::from_blocks(&root, &blocks)).unwrap();
        let (state2, crc2): (Vec<u8>, Option<i64>) = store
            .conn
            .query_row("SELECT state, state_crc FROM doc_snapshot WHERE doc_id='d'", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(crc2, Some(blob_crc(&state2)), "second save re-stamps the crc");
        assert!(store.load_doc("d", 1).unwrap().is_some(), "still loads after re-save");
    }

    /// Primary mutation test, isolated from yrs: corrupt ONLY the checksum, leave
    /// `state` a VALID yrs update. yrs would decode it without panicking, so the
    /// ONLY thing that can produce an error is `verify_blob_crc`. Remove that call
    /// and `load_doc` returns `Ok(Some(_))` → this test fails. That is the
    /// mutation-kills-test guarantee, independent of how yrs reacts to garbage.
    #[test]
    fn corrupt_state_crc_is_caught_before_yrs() {
        let (store, _state) = stored_doc();
        store
            .conn
            .execute("UPDATE doc_snapshot SET state_crc = state_crc + 1 WHERE doc_id='d'", [])
            .unwrap();
        assert!(
            matches!(store.load_doc("d", 1), Err(StoreError::CorruptDoc { .. })),
            "a stale checksum over valid bytes must be caught by the crc alone"
        );
    }

    /// Real bit-rot: flip a byte of `state`, keep the (now-mismatched) crc → the
    /// crc catches it BEFORE the bytes reach yrs (the only defense against the
    /// non-unwinding UB path).
    #[test]
    fn bit_flip_in_state_is_caught_by_crc() {
        let (store, mut state) = stored_doc();
        state[5] ^= 0x01;
        store
            .conn
            .execute("UPDATE doc_snapshot SET state=?1 WHERE doc_id='d'", params![state])
            .unwrap();
        assert!(matches!(store.load_doc("d", 1), Err(StoreError::CorruptDoc { .. })));
    }

    /// The two previously-UNPROTECTED recovery decodes now verify their blob.
    #[test]
    fn restore_and_rollback_verify_crc() {
        // restore_local_version: corrupt a version's crc → CorruptDoc, no panic.
        let (store, _s) = stored_doc();
        let v = store.create_local_version("d", "v1").unwrap().unwrap();
        store
            .conn
            .execute("UPDATE doc_version SET state_crc = state_crc + 1 WHERE id=?1", params![v.id])
            .unwrap();
        assert!(matches!(
            store.restore_local_version("d", &v.id, 1),
            Err(StoreError::CorruptDoc { .. })
        ));

        // rollback_doc: checkpoint, corrupt the backup's crc → CorruptDoc.
        let (store2, _s2) = stored_doc();
        store2.checkpoint_doc("d").unwrap();
        store2
            .conn
            .execute("UPDATE doc_snapshot_backup SET state_crc = state_crc + 1 WHERE doc_id='d'", [])
            .unwrap();
        assert!(matches!(store2.rollback_doc("d", 1), Err(StoreError::CorruptDoc { .. })));
    }

    /// A rotted local update log row is caught (via the log helpers) and never
    /// replayed into yrs.
    #[test]
    fn corrupt_update_log_crc_is_caught() {
        let (store, _s) = stored_doc();
        store.append_update("d", b"\x00\x01\x02").unwrap();
        store
            .conn
            .execute("UPDATE doc_update SET payload_crc = payload_crc + 1 WHERE doc_id='d'", [])
            .unwrap();
        assert!(matches!(store.load_doc("d", 1), Err(StoreError::CorruptDoc { .. })));
    }

    /// `local_version_state` verifies the preview blob it hands out (its decode
    /// happens outside store.rs).
    #[test]
    fn local_version_state_verifies_crc() {
        let (store, _s) = stored_doc();
        let v = store.create_local_version("d", "v1").unwrap().unwrap();
        assert!(store.local_version_state("d", &v.id).unwrap().is_some(), "healthy preview loads");
        store
            .conn
            .execute("UPDATE doc_version SET state_crc = state_crc + 1 WHERE id=?1", params![v.id])
            .unwrap();
        assert!(matches!(
            store.local_version_state("d", &v.id),
            Err(StoreError::CorruptDoc { .. })
        ));
    }

    /// A legacy row (crc NULL) must NOT error on load — the None branch passes
    /// through to exactly today's behavior. And with the crc NULL, integration-
    /// phase corruption still degrades via the guard (not the crc), proving the
    /// fall-through is intact. (Companion to `a_corrupt_snapshot_is_an_error_not_a_panic`.)
    #[test]
    fn legacy_null_crc_still_loads() {
        let (store, _state) = stored_doc();
        store
            .conn
            .execute("UPDATE doc_snapshot SET state_crc=NULL WHERE doc_id='d'", [])
            .unwrap();
        assert!(
            store.load_doc("d", 1).unwrap().is_some(),
            "NULL crc = no checksum recorded → must load, not error"
        );
    }

    /// A pre-checksum v6 store (built with a RAW connection using the OLD DDL, so
    /// the probe-ALTER — not the base DDL — is what adds the column) upgrades
    /// without data loss: column added, existing row's crc NULL, blob byte-
    /// identical, loads fine, version unchanged.
    #[test]
    fn v6_store_without_crc_column_migrates() {
        let path = temp_path();
        let p = path.to_str().unwrap();
        let (root, blocks) = sample();
        let state = MicaDoc::from_blocks(&root, &blocks).encode_state();
        {
            // OLD shape: no state_crc column.
            let conn = Connection::open(p).unwrap();
            conn.execute_batch(
                "CREATE TABLE doc_snapshot(doc_id TEXT PRIMARY KEY, state BLOB NOT NULL, updated_at INTEGER NOT NULL);
                 CREATE TABLE local_meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO doc_snapshot(doc_id,state,updated_at) VALUES('d',?1,0)",
                params![state],
            )
            .unwrap();
            conn.execute("INSERT INTO local_meta(key,value) VALUES('schema_version','6')", [])
                .unwrap();
        }
        let store = LocalStore::open(p).unwrap();
        let has: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('doc_snapshot') WHERE name='state_crc'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(has, 1, "probe-ALTER added state_crc");
        let (blob, crc): (Vec<u8>, Option<i64>) = store
            .conn
            .query_row("SELECT state, state_crc FROM doc_snapshot WHERE doc_id='d'", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(blob, state, "existing blob byte-identical (no data loss)");
        assert!(crc.is_none(), "legacy row's crc stays NULL (not fabricated)");
        assert!(store.load_doc("d", 1).unwrap().is_some(), "legacy row still loads");
        let v: String = store
            .conn
            .query_row("SELECT value FROM local_meta WHERE key='schema_version'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, SCHEMA_VERSION.to_string(), "no version bump");
        let _ = std::fs::remove_file(p);
    }

    /// Guards against an accidental SCHEMA_VERSION bump and confirms a fresh
    /// store carries all five crc columns.
    #[test]
    fn schema_unchanged_and_crc_columns_present() {
        assert_eq!(SCHEMA_VERSION, 6, "adding crc columns must NOT bump the version");
        let store = LocalStore::open_in_memory().unwrap();
        for (table, col) in [
            ("doc_snapshot", "state_crc"),
            ("doc_version", "state_crc"),
            ("doc_snapshot_backup", "state_crc"),
            ("doc_update", "payload_crc"),
            ("doc_remote_update", "payload_crc"),
        ] {
            let has: i64 = store
                .conn
                .query_row(
                    &format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name='{col}'"),
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(has, 1, "{table}.{col} present in a fresh store");
        }
    }

    #[test]
    fn views_survive_reopen() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("mica_views_test_{}.db", std::process::id()));
        let p = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&p);
        {
            let s = LocalStore::open(&p).unwrap();
            s.save_view(&view("v1", None, "Persisted", "0000000010")).unwrap();
        }
        {
            let s = LocalStore::open(&p).unwrap();
            let all = s.list_views("local").unwrap();
            assert_eq!(all.len(), 1);
            assert_eq!(all[0].name, "Persisted");
        }
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn identity_is_stable_across_reopen() {
        // A real file so identity must survive a fresh connection.
        let dir = std::env::temp_dir();
        let path = dir.join(format!("mica_store_test_{}.db", std::process::id()));
        let p = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&p);

        let first = {
            let s = LocalStore::open(&p).unwrap();
            s.identity().unwrap()
        };
        let second = {
            let s = LocalStore::open(&p).unwrap();
            s.identity().unwrap()
        };
        assert_eq!(first, second, "device id + client id persist across reopen");
        assert_ne!(first.client_id, 0);

        let _ = std::fs::remove_file(&p);
    }
}
