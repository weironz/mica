
use axum::{
  Json,
  extract::{Path, State},
  http::HeaderMap,
};
use mica_app_core::{
  AppState,
  documents::DocumentSnapshotPayload,
  store::{self, DocumentRecord},
};
use mica_infra::{ApiError, ApiResult};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use futures_util::{StreamExt, TryStreamExt};
use uuid::Uuid;

use crate::routes::auth::user_id_from_headers;
use super::*;

pub async fn move_view(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, view_id)): Path<(Uuid, Uuid)>,
  Json(payload): Json<MoveViewRequest>,
) -> ApiResult<Json<ViewResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;
  let position = normalize_position(payload.position)?;
  reparent_tx(&state.db, workspace_id, &[view_id], payload.parent_view_id, &[position]).await?;
  let view = sqlx::query_as::<_, View>(
    r#"
      SELECT
        id,
        workspace_id,
        parent_view_id,
        object_id,
        object_type::text AS object_type,
        name,
        icon,
        position,
        is_deleted,
        created_by,
        created_at,
        updated_at
      FROM views
      WHERE id = $1 AND workspace_id = $2
    "#,
  )
  .bind(view_id)
  .bind(workspace_id)
  .fetch_optional(&state.db)
  .await?
  .ok_or(ApiError::NotFound)?;

  Ok(Json(ViewResponse { view }))
}

/// `POST /api/workspaces/{workspace_id}/views/reorder`
///
/// Reorder a parent's children in ONE atomic call: every id in
/// `ordered_view_ids` is set as a child of `parent_view_id` (null = top level)
/// and given an evenly-spaced position in the given order. This is what a "sort
/// this folder" operation needs — the per-view `move` endpoint would take one
/// request per sibling and could interleave a failure. Positions are 10-spaced,
/// zero-padded to a fixed width so they sort lexicographically like the rest.
pub async fn reorder_views(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
  Json(payload): Json<ReorderRequest>,
) -> ApiResult<Json<ReorderResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;

  // Same limits as every batch endpoint: non-empty, no duplicates, and at
  // most MAX_BATCH_VIEWS — this one had none, and validated each id with three
  // queries of its own.
  check_batch_ids(&payload.ordered_view_ids)?;
  move_views_tx(
    &state.db,
    workspace_id,
    &payload.ordered_view_ids,
    payload.parent_view_id,
  )
  .await?;

  Ok(Json(ReorderResponse {
    reordered: payload.ordered_view_ids.len(),
  }))
}


/// `POST /api/workspaces/{workspace_id}/views/batch-move`
///
/// Re-parent many views at once. Unlike trash/restore this cannot be one
/// statement: the folder-only container rule and the cycle check are per view,
/// so every id is validated against the destination BEFORE anything is written
/// — the same order [reorder_views] uses, and for the same reason. The writes
/// then run in one transaction, so a view deleted underneath us rolls the whole
/// move back instead of leaving the tree half-reorganised.
pub async fn batch_move_views(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
  Json(payload): Json<BatchMoveRequest>,
) -> ApiResult<Json<BatchViewsResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;
  check_batch_ids(&payload.view_ids)?;

  move_views_tx(
    &state.db,
    workspace_id,
    &payload.view_ids,
    payload.parent_view_id,
  )
  .await?;

  Ok(Json(BatchViewsResponse {
    affected: payload.view_ids.len(),
    skipped: Vec::new(),
  }))
}

/// Re-parent [view_ids] under [parent_view_id], in the order given, in ONE
/// transaction.
///
/// Positions are `(index + 1) * 10`, so the caller's order IS the sibling order
/// at the destination.
///
/// A view that no longer matches (deleted, or moved to another workspace, under
/// us) makes the WHOLE move fail with [ApiError::Conflict] — the `?` returns
/// before `commit`, so the transaction is dropped and every earlier row in this
/// batch rolls back. That is the difference from trash/restore, which report
/// what they missed in `skipped`: a half-applied reorganisation has no honest
/// way to be reported, because the ids the caller sent no longer describe the
/// tree it is looking at.
///
/// Split out of the handler so a DB test can reach it — the handler needs auth
/// headers a database-level test cannot construct.
pub(super) async fn move_views_tx(
  db: &PgPool,
  workspace_id: Uuid,
  view_ids: &[Uuid],
  parent_view_id: Option<Uuid>,
) -> ApiResult<()> {
  let positions: Vec<String> =
    (1..=view_ids.len()).map(|i| format!("{:010}", i * 10)).collect();
  reparent_tx(db, workspace_id, view_ids, parent_view_id, &positions).await
}

