//! The REST payloads the API server sends and its Rust clients read, defined
//! once (docs/single-source/01-background.md §9).
//!
//! Before this crate each of these was written twice, as the server's
//! `Serialize` type and again as the CLI's `Deserialize` copy with string
//! timestamps and a subset of the fields. The gold files under `contracts/`
//! caught drift, but only after the fact; now the server and the CLI share one
//! definition, and the gold files guard the remaining client that cannot share
//! it (Dart, whose models carry client-only behaviour).
//!
//! Rules for what lives here:
//! - Only types that cross the wire in BOTH directions of the codebase: the
//!   server serialises them and a Rust client deserialises them. Server-only
//!   rows (`UserRow`, `RefreshTokenRow`) and requests stay where they are.
//! - Fields a client might not get from an older server carry `serde(default)`,
//!   so a newer CLI keeps working against an older server.
//! - `ImportJob` is deliberately absent: it is server state (persisted, with a
//!   status enum the server matches on), and the CLI keeps its status as a
//!   string so an old CLI survives a status added later.
//!
//! The `sqlx` feature derives `FromRow` for the types the server reads straight
//! out of a query.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The signed-in account, as `/auth/*` and `/me` describe it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct User {
  pub id: Uuid,
  pub email: String,
  pub display_name: String,
  pub created_at: DateTime<Utc>,
  /// Changes when the picture does; None means no picture. The client builds the
  /// URL from the user id — sending a URL from here as well would be two
  /// representations of one address, and only one of them could be right.
  #[serde(default)]
  pub avatar_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeResponse {
  pub user: User,
}

/// What a login or token refresh answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuthResponse {
  pub access_token: String,
  pub token_type: String,
  pub expires_at: DateTime<Utc>,
  /// Spend this on `/auth/refresh` for a fresh access token. Single-use: the
  /// refresh hands back a new one. Returned in plaintext exactly once — only
  /// its hash is kept.
  #[serde(default)]
  pub refresh_token: String,
  pub user: User,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct Workspace {
  pub id: Uuid,
  pub name: String,
  pub owner_id: Uuid,
  pub role: String,
  pub created_at: DateTime<Utc>,
  pub updated_at: DateTime<Utc>,
  /// Live pages in this workspace — folders excluded, recycle bin excluded.
  ///
  /// Same definition the client's `countPages` uses, deliberately: the sidebar
  /// switcher already shows a count for the OPEN workspace derived from its
  /// loaded view tree, and a list endpoint answering with a different number for
  /// the same workspace would be worse than answering with none. Folders are not
  /// pages, and trashed rows are not either.
  ///
  /// Only the LIST endpoint fills it in; the single-workspace handlers leave it
  /// 0, so 0 means "not known here", not "empty".
  ///
  /// `sqlx(default)` is what makes that true for queries that omit the column:
  /// `serde(default)` alone governs deserialising JSON, and the derived
  /// `FromRow` would otherwise demand the column (creating a workspace answered
  /// 500 "no column found for name: page_count" until that was added).
  #[serde(default)]
  #[cfg_attr(feature = "sqlx", sqlx(default))]
  pub page_count: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceListResponse {
  pub workspaces: Vec<Workspace>,
}

/// One node of a workspace's page tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct View {
  /// The VIEW id — what move / trash / restore address. Distinct from
  /// [`View::object_id`], the document a view points at, which is what the read
  /// and write endpoints take. Confusing the two is the easiest mistake here.
  pub id: Uuid,
  /// This and the other `serde(default)` fields below are ones the CLI never
  /// read while it had its own copy of this type, so it accepted payloads
  /// without them (its tests' fake servers send just id / object_id /
  /// object_type / name). Defaulting keeps that tolerance: nil / epoch / ""
  /// mean "not sent", never a real value. The server always sends them all.
  #[serde(default)]
  pub workspace_id: Uuid,
  #[serde(default)]
  pub parent_view_id: Option<Uuid>,
  pub object_id: Uuid,
  pub object_type: String,
  pub name: String,
  #[serde(default)]
  pub icon: Option<String>,
  #[serde(default)]
  pub position: String,
  #[serde(default)]
  pub is_deleted: bool,
  #[serde(default)]
  pub created_by: Uuid,
  #[serde(default)]
  pub created_at: DateTime<Utc>,
  #[serde(default)]
  pub updated_at: DateTime<Utc>,
  /// Size in bytes of this page's stored CRDT state, present only when the
  /// listing was asked for `with_stats`. Absent everywhere else, so every other
  /// response keeps its exact shape.
  ///
  /// Read it in ONE direction only: **small means nearly empty**, which is what
  /// makes "find the pages that are just a title" a listing instead of a read
  /// of every page. Large does NOT mean lots of text — a CRDT keeps tombstones,
  /// so a page whose content was deleted stays big. Anyone using this as a word
  /// count will be wrong.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  #[cfg_attr(feature = "sqlx", sqlx(default))]
  pub state_bytes: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewListResponse {
  pub views: Vec<View>,
}

/// What a batch call on views actually did.
///
/// `affected` counts every row the statement touched — for trash and restore
/// that INCLUDES descendants pulled along with a folder, so it is normally
/// larger than the number of ids sent. `skipped` lists requested ids the
/// statement did not reach: already in that state, or gone. Echoing the
/// requested count back would be the lie `reorder_views` was fixed for (P1-3):
/// a partial batch reads as a full one and the caller stops looking.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatchOutcome {
  pub affected: usize,
  #[serde(default)]
  pub skipped: Vec<Uuid>,
}

