use axum::{
  Json,
  extract::{Path, State},
  http::HeaderMap,
};
use futures_util::StreamExt as _;
use mica_app_core::{
  AppState,
  documents::DocumentSnapshotPayload,
  store::{self, DocumentRecord},
};
use mica_infra::{ApiError, ApiResult};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use super::*;
use crate::routes::auth::user_id_from_headers;

pub async fn move_view(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, view_id)): Path<(Uuid, Uuid)>,
  Json(payload): Json<MoveViewRequest>,
) -> ApiResult<Json<ViewResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;

  // gh#46: validate and write in ONE transaction, behind the workspace tree
  // lock. The cycle check used to run against the pool's latest snapshot and
  // the UPDATE in a separate statement, so two moves that traded a folder and
  // one of its descendants each validated before the other committed and both
  // wrote — a cycle.
  let mut tx = state.db.begin().await?;
  lock_workspace_tree(&mut tx, workspace_id).await?;
  ensure_view_in_workspace(&mut *tx, workspace_id, view_id).await?;
  if let Some(parent_view_id) = payload.parent_view_id {
    ensure_valid_parent_view(&mut tx, workspace_id, view_id, parent_view_id).await?;
  }

  let position = normalize_position(payload.position)?;
  let view = sqlx::query_as::<_, View>(concat!(
    r#"
      UPDATE views
      SET parent_view_id = $1, position = $2, updated_at = now()
      WHERE id = $3 AND workspace_id = $4 AND is_deleted = false
      RETURNING
        "#,
    view_columns!(),
    "\n    ",
  ))
  .bind(payload.parent_view_id)
  .bind(position)
  .bind(view_id)
  .bind(workspace_id)
  .fetch_optional(&mut *tx)
  .await?
  .ok_or(ApiError::NotFound)?;
  tx.commit().await?;

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

  // The same gate every other batch route takes — this one had NEITHER: no
  // ceiling on the list, and a duplicate id would leave one of the two at a
  // stale position (#48).
  check_batch_ids(&payload.ordered_view_ids)?;

  let mut tx = state.db.begin().await?;
  // gh#46: validate INSIDE the transaction and behind the workspace tree
  // lock. Validating from the pool first and writing second let two
  // concurrent reorders/moves see compatible snapshots and write a cycle.
  lock_workspace_tree(&mut tx, workspace_id).await?;

  // Validate everything BEFORE writing anything, in SET terms (#48): one
  // membership query for the whole list (a missing id gets the same 404 the
  // per-id check gave), then — when re-parenting — one folder check and one
  // cycle check with the recursive walk seeded from ALL ids at once. The old
  // loop spent three round trips per id proving these same facts.
  let live: std::collections::HashSet<Uuid> = sqlx::query_scalar::<_, Uuid>(
    r#"
      SELECT id
      FROM views
      WHERE id = ANY($1) AND workspace_id = $2 AND is_deleted = false
    "#,
  )
  .bind(&payload.ordered_view_ids)
  .bind(workspace_id)
  .fetch_all(&mut *tx)
  .await?
  .into_iter()
  .collect();
  if let Some(&missing) = payload
    .ordered_view_ids
    .iter()
    .find(|id| !live.contains(id))
  {
    tracing::debug!(%missing, "reorder: view not live in the workspace");
    return Err(ApiError::NotFound);
  }

  if let Some(parent) = payload.parent_view_id {
    if payload.ordered_view_ids.contains(&parent) {
      return Err(ApiError::BadRequest(
        "parent_view_id cannot be the same view".to_string(),
      ));
    }
    ensure_parent_accepts_children(&mut *tx, workspace_id, parent).await?;

    let would_cycle = sqlx::query_scalar::<_, bool>(
      r#"
        WITH RECURSIVE descendants (id) AS (
          SELECT id
          FROM views
          WHERE id = ANY($1) AND workspace_id = $2 AND is_deleted = false
          UNION ALL
          SELECT v.id
          FROM views v
          INNER JOIN descendants d ON v.parent_view_id = d.id
          WHERE v.workspace_id = $2 AND v.is_deleted = false
        ) CYCLE id SET is_cycle USING cycle_path
        SELECT EXISTS (
          SELECT 1
          FROM descendants
          WHERE id = $3 AND NOT is_cycle
        )
      "#,
    )
    .bind(&payload.ordered_view_ids)
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

  // ONE statement for the whole batch. RETURNING says exactly which ids
  // matched, so a row that stopped matching between validation and write is a
  // 409 naming that id — not a partial reorder reported as full success (P1-3).
  // The `?`/return drops `tx`, rolling the whole batch back so the client can
  // refetch and retry against fresh state.
  let positions: Vec<String> = (1..=payload.ordered_view_ids.len())
    .map(|i| format!("{:010}", i * 10))
    .collect();
  let touched: Vec<Uuid> = sqlx::query_scalar::<_, Uuid>(
    r#"
      UPDATE views v
      SET parent_view_id = $1, position = t.position, updated_at = now()
      FROM unnest($2::uuid[], $3::text[]) AS t(id, position)
      WHERE v.id = t.id AND v.workspace_id = $4 AND v.is_deleted = false
      RETURNING v.id
    "#,
  )
  .bind(payload.parent_view_id)
  .bind(&payload.ordered_view_ids)
  .bind(&positions)
  .bind(workspace_id)
  .fetch_all(&mut *tx)
  .await?;

  if touched.len() != payload.ordered_view_ids.len() {
    let hit: std::collections::HashSet<Uuid> = touched.iter().copied().collect();
    let id = payload
      .ordered_view_ids
      .iter()
      .find(|id| !hit.contains(id))
      .expect("fewer touched rows than requested means some id missed");
    return Err(ApiError::Conflict(format!(
      "view {id} was modified concurrently during reorder; refetch and retry"
    )));
  }
  tx.commit().await?;

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

  // Validation moved inside [move_views_tx] (gh#46): the cycle check must run
  // in the same transaction as the write, behind the workspace tree lock, or
  // two concurrent moves can validate on each other's pre-commit snapshots.
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
  let mut tx = db.begin().await?;
  lock_workspace_tree(&mut tx, workspace_id).await?;
  // gh#46: the membership and cycle checks belong to this transaction — a
  // mover that validated against an older snapshot and wrote here could
  // re-parent a folder under a descendant its check never saw.
  for id in view_ids {
    ensure_view_in_workspace(&mut *tx, workspace_id, *id)
      .await
      .map_err(|error| match error {
        ApiError::NotFound => ApiError::Conflict(format!(
          "view {id} is no longer available for this move; refresh and retry"
        )),
        other => other,
      })?;
    if let Some(parent) = parent_view_id {
      ensure_valid_parent_view(&mut tx, workspace_id, *id, parent).await?;
    }
  }
  for (i, id) in view_ids.iter().enumerate() {
    let position = format!("{:010}", (i + 1) * 10);
    let affected = sqlx::query(
      r#"
        UPDATE views
        SET parent_view_id = $1, position = $2, updated_at = now()
        WHERE id = $3 AND workspace_id = $4 AND is_deleted = false
      "#,
    )
    .bind(parent_view_id)
    .bind(&position)
    .bind(id)
    .bind(workspace_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if affected == 0 {
      return Err(ApiError::Conflict(format!(
        "view {id} was modified concurrently during the move; refetch and retry"
      )));
    }
  }
  tx.commit().await?;
  Ok(())
}

// ── Cross-workspace transfer (move / copy) ───────────────────────────────────
// Move or copy a page (its whole subtree) or a folder into ANOTHER workspace on
// this server. No note app re-parents in place across workspaces: doc identity
// and blobs are both workspace-namespaced, so an in-place move invites doc-id
// collisions and lets the source's per-workspace blob GC reclaim images the
// moved page still needs. So this is copy-into-destination (new view + new
// document + blobs physically copied into the destination workspace), then a
// soft-delete of the source for a "move". Ordered blobs-first, so a half-failure
// leaves harmless orphan bytes in the destination (its GC reclaims them), never a
// page that lost its images — the thing Notion/AFFiNE get wrong and our Postgres
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
  /// The view's own icon. Its own column, NOT part of the document payload, so
  /// a copy that only rewrites payloads drops it (gh#156).
  icon: Option<String>,
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
          SELECT id, parent_view_id, object_id, object_type::text AS object_type, name, icon, position
          FROM views
          WHERE id = ANY($1) AND workspace_id = $2 AND is_deleted = false
          UNION ALL
          SELECT v.id, v.parent_view_id, v.object_id, v.object_type::text, v.name, v.icon, v.position
          FROM views v JOIN subtree s ON v.parent_view_id = s.id
          WHERE v.is_deleted = false
        ) CYCLE id SET is_cycle USING cycle_path
        SELECT id, parent_view_id, object_id, object_type, name, icon, position
        FROM subtree
        WHERE NOT is_cycle
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
        UNION ALL
        SELECT v.id FROM views v JOIN subtree s ON v.parent_view_id = s.id
        WHERE v.is_deleted = false
      ) CYCLE id SET is_cycle USING cycle_path
      SELECT v.id FROM views v
      WHERE v.id IN (SELECT id FROM subtree WHERE NOT is_cycle)
      FOR UPDATE OF v
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
  sqlx::query("UPDATE views SET is_deleted = true, updated_at = now() WHERE id = ANY($1)")
    .bind(&ids)
    .execute(&mut **tx)
    .await?;
  Ok(())
}

