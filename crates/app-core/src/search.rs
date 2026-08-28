//! In-process body-text index behind `/search` (FTS M2).
//!
//! M1 (migration 0012) made search one SQL statement over a maintained
//! `content_text` column — and left its cost where the measurement said it was:
//! ~75 ms of an 81 ms single-workspace query is Postgres reading hundreds of
//! TOASTed bodies to run `ILIKE` over them, and the cross-workspace route
//! multiplies that by workspace count (docs/roadmap.md, 性能).
//!
//! Why not a Postgres text index (researched 2026-08-28, see the roadmap entry):
//!   - `pg_trgm` fails BOTH ways here. A 1–2 char needle has no extractable
//!     trigram, so `%xx%` degenerates to a full-index scan — 1–2 汉字 queries
//!     are the common case, and that limit is the 3-gram model itself, not a
//!     tuning matter. Separately, trigram extraction asks libc whether a char
//!     is alphanumeric, and under the C locale our `postgres:16-alpine` runs,
//!     CJK is not — the index would hold no CJK trigrams at all, and LC_CTYPE
//!     is fixed at initdb.
//!   - `pg_bigm` / PGroonga fix exactly that but are external extensions with
//!     no Alpine package: adopting one means maintaining a custom Postgres
//!     image, which the deploy pins against.
//!   - Peers dodge the problem rather than solve it: AppFlowy Cloud searches
//!     pgvector embeddings (no full-text at all), AFFiNE ships its own
//!     in-memory index. Nobody self-hosts CJK full-text on stock Postgres.
//!
//! So the body scan moves into the process: a map of `document_id → folded
//! body text`, refreshed INCREMENTALLY at query time by `updated_at` cursor,
//! scanned with plain substring search. ~10k documents is tens of MB — memory
//! a single-node server can spend — and a linear scan of that is milliseconds,
//! independent of how many workspaces the needle spans. 1-char queries cost
//! the same as 10-char ones.
//!
//! What this deliberately is NOT:
//!   - Not a write-path hook. `content_text` is written inside transactions at
//!     three sites across two crates; a cache invalidated from each of them is
//!     the double-representation drift this repo keeps relearning (red line
//!     #1). Pull-based refresh has one choke point — here — and staleness is
//!     bounded by the overlap lap below, not by nobody forgetting a call site.
//!   - Not the authority on anything. The scan yields CANDIDATE ids; the SQL
//!     in `search_views` still applies workspace scope, `is_deleted`, ranking
//!     and LIMIT, and re-reads `content_text` for the ≤50 rows it returns (the
//!     snippet source stays the database column). A stale or extra entry here
//!     costs a wasted candidate id, never a wrong answer about a document's
//!     workspace or existence — which is also why entries for purged documents
//!     are left to linger until restart rather than tracked.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use tokio::sync::RwLock;
use uuid::Uuid;

use mica_infra::ApiResult;

/// Safety lap for the refresh cursor. `updated_at` is `now()` *of the writing
/// transaction's start*, so a row can become visible bearing a timestamp older
/// than rows already indexed; the cursor therefore trails the DATABASE clock by
/// this much instead of standing on the newest timestamp seen. A row is missed
/// only if its transaction ran longer than this before committing — push
/// transactions here are per-request and short.
///
/// Trailing the CLOCK, not the data, is what makes the window close: the first
/// cut trailed `max(updated_at)`, and a bulk import writes thousands of rows
/// sharing one timestamp — every one of them inside its own lap, re-pulled in
/// full on every search forever (measured: the whole 21MB corpus per query).
/// Clock-relative, the same import is re-pulled for five seconds and then
/// never again.
const REFRESH_LAP_SECONDS: f64 = 5.0;

#[derive(Default)]
struct Inner {
  /// `document_id → ASCII-lowercased body text`. Folded at load so the scan is
  /// one `contains` per document; ASCII folding matches what `ILIKE` under the
  /// C locale folds (ASCII only), so the scan agrees with the `v.name ILIKE`
  /// half still done in SQL. CJK has no case to fold.
  docs: HashMap<Uuid, String>,
  /// Newest `updated_at` ever loaded — the incremental cursor.
  seen: Option<DateTime<Utc>>,
}

/// The index. One per process, on `AppState`; interior-mutable so routes can
/// share it immutably.
#[derive(Default)]
pub struct BodyIndex {
  inner: RwLock<Inner>,
}

impl BodyIndex {
  /// Pull every row changed since the last refresh (all of them, first time).
  ///
  /// Called by `search_views` before each scan and once at startup to take the
  /// full load off the first search. Errors propagate: a search that cannot
  /// refresh must fail visibly, not answer from an arbitrarily old snapshot.
  ///
  /// The `WHERE updated_at` filter walks row headers without touching TOASTed
  /// bodies of non-matching rows, so the steady-state cost is a scan of ~10k
  /// timestamps returning nothing.
  pub async fn refresh(&self, db: &PgPool) -> ApiResult<()> {
    // Snapshot the cursor without holding the write lock across the query.
    let since = { self.inner.read().await.seen };
    // The next cursor comes from the DATABASE clock — comparing a client clock
    // against `updated_at` would smuggle clock skew into the visibility rule.
    let (rows, db_now): (Vec<(Uuid, String)>, DateTime<Utc>) = match since {
      Some(seen) => {
        let rows = sqlx::query_as(
          "SELECT document_id, content_text FROM document_yrs_base
           WHERE updated_at > $1",
        )
        .bind(seen)
        .fetch_all(db)
        .await?;
        if rows.is_empty() {
          // Nothing new: keep the cursor where it is (advancing it needs a
          // clock read; an idle refresh should cost one filtered scan, full
          // stop). The lap window stays open until a write moves it.
          return Ok(());
        }
        let now: (DateTime<Utc>,) = sqlx::query_as("SELECT now()").fetch_one(db).await?;
        (rows, now.0)
      }
      None => {
        let now: (DateTime<Utc>,) = sqlx::query_as("SELECT now()").fetch_one(db).await?;
        let now = now.0;
        let rows =
          sqlx::query_as("SELECT document_id, content_text FROM document_yrs_base")
            .fetch_all(db)
            .await?;
        (rows, now)
      }
    };
    let lap = chrono::Duration::milliseconds((REFRESH_LAP_SECONDS * 1000.0) as i64);
    let next_seen = db_now - lap;
    let mut inner = self.inner.write().await;
    // Never regress: a cursor that moved backwards would re-open old windows.
    if inner.seen.is_none_or(|s| next_seen > s) {
      inner.seen = Some(next_seen);
    }
    for (document_id, text) in rows {
      inner.docs.insert(document_id, text.to_ascii_lowercase());
    }
    Ok(())
  }

  /// Ids of documents whose body contains `needle` (ASCII-case-insensitively).
  ///
  /// Plain substring match — the same semantics `content_text ILIKE '%…%'` had,
  /// including single-character CJK needles, which is the case every indexing
  /// scheme above stumbled on.
  pub async fn matching_ids(&self, needle: &str) -> Vec<Uuid> {
    let folded = needle.to_ascii_lowercase();
    let inner = self.inner.read().await;
    inner
      .docs
      .iter()
      .filter(|(_, text)| text.contains(&folded))
      .map(|(id, _)| *id)
      .collect()
  }

  /// How many documents are indexed (startup log line).
  pub async fn len(&self) -> usize {
    self.inner.read().await.docs.len()
  }
}
