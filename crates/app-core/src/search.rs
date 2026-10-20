//! In-process body-text index behind `/search` (FTS M2).
//!
//! M1 (migration 0012) made search one SQL statement over a maintained
//! `content_text` column — and left its cost where the measurement said it was:
//! ~75 ms of an 81 ms single-workspace query is Postgres reading hundreds of
//! TOASTed bodies to run `ILIKE` over them, and the cross-workspace route
//! multiplies that by workspace count (docs/plans/roadmap.md, 性能).
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
//!     and LIMIT. A stale or extra entry here costs a wasted candidate id,
//!     never a wrong answer about a document's workspace or existence — which
//!     is also why entries for purged documents are left to linger until
//!     restart rather than tracked.
//!
//! The SNIPPETS come from here too (2026-10-19). The SQL used to re-read
//! `content_text` for its ≤50 rows just to cut a one-line window out of each:
//! on a cold cache that is every hit's whole body detoasted from disk — 253
//! page reads measured for one query over 3,000 30 KB pages, against zero
//! without the column — paid on exactly the "first search for something new"
//! that felt slow. The text is already in memory, refreshed by the same call
//! that produced the match, so the snippet now agrees with the match by
//! construction instead of by a second read.

use std::{collections::HashMap, future::Future};

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use tokio::sync::{Mutex, RwLock};
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

/// One indexed body.
struct Body {
  /// As stored — what a snippet quotes.
  text: String,
  /// ASCII-lowercased copy, only when it differs from `text` (CJK and
  /// lowercase prose need none, so the common case costs no second copy).
  /// ASCII folding matches what `ILIKE` under the C locale folds, so the scan
  /// agrees with the `v.name ILIKE` half still done in SQL.
  folded: Option<String>,
}

impl Body {
  fn new(text: String) -> Self {
    let folded = text.bytes().any(|b| b.is_ascii_uppercase()).then(|| text.to_ascii_lowercase());
    Self { text, folded }
  }

  fn folded(&self) -> &str {
    self.folded.as_deref().unwrap_or(&self.text)
  }
}

#[derive(Default)]
struct Inner {
  /// `document_id → body`.
  docs: HashMap<Uuid, Body>,
  /// Newest `updated_at` ever loaded — the incremental cursor.
  seen: Option<DateTime<Utc>>,
}

/// The index. One per process, on `AppState`; interior-mutable so routes can
/// share it immutably.
#[derive(Default)]
pub struct BodyIndex {
  /// Serialize fetch-and-apply, so an older DB snapshot cannot finish last and
  /// replace a newer body. Readers of the already-loaded index remain free.
  refresh_lock: Mutex<()>,
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
    self
      .refresh_with(|since| async move {
        // The next cursor comes from the DATABASE clock — comparing a client
        // clock against `updated_at` would smuggle clock skew into visibility.
        let snapshot: (Vec<(Uuid, String)>, DateTime<Utc>) = match since {
          Some(seen) => {
            let rows = sqlx::query_as(
              "SELECT document_id, content_text FROM document_yrs_base
             WHERE updated_at > $1",
            )
            .bind(seen)
            .fetch_all(db)
            .await?;
            if rows.is_empty() {
              // An idle refresh costs one filtered scan; leave the lap open.
              return Ok(None);
            }
            let now: (DateTime<Utc>,) = sqlx::query_as("SELECT now()").fetch_one(db).await?;
            (rows, now.0)
          }
          None => {
            let now: (DateTime<Utc>,) = sqlx::query_as("SELECT now()").fetch_one(db).await?;
            let rows = sqlx::query_as("SELECT document_id, content_text FROM document_yrs_base")
              .fetch_all(db)
              .await?;
            (rows, now.0)
          }
        };
        Ok(Some(snapshot))
      })
      .await
  }

  async fn refresh_with<F, Fut>(&self, fetch: F) -> ApiResult<()>
  where
    F: FnOnce(Option<DateTime<Utc>>) -> Fut,
    Fut: Future<Output = ApiResult<Option<(Vec<(Uuid, String)>, DateTime<Utc>)>>>,
  {
    let _refresh_guard = self.refresh_lock.lock().await;
    let since = self.inner.read().await.seen;
    let Some((rows, db_now)) = fetch(since).await? else {
      return Ok(());
    };
    let lap = chrono::Duration::milliseconds((REFRESH_LAP_SECONDS * 1000.0) as i64);
    let next_seen = db_now - lap;
    let mut inner = self.inner.write().await;
    // Never regress: a cursor that moved backwards would re-open old windows.
    if inner.seen.is_none_or(|s| next_seen > s) {
      inner.seen = Some(next_seen);
    }
    for (document_id, text) in rows {
      inner.docs.insert(document_id, Body::new(text));
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
      .filter(|(_, body)| body.folded().contains(&folded))
      .map(|(id, _)| *id)
      .collect()
  }

  /// One-line snippets around `needle` for these documents, from the indexed
  /// text (see the module doc for why not from the database). Documents with
  /// no body here, or no occurrence of `needle` in it, are simply absent.
  pub async fn snippets(&self, ids: &[Uuid], needle: &str) -> HashMap<Uuid, String> {
    let inner = self.inner.read().await;
    ids
      .iter()
      .filter_map(|id| {
        let body = inner.docs.get(id)?;
        Some((*id, mica_core::search_text::snippet(&body.text, needle)?))
      })
      .collect()
  }

  /// How many documents are indexed (startup log line).
  ///
  /// Not `len`: clippy rightly wants an `is_empty` beside any public `len`,
  /// and this type has no meaning for "empty" worth exposing — an index with
  /// no rows is a database with no documents, which the caller learns from
  /// this count anyway.
  pub async fn indexed_documents(&self) -> usize {
    self.inner.read().await.docs.len()
  }
}