/// Put [view_ids] under [parent_view_id] at [positions] — validation and
/// write in ONE transaction, under a per-workspace lock.
///
/// The cycle check used to run before, and outside, the write: two concurrent
/// moves (A under B, B under A) could each pass it against a tree that did not
/// yet contain the other's change, and together form a cycle — which the
/// recursive walks then followed forever. Moves are the only writes that can
/// close a loop (creates add leaves, restores lift to the top), so they
/// serialise on an advisory lock for the workspace and check what is
/// committed after it.
///
/// Fixed query count whatever the batch size: one existence check, one parent
/// check, one descendant walk seeded with every id, one UPDATE — it used to be
/// three queries per id.
pub(super) async fn reparent_tx(
  db: &PgPool,
  workspace_id: Uuid,
  view_ids: &[Uuid],
  parent_view_id: Option<Uuid>,
  positions: &[String],
) -> ApiResult<()> {
  let mut tx = db.begin().await?;
  sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('mica.views.tree:' || $1::text, 0))")
    .bind(workspace_id)
    .execute(&mut *tx)
    .await?;

  let live: Vec<Uuid> = sqlx::query_scalar(
    "SELECT id FROM views WHERE id = ANY($1) AND workspace_id = $2 AND is_deleted = false",
  )
  .bind(view_ids)
  .bind(workspace_id)
  .fetch_all(&mut *tx)
  .await?;
  if let Some(missing) = view_ids.iter().find(|id| !live.contains(id)) {
    let exists: bool =
      sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM views WHERE id = $1 AND workspace_id = $2)")
        .bind(missing)
        .bind(workspace_id)
        .fetch_one(&mut *tx)
        .await?;
    // Not here at all is a bad id; here but in the bin means the tree changed
    // since the caller looked — refetch, as the rest of the move errors say.
    return Err(if exists {
      ApiError::Conflict(format!(
        "view {missing} was modified concurrently during the move; refetch and retry"
      ))
    } else {
      ApiError::NotFound
    });
  }

  if let Some(parent) = parent_view_id {
    if view_ids.contains(&parent) {
      return Err(ApiError::BadRequest(
        "parent_view_id cannot be the same view".to_string(),
      ));
    }
    ensure_parent_accepts_children(&mut *tx, workspace_id, parent).await?;
    let would_cycle: bool = sqlx::query_scalar(
      r#"
        WITH RECURSIVE descendants (id) AS (
          SELECT id FROM views
          WHERE id = ANY($1) AND workspace_id = $2 AND is_deleted = false
          UNION
          SELECT v.id FROM views v
          JOIN descendants d ON v.parent_view_id = d.id
          WHERE v.workspace_id = $2 AND v.is_deleted = false
        )
        SELECT EXISTS (SELECT 1 FROM descendants WHERE id = $3)
      "#,
    )
    .bind(view_ids)
    .bind(workspace_id)
    .bind(parent)
    .fetch_one(&mut *tx)
    .await?;
    if would_cycle {
      return Err(ApiError::BadRequest(
        "parent_view_id cannot be a descendant view".to_string(),
      ));
    }
  }

  let affected = sqlx::query(
    r#"
      UPDATE views
      SET parent_view_id = $1, position = u.position, updated_at = now()
      FROM unnest($2::uuid[], $3::text[]) AS u(id, position)
      WHERE views.id = u.id AND views.workspace_id = $4 AND views.is_deleted = false
    "#,
  )
  .bind(parent_view_id)
  .bind(view_ids)
  .bind(positions)
  .bind(workspace_id)
  .execute(&mut *tx)
  .await?
  .rows_affected();
  // Trash does not take the tree lock: a view binned between the check and
  // here leaves the UPDATE short. Dropping `tx` rolls every row back.
  if affected != view_ids.len() as u64 {
    return Err(ApiError::Conflict(
      "a view was modified concurrently during the move; refetch and retry".to_string(),
    ));
  }
  tx.commit().await?;
  Ok(())
}


// transaction lets us beat. See docs/features/cross-workspace-move.md.

#[derive(Debug, Deserialize)]
pub struct TransferRequest {
  dest_workspace_id: Uuid,
  /// Parent folder in the destination (null = destination root).
  #[serde(default)]
  parent_view_id: Option<Uuid>,
  /// true = move (soft-delete the source after copying); false = copy (keep source).
  #[serde(default)]
  remove_source: bool,
  /// Report what WOULD happen (counts + dangling links) without mutating anything.
  #[serde(default)]
  dry_run: bool,
}

/// `POST /api/workspaces/{workspace_id}/views/batch-transfer` — same operation,
/// N roots, ONE transaction.
///
/// This exists because the alternative was N sequential single-view transfers,
/// and that is not the same operation performed more times: a failure part-way
/// through leaves some of the selection moved and the rest not, with no record
/// of where it stopped. Moving five pages is one thing the user asked for, so it
/// either happens or it does not (user, 2026-08-27).
#[derive(Debug, Deserialize)]
pub struct BatchTransferRequest {
  dest_workspace_id: Uuid,
  #[serde(default)]
  parent_view_id: Option<Uuid>,
  #[serde(default)]
  remove_source: bool,
  #[serde(default)]
  dry_run: bool,
  /// The selected roots, in the order the caller selected them. A root nested
  /// inside another is dropped rather than refused (see [`independent_roots`]).
  view_ids: Vec<Uuid>,
}

/// What both transfer routes reduce to: N roots going to one destination.
struct TransferPlan {
  roots: Vec<Uuid>,
  dest_workspace_id: Uuid,
  parent_view_id: Option<Uuid>,
  remove_source: bool,
  dry_run: bool,
}

#[derive(Debug, Serialize)]
pub struct DanglingLink {
  /// Name of the moved document whose link now dangles.
  document: String,
  /// The `mica://page/<id>` target that stays in the source workspace.
  target_view_id: Uuid,
}

#[derive(Debug, Serialize)]
pub struct TransferResponse {
  /// The first root's new id. Kept for the single-view route, whose callers
  /// (Dart client, CLI, MCP) read exactly this field.
  new_root_view_id: Option<Uuid>,
  /// Every root's new id, in the order the roots were given. Empty on a dry run.
  new_root_view_ids: Vec<Uuid>,
  documents: usize,
  folders: usize,
  images: usize,
  dangling_links: Vec<DanglingLink>,
  removed_source: bool,
  dry_run: bool,
}

#[derive(Debug, FromRow)]
pub(super) struct TransferRow {
  pub(super) id: Uuid,
  parent_view_id: Option<Uuid>,
  object_id: Uuid,
  object_type: String,
  name: String,
  position: String,
}

/// The LIVE subtrees under `roots` in `workspace_id` — what a transfer copies.
/// One query for both reads of it: the enumeration up front, and the re-check
/// inside the move's transaction ([remove_moved_source]).
pub(super) async fn live_subtree<'e>(
  db: impl sqlx::PgExecutor<'e>,
  roots: &[Uuid],
  workspace_id: Uuid,
) -> ApiResult<Vec<TransferRow>> {
  Ok(
    sqlx::query_as::<_, TransferRow>(
      r#"
        WITH RECURSIVE subtree AS (
          SELECT id, parent_view_id, object_id, object_type::text AS object_type, name, position
          FROM views
          WHERE id = ANY($1) AND workspace_id = $2 AND is_deleted = false
          UNION
          SELECT v.id, v.parent_view_id, v.object_id, v.object_type::text, v.name, v.position
          FROM views v JOIN subtree s ON v.parent_view_id = s.id
          WHERE v.is_deleted = false
        )
        SELECT id, parent_view_id, object_id, object_type, name, position FROM subtree
      "#,
    )
    .bind(roots)
    .bind(workspace_id)
    .fetch_all(db)
    .await?,
  )
}