/// Refuse a move whose documents changed since their payloads were scanned.
///
/// The copy was built from payloads read BEFORE the blob copies, which can take
/// minutes; [remove_moved_source] re-checks the SHAPE of the subtree, which
/// cannot see a body write — that locks the `documents` row and leaves the tree
/// alone. Together the two guards cover what a collaborator can do during the
/// window: add/remove/move a page, and edit one in place.
///
/// `documents.current_seq` is the content version every write path advances
/// (REST, WS and MCP all go through it), so it is the value a move must pin.
/// `FOR SHARE` makes the read a barrier too: a writer already holding its
/// `FOR UPDATE` (see `store::lock_document_tx`) is waited for, and the row this
/// statement returns is the writer's committed version rather than the one it
/// is replacing. A writer that arrives later waits for this transaction to
/// commit and then writes into the source row the move has binned.
async fn ensure_documents_unchanged_tx(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  captured: &std::collections::HashMap<Uuid, i64>,
) -> ApiResult<()> {
  if captured.is_empty() {
    return Ok(());
  }
  // ONE lock order for every transfer, so two of them cannot deadlock.
  let mut ids: Vec<Uuid> = captured.keys().copied().collect();
  ids.sort_unstable();
  let now: std::collections::HashMap<Uuid, i64> = sqlx::query_as::<_, (Uuid, i64)>(
    "SELECT id, current_seq FROM documents WHERE id = ANY($1) ORDER BY id FOR SHARE",
  )
  .bind(&ids)
  .fetch_all(&mut **tx)
  .await?
  .into_iter()
  .collect();
  for (id, seq) in captured {
    match now.get(id) {
      Some(&current) if current == *seq => {}
      // A document that vanished is a structural change the view guard catches;
      // treat it as changed here all the same.
      other => {
        return Err(ApiError::Conflict(format!(
          "a page being moved was edited while its images were copied \
           (seq {} now, {} when copied); nothing was moved — refresh and try again",
          other
            .map(i64::to_string)
            .unwrap_or_else(|| "gone".to_string()),
          seq
        )));
      }
    }
  }
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
          UNION ALL
          SELECT u.root, v.parent_view_id
          FROM up u JOIN views v ON v.id = u.ancestor
          WHERE v.is_deleted = false
        ) CYCLE root, ancestor SET is_cycle USING cycle_path
        SELECT root, ancestor FROM up WHERE ancestor IS NOT NULL AND NOT is_cycle
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
    return Err(ApiError::BadRequest(
      "view_ids must not be empty".to_string(),
    ));
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
  //    ONE query fetches every yrs base (the old loop round-tripped per document
  //    — a large subtree paid N sequential awaits before the first blob moved,
  //    #49); the per-document decode + scan then runs from memory.
  let document_ids: Vec<Uuid> = subtree
    .iter()
    .filter(|r| r.object_type == "document")
    .map(|r| r.object_id)
    .collect();
  // The content versions these payloads are (at least) at, read FIRST: a write
  // landing between the two statements leaves the captured seq BEHIND the
  // payload, which the commit-time guard refuses — a spurious 409 is
  // recoverable, committing a stale copy over a user's edit is not.
  //
  // Only a MOVE needs it: a copy leaves the source live, so its slightly-older
  // snapshot is the instant the copy was taken, and nothing is lost (gh#154).
  let captured_versions: std::collections::HashMap<Uuid, i64> = if request.remove_source {
    sqlx::query_as::<_, (Uuid, i64)>("SELECT id, current_seq FROM documents WHERE id = ANY($1)")
      .bind(&document_ids)
      .fetch_all(&state.db)
      .await?
      .into_iter()
      .collect()
  } else {
    std::collections::HashMap::new()
  };
  let mut payloads = store::current_payloads(&state.db, &document_ids).await?;
  let mut referenced_files: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
  let mut dangling_links: Vec<DanglingLink> = Vec::new();
  let mut documents = 0usize;
  let mut folders = 0usize;
  for row in &subtree {
    if row.object_type == "document" {
      documents += 1;
      // Iterate the subtree rows (not the map) so links keep the deterministic
      // tree order the report always had, and a payload-less document is still
      // counted — both exactly as the old loop did.
      let Some(payload) = payloads.get(&row.object_id) else {
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
    } else {
      folders += 1;
    }
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
  let http = crate::http_client::outbound_client();

  // Resolve every source row in ONE query, then plan one copy per DESTINATION
  // KEY ([plan_blob_copies]): content-addressed keys mean two source rows can
  // be the same bytes, and the old serial loop PUT + registered them twice.
  // A referenced id with no row is a dangling reference already; nothing to
  // copy, and the payload keeps its stale id — the old loop's `continue`.
  let src_files = store::fetch_files(&state.db, src_workspace_id, &ids).await?;
  let (copies, dest_key_of_src) = plan_blob_copies(src_workspace_id, dest_workspace_id, src_files)?;

  // Bounded parallelism: the old loop fetched, lock-marked and re-uploaded one
  // blob fully before starting the next, so a large subtree paid every network
  // round trip in series and a move ran minutes (#49). The STEPS per blob keep
  // their order — pending marker before PUT, registration after — only
  // different blobs overlap. `buffered` yields results in input order, so a
  // multi-blob failure reports the EARLIEST blob's error, deterministically,
  // like the serial loop. The cap bounds concurrent in-memory bytes.
  const BLOB_COPY_CONCURRENCY: usize = 4;
  let results = futures_util::stream::iter(copies.into_iter().map(|(src_file, dest_key)| {
    let http = &http;
    async move {
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
        .header(
          reqwest::header::CONTENT_TYPE,
          mica_infra::stored_content_type(&src_file.mime_type),
        )
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
      Ok::<_, ApiError>((dest_key, dest_file.id))
    }
  }))
  .buffered(BLOB_COPY_CONCURRENCY)
  .collect::<Vec<_>>()
  .await;

  let mut dest_file_by_key: std::collections::HashMap<String, Uuid> =
    std::collections::HashMap::new();
  for result in results {
    let (dest_key, dest_file_id) = result?;
    dest_file_by_key.insert(dest_key, dest_file_id);
  }
  let mut file_map: std::collections::HashMap<Uuid, Uuid> = std::collections::HashMap::new();
  for (src_file_id, dest_key) in &dest_key_of_src {
    if let Some(&dest_file_id) = dest_file_by_key.get(dest_key) {
      file_map.insert(*src_file_id, dest_file_id);
    }
  }

  // 4. One transaction: build the destination tree (new ids), then soft-delete
  //    the source subtree for a move.
  let view_map: std::collections::HashMap<Uuid, Uuid> =
    subtree.iter().map(|r| (r.id, Uuid::new_v4())).collect();
  let ordered = topo_order_subtree(&subtree);

  let mut tx = state.db.begin().await?;
  // Before anything is written: refuse a move whose documents have been edited
  // since their payloads were scanned. The view-ID guard in
  // [remove_moved_source] cannot see that — a body write locks the `documents`
  // row and leaves the tree untouched, so the move used to commit the pre-write
  // payload and bin the only copy holding the new text (gh#154).
  ensure_documents_unchanged_tx(&mut tx, &captured_versions).await?;
  for row in &ordered {
    let new_view_id = view_map[&row.id];
    // Every root lands directly under the chosen destination parent; everything
    // below follows its own (already remapped) parent.
    let dest_parent = if root_ids.contains(&row.id) {
      request.parent_view_id
    } else {
      row.parent_view_id.and_then(|p| view_map.get(&p).copied())
    };
    insert_subtree_tx(
      &mut tx,
      row,
      new_view_id,
      SubtreeNode {
        workspace_id: dest_workspace_id,
        user_id,
        dest_parent,
        name: &row.name,
        position: &row.position,
        icon: row.icon.as_deref(),
        title: None,
      },
      &mut payloads,
      &file_map,
      &view_map,
    )
    .await?;
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

type BlobCopy = (store::FileRecord, String);
type BlobCopyPlan = (Vec<BlobCopy>, std::collections::HashMap<Uuid, String>);

/// Plan one blob copy per DESTINATION key for a transfer.
///
/// Keys are content-addressed (`workspaces/{ws}/{sha256}.{ext}`), so two source
/// rows can carry the SAME bytes — one sha, two file rows. Copying such a pair
/// CONCURRENTLY would race the pending-upload marker and the files-table
/// upsert for the same key, so the plan holds one copy per key; every source
/// id sharing a key later maps to the one destination row for it — exactly the
/// rows the old serial loop's UNIQUE(object_key) upsert produced.
///
/// Returns the copies (source row + destination key, first-row-wins order) plus
/// each source file's destination key. A source row whose key is not under its
/// own workspace prefix is corrupt bookkeeping, not an empty input — Internal,
/// as the old loop's in-place check was.
fn plan_blob_copies(
  src_workspace_id: Uuid,
  dest_workspace_id: Uuid,
  src_files: Vec<store::FileRecord>,
) -> ApiResult<BlobCopyPlan> {
  let mut dest_key_of_src: std::collections::HashMap<Uuid, String> =
    std::collections::HashMap::new();
  let mut copies: Vec<(store::FileRecord, String)> = Vec::new();
  let mut seen_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
  for src_file in src_files {
    let suffix = src_file
      .object_key
      .strip_prefix(&format!("workspaces/{src_workspace_id}/"))
      .ok_or_else(|| ApiError::Internal("source object_key not under its workspace".to_string()))?;
    let dest_key = format!("workspaces/{dest_workspace_id}/{suffix}");
    dest_key_of_src.insert(src_file.id, dest_key.clone());
    if seen_keys.insert(dest_key.clone()) {
      copies.push((src_file, dest_key));
    }
  }
  Ok((copies, dest_key_of_src))
}

// ── Clone (duplicate a view within the same workspace) ───────────────────────
// A same-workspace cousin of transfer: enumerate the subtree, give every node a
// fresh id + doc + yrs base, rewrite in-subtree page links to the new ids. Two
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
        SELECT id, parent_view_id, object_id, object_type::text AS object_type, name, icon, position
        FROM views
        WHERE id = $1 AND workspace_id = $2 AND is_deleted = false
        UNION ALL
        SELECT v.id, v.parent_view_id, v.object_id, v.object_type::text, v.name, v.icon, v.position
        FROM views v JOIN subtree s ON v.parent_view_id = s.id
        WHERE v.is_deleted = false
      ) CYCLE id SET is_cycle USING cycle_path
      SELECT id, parent_view_id, object_id, object_type, name, icon, position
      FROM subtree
      WHERE NOT is_cycle
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

    insert_subtree_tx(
      &mut tx,
      row,
      new_view_id,
      SubtreeNode {
        workspace_id,
        user_id,
        dest_parent,
        name: node_name,
        position: &node_position,
        icon: row.icon.as_deref(),
        title: is_root.then_some(new_name.as_str()),
      },
      &mut payloads,
      &empty_files,
      &view_map,
    )
    .await?;
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

/// What ONE copied node needs that differs per call site: the destination
/// parent (transfer anchors every root at the request's parent; clone anchors
/// only its own root), the name/position/icon (clone renames and repositions
/// only its root) and, for a clone root, the document title the new name
/// implies. Everything else about inserting it is one sequence.
struct SubtreeNode<'a> {
  workspace_id: Uuid,
  user_id: Uuid,
  dest_parent: Option<Uuid>,
  name: &'a str,
  position: &'a str,
  icon: Option<&'a str>,
  /// Write this into the copied document's root block `title` (gh#101). Only a
  /// clone sets it: only a clone gives the copy a name it did not have, and the
  /// title is what the client shows and the exporter names the file after.
  /// A transferred page keeps the title its author wrote, even when it differs
  /// from the view name — the document wins (see `sync::set_document_title`).
  title: Option<&'a str>,
}

/// Insert one copied node into the transfer/clone tree, inside the caller's
/// transaction: for a document — a fresh `documents` row, its payload rewritten
/// for the new home and seeded as the yrs base, plus the view; for a folder —
/// just the view, `object_id` a fresh unused uuid. Both routes used to carry
/// this whole branch inline, the empty-payload fallback inside it copied twice.
async fn insert_subtree_tx(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  row: &TransferRow,
  new_view_id: Uuid,
  node: SubtreeNode<'_>,
  payloads: &mut std::collections::HashMap<Uuid, DocumentSnapshotPayload>,
  file_map: &std::collections::HashMap<Uuid, Uuid>,
  view_map: &std::collections::HashMap<Uuid, Uuid>,
) -> ApiResult<()> {
  if row.object_type == "document" {
    let mut payload = payloads.remove(&row.object_id).unwrap_or_else(|| {
      // Substituting an empty payload is CORRECT for a genuinely-empty doc
      // (current_payload returns None only when the document has no yrs base
      // row at all). It is NOT silent: a base-less document should not exist
      // (every write path seeds the base with the `documents` row), so if one
      // lands here a move would delete the source with the content nowhere
      // else — logging makes that edge diagnosable instead of invisible (P0-4).
      tracing::warn!(
        object_id = %row.object_id,
        "transfer/clone: no payload for document — substituting empty payload"
      );
      DocumentSnapshotPayload {
        schema_version: 1,
        root_block_id: "root".to_string(),
        blocks: Vec::new(),
      }
    });
    rewrite_transferred_payload(&mut payload, file_map, view_map);
    if let Some(title) = node.title {
      // Payload level, not `set_block_prop`: the document does not exist yet,
      // so there is no comment anchor to invalidate — the title rides in the
      // seed and costs no update.
      mica_markdown::set_document_title(&mut payload, title);
    }
    let document = sqlx::query_as::<_, DocumentRecord>(
      r#"
        INSERT INTO documents (workspace_id, root_block_id, created_by)
        VALUES ($1, $2, $3)
        RETURNING id, workspace_id, root_block_id, current_seq, created_by, created_at, updated_at
      "#,
    )
    .bind(node.workspace_id)
    .bind(&payload.root_block_id)
    .bind(node.user_id)
    .fetch_one(&mut **tx)
    .await?;
    mica_app_core::sync::seed_base_tx(&mut *tx, document.id, payload).await?;
    sqlx::query(
      r#"
        INSERT INTO views (id, workspace_id, parent_view_id, object_id, object_type, name, icon, position, created_by)
        VALUES ($1, $2, $3, $4, 'document', $5, $6, $7, $8)
      "#,
    )
    .bind(new_view_id)
    .bind(node.workspace_id)
    .bind(node.dest_parent)
    .bind(document.id)
    .bind(node.name)
    .bind(node.icon)
    .bind(node.position)
    .bind(node.user_id)
    .execute(&mut **tx)
    .await?;
  } else {
    // Folder: a view with no document. object_id is a fresh unused uuid.
    sqlx::query(
      r#"
        INSERT INTO views (id, workspace_id, parent_view_id, object_id, object_type, name, icon, position, created_by)
        VALUES ($1, $2, $3, $4, 'folder', $5, $6, $7, $8)
      "#,
    )
    .bind(new_view_id)
    .bind(node.workspace_id)
    .bind(node.dest_parent)
    .bind(Uuid::new_v4())
    .bind(node.name)
    .bind(node.icon)
    .bind(node.position)
    .bind(node.user_id)
    .execute(&mut **tx)
    .await?;
  }
  Ok(())
}

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
    assert_eq!(
      dedup_sibling_name("X 副本", &["Y".into(), "Z".into()]),
      "X 副本"
    );
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

  // ── plan_blob_copies (#49) ─────────────────────────────────────────────────

  fn src_file(workspace_id: Uuid, suffix: &str) -> store::FileRecord {
    store::FileRecord {
      id: Uuid::new_v4(),
      workspace_id,
      uploaded_by: Uuid::new_v4(),
      object_key: format!("workspaces/{workspace_id}/{suffix}"),
      original_name: suffix.to_string(),
      mime_type: "image/png".to_string(),
      byte_size: 7,
      created_at: chrono::Utc::now(),
    }
  }

  /// Two source rows can be the SAME bytes (content-addressed keys): the plan
  /// must hold ONE copy for the pair, with both source ids pointing at the one
  /// destination key — concurrent copies of one key would race each other.
  #[test]
  fn plan_blob_copies_dedupes_identical_bytes() {
    let src = Uuid::new_v4();
    let dest = Uuid::new_v4();
    let a = src_file(src, "aaa.png");
    let a_id = a.id;
    let b = src_file(src, "aaa.png");
    let b_id = b.id;
    let c = src_file(src, "ccc.png");
    let c_id = c.id;

    let (copies, dest_key_of_src) = plan_blob_copies(src, dest, vec![a, b, c]).unwrap();

    assert_eq!(
      copies.len(),
      2,
      "two distinct keys → two copies, never three"
    );
    assert_eq!(
      dest_key_of_src[&a_id], dest_key_of_src[&b_id],
      "identical bytes share the one destination key"
    );
    assert_ne!(dest_key_of_src[&a_id], dest_key_of_src[&c_id]);
    assert!(
      copies
        .iter()
        .all(|(_, key)| key.starts_with(&format!("workspaces/{dest}/"))),
      "every copy lands under the destination workspace"
    );
  }

  /// A source row whose key is not under its own workspace prefix is corrupt
  /// bookkeeping — Internal, never a copy into an arbitrary key.
  #[test]
  fn plan_blob_copies_refuses_a_key_outside_its_workspace() {
    let src = Uuid::new_v4();
    let dest = Uuid::new_v4();
    let mut stray = src_file(src, "x.png");
    stray.object_key = format!("workspaces/{}/elsewhere.png", Uuid::new_v4());

    let err = plan_blob_copies(src, dest, vec![stray]).unwrap_err();
    assert!(matches!(err, ApiError::Internal(_)), "got {err:?}");
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
      icon: None,
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
    assert!(
      pos(child) < pos(grandchild),
      "child must precede its grandchild"
    );
  }

  // ── Gold-file contract tests (#85) ─────────────────────────────────────────
  //
  // `contracts/transfer.json` / `contracts/clone.json` are decoded by the
  // Flutter test suite (TransferReport / CloneReport); the mcp suite asserts
  // the fields its tools forward. The ids and counts are part of the contract.

  fn ws(id: u64) -> Uuid {
    Uuid::parse_str(&format!("{id:032x}")).unwrap()
  }

  #[test]
  fn gold_transfer_response() {
    crate::routes::contract_gold::check(
      "transfer",
      &serde_json::json!(TransferResponse {
        new_root_view_id: Some(ws(9)),
        new_root_view_ids: vec![ws(9), ws(10)],
        documents: 3,
        folders: 1,
        images: 2,
        dangling_links: vec![DanglingLink {
          document: "周会记录".to_string(),
          target_view_id: ws(4),
        }],
        removed_source: true,
        dry_run: false,
      }),
    );
  }

  #[test]
  fn gold_clone_response() {
    crate::routes::contract_gold::check(
      "clone",
      &serde_json::json!(CloneResponse {
        new_root_view_id: Some(ws(11)),
        new_name: "部署手册 副本".to_string(),
        documents: 1,
        folders: 0,
        dry_run: false,
      }),
    );
  }
}