#[cfg(test)]
mod tests {
  use std::{sync::Arc, time::Duration};

  use tokio::{sync::oneshot, time::timeout};

  use super::*;

  #[tokio::test]
  async fn concurrent_refresh_cannot_apply_an_old_snapshot_last() {
    let index = Arc::new(BodyIndex::default());
    let document_id = Uuid::new_v4();
    let old_clock = Utc::now();
    let (old_fetched_tx, old_fetched_rx) = oneshot::channel();
    let (release_old_tx, release_old_rx) = oneshot::channel::<()>();

    let old_index = Arc::clone(&index);
    let old_refresh = tokio::spawn(async move {
      old_index
        .refresh_with(|since| async move {
          assert!(since.is_none());
          old_fetched_tx.send(()).unwrap();
          release_old_rx.await.unwrap();
          Ok(Some((vec![(document_id, "old text".into())], old_clock)))
        })
        .await
        .unwrap();
    });
    old_fetched_rx.await.unwrap();

    // The second fetch is immediately ready. Without refresh serialization it
    // applies its newer body before the deliberately paused first fetch does.
    let new_index = Arc::clone(&index);
    let mut new_refresh = tokio::spawn(async move {
      new_index
        .refresh_with(|_| async move {
          Ok(Some((
            vec![(document_id, "new text".into())],
            old_clock + chrono::Duration::seconds(1),
          )))
        })
        .await
        .unwrap();
    });
    let finished_before_old = timeout(Duration::from_millis(200), &mut new_refresh).await;
    release_old_tx.send(()).unwrap();
    old_refresh.await.unwrap();
    if let Ok(result) = finished_before_old {
      result.unwrap();
    } else {
      new_refresh.await.unwrap();
    }

    assert_eq!(index.matching_ids("new text").await, vec![document_id]);
    assert!(index.matching_ids("old text").await.is_empty());
  }

  /// Matching folds ASCII case; the snippet quotes the text as written.
  #[tokio::test]
  async fn snippets_quote_the_original_casing_of_the_matched_body() {
    let index = BodyIndex::default();
    let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
    index
      .refresh_with(|_| async move {
        Ok(Some((vec![(a, "Deploy with Docker".into()), (b, "部署流程".into())], Utc::now())))
      })
      .await
      .unwrap();
    assert_eq!(index.matching_ids("DOCKER").await, vec![a]);
    let snippets = index.snippets(&[a, b, Uuid::new_v4()], "docker").await;
    assert_eq!(snippets.get(&a).map(String::as_str), Some("Deploy with Docker"));
    assert_eq!(snippets.len(), 1, "no occurrence, no snippet");
  }
}