/// Soft-delete a move's source subtrees, inside the move's transaction —
/// exactly the rows that were copied (`copied`), or nothing.
///
/// The copy was built from an enumeration taken before the blob copies, which
/// can take minutes. This used to re-walk the subtree here and bin whatever it
/// found, so a page a collaborator created in between was soft-deleted at the
/// source and never copied — while the response said "moved" (C1). Now:
///
/// 1. Lock the live subtree rows (`FOR UPDATE`). From here on nobody can hang
///    a page under them: the parent trigger's `FOR SHARE` and the FK's
///    `KEY SHARE` both wait for this transaction, and then find the folder in
///    the bin (migration 0033). A writer already in flight is waited for here.
/// 2. Re-enumerate in a NEW statement, so it sees everything committed up to
///    the lock — and compare with what was copied. Any difference (a page
///    added, moved out, trashed) is a 409, and the caller's `?` drops the
///    transaction: the destination copy rolls back with it.
/// 3. Bin exactly the copied set.
pub(super) async fn remove_moved_source(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  src_workspace_id: Uuid,
  roots: &[Uuid],
  copied: &std::collections::HashSet<Uuid>,
) -> ApiResult<()> {
  sqlx::query(
    r#"
      WITH RECURSIVE subtree AS (
        SELECT id FROM views WHERE id = ANY($1) AND workspace_id = $2 AND is_deleted = false
        UNION
        SELECT v.id FROM views v JOIN subtree s ON v.parent_view_id = s.id
        WHERE v.is_deleted = false
      )
      SELECT v.id FROM views v WHERE v.id IN (SELECT id FROM subtree) FOR UPDATE OF v
    "#,
  )
  .bind(roots)
  .bind(src_workspace_id)
  .execute(&mut **tx)
  .await?;

  let now: std::collections::HashSet<Uuid> = live_subtree(&mut **tx, roots, src_workspace_id)
    .await?
    .into_iter()
    .map(|row| row.id)
    .collect();
  if &now != copied {
    return Err(ApiError::Conflict(format!(
      "the pages being moved changed while they were copied ({} now, {} copied); \
       nothing was moved — refresh and try again",
      now.len(),
      copied.len()
    )));
  }

  let ids: Vec<Uuid> = copied.iter().copied().collect();
  sqlx::query(
    "UPDATE views SET is_deleted = true, deleted_at = now(), updated_at = now() WHERE id = ANY($1)",
  )
    .bind(&ids)
    .execute(&mut **tx)
    .await?;
  Ok(())
}

/// The roots of a batch transfer, with anything already inside another root
/// dropped.
///
/// A multi-select can hold a folder AND a page inside it — nothing stops the
/// user selecting both, and from the sidebar it looks entirely reasonable.
/// Transferring both would copy that page TWICE: once as part of the folder's
/// subtree, once on its own. The destination would show two of it, and on a move
/// the second copy's source is already soft-deleted by the first.
///
/// [ancestor_pairs] is `(root, one of its ancestors)`, so a root is redundant
/// exactly when one of its ancestors is also being transferred. Kept as pairs
/// rather than a map so the rule is a list operation and the test can state the
/// tree literally.
///
/// Input order is preserved: it is the order the user selected in, and the
/// destination's sibling order follows it.
pub(super) fn independent_roots(roots: &[Uuid], ancestor_pairs: &[(Uuid, Uuid)]) -> Vec<Uuid> {
  let wanted: std::collections::HashSet<Uuid> = roots.iter().copied().collect();
  let mut kept = Vec::new();
  let mut seen = std::collections::HashSet::new();
  for &root in roots {
    // The same id twice is not an error worth refusing over — it is a click
    // that landed twice — but it must not become two copies either.
    if !seen.insert(root) {
      continue;
    }
    let nested = ancestor_pairs
      .iter()
      .any(|&(child, ancestor)| child == root && wanted.contains(&ancestor));
    if !nested {
      kept.push(root);
    }
  }
  kept
}

/// `(root, ancestor)` for every live ancestor of every root, within
/// [workspace_id] — the input [`independent_roots`] needs.
///
/// Walks UP rather than down: a batch's roots are few and shallow, while their
/// subtrees can be the whole workspace. Enumerating downward to answer "is one
/// of these inside another" would read every page to decide something about a
/// handful of them.
///
/// Split out of the handler so a DB test can reach it — the handler needs auth
/// headers, which a database-level test has no way to construct. Same reason
/// `empty_workspace_trash` and `purge_views_batch` live outside theirs.
///
/// Scoped to live rows in this workspace: an id the caller cannot actually
/// transfer contributes no pairs, is therefore kept as "independent", and is
/// then rejected by the caller's membership check with a real error — rather
/// than quietly disqualifying one of its siblings.
pub(super) async fn ancestor_pairs_of_roots(
  db: &PgPool,
  workspace_id: Uuid,
  roots: &[Uuid],
) -> ApiResult<Vec<(Uuid, Uuid)>> {
  if roots.is_empty() {
    return Ok(Vec::new());
  }
  Ok(
    sqlx::query_as::<_, (Uuid, Uuid)>(
      r#"
        WITH RECURSIVE up AS (
          SELECT id AS root, parent_view_id AS ancestor
          FROM views
          WHERE id = ANY($1) AND workspace_id = $2 AND is_deleted = false
          UNION
          SELECT u.root, v.parent_view_id
          FROM up u JOIN views v ON v.id = u.ancestor
          WHERE v.is_deleted = false
        )
        SELECT root, ancestor FROM up WHERE ancestor IS NOT NULL
      "#,
    )
    .bind(roots)
    .bind(workspace_id)
    .fetch_all(db)
    .await?,
  )
}

/// `POST /api/workspaces/{workspace_id}/views/{view_id}/transfer`
pub async fn transfer_view(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((src_workspace_id, view_id)): Path<(Uuid, Uuid)>,
  Json(request): Json<TransferRequest>,
) -> ApiResult<Json<TransferResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  transfer_roots(
    &state,
    user_id,
    src_workspace_id,
    TransferPlan {
      roots: vec![view_id],
      dest_workspace_id: request.dest_workspace_id,
      parent_view_id: request.parent_view_id,
      remove_source: request.remove_source,
      dry_run: request.dry_run,
    },
  )
  .await
  .map(Json)
}

/// `POST /api/workspaces/{workspace_id}/views/batch-transfer`
pub async fn batch_transfer_views(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(src_workspace_id): Path<Uuid>,
  Json(request): Json<BatchTransferRequest>,
) -> ApiResult<Json<TransferResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  // Same ceiling as the other batch routes, and for the same reason — except
  // here it also bounds how long ONE transaction holds its locks.
  //
  // Deliberately NOT `check_batch_ids`: that rejects a repeated id, and a repeat
  // here is a click that landed twice on a selection, which `independent_roots`
  // already collapses along with the nesting case it cannot be separated from.
  if request.view_ids.is_empty() {
    return Err(ApiError::BadRequest("view_ids must not be empty".to_string()));
  }
  if request.view_ids.len() > MAX_BATCH_VIEWS {
    return Err(ApiError::BadRequest(format!(
      "at most {MAX_BATCH_VIEWS} view_ids per request, got {}",
      request.view_ids.len()
    )));
  }
  transfer_roots(
    &state,
    user_id,
    src_workspace_id,
    TransferPlan {
      roots: request.view_ids,
      dest_workspace_id: request.dest_workspace_id,
      parent_view_id: request.parent_view_id,
      remove_source: request.remove_source,
      dry_run: request.dry_run,
    },
  )
  .await
  .map(Json)
}

