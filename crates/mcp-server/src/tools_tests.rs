//! Tool-inventory and parameter-validation tests for the `tools_*.rs` layer
//! (issue #86).
//!
//! The other suites bracket this layer but never land on it: handshake_tests
//! pins the wire shape (count, schemas, capabilities) and a couple of
//! behaviours; wire_tests drives real tools/call traffic against a fake API and
//! so only ever exercises paths that pass validation. Nothing asserted WHICH
//! 39 tools are listed — an unmerged router that swaps one tool for another
//! keeps the count at 39 and every existing test green — and nothing exercised
//! the validation that runs BEFORE any HTTP call. This file does both, against
//! a base URL that refuses every connection (`example.invalid`): a refusal with
//! the guard's own message proves the check ran locally, and "Mica request
//! failed" in a message proves the call reached the network.
use rmcp::handler::server::wrapper::Parameters;
use serde_json::json;
use uuid::Uuid;

use crate::args::*;
use crate::*;

/// Any request to this base fails DNS immediately — the marker that a call
/// escaped local validation and hit the network.
const NETWORK_ERROR: &str = "Mica request failed";

fn server() -> MicaMcp {
  MicaMcp::new("https://example.invalid".into(), "pat".into(), false)
}

fn text(result: &rmcp::model::CallToolResult) -> String {
  format!("{:?}", result.content)
}

// ── Tool inventory ──────────────────────────────────────────────────────────

/// The count is already pinned at 39 (handshake_tests), but a COUNT cannot see
/// a tool silently swapped out of a router merge: `full_tool_router` merges five
/// separately `#[tool_router]`-built pieces, and dropping a tool from one while
/// adding one to another still totals 39. The lib.rs merge comment says the
/// failure mode is "caught by the CI MCP-conformance job, not by compilation" —
/// this is the unit-level pin of the exact set, so the failure shows up in
/// `cargo test`, not only in the conformance job.
#[test]
fn the_manifest_is_exactly_these_39_tools() {
  let mut expected: Vec<&str> = [
    // read_router
    "mica_list_workspaces",
    "mica_list_pages",
    "mica_search",
    "mica_read_document",
    "mica_read_documents",
    "mica_get_outline",
    // write_router
    "mica_create_document",
    "mica_create_workspace",
    "mica_update_document",
    "mica_apply_edits",
    "mica_add_image",
    "mica_read_image",
    // organize_router
    "mica_move_document",
    "mica_transfer_view",
    "mica_clone_document",
    "mica_reorder",
    "mica_rename",
    "mica_create_folder",
    "mica_list_trash",
    "mica_restore_view",
    "mica_list_versions",
    "mica_create_version",
    "mica_restore_version",
    // trash_router
    "mica_trash_view",
    "mica_trash_views",
    "mica_restore_views",
    "mica_move_views",
    "mica_empty_trash",
    "mica_purge_view",
    "mica_purge_views",
    // admin_router
    "mica_delete_workspace",
    "mica_share",
    "mica_unshare",
    "mica_export_workspace",
    // collab_router
    "mica_list_comments",
    "mica_delete_comment",
    "mica_list_members",
    "mica_remove_member",
    "mica_delete_file",
  ]
  .to_vec();
  expected.sort_unstable();

  let mut actual: Vec<String> = MicaMcp::full_tool_router()
    .list_all()
    .iter()
    .map(|t| t.name.to_string())
    .collect();
  actual.sort_unstable();
  // Sorting both makes a duplicate visible too: a name registered in two
  // routers would push the vector one longer than `expected`.
  assert_eq!(
    actual, expected,
    "a tool was added, dropped, or duplicated without updating the manifest"
  );
}

/// The description is the model's ONLY knowledge of what a tool does (see the
/// mica_search doc comment: a wrong one hid body-text search from every agent).
/// A tool listed with an empty description is discoverable-but-unused — the
/// exact failure the get_info instructions warn about.
#[test]
fn every_tool_has_a_nonempty_description() {
  for t in MicaMcp::full_tool_router().list_all() {
    let description = t.description.as_deref().unwrap_or("").trim();
    assert!(
      !description.is_empty(),
      "{}: a tool without a description cannot be used purposefully",
      t.name
    );
  }
}

/// `read_only_hint` is how a client (and a cautious model) tells which calls
/// cannot mutate. The flag is per-tool metadata set by hand, so it can drift —
/// a mutating tool mis-flagged read-only invites a client to skip its guard
/// rails. Pin the exact set.
#[test]
fn read_only_hints_flag_exactly_the_read_tools() {
  let mut expected: Vec<&str> = [
    "mica_list_workspaces",
    "mica_list_pages",
    "mica_search",
    "mica_read_document",
    "mica_read_documents",
    "mica_get_outline",
    "mica_read_image",
    "mica_list_trash",
    "mica_list_versions",
    "mica_export_workspace",
    "mica_list_comments",
    "mica_list_members",
  ]
  .to_vec();
  expected.sort_unstable();

  let mut actual: Vec<String> = MicaMcp::full_tool_router()
    .list_all()
    .iter()
    .filter(|t| {
      t.annotations
        .as_ref()
        .and_then(|a| a.read_only_hint)
        .unwrap_or(false)
    })
    .map(|t| t.name.to_string())
    .collect();
  actual.sort_unstable();
  assert_eq!(actual, expected);
}