/// A personal access token as listed (never with its secret).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct TokenInfo {
  pub id: Uuid,
  pub name: String,
  pub scopes: Vec<String>,
  pub created_at: DateTime<Utc>,
  #[serde(default)]
  pub last_used_at: Option<DateTime<Utc>>,
  #[serde(default)]
  pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TokenListResponse {
  pub tokens: Vec<TokenInfo>,
}

/// A freshly minted token, the only response that carries the secret.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreatedToken {
  pub id: Uuid,
  pub name: String,
  pub scopes: Vec<String>,
  /// The secret — shown ONCE, never retrievable again.
  pub token: String,
  pub created_at: DateTime<Utc>,
  #[serde(default)]
  pub expires_at: Option<DateTime<Utc>>,
}

/// The incremental-export inventory: what changed since the caller's last run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportManifest {
  pub version: u32,
  pub tree_revision: String,
  pub structure_revision: String,
  pub manifest: serde_json::Value,
  pub pages: Vec<ExportPageRevision>,
  pub files: Vec<ExportFileRevision>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportPageRevision {
  pub view_id: Uuid,
  pub document_id: Uuid,
  pub revision: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportFileRevision {
  pub file_id: Uuid,
  pub revision: String,
}

/// One image an import was asked to bring in and left as a link — still
/// pointing at somebody else's server. Recorded per occurrence (page + block),
/// and persisted with the import job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageFailure {
  /// The link the block still points at. Also the thing to fetch on a retry.
  pub url: String,
  /// The page it sits on — a list of urls with no page names is a list nobody
  /// can act on.
  #[serde(default)]
  pub page: String,
  /// Where to patch it. `POST /workspaces/{ws}/documents/{document_id}
  /// /rehost-image?block_id=…` takes this pair, which is why they are stored
  /// rather than re-derived by walking every document later.
  pub document_id: Uuid,
  pub block_id: String,
  /// Why it did not land, in the words the fetch itself used.
  pub reason: String,
  /// False when the circuit breaker refused it WITHOUT a request, because its
  /// host had already timed out repeatedly.
  ///
  /// Called out separately because it is the most invisible kind: nothing
  /// failed, it simply never happened, and a per-request error log has no line
  /// to write.
  #[serde(default)]
  pub attempted: bool,
}