/// Copy N subtrees into another workspace, optionally soft-deleting the sources.
///
/// One root or twenty is the same code path — the phases below were already a
/// loop over rows, and `topo_order_subtree` already anchors every row whose
/// parent is outside the set, so a forest needs no special case.
///
/// What "atomic" does and does not mean here, stated plainly because the gap is
/// real: phase 4 is ONE transaction, so the destination tree and the source
/// soft-delete either both land or neither does — no half-moved selection. The
/// blob copies in phase 3 are deliberately outside it (see the phase comment),
/// so a failure after them can leave orphan BYTES in the destination, which its
/// GC reclaims. That is the same guarantee a single transfer has always had.
/// How many documents / blobs a transfer works on at once.
const TRANSFER_CONCURRENCY: usize = 8;

/// Copy one source file into the destination workspace; `(src, dest)` ids, or
/// `None` for a reference to a file that no longer exists.
async fn copy_blob(
  state: &AppState,
  storage: &mica_infra::S3Config,
  http: &reqwest::Client,
  user_id: Uuid,
  src_workspace_id: Uuid,
  dest_workspace_id: Uuid,
  src_file_id: Uuid,
) -> ApiResult<Option<(Uuid, Uuid)>> {
  let Some(src_file) = store::fetch_file(&state.db, src_workspace_id, src_file_id).await? else {
    return Ok(None); // dangling reference already; nothing to copy
  };
  let suffix = src_file
    .object_key
    .strip_prefix(&format!("workspaces/{src_workspace_id}/"))
    .ok_or_else(|| ApiError::Internal("source object_key not under its workspace".to_string()))?;
  let dest_key = format!("workspaces/{dest_workspace_id}/{suffix}");

  let bytes = http
    .get(storage.presign_get_server(&src_file.object_key))
    .send()
    .await
    .map_err(|e| ApiError::Internal(format!("blob fetch failed: {}", e.without_url())))?
    .error_for_status()
    .map_err(|e| ApiError::Internal(format!("blob fetch returned: {}", e.without_url())))?
    .bytes()
    .await
    .map_err(|e| ApiError::Internal(format!("blob read failed: {}", e.without_url())))?;
  // Commit this marker (and clear any GC mark) before PUT: a crash after
  // writing bytes must leave a discoverable key for the orphan sweep, and
  // neither sweep may take the key while the PUT runs outside a transaction.
  store::record_pending_file_upload(
    &state.db,
    dest_workspace_id,
    &dest_key,
    storage.presign_ttl_seconds,
  )
  .await?;
  let upload = storage.presign_put_server(&dest_key);
  let checksum = {
    use base64::Engine as _;
    use sha2::{Digest, Sha256};
    base64::engine::general_purpose::STANDARD.encode(Sha256::digest(&bytes))
  };
  let put = http
    .put(&upload.url)
    .header(reqwest::header::CONTENT_TYPE, mica_infra::stored_content_type(&src_file.mime_type))
    .header("x-amz-checksum-sha256", checksum)
    .body(bytes.to_vec())
    .send()
    .await
    .map_err(|e| ApiError::Internal(format!("blob upload failed: {}", e.without_url())))?;
  if !put.status().is_success() {
    return Err(ApiError::Internal(format!(
      "blob upload returned {}",
      put.status()
    )));
  }

  let dest_file = store::register_put_file(
    &state.db,
    dest_workspace_id,
    user_id,
    &dest_key,
    &src_file.original_name,
    &src_file.mime_type,
    src_file.byte_size,
  )
  .await?;
  Ok(Some((src_file_id, dest_file.id)))
}