/// The counterpart of the confirm/expect_name guards: `destructive_hint` must
/// be on every tool that can discard content, so a client that surfaces those
/// hints warns in the right places. The confirm guards are exercised in
/// wire_tests; the flag that makes a client ASK is pinned here.
#[test]
fn destructive_hints_flag_exactly_the_content_discarding_tools() {
  let mut expected: Vec<&str> = [
    // Rewrites or removes parts of a page.
    "mica_update_document",
    "mica_apply_edits",
    "mica_restore_version",
    // The recycle bin and what empties it.
    "mica_trash_view",
    "mica_trash_views",
    "mica_empty_trash",
    "mica_purge_view",
    "mica_purge_views",
    // The one call with no bin behind it at all.
    "mica_delete_workspace",
    // The collaboration surface: none of these has a bin either.
    "mica_delete_comment",
    "mica_remove_member",
    "mica_delete_file",
  ]
  .to_vec();
  expected.sort_unstable();

  let mut actual: Vec<String> = MicaMcp::full_tool_router()
    .list_all()
    .iter()
    .filter(|t| {
      t.annotations
        .as_ref()
        .and_then(|a| a.destructive_hint)
        .unwrap_or(false)
    })
    .map(|t| t.name.to_string())
    .collect();
  actual.sort_unstable();
  assert_eq!(actual, expected);
}

// ── Parameter validation (before any HTTP) ──────────────────────────────────

/// An empty batch would round-trip to the API and come back as a bare 400 —
/// refused locally instead, with a reason the model can act on.
#[tokio::test]
async fn an_empty_edit_batch_is_refused_before_any_network() {
  let result = server()
    .mica_apply_edits(Parameters(ApplyEditsArgs {
      workspace_id: Uuid::nil(),
      document_id: Uuid::nil(),
      edits: vec![],
      expected_seq: None,
    }))
    .await
    .expect("validation must be a tool error, not a protocol error");
  assert_eq!(result.is_error, Some(true));
  let text = text(&result);
  assert!(text.contains("edits is empty"), "names the fix: {text}");
  assert!(
    !text.contains(NETWORK_ERROR),
    "must not reach the network: {text}"
  );
}

#[tokio::test]
async fn an_empty_batch_read_is_refused_before_any_network() {
  let result = server()
    .mica_read_documents(Parameters(ReadDocumentsArgs {
      workspace_id: Uuid::nil(),
      document_ids: vec![],
      mode: None,
    }))
    .await
    .expect("validation must be a tool error, not a protocol error");
  assert_eq!(result.is_error, Some(true));
  let text = text(&result);
  assert!(
    text.contains("document_ids is empty"),
    "names the fix: {text}"
  );
  assert!(
    !text.contains(NETWORK_ERROR),
    "must not reach the network: {text}"
  );
}

/// The batch-read ceiling mirrors the server's own MAX_BATCH_VIEWS so the list
/// is refused with a readable message instead of a 400 after the round trip
/// (see args.rs). 1000 is the documented ceiling: it must be the boundary, not
/// the refusal — the 1000-id call escapes to the network, the 1001st never
/// leaves the process.
#[tokio::test]
async fn an_over_large_batch_read_is_refused_at_exactly_the_ceiling() {
  fn ids(n: usize) -> Vec<Uuid> {
    (0..n).map(|i| Uuid::from_u128(i as u128 + 1)).collect()
  }
  let server = server();

  let at_ceiling = server
    .mica_read_documents(Parameters(ReadDocumentsArgs {
      workspace_id: Uuid::nil(),
      document_ids: ids(MAX_BATCH_DOCUMENTS),
      mode: None,
    }))
    .await
    .expect("an accepted batch must not be a protocol error");
  assert_eq!(at_ceiling.is_error, Some(true), "the API is unreachable");
  assert!(
    text(&at_ceiling).contains(NETWORK_ERROR),
    "exactly {MAX_BATCH_DOCUMENTS} ids must pass the local check: {}",
    text(&at_ceiling)
  );

  let over = server
    .mica_read_documents(Parameters(ReadDocumentsArgs {
      workspace_id: Uuid::nil(),
      document_ids: ids(MAX_BATCH_DOCUMENTS + 1),
      mode: None,
    }))
    .await
    .expect("validation must be a tool error, not a protocol error");
  assert_eq!(over.is_error, Some(true));
  let text = text(&over);
  assert!(
    text.contains("at most 1000 document_ids per call, got 1001"),
    "the readable refusal, with the count: {text}"
  );
  assert!(
    !text.contains(NETWORK_ERROR),
    "must not reach the network: {text}"
  );
}

