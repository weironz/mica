
use axum::{
  Json,
  extract::{Path, State},
  http::HeaderMap,
};

use mica_app_core::AppState;
use mica_infra::{ApiError, ApiResult};
use sqlx::PgPool;
use uuid::Uuid;

use crate::routes::auth::user_id_from_headers;
use super::*;

pub async fn list_trash(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
) -> ApiResult<Json<ViewListResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;

  let views = fetch_deleted_workspace_views(&state.db, workspace_id).await?;

  Ok(Json(ViewListResponse { views }))
}

pub async fn restore_view(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, view_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ViewListResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;

  // The batch restore of one id: one statement, one rule, one transaction
  // (this used to be two separate statements, with the orphan lift outside
  // any transaction). Nothing restored — not in the bin, or not here — is 404.
  if restore_views_batch(&state.db, workspace_id, &[view_id]).await?.is_empty() {
    return Err(ApiError::NotFound);
  }

  let views = fetch_workspace_views(&state.db, workspace_id).await?;

  Ok(Json(ViewListResponse { views }))
}

/// Permanently remove one page + its subtree AND the documents they back (so no
/// orphaned content/shares survive "永久删除") — but only from the BIN.
///
/// This used to run its own statement whose seed had no `is_deleted` filter, so
/// one DELETE from any editor erased a LIVE page with no recoverable step
/// (2026-10-21 review P1-8; the batch purge never had that hole). It is now the
/// batch purge of one id: one statement, one rule. Not in the bin — or not in
/// this workspace — is a 404 either way.
pub(super) async fn purge_one(db: &PgPool, workspace_id: Uuid, view_id: Uuid) -> ApiResult<()> {
  if purge_views_batch(db, workspace_id, &[view_id]).await?.is_empty() {
    return Err(ApiError::NotFound);
  }
  Ok(())
}

pub async fn purge_view(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, view_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ViewListResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;

  purge_one(&state.db, workspace_id, view_id).await?;
  let views = fetch_deleted_workspace_views(&state.db, workspace_id).await?;

  Ok(Json(ViewListResponse { views }))
}

/// Empty the whole recycle bin: every trashed view in the workspace, plus the
/// documents they back.
///
/// One statement rather than a loop over subtree roots. No recursive CTE is
/// needed either: trashing a folder marks its entire subtree `is_deleted`
/// (`delete_view` cascades), so "every row with is_deleted = true" already *is*
/// the closure — and doing it in a single statement means a caller cannot end up
/// with half a bin emptied.
///
/// Idempotent by nature: an already-empty bin returns zeros rather than 404. The
/// single-view [`purge_view`] does 404 on a miss, but there the id came from a row
/// the user clicked; here the request is "make it empty", which an empty bin
/// already satisfies.
pub async fn purge_workspace_trash(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
) -> ApiResult<Json<PurgeTrashResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;

  let (views_deleted, documents_deleted) =
    empty_workspace_trash(&state.db, workspace_id).await?;

  Ok(Json(PurgeTrashResponse {
    views_deleted,
    documents_deleted,
  }))
}

/// Delete every trashed view in the workspace and the documents they back.
/// Returns `(views_deleted, documents_deleted)`.
///
/// Split out of the handler so a test can reach it: the handler needs auth
/// headers, which a DB-level test has no way to construct.
pub(super) async fn empty_workspace_trash(db: &PgPool, workspace_id: Uuid) -> ApiResult<(i64, i64)> {
  let row = sqlx::query_as::<_, (i64, i64)>(
    r#"
      WITH deleted_views AS (
        DELETE FROM views
        WHERE workspace_id = $1 AND is_deleted = true
        RETURNING object_id, object_type
      ),
      purged_docs AS (
        DELETE FROM documents
        WHERE id IN (
          SELECT object_id FROM deleted_views WHERE object_type::text = 'document'
        )
        RETURNING id
      )
      SELECT
        (SELECT count(*) FROM deleted_views)::bigint,
        (SELECT count(*) FROM purged_docs)::bigint
    "#,
  )
  .bind(workspace_id)
  .fetch_one(db)
  .await?;
  Ok((row.0, row.1))
}


