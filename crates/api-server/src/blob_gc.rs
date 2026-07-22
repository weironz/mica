//! Reclaim blobs no document points at any more.
//!
//! Orphans are not hypothetical here, and they cannot be caught at the moment of
//! deletion — which is the obvious design and the wrong one:
//!
//!  - `complete` writes the `files` row when the upload lands, BEFORE the image
//!    reaches a document. Paste an image and press undo and the blob is already
//!    orphaned, with no delete action anywhere to hook.
//!  - Removing an image from a page is not a "delete" — it is an ordinary edit,
//!    indistinguishable from deleting a word, and undoable. Freeing the blob
//!    there would break undo, and break cut-paste between pages outright.
//!  - `purge_view` deletes `views` rows only, so a purged page's blobs are
//!    stranded with no owner at all.
//!
//! So: periodic mark-and-sweep over a recomputed reference set, the shape
//! AFFiNE arrived at (PR #15165). Deliberately NOT refcounting — sha256 dedup
//! makes references many-to-one, and a counter must be paired perfectly with
//! every edit, undo, merge and concurrent write forever. Drift in a counter is
//! silent and permanent; a set recomputed each run heals itself in one cycle.
//! AppFlowy dedups without refcounting and simply never deletes, which is the
//! same conclusion reached from the other side.
//!
//! Two rules keep this honest, both chosen so that being wrong costs disk rather
//! than data:
//!  - The reference set OVER-approximates: any block carrying a `file_id`
//!    counts, whatever its kind. Export filters to `kind == "image"`, which is
//!    right for export and would be a data-loss bug here — a new block type with
//!    a file_id would silently become collectable.
//!  - Anything unexpected fails closed for the WHOLE workspace: one unreadable
//!    document and nothing in that workspace is touched.
use std::collections::HashSet;
use std::time::Duration;

use mica_app_core::store;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use mica_infra::storage::S3Config;

/// How long a blob must have been unreferenced before it may be deleted.
///
/// The clock starts at PURGE, not at delete: a page in the recycle bin still
/// owns its `views` row, so its blobs stay referenced (see
/// [referenced_file_ids]). That makes the effective margin "recycle-bin
/// retention + this", and makes restore-from-trash safe by construction rather
/// than by racing a timer. AFFiNE's equivalent misses exactly this: it walks the
/// root doc with `include_trash = false`, so trashing a page frees its blobs and
/// restoring it returns broken images.
const UNREFERENCED_GRACE: chrono::Duration = chrono::Duration::days(30);

/// A blob must also be this old before it may be deleted, regardless of the
/// above. Covers a different race entirely: the window between `complete`
/// writing the row and the document update that references it arriving. A fresh
/// upload looks exactly like an orphan until the page saves.
const MIN_OBJECT_AGE: chrono::Duration = chrono::Duration::days(7);

/// How often the sweep runs.
const SWEEP_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);


/// What the sweep decides for one file. Pure so the grace-period rules — the
/// part most likely to be subtly wrong, and the part whose bug deletes user
/// data — are testable without a database or object store.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Referenced right now; clear any mark.
    Live,
    /// Unreferenced and not marked yet; start the clock.
    StartClock,
    /// Unreferenced but still inside one of the two margins.
    Waiting,
    /// Unreferenced past both margins.
    Collect,
}