async fn transfer_roots(
  state: &AppState,
  user_id: Uuid,
  src_workspace_id: Uuid,
  request: TransferPlan,
) -> ApiResult<TransferResponse> {
  let dest_workspace_id = request.dest_workspace_id;
  if dest_workspace_id == src_workspace_id {
    return Err(ApiError::BadRequest(
      "destination workspace must differ from the source".to_string(),
    ));
  }
  // Editor in BOTH: to remove from the source and to create in the destination.
  ensure_workspace_editor(&state.db, src_workspace_id, user_id).await?;
  ensure_workspace_editor(&state.db, dest_workspace_id, user_id).await?;

  // A destination parent, if given, must be a live folder in the destination.
  if let Some(parent) = request.parent_view_id {
    ensure_parent_accepts_children(&state.db, dest_workspace_id, parent).await?;
  }

  // 0. Drop roots that sit inside another root, BEFORE enumerating anything —
  //    otherwise the `= ANY` walk below visits the overlap twice and the
  //    destination gets two copies of it.
  let ancestor_pairs = ancestor_pairs_of_roots(&state.db, src_workspace_id, &request.roots).await?;
  let roots = independent_roots(&request.roots, &ancestor_pairs);
  let root_ids: std::collections::HashSet<Uuid> = roots.iter().copied().collect();

  // 1. Enumerate the subtrees (live rows only). Disjoint by construction after
  //    step 0, so no row can appear twice.
  let subtree = live_subtree(&state.db, &roots, src_workspace_id).await?;
  let subtree_view_ids: std::collections::HashSet<Uuid> = subtree.iter().map(|r| r.id).collect();

  // A root that produced no row is deleted, or in another workspace, or made up.
  // Refuse the whole batch rather than transferring the rest: the caller asked
  // for these N, and silently delivering N-1 is the kind of success nobody
  // checks. This also replaces the single route's `ensure_view_in_workspace`.
  if let Some(&missing) = roots.iter().find(|r| !subtree_view_ids.contains(r)) {
    tracing::debug!(%missing, "transfer: root not live in the source workspace");
    return Err(ApiError::NotFound);
  }

  // The whole batch counts as "staying together" for the dangling-link report:
  // a link from one selected page to another is not breakage, it is a link that
  // follows. Before the batch endpoint existed this had to be declared by the
  // caller (`also_moving`), because each request knew only its own subtree.
  let moving_view_ids = &subtree_view_ids;

  // 2. Pre-scan documents: referenced file_ids + cross-workspace dangling links.
  let mut payloads: std::collections::HashMap<Uuid, DocumentSnapshotPayload> =
    std::collections::HashMap::new();
  let mut referenced_files: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
  let mut dangling_links: Vec<DanglingLink> = Vec::new();
  let doc_ids: Vec<Uuid> = subtree
    .iter()
    .filter(|r| r.object_type == "document")
    .map(|r| r.object_id)
    .collect();
  let documents = doc_ids.len();
  let folders = subtree.len() - documents;
  // Loaded TRANSFER_CONCURRENCY at a time: one at a time made a 500-page
  // subtree 500 sequential round trips before anything else could start.
  let db = &state.db;
  let loaded: Vec<Option<DocumentSnapshotPayload>> = futures_util::stream::iter(doc_ids)
    .map(|id| store::current_payload(db, id))
    .buffered(TRANSFER_CONCURRENCY)
    .try_collect()
    .await?;
  let doc_rows = subtree.iter().filter(|r| r.object_type == "document");
  for (row, payload) in doc_rows.zip(loaded) {
    let Some(payload) = payload else {
      continue;
    };
    for block in &payload.blocks {
      if let Some(fid) = block
        .data
        .get("file_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
      {
        referenced_files.insert(fid);
      }
      for target in page_link_targets(&block.data) {
        if let Ok(tid) = Uuid::parse_str(&target) {
          // `moving_view_ids`, not `subtree_view_ids`: a link into another root
          // of the same batch is not breakage, it is a link that follows.
          if !moving_view_ids.contains(&tid) {
            dangling_links.push(DanglingLink {
              document: row.name.clone(),
              target_view_id: tid,
            });
          }
        }
      }
    }
    payloads.insert(row.object_id, payload);
  }
  let images = referenced_files.len();

  if request.dry_run {
    return Ok(TransferResponse {
      new_root_view_id: None,
      new_root_view_ids: Vec::new(),
      documents,
      folders,
      images,
      dangling_links,
      removed_source: false,
      dry_run: true,
    });
  }

  // The destination's quota, for the bytes it does not hold yet. Every other
  // byte-storing path is metered; this one copied gigabytes into a nearly
  // full workspace unchecked.
  let ids: Vec<Uuid> = referenced_files.iter().copied().collect();
  let incoming: i64 = sqlx::query_scalar(
    r#"
      SELECT coalesce(sum(s.byte_size), 0)::bigint
      FROM files s
      WHERE s.workspace_id = $1 AND s.id = ANY($3)
        AND NOT EXISTS (
          SELECT 1 FROM files d
          WHERE d.object_key = 'workspaces/' || $2::text || '/'
            || substr(s.object_key, length('workspaces/' || $1::text || '/') + 1)
        )
    "#,
  )
  .bind(src_workspace_id)
  .bind(dest_workspace_id)
  .bind(&ids)
  .fetch_one(&state.db)
  .await?;
  crate::routes::files::ensure_quota_room(state, dest_workspace_id, incoming, None).await?;

  // 3. Copy blobs into the destination BEFORE the transaction: content-addressed
  //    keys make the PUT idempotent, and a half-failure leaves only orphan bytes
  //    in dest (its GC reclaims them), never a page with broken images.
  let storage = state
    .storage
    .as_ref()
    .ok_or_else(|| ApiError::Internal("file storage is not configured".to_string()))?;
  let http = reqwest::Client::new();
  // Each blob is a GET and a PUT against object storage; done one by one, a
  // subtree with a few hundred images took minutes. Bounded, so a large move
  // cannot open hundreds of connections at once. The first failure fails the
  // move (the rest are dropped mid-flight) — still before the transaction, so
  // at worst orphan bytes in the destination, as before.
  let file_map: std::collections::HashMap<Uuid, Uuid> =
    futures_util::stream::iter(referenced_files.iter().copied())
      .map(|src_file_id| {
        copy_blob(state, storage, &http, user_id, src_workspace_id, dest_workspace_id, src_file_id)
      })
      .buffer_unordered(TRANSFER_CONCURRENCY)
      .try_filter_map(|copied| async move { Ok(copied) })
      .try_collect()
      .await?;

  // 4. One transaction: build the destination tree (new ids), then soft-delete
  //    the source subtree for a move.
  let view_map: std::collections::HashMap<Uuid, Uuid> =
    subtree.iter().map(|r| (r.id, Uuid::new_v4())).collect();
  let ordered = topo_order_subtree(&subtree);

  // Filled as the subtree is rebuilt; each copied document gets its yrs base
  // built after the commit (see bootstrap_bases_best_effort).
  let mut created_document_ids: Vec<Uuid> = Vec::new();
  let mut tx = state.db.begin().await?;
  for row in &ordered {
    let new_view_id = view_map[&row.id];
    // Every root lands directly under the chosen destination parent; everything
    // below follows its own (already remapped) parent.
    let dest_parent = if root_ids.contains(&row.id) {
      request.parent_view_id
    } else {
      row.parent_view_id.and_then(|p| view_map.get(&p).copied())
    };
    if row.object_type == "document" {
      let mut payload = payloads.remove(&row.object_id).unwrap_or_else(|| {
        // Substituting an empty payload is CORRECT for a genuinely-empty doc
        // (current_payload returns None only when no snapshot row exists). It is
        // NOT silent anymore: the one way this loses content — a doc with a yrs
        // base but no snapshot row — would land here, and a move then deletes the
        // source. Logging makes that edge diagnosable instead of invisible (P0-4).
        tracing::warn!(
          object_id = %row.object_id,
          "transfer/clone: no snapshot for document — substituting empty payload"
        );
        DocumentSnapshotPayload {
          schema_version: 1,
          root_block_id: "root".to_string(),
          blocks: Vec::new(),
        }
      });
      rewrite_transferred_payload(&mut payload, &file_map, &view_map);
      let document = sqlx::query_as::<_, DocumentRecord>(
        r#"
          INSERT INTO documents (workspace_id, root_block_id, created_by)
          VALUES ($1, $2, $3)
          RETURNING id, workspace_id, root_block_id, current_seq, created_by, created_at, updated_at
        "#,
      )
      .bind(dest_workspace_id)
      .bind(&payload.root_block_id)
      .bind(user_id)
      .fetch_one(&mut *tx)
      .await?;
      mica_app_core::sync::seed_base_tx(&mut tx, document.id, payload).await?;
      created_document_ids.push(document.id);
      sqlx::query(
        r#"
          INSERT INTO views (id, workspace_id, parent_view_id, object_id, object_type, name, position, created_by)
          VALUES ($1, $2, $3, $4, 'document', $5, $6, $7)
        "#,
      )
      .bind(new_view_id)
      .bind(dest_workspace_id)
      .bind(dest_parent)
      .bind(document.id)
      .bind(&row.name)
      .bind(&row.position)
      .bind(user_id)
      .execute(&mut *tx)
      .await?;
    } else {
      // Folder: a view with no document. object_id is a fresh unused uuid.
      sqlx::query(
        r#"
          INSERT INTO views (id, workspace_id, parent_view_id, object_id, object_type, name, position, created_by)
          VALUES ($1, $2, $3, $4, 'folder', $5, $6, $7)
        "#,
      )
      .bind(new_view_id)
      .bind(dest_workspace_id)
      .bind(dest_parent)
      .bind(Uuid::new_v4())
      .bind(&row.name)
      .bind(&row.position)
      .bind(user_id)
      .execute(&mut *tx)
      .await?;
    }
  }

  if request.remove_source {
    // Inside the SAME transaction as the destination tree above: that is what
    // makes a move a move rather than a copy followed by a hopeful delete.
    remove_moved_source(&mut tx, src_workspace_id, &roots, &subtree_view_ids).await?;
  }

  tx.commit().await?;

  let new_root_view_ids: Vec<Uuid> = roots.iter().map(|r| view_map[r]).collect();
  Ok(TransferResponse {
    new_root_view_id: new_root_view_ids.first().copied(),
    new_root_view_ids,
    documents,
    folders,
    images,
    dangling_links,
    removed_source: request.remove_source,
    dry_run: false,
  })
}

// ── Clone (duplicate a view within the same workspace) ───────────────────────
// A same-workspace cousin of transfer: enumerate the subtree, give every node a
// fresh id + doc + snapshot, rewrite in-subtree page links to the new ids. Two
// deliberate differences from transfer:
//   - Blobs are NOT copied. object_key is content-addressed and workspace-scoped
//     (workspaces/{id}/{sha256}.{ext}); within ONE workspace the copy references
//     the same file_id — sharing the bytes is exactly the sha256-dedup intent,
//     and there's nothing to re-upload. So file_map stays empty and file_ids
//     pass through unchanged.
//   - The source is never removed, and the root copy gets a fresh name (deduped
//     among its siblings) + a fresh position so it sits beside the original.

#[derive(Debug, Deserialize)]
pub struct CloneRequest {
  /// Parent for the copy's root (null = the source's own parent, i.e. beside it).
  #[serde(default)]
  parent_view_id: Option<Uuid>,
  /// The copy's root name, locale-aware, computed by the caller (e.g. "X 副本").
  /// Deduped against siblings server-side. Absent → a "{source} 副本" fallback.
  #[serde(default)]
  name: Option<String>,
  /// Report what WOULD happen (counts) without mutating anything.
  #[serde(default)]
  dry_run: bool,
}

#[derive(Debug, Serialize)]
pub struct CloneResponse {
  new_root_view_id: Option<Uuid>,
  new_name: String,
  documents: usize,
  folders: usize,
  dry_run: bool,
}

/// `POST /api/workspaces/{workspace_id}/views/{view_id}/clone`
pub async fn clone_view(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, view_id)): Path<(Uuid, Uuid)>,
  Json(request): Json<CloneRequest>,
) -> ApiResult<Json<CloneResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;
  ensure_view_in_workspace(&state.db, workspace_id, view_id).await?;

  // 1. Enumerate the subtree (live rows only) — same shape as transfer.
  let subtree = sqlx::query_as::<_, TransferRow>(
    r#"
      WITH RECURSIVE subtree AS (
        SELECT id, parent_view_id, object_id, object_type::text AS object_type, name, position
        FROM views
        WHERE id = $1 AND workspace_id = $2 AND is_deleted = false
        UNION
        SELECT v.id, v.parent_view_id, v.object_id, v.object_type::text, v.name, v.position
        FROM views v JOIN subtree s ON v.parent_view_id = s.id
        WHERE v.is_deleted = false
      )
      SELECT id, parent_view_id, object_id, object_type, name, position FROM subtree
    "#,
  )
  .bind(view_id)
  .bind(workspace_id)
  .fetch_all(&state.db)
  .await?;
  let Some(root) = subtree.iter().find(|r| r.id == view_id) else {
    return Err(ApiError::NotFound);
  };

  // 2. Resolve the copy's parent: an explicit folder in this workspace, or the
  //    source's own parent (beside the original).
  let target_parent = match request.parent_view_id {
    Some(parent) => {
      ensure_parent_accepts_children(&state.db, workspace_id, parent).await?;
      Some(parent)
    }
    None => root.parent_view_id,
  };

  // 3. Name the copy: caller's locale-aware base (fallback "{source} 副本"),
  //    deduped against the live siblings under the target parent.
  let base_name = request
    .name
    .clone()
    .unwrap_or_else(|| format!("{} 副本", root.name));
  let siblings = sqlx::query_scalar::<_, String>(
    "SELECT name FROM views WHERE workspace_id = $1 AND is_deleted = false \
     AND parent_view_id IS NOT DISTINCT FROM $2",
  )
  .bind(workspace_id)
  .bind(target_parent)
  .fetch_all(&state.db)
  .await?;
  let new_name = dedup_sibling_name(&base_name, &siblings);

  // 4. Pre-scan document payloads (needed to rewrite in-subtree links). No blob
  //    copy, no dangling-link scan: every link target stays in this workspace.
  let mut payloads: std::collections::HashMap<Uuid, DocumentSnapshotPayload> =
    std::collections::HashMap::new();
  let mut documents = 0usize;
  let mut folders = 0usize;
  for row in &subtree {
    if row.object_type != "document" {
      folders += 1;
      continue;
    }
    documents += 1;
    if let Some(payload) = store::current_payload(&state.db, row.object_id).await? {
      payloads.insert(row.object_id, payload);
    }
  }

  if request.dry_run {
    return Ok(Json(CloneResponse {
      new_root_view_id: None,
      new_name,
      documents,
      folders,
      dry_run: true,
    }));
  }

  // Filled as the subtree is rebuilt; each copied document gets its yrs base
  // built after the commit (see bootstrap_bases_best_effort).
  let mut created_document_ids: Vec<Uuid> = Vec::new();
  // 5. One transaction: build the copied tree with fresh ids. file_map is empty
  //    (blobs shared), view_map remaps in-subtree links to the new ids.
  let view_map: std::collections::HashMap<Uuid, Uuid> =
    subtree.iter().map(|r| (r.id, Uuid::new_v4())).collect();
  let empty_files: std::collections::HashMap<Uuid, Uuid> = std::collections::HashMap::new();
  let ordered = topo_order_subtree(&subtree);

  let mut tx = state.db.begin().await?;
  for row in &ordered {
    let new_view_id = view_map[&row.id];
    let is_root = row.id == view_id;
    let dest_parent = if is_root {
      target_parent
    } else {
      row.parent_view_id.and_then(|p| view_map.get(&p).copied())
    };
    // The root sits beside the original: fresh name + fresh position so it does
    // not collide with the source under the same parent. Inner nodes keep their
    // name/position — their parent is a new view, so nothing collides there.
    let node_name = if is_root { &new_name } else { &row.name };
    let node_position = if is_root {
      Uuid::now_v7().to_string()
    } else {
      row.position.clone()
    };

    if row.object_type == "document" {
      let mut payload = payloads.remove(&row.object_id).unwrap_or_else(|| {
        // Substituting an empty payload is CORRECT for a genuinely-empty doc
        // (current_payload returns None only when no snapshot row exists). It is
        // NOT silent anymore: the one way this loses content — a doc with a yrs
        // base but no snapshot row — would land here, and a move then deletes the
        // source. Logging makes that edge diagnosable instead of invisible (P0-4).
        tracing::warn!(
          object_id = %row.object_id,
          "transfer/clone: no snapshot for document — substituting empty payload"
        );
        DocumentSnapshotPayload {
          schema_version: 1,
          root_block_id: "root".to_string(),
          blocks: Vec::new(),
        }
      });
      rewrite_transferred_payload(&mut payload, &empty_files, &view_map);
      let document = sqlx::query_as::<_, DocumentRecord>(
        r#"
          INSERT INTO documents (workspace_id, root_block_id, created_by)
          VALUES ($1, $2, $3)
          RETURNING id, workspace_id, root_block_id, current_seq, created_by, created_at, updated_at
        "#,
      )
      .bind(workspace_id)
      .bind(&payload.root_block_id)
      .bind(user_id)
      .fetch_one(&mut *tx)
      .await?;
      mica_app_core::sync::seed_base_tx(&mut tx, document.id, payload).await?;
      created_document_ids.push(document.id);
      sqlx::query(
        r#"
          INSERT INTO views (id, workspace_id, parent_view_id, object_id, object_type, name, position, created_by)
          VALUES ($1, $2, $3, $4, 'document', $5, $6, $7)
        "#,
      )
      .bind(new_view_id)
      .bind(workspace_id)
      .bind(dest_parent)
      .bind(document.id)
      .bind(node_name)
      .bind(&node_position)
      .bind(user_id)
      .execute(&mut *tx)
      .await?;
    } else {
      sqlx::query(
        r#"
          INSERT INTO views (id, workspace_id, parent_view_id, object_id, object_type, name, position, created_by)
          VALUES ($1, $2, $3, $4, 'folder', $5, $6, $7)
        "#,
      )
      .bind(new_view_id)
      .bind(workspace_id)
      .bind(dest_parent)
      .bind(Uuid::new_v4())
      .bind(node_name)
      .bind(&node_position)
      .bind(user_id)
      .execute(&mut *tx)
      .await?;
    }
  }
  tx.commit().await?;

  Ok(Json(CloneResponse {
    new_root_view_id: Some(view_map[&view_id]),
    new_name,
    documents,
    folders,
    dry_run: false,
  }))
}

