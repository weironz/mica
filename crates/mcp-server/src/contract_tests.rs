//! Gold-file contract tests (#85): run the REAL response-shaping code over
//! `/contracts/*.json` — produced by the server's DTO types — and assert the
//! fields the tools read.
//!
//! The mcp server deliberately has no DTO structs of its own: it proxies
//! `serde_json::Value` and picks fields out by pointer (`/view/id`,
//! `/snapshot/payload/blocks`, …). That is exactly why it needs this test — a
//! server-side rename compiles green here and silently breaks every tool that
//! reads the renamed field.

use rmcp::model::CallToolResult;
use serde_json::Value;

use crate::results::{slim_pages, write_ack};

/// The repo-root `contracts/` dir, from this crate's manifest dir.
fn gold(name: &str) -> Value {
  let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
    .join("../../contracts")
    .join(format!("{name}.json"));
  let raw = std::fs::read_to_string(&path)
    .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()));
  serde_json::from_str(&raw).expect("gold file is valid JSON")
}

/// The JSON a tool result carries — the pretty text the model actually reads.
fn result_json(result: CallToolResult) -> Value {
  let text = result
    .content
    .iter()
    .find_map(|content| content.as_text())
    .expect("a text content block");
  serde_json::from_str(&text.text).expect("tool result text is JSON")
}

/// `mica_list_pages` answers with `slim_pages(view list)`: the fields a model
/// navigates by survive, the storage bookkeeping does not.
#[test]
fn slim_pages_keeps_the_navigable_fields_of_the_gold_view_list() {
  let slimmed = slim_pages(gold("view_list"));

  let pages = slimmed["pages"].as_array().expect("pages array");
  assert_eq!(pages.len(), 3, "one row per view: {slimmed}");

  let folder = &pages[0];
  assert_eq!(folder["id"], "33333333-3333-4333-8333-333333333333");
  assert_eq!(folder["object_id"], "33333333-3333-4333-8333-333333333333");
  assert_eq!(folder["object_type"], "folder");
  assert_eq!(folder["name"], "Gold Folder");
  assert_eq!(
    folder["icon"], "📗",
    "an emoji is kept — a null would be dropped"
  );
  assert!(
    folder.get("workspace_id").is_none()
      && folder.get("position").is_none()
      && folder.get("created_by").is_none()
      && folder.get("is_deleted").is_none(),
    "the storage bookkeeping is dropped: {folder}"
  );

  let stats = &pages[1];
  assert_eq!(
    stats["state_bytes"], 4096,
    "state_bytes rides along when asked for"
  );
  assert_eq!(
    stats["parent_view_id"],
    "33333333-3333-4333-8333-333333333333"
  );

  let plain = &pages[2];
  assert!(
    plain.get("icon").is_none() && plain.get("state_bytes").is_none(),
    "absent server fields stay absent, not null: {plain}"
  );
}

/// `write_ack` over the gold bootstrap: the editor-facing payload carries
/// document + view + snapshot, so the ack must recognise ALL of it — the ids,
/// the block count, the last top-level block (the anchor for the next
/// insert_at) and the seq.
#[test]
fn write_ack_reads_every_field_out_of_the_gold_bootstrap() {
  let ack = result_json(write_ack(gold("document_bootstrap")).expect("ack result"));
  assert_eq!(ack["ok"], true);
  assert_eq!(ack["document_id"], "66666666-6666-4666-8666-666666666666");
  assert_eq!(ack["view_id"], "44444444-4444-4444-8444-444444444444");
  assert_eq!(ack["name"], "Bootstrapped Page");
  assert_eq!(ack["blocks"], 3, "a count, not the tree");
  assert_eq!(ack["last_block_id"], "block_b", "the root's LAST child");
}

/// A create answer carries document + view but no snapshot, and an update
/// answer document + snapshot + update but no view — the ack must degrade to
/// whichever subset is present rather than demand the whole shape.
#[test]
fn write_ack_degrades_to_whichever_subset_the_endpoint_returns() {
  let create = result_json(write_ack(gold("document_create")).expect("ack result"));
  assert_eq!(create["view_id"], "44444444-4444-4444-8444-444444444444");
  assert_eq!(create["name"], "Created Page");
  assert!(create.get("blocks").is_none(), "no snapshot on a create");
  assert!(create.get("seq").is_none());

  let update = result_json(write_ack(gold("document_update")).expect("ack result"));
  assert_eq!(
    update["document_id"],
    "66666666-6666-4666-8666-666666666666"
  );
  assert_eq!(update["blocks"], 3);
  assert_eq!(update["last_block_id"], "block_b");
  assert_eq!(
    update["seq"], 4,
    "the update's seq, for conflict-checked writes"
  );
  assert!(update.get("view_id").is_none(), "an update answers no view");
}

/// The tools that only pick a field out of a payload they otherwise forward
/// verbatim: each pointer below is read by exactly one tool, and a gold rename
/// breaks it silently — this is the only thing that would catch it.
#[test]
fn the_pointer_paths_the_tools_read_exist_in_the_golds() {
  let create = gold("document_create");
  // mica_create_folder hands back `/view/id` — the whole point of the tool.
  assert_eq!(
    create.pointer("/view/id"),
    Some(&Value::String(
      "44444444-4444-4444-8444-444444444444".to_string()
    ))
  );

  let history = gold("history");
  // mica_list_versions forwards `/versions` and nothing else.
  let versions = history
    .pointer("/versions")
    .and_then(Value::as_array)
    .expect("versions");
  assert_eq!(versions.len(), 2);
  assert_eq!(
    versions[0]["label"], "before the rewrite",
    "named first — the endpoint is newest-first"
  );

  let version = gold("version");
  // mica_create_version reads `/version/id` to hand the caller the checkpoint.
  assert_eq!(
    version.pointer("/version/id"),
    Some(&Value::String(
      "aaaaaaaa-0000-4000-8000-000000000001".to_string()
    ))
  );

  let batch = gold("batch_read");
  // batch_read_full keys every row by `document_id` parsed as a Uuid.
  let docs = batch["documents"].as_array().expect("documents array");
  for doc in docs {
    let id = doc["document_id"]
      .as_str()
      .expect("document_id is a string");
    id.parse::<uuid::Uuid>()
      .expect("document_id parses as a Uuid");
  }
  assert_eq!(docs[0]["markdown"].as_str(), Some("# First\n\nbody"));
}