pub fn verdict(
    now: chrono::DateTime<chrono::Utc>,
    referenced: bool,
    unreferenced_since: Option<chrono::DateTime<chrono::Utc>>,
    created_at: chrono::DateTime<chrono::Utc>,
) -> Verdict {
    if referenced {
        return Verdict::Live;
    }
    let Some(since) = unreferenced_since else {
        return Verdict::StartClock;
    };
    // Both margins must pass. They guard different races and are not
    // interchangeable: `since` covers "someone may still want this back",
    // `created_at` covers "the upload landed but the page has not saved yet".
    if now - since < UNREFERENCED_GRACE || now - created_at < MIN_OBJECT_AGE {
        return Verdict::Waiting;
    }
    Verdict::Collect
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SweepReport {
    pub workspaces_scanned: i64,
    pub workspaces_skipped: i64,
    pub newly_unreferenced: i64,
    pub re_referenced: i64,
    pub deleted: i64,
    pub bytes_freed: i64,
}

/// Every `file_id` reachable from a document that still exists in [workspace_id].
///
/// "Reachable" means a `views` row points at it — INCLUDING trashed ones
/// (`is_deleted = true`), which is what makes the recycle bin a real grace
/// period. A purged page has no view, so its blobs correctly fall out of the set.
///
/// Returns None if any document could not be read: a partial set would condemn
/// live blobs, so the caller must skip the whole workspace instead.
async fn referenced_file_ids(db: &PgPool, workspace_id: Uuid) -> Option<HashSet<String>> {
    let rows = sqlx::query(
        r#"
          SELECT DISTINCT object_id
          FROM views
          WHERE workspace_id = $1 AND object_type = 'document'
        "#,
    )
    .bind(workspace_id)
    .fetch_all(db)
    .await
    .ok()?;

    let mut refs = HashSet::new();
    for row in rows {
        let document_id: Uuid = row.try_get("object_id").ok()?;
        // A document with no payload yet is empty, not unreadable — it simply
        // contributes nothing. Any other failure poisons the workspace.
        let payload = match store::current_payload(db, document_id).await {
            Ok(Some(payload)) => payload,
            Ok(None) => continue,
            Err(error) => {
                tracing::warn!(%workspace_id, %document_id, %error, "blob gc: unreadable document, skipping workspace");
                return None;
            }
        };
        for block in &payload.blocks {
            if let Some(id) = block.data.get("file_id").and_then(|v| v.as_str()) {
                refs.insert(id.to_string());
            }
        }
    }
    Some(refs)
}

/// Recompute liveness for one workspace and delete what has been dead long
/// enough. Errors are contained: a failure here must never take down the caller.
pub async fn sweep_workspace(
    db: &PgPool,
    storage: &S3Config,
    workspace_id: Uuid,
    dry_run: bool,
    report: &mut SweepReport,
) {
    let http = reqwest::Client::new();
    let Some(referenced) = referenced_file_ids(db, workspace_id).await else {
        report.workspaces_skipped += 1;
        return;
    };
    report.workspaces_scanned += 1;

    let rows = match sqlx::query(
        "SELECT id, object_key, byte_size, created_at, unreferenced_since \
         FROM files WHERE workspace_id = $1",
    )
    .bind(workspace_id)
    .fetch_all(db)
    .await
    {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(%workspace_id, %error, "blob gc: cannot list files");
            report.workspaces_skipped += 1;
            report.workspaces_scanned -= 1;
            return;
        }
    };

    let now = chrono::Utc::now();
    for row in rows {
        let id: Uuid = row.get("id");
        let object_key: String = row.get("object_key");
        let byte_size: i64 = row.get("byte_size");
        let created_at: chrono::DateTime<chrono::Utc> = row.get("created_at");
        let since: Option<chrono::DateTime<chrono::Utc>> = row.get("unreferenced_since");

        match verdict(now, referenced.contains(&id.to_string()), since, created_at) {
            Verdict::Live => {
                // Clear a stale mark: seen alive, so the clock restarts.
                if since.is_some() && !dry_run {
                    let _ = sqlx::query("UPDATE files SET unreferenced_since = NULL WHERE id = $1")
                        .bind(id)
                        .execute(db)
                        .await;
                    report.re_referenced += 1;
                }
                continue;
            }
            Verdict::StartClock => {
                if !dry_run {
                    let _ = sqlx::query("UPDATE files SET unreferenced_since = $2 WHERE id = $1")
                        .bind(id)
                        .bind(now)
                        .execute(db)
                        .await;
                }
                report.newly_unreferenced += 1;
                continue;
            }
            Verdict::Waiting => continue,
            Verdict::Collect => {}
        }

        if dry_run {
            report.deleted += 1;
            report.bytes_freed += byte_size;
            continue;
        }

        // Object first, row second. The crash window then leaves a row whose
        // object is gone — visible, and harmless to re-delete. The other order
        // leaks the object forever with nothing left pointing to it.
        match http.delete(storage.presign_delete(&object_key)).send().await {
            // 404 counts as done: the object is already gone, and leaving the
            // row would strand it forever with nothing to point at.
            Ok(resp) if resp.status().is_success() || resp.status().as_u16() == 404 => {}
            Ok(resp) => {
                tracing::warn!(%workspace_id, %object_key, status = %resp.status(), "blob gc: delete rejected, keeping row");
                continue;
            }
            Err(error) => {
                tracing::warn!(%workspace_id, %object_key, %error, "blob gc: delete failed, keeping row");
                continue;
            }
        }
        if sqlx::query("DELETE FROM files WHERE id = $1")
            .bind(id)
            .execute(db)
            .await
            .is_ok()
        {
            report.deleted += 1;
            report.bytes_freed += byte_size;
            tracing::info!(%workspace_id, %object_key, byte_size, "blob gc: reclaimed");
        }
    }
}

/// Sweep every workspace. [dry_run] reports what would go without touching
/// anything.
pub async fn sweep_all(db: &PgPool, storage: &S3Config, dry_run: bool) -> SweepReport {
    let mut report = SweepReport::default();
    let Ok(rows) = sqlx::query("SELECT id FROM workspaces").fetch_all(db).await else {
        return report;
    };
    for row in rows {
        let workspace_id: Uuid = row.get("id");
        sweep_workspace(db, storage, workspace_id, dry_run, &mut report).await;
    }
    report
}