/// Pick a sibling-unique name: `base` if free, else `base 2`, `base 3`, … The
/// number is locale-neutral, so the caller supplies the localized base ("X 副本"
/// / "X copy") and we only break ties.
fn dedup_sibling_name(base: &str, siblings: &[String]) -> String {
  if !siblings.iter().any(|s| s == base) {
    return base.to_string();
  }
  (2..)
    .map(|n| format!("{base} {n}"))
    .find(|c| !siblings.iter().any(|s| s == c))
    .expect("an unused suffix always exists")
}

/// The `mica://page/<viewId>` targets referenced by a block's link marks.
///
/// Re-exported from app-core, which owns it because the write paths there derive
/// `document_yrs_base.link_targets` with it. A copy here would be a second source
/// of truth for what counts as a page link.
use mica_app_core::sync::page_link_targets;

/// Rewrite a transferred document's blocks for their new home: remap uploaded-
/// image `file_id`s to the destination copies, and remap in-subtree page links to
/// the new view ids. Links to pages left in the source keep their `mica://` href
/// (they dangle — surfaced to the user as a warning before the move).
fn rewrite_transferred_payload(
  payload: &mut DocumentSnapshotPayload,
  file_map: &std::collections::HashMap<Uuid, Uuid>,
  view_map: &std::collections::HashMap<Uuid, Uuid>,
) {
  const SCHEME: &str = "mica://page/";
  for block in &mut payload.blocks {
    let Some(data) = block.data.as_object_mut() else {
      continue;
    };
    // Image file_id → destination copy. Drop any cached blob `url` so the client
    // re-resolves it against the destination workspace.
    if let Some(new_id) = data
      .get("file_id")
      .and_then(|v| v.as_str())
      .and_then(|s| Uuid::parse_str(s).ok())
      .and_then(|old| file_map.get(&old))
    {
      let new_id = *new_id;
      data.insert("file_id".into(), serde_json::json!(new_id.to_string()));
      data.remove("url");
    }
    // In-subtree page links → new view ids.
    if let Some(marks) = data.get_mut("marks").and_then(|m| m.as_array_mut()) {
      for mark in marks {
        let Some(obj) = mark.as_object_mut() else {
          continue;
        };
        let Some(new_href) = obj
          .get("href")
          .and_then(|h| h.as_str())
          .and_then(|href| href.strip_prefix(SCHEME))
          .and_then(|id| Uuid::parse_str(id).ok())
          .and_then(|old| view_map.get(&old))
          .map(|new| format!("{SCHEME}{new}"))
        else {
          continue;
        };
        obj.insert("href".into(), serde_json::json!(new_href));
      }
    }
  }
}

