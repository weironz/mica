//! Server-side yrs CRDT sync (P2-M4), parallel to the op model.
//!
//! The cloud keeps a per-workspace monotonic update stream (`workspace_updates`,
//! ordered by a global bigserial `rid`) plus a folded yrs base per document
//! (`document_yrs_base`). Clients push their local yrs updates and get back a
//! `rid`; they catch up by pulling everything in the workspace after their last
//! `rid`. CRDT merge makes pushes commutative + idempotent, so the same update
//! arriving twice (or out of order) is harmless.
//!
//! The op model (`documents.rs`/`store.rs`) still runs; the first time a document
//! is touched through this path its base is built lazily from the latest op-model
//! snapshot, so existing data flows in without a separate migration pass.

use mica_core::{Block as CoreBlock, DocError, MicaDoc};
use mica_infra::{ApiError, ApiResult};
use mica_markdown::DocumentSnapshotPayload;
use serde::Serialize;
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::store::latest_snapshot_tx;

/// `MicaDoc::from_update`, but a yrs PANIC on malformed bytes becomes a
/// `DocError` instead of unwinding out of the request handler.
///
/// yrs 0.27.x panics (index out of bounds while integrating a crafted update)
/// rather than returning `Err` — and on the server the bytes are client-supplied
/// (`push_update`) or caller-supplied (`restore_yrs_version`). Without this the
/// panic unwinds; tokio catches it at the task boundary so the PROCESS survives,
/// but that one connection dies with a scary stack trace instead of a clean 400.
/// Wrapping the call turns it into the same `DocError` the `.map_err` at each
/// site already handles, so the request is rejected, not dropped.
///
/// This does NOT catch yrs's upstream NON-unwinding UB (the `unreachable_unchecked`
/// path — see mica-core `store.rs` P0-0 / `yrs_corrupt_input_is_unsound`):
/// `catch_unwind` structurally cannot, and in release it is plain UB. That needs
/// an upstream fix; this only closes the recoverable unwinding-panic class. The
/// server's panic strategy is `unwind` (no `panic = "abort"`), so this works.
fn guarded_from_update(bytes: &[u8]) -> Result<MicaDoc, DocError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| MicaDoc::from_update(bytes)))
        .unwrap_or_else(|_| Err(DocError::Decode("yrs panicked while decoding an update".into())))
}

/// [`MicaDoc::apply_update`] with the same panic containment as
/// [`guarded_from_update`] — the client-update apply in `push_update` is the
/// primary remotely-reachable path.
fn guarded_apply(doc: &mut MicaDoc, update: &[u8]) -> Result<(), DocError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| doc.apply_update(update)))
        .unwrap_or_else(|_| Err(DocError::Decode("yrs panicked while applying an update".into())))
}

/// Keep at least this many already-folded updates on the stream so a briefly
/// offline client can catch up incrementally; older ones are pruned (they live
/// in the base). A client further behind re-bootstraps from the base instead.
const STREAM_KEEP_MARGIN: i64 = 64;
/// Prune roughly once every this-many pushes (keeps the prune off the hot path).
const STREAM_PRUNE_EVERY: i64 = 32;

/// Result of catching a client up from its cursor: either the incremental
/// updates, or a base to re-bootstrap from when the cursor fell behind the
/// pruned window (P2-M4 §10 large-doc / stream-growth control).
pub enum CatchUp {
    Updates(Vec<StreamUpdate>),
    Rebootstrap(YrsBase),
}

/// One update on the per-workspace stream, returned by catch-up pulls.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct StreamUpdate {
    pub rid: i64,
    pub document_id: Uuid,
    pub actor_id: Uuid,
    pub payload: Vec<u8>,
}

/// The bootstrap base a fresh client applies before pulling stream updates.
#[derive(Debug, Clone)]
pub struct YrsBase {
    pub state: Vec<u8>,
    pub state_vector: Vec<u8>,
    /// Highest stream `rid` already folded into `state`.
    pub base_rid: i64,
}

