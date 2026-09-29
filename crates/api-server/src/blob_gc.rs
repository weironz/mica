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
use std::time::{Duration, Instant};

use mica_app_core::store;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use mica_infra::storage::S3Config;

/// How long a blob must have been unreferenced before it may be deleted.
///
/// The clock starts at PURGE, not at delete: a page in the recycle bin still
/// owns its `views` row, so its blobs stay referenced (see
/// [referenced_file_ids]). Restore-from-trash is therefore safe by construction
/// rather than by racing a timer.
///
/// Note what that does NOT say: there is no "recycle-bin retention" to add to
/// this window. The bin never empties itself — only the user does, through
/// restore / purge / empty-trash — so a trashed page's blobs are held for
/// [0, until someone clears it), and this grace period starts counting only
/// afterwards. Deliberate, see docs/roadmap.md「回收站不做自动清空」.
///
/// AFFiNE's equivalent misses the reachability half: it walks the root doc with
/// `include_trash = false`, so trashing a page frees its blobs and restoring it
/// returns broken images.
const UNREFERENCED_GRACE: chrono::Duration = chrono::Duration::days(30);

/// A blob must also be this old before it may be deleted, regardless of the
/// above. Covers a different race entirely: the window between `complete`
/// writing the row and the document update that references it arriving. A fresh
/// upload looks exactly like an orphan until the page saves.
const MIN_OBJECT_AGE: chrono::Duration = chrono::Duration::days(7);

/// Browser PUT URLs stay tracked until they have expired, with another day for
/// clock skew and in-flight completion. Newly issued URLs extend the deadline.
const PENDING_UPLOAD_MARGIN: chrono::Duration = chrono::Duration::days(1);
/// A missing file row is not proof that an upload was abandoned immediately.
const PENDING_UPLOAD_GRACE: chrono::Duration = chrono::Duration::days(30);
const PENDING_OBJECT_AGE: chrono::Duration = chrono::Duration::days(1);

/// How often the sweep runs.
const SWEEP_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const PENDING_SWEEP_BATCH: i64 = 200;
const PENDING_SWEEP_MAX_KEYS: usize = 10_000;
const PENDING_SWEEP_TIME_BUDGET: Duration = Duration::from_secs(2 * 60);


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
    /// Eligible unregistered objects found in the pending-upload ledger.
    pub orphan_objects: i64,
    pub orphan_bytes: i64,
}

#[derive(Debug, PartialEq, Eq)]
enum PendingUploadVerdict {
    Waiting,
    Collect,
}

fn pending_upload_verdict(
    now: chrono::DateTime<chrono::Utc>,
    created_at: chrono::DateTime<chrono::Utc>,
    expires_at: chrono::DateTime<chrono::Utc>,
    object_modified_at: chrono::DateTime<chrono::Utc>,
    min_object_age: chrono::Duration,
) -> PendingUploadVerdict {
    if now - created_at < PENDING_UPLOAD_GRACE
        || now - expires_at < PENDING_UPLOAD_MARGIN
        || now - object_modified_at < min_object_age
    {
        return PendingUploadVerdict::Waiting;
    }
    PendingUploadVerdict::Collect
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
                crate::metrics::METRICS.blob_gc_failed();
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
            crate::metrics::METRICS.blob_gc_failed();
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
                crate::metrics::METRICS.blob_gc_failed();
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

/// Reclaim browser PUTs that never became a `files` row. The pending table is
/// intentionally global: deleting a workspace must not erase the only record
/// of an object that was uploaded but never completed.
async fn sweep_pending_uploads(
    db: &PgPool,
    storage: &S3Config,
    dry_run: bool,
    report: &mut SweepReport,
) {
    let http = match reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
    {
        Ok(http) => http,
        Err(error) => {
            tracing::warn!(%error, "blob gc: cannot create storage client");
            crate::metrics::METRICS.blob_gc_failed();
            return;
        }
    };

    let started = Instant::now();
    let mut cursor: Option<(chrono::DateTime<chrono::Utc>, String)> = None;
    let mut scanned = 0usize;
    while scanned < PENDING_SWEEP_MAX_KEYS && started.elapsed() < PENDING_SWEEP_TIME_BUDGET {
        let batch_size = PENDING_SWEEP_BATCH.min((PENDING_SWEEP_MAX_KEYS - scanned) as i64);
        let rows: Vec<(String, chrono::DateTime<chrono::Utc>)> = match sqlx::query_as(
            "SELECT object_key, expires_at FROM pending_file_uploads \
             WHERE created_at <= clock_timestamp() - ($1::bigint * interval '1 second') \
               AND expires_at <= clock_timestamp() - ($2::bigint * interval '1 second') \
               AND ($3::timestamptz IS NULL OR (expires_at, object_key) > ($3, $4)) \
             ORDER BY expires_at, object_key LIMIT $5",
        )
        .bind(PENDING_UPLOAD_GRACE.num_seconds())
        .bind(PENDING_UPLOAD_MARGIN.num_seconds())
        .bind(cursor.as_ref().map(|(expires, _)| *expires))
        .bind(cursor.as_ref().map(|(_, key)| key.as_str()))
        .bind(batch_size)
        .fetch_all(db)
        .await
        {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(%error, "blob gc: cannot page through pending uploads");
                crate::metrics::METRICS.blob_gc_failed();
                return;
            }
        };
        if rows.is_empty() {
            break;
        }
        let full_batch = rows.len() == batch_size as usize;
        for (object_key, expires_at) in rows {
            cursor = Some((expires_at, object_key.clone()));
            scanned += 1;
            if let Err(error) = sweep_pending_key(
                db,
                storage,
                &http,
                &object_key,
                dry_run,
                PENDING_OBJECT_AGE,
                report,
            )
            .await {
                tracing::warn!(%object_key, %error, "blob gc: pending upload sweep failed, keeping ledger row");
                crate::metrics::METRICS.blob_gc_failed();
            }
            if started.elapsed() >= PENDING_SWEEP_TIME_BUDGET {
                break;
            }
        }
        if !full_batch {
            break;
        }
    }
}