/// Order the subtree so a parent always precedes its children (root first). The
/// recursive CTE does not guarantee parent-first, and we insert with FK-linked
/// parents, so the order matters.
fn topo_order_subtree(subtree: &[TransferRow]) -> Vec<&TransferRow> {
  let ids: std::collections::HashSet<Uuid> = subtree.iter().map(|r| r.id).collect();
  let mut by_parent: std::collections::HashMap<Option<Uuid>, Vec<&TransferRow>> =
    std::collections::HashMap::new();
  for r in subtree {
    // The root's real parent is outside the subtree — anchor it at None.
    let key = r.parent_view_id.filter(|p| ids.contains(p));
    by_parent.entry(key).or_default().push(r);
  }
  let mut out: Vec<&TransferRow> = Vec::with_capacity(subtree.len());
  let mut stack: Vec<Option<Uuid>> = vec![None];
  while let Some(parent) = stack.pop() {
    if let Some(children) = by_parent.get(&parent) {
      for child in children {
        out.push(child);
        stack.push(Some(child.id));
      }
    }
  }
  out
}

#[cfg(test)]
mod tests {
  fn image_block(data: serde_json::Value) -> mica_app_core::documents::Block {
    mica_app_core::documents::Block {
      id: "img_1".to_string(),
      kind: "image".to_string(),
      text: String::new(),
      data,
      children: Vec::new(),
    }
  }
  use super::*;
  use mica_app_core::sync::page_link_targets;