/// Global row-level lifecycle cleanup that shares the blob GC's cadence but not
/// its object-store concern — it only prunes DB rows that have outlived their
/// purpose and that no per-request path is positioned to reap:
///
///  - Expired refresh tokens, past a 7-day grace beyond `expires_at`. Rotation
///    only ever touches the token being presented, so a family that simply goes
///    quiet (browser closed, device retired) leaves its rows behind forever.
///    The grace keeps a just-expired token around long enough for reuse
///    detection to still fire on a late replay.
///  - Expired AUTO version snapshots. `sync::push_update` prunes these, but only
///    for a document that is itself still being edited (the prune rides its own
///    push cadence); a document that stops changing keeps its expired autos
///    indefinitely. `expires_at IS NOT NULL` is what scopes this to autos —
///    named checkpoints carry a NULL `expires_at` and are never collected.
async fn cleanup_lifecycle_rows(db: &PgPool) {
    match sqlx::query("DELETE FROM refresh_tokens WHERE expires_at < now() - interval '7 days'")
        .execute(db)
        .await
    {
        Ok(result) => {
            if result.rows_affected() > 0 {
                tracing::info!(deleted = result.rows_affected(), "blob gc: pruned expired refresh tokens");
            }
        }
        Err(error) => tracing::warn!(%error, "blob gc: refresh-token cleanup failed"),
    }

    match sqlx::query(
        "DELETE FROM document_yrs_versions WHERE expires_at IS NOT NULL AND expires_at < now()",
    )
    .execute(db)
    .await
    {
        Ok(result) => {
            if result.rows_affected() > 0 {
                tracing::info!(deleted = result.rows_affected(), "blob gc: pruned expired auto versions");
            }
        }
        Err(error) => tracing::warn!(%error, "blob gc: auto-version cleanup failed"),
    }
}

/// Run the sweep forever on [SWEEP_INTERVAL]. Spawned at boot; never panics the
/// server — a GC that fails is a disk-space problem, not an outage.
pub fn spawn(db: PgPool, storage: std::sync::Arc<S3Config>) {
    tokio::spawn(async move {
        // Not immediately on boot: a restart loop would otherwise sweep on every
        // start, and the first pass has nothing urgent to reclaim.
        tokio::time::sleep(Duration::from_secs(5 * 60)).await;
        loop {
            let report = sweep_all(&db, &storage, false).await;
            tracing::info!(
                scanned = report.workspaces_scanned,
                skipped = report.workspaces_skipped,
                newly_unreferenced = report.newly_unreferenced,
                re_referenced = report.re_referenced,
                deleted = report.deleted,
                bytes_freed = report.bytes_freed,
                "blob gc: sweep complete"
            );
            cleanup_lifecycle_rows(&db).await;
            tokio::time::sleep(SWEEP_INTERVAL).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(days_ago: i64) -> chrono::DateTime<chrono::Utc> {
        chrono::Utc::now() - chrono::Duration::days(days_ago)
    }

    #[test]
    fn a_referenced_blob_is_never_collected_however_old() {
        let now = chrono::Utc::now();
        assert_eq!(verdict(now, true, None, t(9999)), Verdict::Live);
        // Even one carrying a stale mark from a previous run: seeing it alive
        // again clears the mark. This is the self-healing property that makes a
        // recomputed set safe where a refcount would not be.
        assert_eq!(verdict(now, true, Some(t(9999)), t(9999)), Verdict::Live);
    }

    #[test]
    fn an_orphan_is_only_marked_on_first_sight_never_deleted() {
        assert_eq!(
            verdict(chrono::Utc::now(), false, None, t(9999)),
            Verdict::StartClock
        );
    }

    #[test]
    fn the_clock_is_time_since_unreferenced_not_the_file_age() {
        let now = chrono::Utc::now();
        // The bug this pins, and the one AFFiNE has: an ancient upload removed
        // from its page yesterday must NOT be collectable. Gating on file age
        // would collect it immediately — the file is a year old, but nobody has
        // had a chance to notice it is gone.
        assert_eq!(verdict(now, false, Some(t(1)), t(365)), Verdict::Waiting);
        // Only once it has been unreferenced long enough.
        assert_eq!(verdict(now, false, Some(t(31)), t(365)), Verdict::Collect);
    }

    #[test]
    fn a_fresh_upload_survives_even_if_long_unreferenced() {
        let now = chrono::Utc::now();
        // Guards the upload→reference window: `complete` writes the row before
        // the page saves, so a brand-new blob looks exactly like an orphan.
        // Impossible in practice (it cannot be unreferenced for 31 days AND be
        // 1 day old) but the margin must not depend on that.
        assert_eq!(verdict(now, false, Some(t(31)), t(1)), Verdict::Waiting);
    }

    #[test]
    fn both_margins_must_pass_to_collect() {
        let now = chrono::Utc::now();
        assert_eq!(verdict(now, false, Some(t(31)), t(31)), Verdict::Collect);
        assert_eq!(verdict(now, false, Some(t(29)), t(31)), Verdict::Waiting);
        assert_eq!(verdict(now, false, Some(t(31)), t(6)), Verdict::Waiting);
    }
}