batch_view_handler! {
  /// `POST /api/workspaces/{workspace_id}/views/batch-trash`
  ///
  /// Soft-delete many pages (each with its whole subtree) in ONE statement. Same
  /// recursive CTE as [delete_view], seeded from a set instead of a single id, so
  /// 300 pages cost one query rather than 300 round trips.
  pub async fn batch_trash_views, trash_views_batch
}


/// Soft-delete [view_ids] and everything under them. Returns the ids it changed.
///
/// Split out of the handler for the same reason [purge_views_batch] is: the
/// handler needs auth headers a DB-level test cannot construct. Until 2026-08-27
/// this statement lived inline and had no test at all — the guarantee was "the
/// SQL parses and the route table has no conflict", not "300 ids in, the right
/// rows out".
///
/// `is_deleted = false` is in the WHERE, not the seed: a row already in the bin
/// must not be re-stamped (that would move its `updated_at` and make it look
/// freshly deleted) and must come back in `skipped` so the caller is told it did
/// less than it asked.
pub(super) async fn trash_views_batch(
  db: &PgPool,
  workspace_id: Uuid,
  view_ids: &[Uuid],
) -> ApiResult<Vec<Uuid>> {
  Ok(
    sqlx::query_scalar::<_, Uuid>(
      r#"
        WITH RECURSIVE subtree AS (
          SELECT id FROM views WHERE id = ANY($1) AND workspace_id = $2
          UNION
          SELECT v.id FROM views v JOIN subtree s ON v.parent_view_id = s.id
        )
        UPDATE views
        SET is_deleted = true, deleted_at = now(), updated_at = now()
        WHERE id IN (SELECT id FROM subtree)
          AND workspace_id = $2
          AND is_deleted = false
        RETURNING id
      "#,
    )
    .bind(view_ids)
    .bind(workspace_id)
    .fetch_all(db)
    .await?,
  )
}

batch_view_handler! {
  /// `POST /api/workspaces/{workspace_id}/views/batch-restore`
  ///
  /// Bring many pages back out of the recycle bin, each with the subtree that was
  /// deleted with it. The mirror of [batch_trash_views], and the reason a bulk
  /// delete is safe to attempt: it is one call to undo.
  pub async fn batch_restore_views, restore_views_batch
}


/// Bring [view_ids] and their trashed subtrees back. Returns the ids it changed.
///
/// The exact mirror of [trash_views_batch] — same CTE, the two `is_deleted`
/// booleans flipped. Extracted for the same reason, and tested beside it so the
/// pair cannot drift apart silently.
pub(super) async fn restore_views_batch(
  db: &PgPool,
  workspace_id: Uuid,
  view_ids: &[Uuid],
) -> ApiResult<Vec<Uuid>> {
  let mut tx = db.begin().await?;
  // Only what went to the bin WITH each requested page: the walk carries the
  // seed's `deleted_at` and descends only into rows trashed in that same
  // action. A child the user had trashed on its own before deleting its folder
  // stays in the bin — restoring the folder used to resurrect it. Rows from
  // before the stamp existed are all NULL and so still restore as one batch.
  //
  // A live seed carries no stamp of its own, so beneath it the walk descends
  // only through other live rows and every trashed descendant restores — the
  // long-standing "restore a live folder brings its trashed pages back".
  let restored = sqlx::query_scalar::<_, Uuid>(
    r#"
      WITH RECURSIVE subtree AS (
        SELECT id, is_deleted, deleted_at AS stamp
        FROM views WHERE id = ANY($1) AND workspace_id = $2
        UNION
        SELECT v.id, v.is_deleted, CASE WHEN s.is_deleted THEN s.stamp ELSE v.deleted_at END
        FROM views v JOIN subtree s ON v.parent_view_id = s.id
        WHERE NOT s.is_deleted OR (v.is_deleted AND v.deleted_at IS NOT DISTINCT FROM s.stamp)
      )
      UPDATE views
      SET is_deleted = false, deleted_at = NULL, updated_at = now()
      WHERE id IN (SELECT id FROM subtree)
        AND workspace_id = $2
        AND is_deleted = true
      RETURNING id
    "#,
  )
  .bind(view_ids)
  .bind(workspace_id)
  .fetch_all(&mut *tx)
  .await?;

  // The lift single-item restore has always done, which this batch twin
  // lacked: restoring a page whose folder is STILL in the trash left it live
  // under a deleted parent — invisible in the tree, and then deleted for real
  // by `parent_view_id ON DELETE CASCADE` when the trash was emptied. Same
  // transaction, so nothing can observe the orphan in between.
  sqlx::query(
    r#"
      UPDATE views
      SET parent_view_id = NULL, updated_at = now()
      WHERE id = ANY($1) AND workspace_id = $2 AND parent_view_id IS NOT NULL
        AND parent_view_id NOT IN (
          SELECT id FROM views WHERE workspace_id = $2 AND is_deleted = false
        )
    "#,
  )
  .bind(&restored)
  .bind(workspace_id)
  .execute(&mut *tx)
  .await?;
  tx.commit().await?;
  Ok(restored)
}

