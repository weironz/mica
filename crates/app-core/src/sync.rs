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

use mica_core::{Block as CoreBlock, MicaDoc};
use mica_infra::{ApiError, ApiResult};
use mica_markdown::DocumentSnapshotPayload;
use serde::Serialize;
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::store::latest_snapshot_tx;

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

fn to_core_block(b: mica_markdown::Block) -> CoreBlock {
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
    let mut doc = MicaDoc::from_update(&base.state)
        .map_err(|e| ApiError::Internal(format!("corrupt base state: {e}")))?;
    doc.apply_update(update)
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

    tx.commit().await?;
    Ok(rid)
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
}