  fn text_block(id: &str, data: serde_json::Value) -> mica_app_core::documents::Block {
    mica_app_core::documents::Block {
      id: id.to_string(),
      kind: "text".to_string(),
      text: "x".to_string(),
      data,
      children: Vec::new(),
    }
  }

  /// A transferred doc must point its image at the DESTINATION file copy (else
  /// the source's per-workspace GC reclaims the bytes and the moved page breaks),
  /// remap links to pages that came along, and leave links to pages left behind
  /// untouched (they dangle — surfaced as a warning, not silently rewritten).
  #[test]
  fn transfer_rewrites_file_ids_and_in_subtree_links_only() {
    let old_file = Uuid::new_v4();
    let new_file = Uuid::new_v4();
    let in_sub_old = Uuid::new_v4();
    let in_sub_new = Uuid::new_v4();
    let outside = Uuid::new_v4();

    let file_map = std::collections::HashMap::from([(old_file, new_file)]);
    let view_map = std::collections::HashMap::from([(in_sub_old, in_sub_new)]);

    let mut payload = DocumentSnapshotPayload {
      schema_version: 1,
      root_block_id: "root".to_string(),
      blocks: vec![
        image_block(serde_json::json!({
          "file_id": old_file.to_string(),
          "name": "x.png",
          "url": format!("/api/workspaces/{}/files/{}/blob/x.png", Uuid::new_v4(), old_file),
        })),
        text_block(
          "t1",
          serde_json::json!({"marks": [
            {"type": "link", "href": format!("mica://page/{in_sub_old}")},
            {"type": "link", "href": format!("mica://page/{outside}")},
          ]}),
        ),
      ],
    };
    rewrite_transferred_payload(&mut payload, &file_map, &view_map);

    assert_eq!(
      payload.blocks[0].data["file_id"],
      serde_json::json!(new_file.to_string()),
      "image file_id must point at the destination copy",
    );
    assert!(
      payload.blocks[0].data.get("url").is_none(),
      "stale cached blob url must be dropped so the client re-resolves",
    );
    let marks = payload.blocks[1].data["marks"].as_array().unwrap();
    assert_eq!(
      marks[0]["href"],
      serde_json::json!(format!("mica://page/{in_sub_new}")),
      "a link to a page that came along is remapped",
    );
    assert_eq!(
      marks[1]["href"],
      serde_json::json!(format!("mica://page/{outside}")),
      "a link to a page left behind is preserved (dangles, warned)",
    );
  }

  #[test]
  fn dedup_sibling_name_only_numbers_on_collision() {
    // Free name → used as-is.
    assert_eq!(dedup_sibling_name("日志方案 副本", &[]), "日志方案 副本");
    // Collision → first free numbered suffix (locale-neutral number).
    assert_eq!(
      dedup_sibling_name("日志方案 副本", &["日志方案 副本".into()]),
      "日志方案 副本 2"
    );
    // Skips taken numbers, does not reuse them.
    assert_eq!(
      dedup_sibling_name(
        "X 副本",
        &["X 副本".into(), "X 副本 2".into(), "X 副本 4".into()]
      ),
      "X 副本 3"
    );
    // Unrelated siblings never force a suffix.
    assert_eq!(dedup_sibling_name("X 副本", &["Y".into(), "Z".into()]), "X 副本");
  }

  #[test]
  fn page_link_targets_extracts_only_mica_page_ids() {
    let a = Uuid::new_v4();
    let data = serde_json::json!({"marks": [
      {"type": "link", "href": format!("mica://page/{a}")},
      {"type": "link", "href": "https://example.com"},
      {"type": "bold"},
    ]});
    assert_eq!(page_link_targets(&data), vec![a.to_string()]);
    assert!(page_link_targets(&serde_json::json!({})).is_empty());
  }

  #[test]
  fn topo_order_puts_parents_before_children() {
    let root = Uuid::new_v4();
    let child = Uuid::new_v4();
    let grandchild = Uuid::new_v4();
    let outside_parent = Uuid::new_v4(); // root's real parent, outside the subtree
    let row = |id, parent| TransferRow {
      id,
      parent_view_id: parent,
      object_id: Uuid::new_v4(),
      object_type: "folder".to_string(),
      name: "n".to_string(),
      position: "0".to_string(),
    };
    // Deliberately not parent-first, mimicking the CTE's arbitrary order.
    let subtree = vec![
      row(grandchild, Some(child)),
      row(root, Some(outside_parent)),
      row(child, Some(root)),
    ];
    let ordered: Vec<Uuid> = topo_order_subtree(&subtree).iter().map(|r| r.id).collect();
    let pos = |id: Uuid| ordered.iter().position(|&x| x == id).unwrap();
    assert_eq!(ordered.len(), 3);
    assert!(pos(root) < pos(child), "root must precede its child");
    assert!(pos(child) < pos(grandchild), "child must precede its grandchild");
  }

}