batch_view_handler! {
  /// `POST /api/workspaces/{workspace_id}/views/batch-purge`
  ///
  /// PERMANENTLY delete many trashed pages (each with its subtree) in ONE
  /// statement — the middle rung that was missing between [purge_view] (one at a
  /// time) and [purge_workspace_trash] (all or nothing). Clearing 200 specific
  /// items meant 200 round trips, which is exactly the loop the batch endpoints
  /// exist to remove.
  ///
  /// **Only touches views that are ALREADY in the recycle bin**, and that is a
  /// deliberate difference from [purge_view], which has no such filter and will
  /// permanently destroy a LIVE page. Reproducing that here would mean one call
  /// could erase 300 pages that were never deleted, with no recoverable step
  /// anywhere — a new capability, not a batching of an old one. Anything not
  /// trashed comes back in `skipped`, so the caller is told what did not happen
  /// rather than quietly getting less than it asked for.
  ///
  /// The documents behind those views go
  /// too, taking their yrs base, snapshots, versions and public share tokens with
  /// them. Blobs are left to `blob_gc` — they are content-addressed and may still
  /// be referenced by a page that is staying.
  pub async fn batch_purge_views, purge_views_batch
}


/// Permanently delete the trashed views named by [view_ids], each with its
/// subtree, and the documents they back. Returns every view id that went.
///
/// Split out of the handler for the same reason [empty_workspace_trash] is: the
/// handler needs auth headers a DB-level test cannot construct, and an
/// irreversible statement is the last one that should go untested.
pub(super) async fn purge_views_batch(
  db: &PgPool,
  workspace_id: Uuid,
  view_ids: &[Uuid],
) -> ApiResult<Vec<Uuid>> {
  // `purged_docs` is never read by the final SELECT, and that is fine: a
  // data-modifying CTE runs exactly once whether or not anything references it.
  //
  // `is_deleted = true` is on the SEED only. The recursive arm must not repeat
  // it: trashing a folder cascades the flag to its subtree, so the descendants
  // are already marked — but if a future change stops cascading, filtering here
  // too would silently start leaving children behind instead of failing loudly.
  let touched = sqlx::query_scalar::<_, Uuid>(
    r#"
      WITH RECURSIVE subtree AS (
        SELECT id, object_id, object_type FROM views
        WHERE id = ANY($1) AND workspace_id = $2 AND is_deleted = true
        UNION
        SELECT v.id, v.object_id, v.object_type
        FROM views v JOIN subtree s ON v.parent_view_id = s.id
      ),
      deleted_views AS (
        DELETE FROM views
        WHERE id IN (SELECT id FROM subtree) AND workspace_id = $2
        RETURNING id, object_id, object_type
      ),
      purged_docs AS (
        DELETE FROM documents
        WHERE id IN (
          SELECT object_id FROM deleted_views WHERE object_type::text = 'document'
        )
        RETURNING id
      )
      SELECT id FROM deleted_views
    "#,
  )
  .bind(view_ids)
  .bind(workspace_id)
  .fetch_all(db)
  .await?;
  Ok(touched)
}

