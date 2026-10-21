//! Mica MCP server — a thin proxy that exposes the Mica REST API as MCP tools so
//! an AI (Claude Desktop / Code / any MCP client) can list, read, create, and
//! write Mica documents. It calls the API over HTTP with a PAT; it holds no DB
//! or storage access of its own. "Backup/restore" here is just export/import —
//! real backups are the job of external tools pointed at the exports.
//!
//! Shipped as `mica-cli mcp` (this crate is a library; the CLI resolves the
//! server URL + PAT through its usual chain — env, flags, saved login — and
//! calls [serve_stdio]). Read-only mode makes every write tool refuse at call
//! time while the read tools stay listed.
//!
//! Every id here is a `Uuid`, never a `String`. The tools splice ids into
//! request paths, and reqwest resolves a URL the WHATWG way — `..` segments
//! collapse and everything after `#` is dropped — so a string id of
//! `../../../workspaces/<V>#` turned `mica_unshare` into
//! `DELETE /api/workspaces/<V>`, past every confirm/expect_name guard. Every id
//! the API routes on is a UUID, so the type is the whole check: a non-UUID is
//! refused at deserialization, before any tool code (or request) runs, and a
//! `Uuid` can only ever print as hex and hyphens. The schema the model sees is
//! still `"type": "string", "format": "uuid"`.
use rmcp::schemars;
use serde::Deserialize;
use uuid::Uuid;


/// Refuse to inline an image so large it would blow the context it is being
/// read into. MCP has no streaming for ImageContent — it is one base64 string
/// in one tool result — and base64 inflates by 4/3 before the model's encoder
/// ever sees it. A cap with a readable reason beats a 40MB reply that wedges
/// the conversation; the href still works in a browser.
pub(crate) const MAX_INLINE_IMAGE_BYTES: usize = 4 * 1024 * 1024;