/// The latex guard fires on EVERY authored-content field of EVERY write path,
/// ahead of any read or write request: `mica_create_document`'s markdown
/// (pinned in handshake_tests), `mica_update_document`'s markdown, a
/// find_replace `replace` (the swapped-in formula is mangled by the same
/// under-escaping), and each op inside an `mica_apply_edits` batch. unique=false
/// keeps find_replace from touching the export endpoint, so a refusal here can
/// only have come from the guard.
#[tokio::test]
async fn mangled_latex_is_refused_in_every_write_path_before_any_network() {
  // tab + "imes" — exactly what an under-escaped `\times` decodes to.
  let mangled = || format!("$x {}imes y$", '\u{0009}');
  let server = server();

  let update = server
    .mica_update_document(Parameters(UpdateDocArgs {
      workspace_id: Uuid::nil(),
      document_id: Uuid::nil(),
      mode: "append".to_string(),
      markdown: Some(mangled()),
      anchor: None,
      find: None,
      replace: None,
      replacement_format: "markdown".to_string(),
      unique: false,
      expected_seq: None,
    }))
    .await
    .expect("validation must be a tool error, not a protocol error");
  assert_eq!(update.is_error, Some(true));
  assert!(text(&update).contains("doubled"), "says how to fix it");

  let find_replace = server
    .mica_update_document(Parameters(UpdateDocArgs {
      workspace_id: Uuid::nil(),
      document_id: Uuid::nil(),
      mode: "find_replace".to_string(),
      markdown: None,
      anchor: None,
      find: Some("old".to_string()),
      replace: Some(mangled()),
      replacement_format: "markdown".to_string(),
      unique: false,
      expected_seq: None,
    }))
    .await
    .expect("validation must be a tool error, not a protocol error");
  assert_eq!(find_replace.is_error, Some(true));
  let fr_text = text(&find_replace);
  assert!(fr_text.contains("doubled"), "says how to fix it: {fr_text}");
  assert!(
    !fr_text.contains(NETWORK_ERROR),
    "must not reach the network: {fr_text}"
  );

  let batch = server
    .mica_apply_edits(Parameters(ApplyEditsArgs {
      workspace_id: Uuid::nil(),
      document_id: Uuid::nil(),
      edits: vec![EditOp {
        mode: "append".to_string(),
        markdown: Some(mangled()),
        anchor: None,
        find: None,
        replace: None,
        replacement_format: "markdown".to_string(),
        unique: false,
      }],
      expected_seq: None,
    }))
    .await
    .expect("validation must be a tool error, not a protocol error");
  assert_eq!(batch.is_error, Some(true));
  let batch_text = text(&batch);
  assert!(
    batch_text.contains("doubled"),
    "says how to fix it: {batch_text}"
  );
  assert!(
    !batch_text.contains(NETWORK_ERROR),
    "must not reach the network: {batch_text}"
  );
}

// ── args.rs serde / default behaviour ───────────────────────────────────────

/// Batch-read mode is a free STRING, and the code only special-cases exactly
/// "outline" — anything else (including a typo) is a full read, the expensive
/// path. Pin both halves so a silent downgrade is a deliberate contract, not an
/// accident, and so tightening it later (refusing unknown modes) is a visible
/// decision.
#[test]
fn batch_read_mode_defaults_to_full_and_only_outline_is_outlined() {
  let plain: ReadDocumentsArgs = serde_json::from_value(json!({
    "workspace_id": Uuid::nil(),
    "document_ids": [Uuid::nil()],
  }))
  .expect("mode is optional");
  assert_eq!(plain.mode.as_deref(), None, "omitted mode is a full read");

  for (mode, outlined) in [("outline", true), ("full", false), ("Outline", false)] {
    let parsed: ReadDocumentsArgs = serde_json::from_value(json!({
      "workspace_id": Uuid::nil(),
      "document_ids": [Uuid::nil()],
      "mode": mode,
    }))
    .unwrap();
    assert_eq!(
      parsed.mode.as_deref() == Some("outline"),
      outlined,
      "mode={mode:?}: {}",
      if outlined {
        "the cheap path"
      } else {
        "falls to a full read (case-sensitive, exact match)"
      }
    );
  }
}

/// `confirm` on the BATCH destructive tools defaults to false: omitting it is a
/// refusal, not a deserialisation error — a uniform "forgot to confirm" story
/// whatever the client sends. (The single-view `TrashArgs::confirm` has no
/// default on purpose: the key itself is required there, and wire_tests pins
/// that asymmetry from the wire side.)
#[test]
fn confirm_defaults_to_false_on_the_batch_destructive_tools() {
  let batch: BatchViewsArgs = serde_json::from_value(json!({
    "workspace_id": Uuid::nil(),
    "view_ids": [Uuid::nil()],
  }))
  .expect("an omitted confirm must still deserialise");
  assert!(!batch.confirm, "omitted confirm refuses by default");

  let bin: ConfirmArg = serde_json::from_value(json!({
    "workspace_id": Uuid::nil(),
  }))
  .expect("an omitted confirm must still deserialise");
  assert!(!bin.confirm, "omitted confirm refuses by default");
}