async fn sweep_pending_key(
    db: &PgPool,
    storage: &S3Config,
    http: &reqwest::Client,
    object_key: &str,
    dry_run: bool,
    min_object_age: chrono::Duration,
    report: &mut SweepReport,
) -> mica_infra::ApiResult<()> {
    let mut tx = db.begin().await?;
    store::lock_file_object_key(&mut tx, object_key).await?;

    // Recheck under the key lock: presign may have extended the URL lifetime
    // after the outer candidate scan. A completed upload may have gained its
    // file row at the same time. Both paths hold this exact lock.
    let pending: Option<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)> =
        sqlx::query_as(
            "SELECT created_at, expires_at FROM pending_file_uploads WHERE object_key = $1",
        )
        .bind(object_key)
        .fetch_optional(&mut *tx)
        .await?;
    let Some((created_at, expires_at)) = pending else {
        return Ok(());
    };
    let now: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut *tx)
            .await?;
    let registered: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM files WHERE object_key = $1)")
            .bind(object_key)
            .fetch_one(&mut *tx)
            .await?;
    if registered {
        // Keep the ledger while a URL may still be valid. At this point both
        // age margins have passed, so a later file-row deletion cannot turn a
        // still-live URL into an untracked object.
        if now - created_at >= PENDING_UPLOAD_GRACE
            && now - expires_at >= PENDING_UPLOAD_MARGIN
            && !dry_run
        {
            sqlx::query("DELETE FROM pending_file_uploads WHERE object_key = $1")
                .bind(object_key)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
        }
        return Ok(());
    }
    if now - created_at < PENDING_UPLOAD_GRACE || now - expires_at < PENDING_UPLOAD_MARGIN {
        return Ok(());
    }

    let head = http
        .head(storage.presign_head_object(object_key))
        .send()
        .await
        .map_err(|error| mica_infra::ApiError::Internal(format!("orphan HEAD failed: {error}")))?;
    if head.status().as_u16() == 404 {
        if !dry_run {
            sqlx::query("DELETE FROM pending_file_uploads WHERE object_key = $1")
                .bind(object_key)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
        }
        return Ok(());
    }
    if !head.status().is_success() {
        return Err(mica_infra::ApiError::Internal(format!(
            "orphan HEAD returned {}",
            head.status()
        )));
    }
    let headers = head.headers();
    let size: i64 = headers
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| mica_infra::ApiError::Internal("orphan HEAD has no valid size".into()))?;
    let modified = headers
        .get(reqwest::header::LAST_MODIFIED)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| chrono::DateTime::parse_from_rfc2822(value).ok())
        .map(|value| value.with_timezone(&chrono::Utc))
        .ok_or_else(|| mica_infra::ApiError::Internal("orphan HEAD has no valid Last-Modified".into()))?;
    if pending_upload_verdict(now, created_at, expires_at, modified, min_object_age)
        != PendingUploadVerdict::Collect
    {
        return Ok(());
    }
    report.orphan_objects += 1;
    report.orphan_bytes += size;
    if dry_run {
        report.deleted += 1;
        report.bytes_freed += size;
        return Ok(());
    }

    // The lock covers the final DB check, storage DELETE, and ledger cleanup.
    // On a crash after object deletion, the ledger remains and the next sweep
    // observes 404; the opposite order would lose the object forever.
    let deleted = http
        .delete(storage.presign_delete(object_key))
        .send()
        .await
        .map_err(|error| mica_infra::ApiError::Internal(format!("orphan DELETE failed: {error}")))?;
    if !deleted.status().is_success() && deleted.status().as_u16() != 404 {
        return Err(mica_infra::ApiError::Internal(format!(
            "orphan DELETE returned {}",
            deleted.status()
        )));
    }
    sqlx::query("DELETE FROM pending_file_uploads WHERE object_key = $1")
        .bind(object_key)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    report.deleted += 1;
    report.bytes_freed += size;
    tracing::info!(%object_key, byte_size = size, "blob gc: reclaimed uncompleted upload");
    Ok(())
}