// ── Tool parameters ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct WorkspaceArg {
  /// The workspace id (from `mica_list_workspaces`).
  pub(crate) workspace_id: Uuid,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct SearchArgs {
  pub(crate) workspace_id: Uuid,
  /// Free-text query matched against page titles.
  pub(crate) query: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct DocArg {
  pub(crate) workspace_id: Uuid,
  /// The document's object id (a page view's `object_id`).
  pub(crate) document_id: Uuid,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct ReadDocArgs {
  pub(crate) workspace_id: Uuid,
  /// The document's object id (a page view's `object_id`).
  pub(crate) document_id: Uuid,
  /// Read only a WINDOW: 1-based first line to return. Omit to start at line 1.
  /// Windowed reads save tokens on long pages — pull a slice, not the whole doc.
  #[serde(default)]
  pub(crate) offset: Option<usize>,
  /// Read only a WINDOW: max lines to return from `offset`. Omit for the rest.
  #[serde(default)]
  pub(crate) limit: Option<usize>,
  /// Read only the SECTION under a heading whose text contains this (case-
  /// insensitive), up to the next heading of the same or higher level. Takes
  /// precedence over offset/limit. Use `mica_get_outline` to see headings.
  #[serde(default)]
  pub(crate) section: Option<String>,
  /// Force a fresh fetch, bypassing the warm cache. Rarely needed — the cache is
  /// invalidated by your own writes; set this only if the page may have been
  /// edited elsewhere since you last read it.
  #[serde(default)]
  pub(crate) refresh: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct ReadDocumentsArgs {
  pub(crate) workspace_id: Uuid,
  /// The document object ids to read (each a page's `object_id` from
  /// mica_list_pages) — one call reads them all instead of N.
  pub(crate) document_ids: Vec<Uuid>,
  /// "full" (default — each doc's whole Markdown) or "outline" (headings + block
  /// ids + seq, far cheaper). Use "outline" to SCAN structure across many docs
  /// token-cheaply, then read the few you need in full.
  #[serde(default)]
  pub(crate) mode: Option<String>,
}


#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct CreateDocArgs {
  pub(crate) workspace_id: Uuid,
  /// Page title.
  pub(crate) name: String,
  /// Initial body as Markdown. Omit for an empty page.
  #[serde(default)]
  pub(crate) markdown: Option<String>,
  /// Parent view id to nest under (a folder). Omit for a top-level page.
  #[serde(default)]
  pub(crate) parent_view_id: Option<Uuid>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct CreateWorkspaceArgs {
  /// The new workspace's name.
  pub(crate) name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct UpdateDocArgs {
  pub(crate) workspace_id: Uuid,
  /// The document's object id (a page's `object_id` from mica_list_pages) —
  /// NOT its view id (that's for move/trash).
  pub(crate) document_id: Uuid,
  /// One of: "append" (add after current content — safe default),
  /// "replace_all" (rewrite the page), "insert_at" (insert after `anchor`),
  /// "find_replace" (swap text; uses `find`/`replace`, not `markdown`),
  /// "delete" (remove the block at `anchor`, and its subtree; no `markdown`).
  pub(crate) mode: String,
  /// Markdown to write (append/replace_all/insert_at). Omit for find_replace
  /// and delete.
  #[serde(default)]
  pub(crate) markdown: Option<String>,
  /// insert_at: WHERE to insert after — a block id from `mica_get_outline`, OR
  /// a heading's text (resolved to its block id for you). Heading text is safer
  /// than a position: it survives edits above it.
  ///
  /// delete: WHICH block to remove, same two forms — but heading text that
  /// matches more than one heading is refused rather than resolved, and the
  /// error lists the candidates.
  #[serde(default)]
  pub(crate) anchor: Option<String>,
  /// find_replace: the text to find and its replacement.
  #[serde(default)]
  pub(crate) find: Option<String>,
  #[serde(default)]
  pub(crate) replace: Option<String>,
  /// find_replace: require EXACTLY ONE occurrence, else refuse and report the
  /// count — a precise single edit instead of a blind replace-all. Default false
  /// (replace every match, the existing behaviour).
  #[serde(default)]
  pub(crate) unique: bool,
  /// Optimistic concurrency: the `seq` from `mica_get_outline` or a prior write
  /// ack. If given and the doc has changed since, the write is refused (409) so
  /// you re-read first — no silent clobber of a concurrent edit. Omit for the
  /// default last-writer-wins.
  #[serde(default)]
  pub(crate) expected_seq: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct EditOp {
  /// One of: append, replace_all, insert_at, find_replace, delete — same as
  /// mica_update_document's `mode`.
  pub(crate) mode: String,
  /// Markdown to write (append/replace_all/insert_at). Omit for find_replace
  /// and delete.
  #[serde(default)]
  pub(crate) markdown: Option<String>,
  /// insert_at: the block id OR heading text to place after.
  #[serde(default)]
  pub(crate) anchor: Option<String>,
  /// find_replace: text to find / its replacement.
  #[serde(default)]
  pub(crate) find: Option<String>,
  #[serde(default)]
  pub(crate) replace: Option<String>,
  /// find_replace: require EXACTLY ONE match (a precise single edit).
  #[serde(default)]
  pub(crate) unique: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct ApplyEditsArgs {
  pub(crate) workspace_id: Uuid,
  /// The document's object id (a page's `object_id`).
  pub(crate) document_id: Uuid,
  /// The edits to apply IN ORDER — one tool call for a whole burst instead of N.
  /// Each is like a mica_update_document call; later edits see earlier ones.
  pub(crate) edits: Vec<EditOp>,
  /// Optimistic concurrency for the batch START: the seq you last saw (outline /
  /// prior ack). Checked on the FIRST edit only — later edits ride the seq the
  /// batch itself advances.
  #[serde(default)]
  pub(crate) expected_seq: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct MoveDocArgs {
  pub(crate) workspace_id: Uuid,
  /// The page's VIEW id (from `mica_list_pages`), not its object_id.
  pub(crate) view_id: Uuid,
  /// New parent view id (a folder), or omit/null to move to the top level.
  #[serde(default)]
  pub(crate) parent_view_id: Option<Uuid>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct TransferArgs {
  /// The SOURCE workspace id (where the page/folder currently lives).
  pub(crate) workspace_id: Uuid,
  /// The VIEW id of the page or folder to transfer (from `mica_list_pages`).
  pub(crate) view_id: Uuid,
  /// The DESTINATION workspace id (from `mica_list_workspaces`; must differ from
  /// the source).
  pub(crate) dest_workspace_id: Uuid,
  /// Destination parent folder view id; omit/null for the destination root.
  #[serde(default)]
  pub(crate) parent_view_id: Option<Uuid>,
  /// true = COPY (keep the source); false/omit = MOVE (the source is trashed to
  /// the recycle bin after copying).
  #[serde(default)]
  pub(crate) copy: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct CloneArgs {
  /// The workspace id (the copy is created in the SAME workspace).
  pub(crate) workspace_id: Uuid,
  /// The VIEW id of the page or folder to duplicate (from `mica_list_pages`).
  pub(crate) view_id: Uuid,
  /// Destination parent folder view id; omit/null to place the copy beside the
  /// original (under its own parent).
  #[serde(default)]
  pub(crate) parent_view_id: Option<Uuid>,
  /// Name for the copy; omit for a "{source} 副本" default. Deduped against
  /// siblings server-side.
  #[serde(default)]
  pub(crate) name: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct ReorderArgs {
  pub(crate) workspace_id: Uuid,
  /// The folder whose children to reorder (a folder's view id), or omit/null
  /// for the workspace's top level.
  #[serde(default)]
  pub(crate) parent_view_id: Option<Uuid>,
  /// The COMPLETE list of that parent's child VIEW ids, in the order you want
  /// them. Pass every child — leaving one out keeps it at a stale position that
  /// interleaves with the sorted ones.
  pub(crate) ordered_view_ids: Vec<Uuid>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct RenameArgs {
  pub(crate) workspace_id: Uuid,
  /// The VIEW id to rename (a page OR a folder), from `mica_list_pages`.
  pub(crate) view_id: Uuid,
  /// The new name.
  pub(crate) name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct CreateFolderArgs {
  pub(crate) workspace_id: Uuid,
  /// The new folder's name.
  pub(crate) name: String,
  /// Parent folder view id, or omit/null for the workspace's top level.
  #[serde(default)]
  pub(crate) parent_view_id: Option<Uuid>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct ViewArg {
  pub(crate) workspace_id: Uuid,
  /// The VIEW id (page or folder).
  pub(crate) view_id: Uuid,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct BatchViewsArgs {
  pub(crate) workspace_id: Uuid,
  /// The VIEW ids (pages or folders) to act on — one call instead of N. A
  /// folder carries its whole subtree, so list the folder, not its children.
  pub(crate) view_ids: Vec<Uuid>,
  /// Must be true to proceed. Deliberate friction on a bulk destructive call:
  /// the single-view form asks for it too, and here the blast radius is larger.
  #[serde(default)]
  pub(crate) confirm: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct DeleteWorkspaceArgs {
  pub(crate) workspace_id: Uuid,
  /// The workspace's exact name, as `mica_list_workspaces` reports it.
  ///
  /// Not redundant with `workspace_id`, and not ceremony. This is the only tool
  /// here that destroys content nobody deleted first — there is no workspace
  /// recycle bin — and an agent holding thirty workspace UUIDs is one
  /// transposition away from erasing the wrong one. An id is unverifiable by
  /// eye; the name is the thing the user actually said. A mismatch is refused
  /// with both names shown.
  pub(crate) expect_name: String,
  /// Must be true to proceed.
  #[serde(default)]
  pub(crate) confirm: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct ListPagesArgs {
  pub(crate) workspace_id: Uuid,
  /// List only what is under this VIEW id. Omit to start at the top level.
  #[serde(default)]
  pub(crate) parent_view_id: Option<Uuid>,
  /// Levels to descend from there; 1 = direct children only. Omit for all.
  #[serde(default)]
  pub(crate) depth: Option<i32>,
  /// Page size. Omit for everything (no silent ceiling is applied).
  #[serde(default)]
  pub(crate) limit: Option<i64>,
  #[serde(default)]
  pub(crate) offset: Option<i64>,
  /// Add `state_bytes` per page. Small = nearly empty; large does NOT mean
  /// long. Off by default because it joins the content table.
  #[serde(default)]
  pub(crate) with_stats: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct ConfirmArg {
  pub(crate) workspace_id: Uuid,
  /// Must be true to proceed with an unrecoverable delete.
  #[serde(default)]
  pub(crate) confirm: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct BatchRestoreArgs {
  pub(crate) workspace_id: Uuid,
  /// The VIEW ids to bring back out of the recycle bin (from mica_list_trash).
  pub(crate) view_ids: Vec<Uuid>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct BatchMoveArgs {
  pub(crate) workspace_id: Uuid,
  /// The VIEW ids to re-parent, in the order they should end up in.
  pub(crate) view_ids: Vec<Uuid>,
  /// The destination FOLDER's view id. Omit for the workspace root. Only a
  /// folder can hold children; a page id is refused with a readable reason.
  #[serde(default)]
  pub(crate) parent_view_id: Option<Uuid>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct CreateVersionArgs {
  pub(crate) workspace_id: Uuid,
  /// The document's object id (a page view's `object_id`).
  pub(crate) document_id: Uuid,
  /// A label for this checkpoint (e.g. "before the rewrite").
  pub(crate) name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct RestoreVersionArgs {
  pub(crate) workspace_id: Uuid,
  /// The document's object id.
  pub(crate) document_id: Uuid,
  /// The version id to restore to (from `mica_list_versions`).
  pub(crate) version_id: Uuid,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct AddImageArgs {
  pub(crate) workspace_id: Uuid,
  /// A public http(s) URL. The server fetches it once and keeps the bytes;
  /// identical bytes deduplicate to one stored file.
  pub(crate) url: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct FileArgs {
  pub(crate) workspace_id: Uuid,
  /// The file's id — the uuid in an image href's `/files/{file_id}/blob/…`.
  pub(crate) file_id: Uuid,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct TrashArgs {
  pub(crate) workspace_id: Uuid,
  /// The VIEW id to trash (its whole subtree goes to the recycle bin).
  pub(crate) view_id: Uuid,
  /// Must be true to proceed — a guard against accidental deletion.
  pub(crate) confirm: bool,
}

// ── Tools ───────────────────────────────────────────────────────────────────