pub(crate) fn to_core_block(b: mica_markdown::Block) -> CoreBlock {
    CoreBlock {
        id: b.id,
        kind: b.kind,
        text: b.text,
        data: b.data,
        children: b.children,
    }
}

/// Load the document's folded yrs base, building it from the latest op-model
/// snapshot on first access (the lazy migration bridge). Errors if the document
/// has neither a yrs base nor a snapshot.
pub(crate) async fn ensure_base_tx(
    tx: &mut Transaction<'_, Postgres>,
    document_id: Uuid,
) -> ApiResult<YrsBase> {
    if let Some((state, state_vector, base_rid)) =
        sqlx::query_as::<_, (Vec<u8>, Vec<u8>, i64)>(
            "SELECT state, state_vector, base_rid FROM document_yrs_base WHERE document_id = $1",
        )
        .bind(document_id)
        .fetch_optional(&mut **tx)
        .await?
    {
        return Ok(YrsBase {
            state,
            state_vector,
            base_rid,
        });
    }

    // No yrs base yet — build one from the existing op-model snapshot.
    let snapshot = latest_snapshot_tx(tx, document_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let payload: DocumentSnapshotPayload = serde_json::from_value(snapshot.payload)
        .map_err(|e| ApiError::Internal(format!("bad snapshot payload: {e}")))?;
    let blocks: Vec<CoreBlock> = payload.blocks.into_iter().map(to_core_block).collect();
    let doc = MicaDoc::from_blocks(&payload.root_block_id, &blocks);
    let state = doc.encode_state();
    let state_vector = doc.state_vector();
    sqlx::query(
        "INSERT INTO document_yrs_base(document_id, state, state_vector, base_rid, updated_at)
         VALUES ($1, $2, $3, 0, now())
         ON CONFLICT (document_id) DO NOTHING",
    )
    .bind(document_id)
    .bind(&state)
    .bind(&state_vector)
    .execute(&mut **tx)
    .await?;
    Ok(YrsBase {
        state,
        state_vector,
        base_rid: 0,
    })
}

/// The document's current bootstrap base (None if never touched via yrs sync).
pub async fn document_base(db: &PgPool, document_id: Uuid) -> ApiResult<Option<YrsBase>> {
    let row = sqlx::query_as::<_, (Vec<u8>, Vec<u8>, i64)>(
        "SELECT state, state_vector, base_rid FROM document_yrs_base WHERE document_id = $1",
    )
    .bind(document_id)
    .fetch_optional(db)
    .await?;
    Ok(row.map(|(state, state_vector, base_rid)| YrsBase {
        state,
        state_vector,
        base_rid,
    }))
}

/// The document's bootstrap base, building it from the op-model snapshot on first
/// access. Use this for the WS bootstrap of a client opening the document.
pub async fn bootstrap_base(db: &PgPool, document_id: Uuid) -> ApiResult<YrsBase> {
    let mut tx = db.begin().await?;
    let base = ensure_base_tx(&mut tx, document_id).await?;
    tx.commit().await?;
    Ok(base)
}

/// P4-3 (state-vector fast reconciliation): the minimal yrs update that brings a
/// client at `client_sv` (its v1-encoded state vector) up to `base` — orders of
/// magnitude smaller than the full base for a client that already holds most of
/// the doc (seeded from its on-device store / reconnecting after stream pruning).
///
/// Returns `None` when the diff can't be computed (corrupt base, garbage sv) —
/// the caller falls back to the full base, so a bad sv can never break
/// bootstrap. Applying the diff is the same client operation as applying the
/// full base (both are yrs updates), which keeps the protocol change purely
/// additive: old clients never send an sv and old servers ignore it.
pub fn diff_from_base(base: &YrsBase, client_sv: &[u8]) -> Option<Vec<u8>> {
    let doc = guarded_from_update(&base.state).ok()?;
    doc.encode_diff(client_sv).ok()
}

/// Append a client's yrs `update` to the workspace stream and fold it into the
/// document base, returning the assigned `rid`. The update is validated by
/// applying it onto the current base first; a malformed/undecodable update is
/// rejected (§10 integrity gate — never persist unverifiable state).
pub async fn push_update(
    db: &PgPool,
    workspace_id: Uuid,
    document_id: Uuid,
    actor_id: Uuid,
    update: &[u8],
) -> ApiResult<i64> {
    let mut tx = db.begin().await?;
    let base = ensure_base_tx(&mut tx, document_id).await?;

    // Validate + fold: reconstruct the base, merge the update.
    let mut doc = guarded_from_update(&base.state)
        .map_err(|e| ApiError::Internal(format!("corrupt base state: {e}")))?;
    guarded_apply(&mut doc, update)
        .map_err(|_| ApiError::BadRequest("invalid yrs update".into()))?;

    let rid: i64 = sqlx::query_scalar(
        "INSERT INTO workspace_updates(workspace_id, document_id, actor_id, payload)
         VALUES ($1, $2, $3, $4) RETURNING rid",
    )
    .bind(workspace_id)
    .bind(document_id)
    .bind(actor_id)
    .bind(update)
    .fetch_one(&mut *tx)
    .await?;

    let state = doc.encode_state();
    let state_vector = doc.state_vector();
    sqlx::query(
        "INSERT INTO document_yrs_base(document_id, state, state_vector, base_rid, updated_at)
         VALUES ($1, $2, $3, $4, now())
         ON CONFLICT (document_id) DO UPDATE SET
             state = excluded.state, state_vector = excluded.state_vector,
             base_rid = excluded.base_rid, updated_at = now()",
    )
    .bind(document_id)
    .bind(&state)
    .bind(&state_vector)
    .bind(rid)
    .execute(&mut *tx)
    .await?;

    // Version history (docs/version-history-plan.md): archive the just-folded
    // full state as an AUTO snapshot — but only if none was taken in the last
    // VERSION_AUTO_INTERVAL, so a burst of edits converges to one version per
    // window (AFFiNE's min-interval, 10 min). `state` is already in hand, so
    // this is one indexed insert-or-nothing per push. Auto rows carry an
    // expires_at (VERSION_AUTO_RETENTION, 30 days); named rows are inserted
    // elsewhere with expires_at NULL and never expire.
    sqlx::query(
        "INSERT INTO document_yrs_versions (document_id, rid, state, expires_at)
         SELECT $1, $2, $3, now() + interval '30 days'
         WHERE NOT EXISTS (
             SELECT 1 FROM document_yrs_versions
             WHERE document_id = $1 AND label IS NULL
               AND created_at > now() - interval '10 minutes'
         )",
    )
    .bind(document_id)
    .bind(rid)
    .bind(&state)
    .execute(&mut *tx)
    .await?;

    // Periodically prune updates already folded into the base (keep a margin for
    // fast incremental catch-up). Bounds unbounded stream growth on big/old docs.
    if rid % STREAM_PRUNE_EVERY == 0 {
        sqlx::query("DELETE FROM workspace_updates WHERE document_id = $1 AND rid <= $2")
            .bind(document_id)
            .bind(rid - STREAM_KEEP_MARGIN)
            .execute(&mut *tx)
            .await?;
        // Same cadence, retention pass: drop expired auto versions. Named
        // versions (expires_at IS NULL) are kept forever.
        sqlx::query(
            "DELETE FROM document_yrs_versions
             WHERE document_id = $1 AND expires_at IS NOT NULL AND expires_at < now()",
        )
        .bind(document_id)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(rid)
}

/// Restore a document to a stored version's content, CRDT-safely. Re-applying
/// the old *state* as an update can't revert a CRDT (merge is a union), so this
/// re-derives the version's blocks as a NEW forward update on the current base
/// (via [`MicaDoc::set_blocks`]), then pushes it like any edit. Returns the
/// assigned `rid` and the update bytes to broadcast so open editors converge —
/// or `(current_rid, empty)` when the target already matches the live state.
pub async fn restore_yrs_version(
    db: &PgPool,
    workspace_id: Uuid,
    document_id: Uuid,
    actor_id: Uuid,
    version_state: &[u8],
) -> ApiResult<(i64, Vec<u8>)> {
    let base = bootstrap_base(db, document_id).await?;
    let mut doc = guarded_from_update(&base.state)
        .map_err(|e| ApiError::Internal(format!("corrupt base state: {e}")))?;
    let target = guarded_from_update(version_state)
        .map_err(|e| ApiError::BadRequest(format!("corrupt version state: {e}")))?;

    let sv = doc.state_vector();
    doc.set_blocks(&target.root_block_id(), &target.to_blocks());
    let update = doc
        .encode_diff(&sv)
        .map_err(|e| ApiError::Internal(format!("restore diff: {e}")))?;
    if update.is_empty() {
        return Ok((base.base_rid, Vec::new()));
    }
    let rid = push_update(db, workspace_id, document_id, actor_id, &update).await?;
    Ok((rid, update))
}

/// Catch a client up from `since_rid`: the incremental updates, or — when the
/// cursor fell behind the pruned window (the gap was folded into the base) — a
/// base to re-bootstrap from. Drives offline reconnect after stream pruning.
pub async fn catch_up_document(
    db: &PgPool,
    document_id: Uuid,
    since_rid: i64,
    limit: i64,
) -> ApiResult<CatchUp> {
    let min_rid: Option<i64> =
        sqlx::query_scalar("SELECT MIN(rid) FROM workspace_updates WHERE document_id = $1")
            .bind(document_id)
            .fetch_one(db)
            .await?;
    let base_rid: i64 =
        sqlx::query_scalar("SELECT base_rid FROM document_yrs_base WHERE document_id = $1")
            .bind(document_id)
            .fetch_optional(db)
            .await?
            .unwrap_or(0);

    // A gap exists when the earliest retained update is beyond `since_rid + 1`
    // (or the stream is empty) AND the base is ahead of the cursor — i.e. the
    // missing range was pruned. Fast-forward from the base in that case.
    let gap = match min_rid {
        Some(m) => since_rid + 1 < m && since_rid < base_rid,
        None => since_rid < base_rid,
    };
    if gap {
        return Ok(CatchUp::Rebootstrap(bootstrap_base(db, document_id).await?));
    }
    Ok(CatchUp::Updates(
        pull_document_updates(db, document_id, since_rid, limit).await?,
    ))
}

/// Everything in a workspace after `after_rid` (0 = from the start), ordered by
/// `rid`, capped at `limit`. Drives both cold bootstrap and offline reconnect.
pub async fn pull_workspace_updates(
    db: &PgPool,
    workspace_id: Uuid,
    after_rid: i64,
    limit: i64,
) -> ApiResult<Vec<StreamUpdate>> {
    let rows = sqlx::query_as::<_, StreamUpdate>(
        "SELECT rid, document_id, actor_id, payload
         FROM workspace_updates
         WHERE workspace_id = $1 AND rid > $2
         ORDER BY rid
         LIMIT $3",
    )
    .bind(workspace_id)
    .bind(after_rid)
    .bind(limit)
    .fetch_all(db)
    .await?;
    Ok(rows)
}

/// A single document's updates after `after_rid` (0 = from the start), ordered
/// by `rid`, capped at `limit`. Drives the per-document WS catch-up path (rooms
/// are per-document, so the client keeps a per-document cursor).
pub async fn pull_document_updates(
    db: &PgPool,
    document_id: Uuid,
    after_rid: i64,
    limit: i64,
) -> ApiResult<Vec<StreamUpdate>> {
    let rows = sqlx::query_as::<_, StreamUpdate>(
        "SELECT rid, document_id, actor_id, payload
         FROM workspace_updates
         WHERE document_id = $1 AND rid > $2
         ORDER BY rid
         LIMIT $3",
    )
    .bind(document_id)
    .bind(after_rid)
    .bind(limit)
    .fetch_all(db)
    .await?;
    Ok(rows)
}

/// The current high-water `rid` of a single document (0 if none yet).
pub async fn document_head(db: &PgPool, document_id: Uuid) -> ApiResult<i64> {
    let head: Option<i64> = sqlx::query_scalar(
        "SELECT MAX(rid) FROM workspace_updates WHERE document_id = $1",
    )
    .bind(document_id)
    .fetch_one(db)
    .await?;
    Ok(head.unwrap_or(0))
}

/// The current high-water `rid` of a workspace (0 if it has no updates yet).
pub async fn workspace_head(db: &PgPool, workspace_id: Uuid) -> ApiResult<i64> {
    let head: Option<i64> = sqlx::query_scalar(
        "SELECT MAX(rid) FROM workspace_updates WHERE workspace_id = $1",
    )
    .bind(workspace_id)
    .fetch_one(db)
    .await?;
    Ok(head.unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A malformed update that makes yrs PANIC must come back as a `DocError`,
    /// not unwind out of the server handler. Byte 9 flipped survives `decode_v1`
    /// and blows up during integration (the same class the desktop store hit in
    /// P0-0). With the guard removed, `guarded_from_update` panics and this test
    /// fails — the mutation-kills-test guarantee.
    #[test]
    fn a_panicking_update_becomes_an_error_not_a_panic() {
        // PIN the client id — `from_blocks` mints a RANDOM one, which makes the
        // encoded bytes (and therefore which yrs failure mode a flip triggers)
        // differ per run. That non-determinism made the sibling test in
        // mica-core's store.rs flaky in CI; don't repeat it here.
        let good = MicaDoc::from_blocks_with_client_id(
            "r",
            &[
                CoreBlock::new("r", "page").with_children(vec!["a".into()]),
                CoreBlock::new("a", "paragraph").with_text("hello world"),
            ],
            Some(1),
        )
        .encode_state();
        // Sanity: the healthy blob decodes fine through the guard.
        assert!(guarded_from_update(&good).is_ok());

        let mut bad = good.clone();
        bad[0] ^= 0xff;
        // The contract: a malformed update comes back as an Err instead of
        // unwinding out of the request handler. Both failure modes (decode_v1
        // rejecting it, or the guard converting an integration panic) surface as
        // `DocError`, and which one fires depends on the bytes — so assert the
        // invariant, not a mode. The catch_unwind mechanism itself is pinned
        // deterministically by mica-core's
        // `contain_yrs_panic_converts_a_panic_into_corrupt_doc`.
        assert!(
            guarded_from_update(&bad).is_err(),
            "a malformed blob must be contained as an error, not unwind"
        );

        let mut doc =
            MicaDoc::from_blocks_with_client_id("r", &[CoreBlock::new("r", "page")], Some(1));
        assert!(
            guarded_apply(&mut doc, &bad).is_err(),
            "guarded_apply must contain it as well"
        );
    }

    // The lazy-migration bridge: an op-model snapshot payload must reconstruct
    // into a yrs doc whose blocks/marks survive an encode→decode round trip (the
    // exact path `ensure_base_tx` takes when building a base for an existing doc).
    #[test]
    fn snapshot_payload_bridges_into_yrs_round_trip() {
        let root_id = "r".to_string();
        let payload = DocumentSnapshotPayload {
            schema_version: 1,
            root_block_id: root_id.clone(),
            blocks: vec![
                mica_markdown::Block {
                    id: "r".into(),
                    kind: "page".into(),
                    text: String::new(),
                    data: json!(null),
                    children: vec!["a".into(), "b".into()],
                },
                mica_markdown::Block {
                    id: "a".into(),
                    kind: "paragraph".into(),
                    text: "Hello".into(),
                    data: json!({ "marks": [{ "start": 0, "end": 5, "type": "bold" }] }),
                    children: vec![],
                },
                mica_markdown::Block {
                    id: "b".into(),
                    kind: "heading".into(),
                    text: "Title".into(),
                    data: json!({ "level": 1 }),
                    children: vec![],
                },
            ],
        };

        let blocks: Vec<CoreBlock> = payload.blocks.into_iter().map(to_core_block).collect();
        let doc = MicaDoc::from_blocks(&root_id, &blocks);
        // What the server persists + serves: encode → decode.
        let restored = MicaDoc::from_update(&doc.encode_state()).unwrap();
        let out = restored.to_blocks();

        let root = out.iter().find(|b| b.id == "r").unwrap();
        assert_eq!(root.children, vec!["a", "b"]);
        let a = out.iter().find(|b| b.id == "a").unwrap();
        assert_eq!(a.text, "Hello");
        assert_eq!(a.data["marks"][0]["type"], "bold");
        let b = out.iter().find(|b| b.id == "b").unwrap();
        assert_eq!(b.kind, "heading");
        assert_eq!(b.data["level"], 1);
    }

    // A peer's update applied onto the base converges the same as the local
    // merge — proving the server-side fold in `push_update` is sound.
    #[test]
    fn server_fold_matches_client_merge() {
        let base = MicaDoc::from_blocks(
            "r",
            &[
                CoreBlock::new("r", "page").with_children(vec!["a".into()]),
                CoreBlock::new("a", "paragraph").with_text("Hi"),
            ],
        );
        let state = base.encode_state();

        // Client edits its replica and ships the diff.
        let mut client = MicaDoc::from_update(&state).unwrap();
        let sv = client.state_vector();
        client.text_insert("a", 2, " there");
        let update = client.encode_diff(&sv).unwrap();

        // Server folds the update onto its base copy.
        let mut server = MicaDoc::from_update(&state).unwrap();
        server.apply_update(&update).unwrap();

        assert_eq!(server.to_blocks(), client.to_blocks());
        assert_eq!(
            server.to_blocks().into_iter().find(|b| b.id == "a").unwrap().text,
            "Hi there"
        );
    }

    // P4-3: a warm client's sv yields a diff that (a) converges it to the base
    // and (b) is much smaller than the full base; a garbage sv falls back (None).
    #[test]
    fn diff_from_base_converges_and_shrinks() {
        // A doc with enough content that "full base ≫ one-edit diff" is obvious.
        let mut server = MicaDoc::from_blocks(
            "r",
            &[
                CoreBlock::new("r", "page").with_children(vec!["a".into()]),
                CoreBlock::new("a", "paragraph")
                    .with_text("lorem ipsum dolor sit amet ".repeat(50)),
            ],
        );
        // Client seeded from this state (its on-device mirror).
        let mut client = MicaDoc::from_update(&server.encode_state()).unwrap();
        let client_sv = client.state_vector();
        // The doc moves on server-side (a peer edit folded into the base).
        server.text_insert("a", 0, "peer: ");
        let base = YrsBase {
            state: server.encode_state(),
            state_vector: server.state_vector(),
            base_rid: 7,
        };

        let diff = diff_from_base(&base, &client_sv).expect("valid sv → diff");
        assert!(
            diff.len() * 10 < base.state.len(),
            "diff ({}) should be far smaller than the full base ({})",
            diff.len(),
            base.state.len()
        );
        client.apply_update(&diff).unwrap();
        assert_eq!(client.to_blocks(), server.to_blocks());

        // Garbage sv → None (caller falls back to the full base).
        assert!(diff_from_base(&base, b"not a state vector").is_none());
    }
}