/// Sweep every workspace. [dry_run] reports what would go without touching
/// anything.
pub async fn sweep_all(db: &PgPool, storage: &S3Config, dry_run: bool) -> SweepReport {
    let mut report = SweepReport::default();
    match sqlx::query("SELECT id FROM workspaces").fetch_all(db).await {
        Ok(rows) => {
            for row in rows {
                let workspace_id: Uuid = row.get("id");
                sweep_workspace(db, storage, workspace_id, dry_run, &mut report).await;
            }
        }
        Err(error) => {
            tracing::warn!(%error, "blob gc: cannot list workspaces");
            crate::metrics::METRICS.blob_gc_failed();
        }
    }
    sweep_pending_uploads(db, storage, dry_run, &mut report).await;
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
                eligible_orphan_objects = report.orphan_objects,
                eligible_orphan_bytes = report.orphan_bytes,
                "blob gc: sweep complete"
            );
            // Same numbers the log line carries. A sweep that quietly stops
            // reclaiming is a disk problem that only shows up as a disk problem
            // — weeks later, as the incident.
            crate::metrics::METRICS.record_blob_gc(
                report.workspaces_scanned as u64,
                report.deleted as u64,
                report.bytes_freed.max(0) as u64,
                report.orphan_objects as u64,
                report.orphan_bytes.max(0) as u64,
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

    #[test]
    fn pending_upload_waits_for_both_url_expiry_and_upload_age() {
        let now = chrono::Utc::now();
        let old = now - chrono::Duration::days(31);
        let fresh = now - chrono::Duration::hours(1);
        assert_eq!(
            pending_upload_verdict(now, old, now + chrono::Duration::hours(1), old, PENDING_OBJECT_AGE),
            PendingUploadVerdict::Waiting,
            "a newly issued URL extends the deadline even for an old object"
        );
        assert_eq!(
            pending_upload_verdict(now, old, old, fresh, PENDING_OBJECT_AGE),
            PendingUploadVerdict::Waiting,
            "a recent server-side PUT of the same key must survive"
        );
        assert_eq!(
            pending_upload_verdict(now, fresh, old, old, PENDING_OBJECT_AGE),
            PendingUploadVerdict::Waiting,
            "the pending row itself needs the full grace period"
        );
        assert_eq!(
            pending_upload_verdict(now, old, old, old, PENDING_OBJECT_AGE),
            PendingUploadVerdict::Collect
        );
    }

    /// Explicitly run against disposable Postgres and RustFS with DATABASE_URL
    /// and S3_* set. Only these random object keys are touched. The object-age
    /// margin is zero here because a real store cannot backdate Last-Modified;
    /// the production margin is pinned by the pure test above.
    #[tokio::test]
    #[ignore = "requires disposable Postgres and S3-compatible object store"]
    async fn pending_upload_gc_checks_real_objects_and_registration_race() {
        let db = PgPool::connect(&std::env::var("DATABASE_URL").expect("set DATABASE_URL"))
            .await
            .unwrap();
        mica_infra::run_migrations(&db).await.unwrap();
        let storage = S3Config::from_env().expect("set S3_* for a disposable store");
        let http = reqwest::Client::new();
        let workspace_id = Uuid::new_v4();
        let user_id = Uuid::new_v4();
        sqlx::query("INSERT INTO users(id,email,display_name,password_hash) VALUES ($1,$2,'GC','x')")
            .bind(user_id)
            .bind(format!("{user_id}@orphan-gc.test"))
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO workspaces(id,name,owner_id) VALUES ($1,'GC',$2)")
            .bind(workspace_id)
            .bind(user_id)
            .execute(&db)
            .await
            .unwrap();

        let key = |name: &str| format!("workspaces/{workspace_id}/{name}-{}", Uuid::new_v4());
        let abandoned = key("abandoned");
        let active_url = key("active-url");
        let completed = key("completed");
        for object_key in [&abandoned, &active_url, &completed] {
            let put = http
                .put(storage.presign_put_server(object_key).url)
                .header("x-amz-checksum-sha256", "ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0=")
                .body("abc")
                .send()
                .await
                .unwrap();
            assert!(put.status().is_success(), "test PUT: {}", put.status());
            let checksum_head = http
                .head(storage.presign_head_object_with_checksum(object_key))
                .header("x-amz-checksum-mode", "ENABLED")
                .send()
                .await
                .unwrap();
            assert_eq!(
                checksum_head
                    .headers()
                    .get("x-amz-checksum-sha256")
                    .and_then(|value| value.to_str().ok()),
                Some("ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0=")
            );
            let mut tx = db.begin().await.unwrap();
            store::lock_file_object_key(&mut tx, object_key).await.unwrap();
            store::record_pending_file_upload_tx(&mut tx, workspace_id, object_key, 900)
                .await
                .unwrap();
            tx.commit().await.unwrap();
        }
        let first_expiry: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
            "SELECT expires_at FROM pending_file_uploads WHERE object_key = $1",
        )
        .bind(&active_url)
        .fetch_one(&db)
        .await
        .unwrap();
        store::record_pending_file_upload(&db, workspace_id, &active_url, 3600)
            .await
            .unwrap();
        let extended_expiry: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
            "SELECT expires_at FROM pending_file_uploads WHERE object_key = $1",
        )
        .bind(&active_url)
        .fetch_one(&db)
        .await
        .unwrap();
        assert!(extended_expiry > first_expiry, "a new URL must extend GC protection");
        sqlx::query(
            "UPDATE pending_file_uploads SET created_at = now() - interval '31 days', \
             expires_at = now() - interval '2 days' WHERE object_key = ANY($1)",
        )
        .bind(vec![abandoned.clone(), completed.clone()])
        .execute(&db)
        .await
        .unwrap();
        sqlx::query(
            "UPDATE pending_file_uploads SET created_at = now() - interval '31 days', \
             expires_at = now() + interval '1 hour' WHERE object_key = $1",
        )
        .bind(&active_url)
        .execute(&db)
        .await
        .unwrap();

        let mut dry = SweepReport::default();
        sweep_pending_key(
            &db,
            &storage,
            &http,
            &abandoned,
            true,
            chrono::Duration::zero(),
            &mut dry,
        )
        .await
        .unwrap();
        assert_eq!((dry.orphan_objects, dry.orphan_bytes, dry.deleted), (1, 3, 1));
        assert!(http.head(storage.presign_head_object(&abandoned)).send().await.unwrap().status().is_success());

        let mut actual = SweepReport::default();
        sweep_pending_key(
            &db,
            &storage,
            &http,
            &abandoned,
            false,
            chrono::Duration::zero(),
            &mut actual,
        )
        .await
        .unwrap();
        assert_eq!((actual.orphan_objects, actual.deleted, actual.bytes_freed), (1, 1, 3));
        assert_eq!(http.head(storage.presign_head_object(&abandoned)).send().await.unwrap().status().as_u16(), 404);
        let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM pending_file_uploads WHERE object_key = $1")
            .bind(&abandoned)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(pending, 0);

        let mut active_report = SweepReport::default();
        sweep_pending_key(
            &db,
            &storage,
            &http,
            &active_url,
            false,
            chrono::Duration::zero(),
            &mut active_report,
        )
        .await
        .unwrap();
        assert_eq!(active_report.deleted, 0);
        assert!(http.head(storage.presign_head_object(&active_url)).send().await.unwrap().status().is_success());

        // `complete` has inserted but not committed yet. GC must wait on the
        // same key lock, then observe the row after commit and leave the object.
        let mut completion = db.begin().await.unwrap();
        store::lock_file_object_key(&mut completion, &completed).await.unwrap();
        store::insert_file_tx(&mut completion, workspace_id, user_id, &completed, "c.png", "image/png", 3)
            .await
            .unwrap();
        let gc_db = db.clone();
        let gc_storage = storage.clone();
        let gc_http = http.clone();
        let gc_key = completed.clone();
        let mut gc = tokio::spawn(async move {
            let mut report = SweepReport::default();
            sweep_pending_key(
                &gc_db,
                &gc_storage,
                &gc_http,
                &gc_key,
                false,
                chrono::Duration::zero(),
                &mut report,
            )
            .await
            .unwrap();
            report
        });
        assert!(tokio::time::timeout(Duration::from_millis(100), &mut gc).await.is_err());
        completion.commit().await.unwrap();
        assert_eq!(gc.await.unwrap().deleted, 0);
        assert!(http.head(storage.presign_head_object(&completed)).send().await.unwrap().status().is_success());

        for object_key in [&active_url, &completed] {
            let deletion = http.delete(storage.presign_delete(object_key)).send().await.unwrap();
            assert!(deletion.status().is_success(), "test cleanup: {}", deletion.status());
        }
        sqlx::query("DELETE FROM pending_file_uploads WHERE workspace_id = $1")
            .bind(workspace_id)
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("DELETE FROM workspaces WHERE id = $1")
            .bind(workspace_id)
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(user_id)
            .execute(&db)
            .await
            .unwrap();
    }
}
