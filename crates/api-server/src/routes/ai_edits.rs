//! Validate an AI edit plan and render its preview. This route never writes a
//! document and never calls an AI provider; the editor applies accepted edits.

use std::collections::{HashMap, HashSet};

use axum::{Json, extract::State, http::HeaderMap};
use mica_app_core::AppState;
use mica_infra::{ApiError, ApiResult};
use mica_markdown::{Block, DocumentSnapshotPayload, export_markdown, import_markdown_fragment};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::auth::existing_user_id_from_headers;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewRequest {
  response: String,
  blocks: Vec<SourceBlock>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceBlock {
  id: String,
  #[serde(rename = "type")]
  kind: String,
  text: String,
  data: Value,
  #[serde(default)]
  children: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EditPlan {
  edits: Vec<ProposedEdit>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProposedEdit {
  block_ids: Vec<String>,
  markdown: String,
}

#[derive(Debug, Serialize)]
pub struct PreviewResponse {
  edits: Vec<PreviewEdit>,
}

#[derive(Debug, Serialize)]
struct PreviewEdit {
  block_ids: Vec<String>,
  before: String,
  after: String,
  replacement: Vec<ReplacementBlock>,
}

#[derive(Debug, Serialize)]
struct ReplacementBlock {
  #[serde(rename = "type")]
  kind: String,
  text: String,
  data: Value,
}

/// `POST /api/ai/edits/preview`: authenticated, pure validation of supplied
/// editor content. No workspace/document ID is accepted or loaded from storage.
pub async fn preview(
  State(state): State<AppState>,
  headers: HeaderMap,
  Json(payload): Json<PreviewRequest>,
) -> ApiResult<Json<PreviewResponse>> {
  existing_user_id_from_headers(&state, &headers).await?;
  Ok(Json(build_preview(payload)?))
}

fn invalid(message: &str) -> ApiError {
  ApiError::BadRequest(message.to_string())
}

/// Accept only a JSON object, optionally surrounded by ONE whole-response JSON
/// fence. Do not fish a JSON-looking substring out of prose or a truncated reply.
fn response_json(response: &str) -> ApiResult<&str> {
  let response = response.trim();
  if !response.starts_with("```") {
    return Ok(response);
  }
  let (opening, rest) = response
    .split_once('\n')
    .ok_or_else(|| invalid("AI edit response has an incomplete JSON fence"))?;
  if !matches!(opening.trim_end(), "```json" | "```") {
    return Err(invalid("AI edit response must use a JSON fence"));
  }
  let (body, closing) = rest
    .rsplit_once('\n')
    .ok_or_else(|| invalid("AI edit response has an incomplete JSON fence"))?;
  if closing.trim() != "```" {
    return Err(invalid("AI edit response has an incomplete JSON fence"));
  }
  Ok(body.trim())
}

fn build_preview(payload: PreviewRequest) -> ApiResult<PreviewResponse> {
  if payload.blocks.is_empty() {
    return Err(invalid("Current page has no editable blocks"));
  }
  let mut positions = HashMap::new();
  for (index, block) in payload.blocks.iter().enumerate() {
    if block.id.trim().is_empty() || positions.insert(block.id.as_str(), index).is_some() {
      return Err(invalid(
        "Current page block IDs must be nonempty and unique",
      ));
    }
    if !block.children.is_empty() {
      return Err(invalid("Current page must contain flat body blocks only"));
    }
  }
  let plan: EditPlan = serde_json::from_str(response_json(&payload.response)?)
    .map_err(|_| invalid("AI response is not a valid edit plan JSON object"))?;
  if plan.edits.is_empty() {
    return Err(invalid("AI edit plan must contain at least one edit"));
  }

  let mut touched = HashSet::new();
  let mut ordered = Vec::with_capacity(plan.edits.len());
  for edit in plan.edits {
    if edit.block_ids.is_empty() {
      return Err(invalid("Each AI edit must target at least one block"));
    }
    let mut indices = Vec::with_capacity(edit.block_ids.len());
    for id in &edit.block_ids {
      let index = positions
        .get(id.as_str())
        .copied()
        .ok_or_else(|| invalid("AI edit targets an unknown block ID"))?;
      if !touched.insert(index) {
        return Err(invalid("AI edit ranges overlap or repeat a block ID"));
      }
      indices.push(index);
    }
    if !indices.windows(2).all(|pair| pair[1] == pair[0] + 1) {
      return Err(invalid(
        "AI edit block IDs must be consecutive and in page order",
      ));
    }
    let start = indices[0];
    let end = start + indices.len();
    validate_container_context(&payload.blocks, start, end)?;
    let before = export_body(&payload.blocks[start..end])?;
    // A fragment treats leading `---` as body Markdown, never front matter.
    // Its synthetic root is deliberately discarded; page properties cannot be
    // replaced by a body edit.
    let mut fragment = import_markdown_fragment(&edit.markdown, "ai-edit-root");
    // qbreak is a boundary relative to the PRECEDING source group. An isolated
    // fragment cannot infer it from its first quote, so retain that boundary
    // when the replacement still begins with a quote. Interior group boundaries
    // are parsed normally from the replacement Markdown.
    if payload.blocks[start]
      .data
      .get("qbreak")
      .and_then(Value::as_bool)
      == Some(true)
      && let Some(first_id) = fragment.blocks[0].children.first().cloned()
      && let Some(first) = fragment
        .blocks
        .iter_mut()
        .find(|block| block.id == first_id)
      && quote_depth(&first.kind, &first.data) > 0
    {
      if first.data.is_null() {
        first.data = Value::Object(Default::default());
      }
      if let Some(data) = first.data.as_object_mut() {
        data.insert("qbreak".into(), Value::Bool(true));
      }
    }
    let root = &fragment.blocks[0];
    let by_id: HashMap<_, _> = fragment.blocks.iter().map(|b| (b.id.as_str(), b)).collect();
    let replacement: Vec<_> = root
      .children
      .iter()
      .map(|id| {
        let block = by_id[id.as_str()];
        ReplacementBlock {
          kind: block.kind.clone(),
          text: block.text.clone(),
          data: block.data.clone(),
        }
      })
      .collect();
    let after = export_markdown(&fragment)
      .map_err(|_| invalid("AI replacement could not be exported as Markdown"))?;
    ordered.push((
      start,
      PreviewEdit {
        block_ids: edit.block_ids,
        before,
        after,
        replacement,
      },
    ));
  }
  // Deletion is a legitimate local edit, but clearing the entire page is not.
  if touched.len() == payload.blocks.len()
    && ordered.iter().all(|(_, edit)| edit.replacement.is_empty())
  {
    return Err(invalid("AI edits cannot delete the entire page"));
  }
  ordered.sort_by_key(|(start, _)| *start);
  Ok(PreviewResponse {
    edits: ordered.into_iter().map(|(_, edit)| edit).collect(),
  })
}

fn quote_depth(kind: &str, data: &Value) -> u64 {
  let depth = data.get("quote").and_then(Value::as_u64).unwrap_or(0);
  if kind == "quote" { depth.max(1) } else { depth }
}

fn needs_list_parent(block: &SourceBlock) -> bool {
  // li=0 is STILL a child of a level-zero item, unlike indent=0.
  block.data.get("li").and_then(Value::as_u64).is_some()
    || (matches!(
      block.kind.as_str(),
      "bulleted_list" | "numbered_list" | "todo"
    ) && block
      .data
      .get("indent")
      .and_then(Value::as_u64)
      .unwrap_or(0)
      > 0)
}

fn validate_container_context(blocks: &[SourceBlock], start: usize, end: usize) -> ApiResult<()> {
  // The Markdown parser's list stack starts empty. Reparsing a lone nested item
  // or an item-child code/paragraph block silently makes it top-level. Require
  // a self-contained parent + complete subtree instead of guessing a new depth.
  if needs_list_parent(&blocks[start]) || blocks.get(end).is_some_and(needs_list_parent) {
    return Err(invalid(
      "AI edit splits a list container; regenerate with its outer parent item and all child blocks",
    ));
  }
  let in_same_quote_group = |left: &SourceBlock, right: &SourceBlock| {
    quote_depth(&left.kind, &left.data) > 0
      && quote_depth(&right.kind, &right.data) > 0
      && right.data.get("qbreak").and_then(Value::as_bool) != Some(true)
  };
  // A quote group can carry alert/list metadata on its first block; editing a
  // partial group without its boundary can discard or move that metadata.
  if (start > 0 && in_same_quote_group(&blocks[start - 1], &blocks[start]))
    || blocks
      .get(end)
      .is_some_and(|next| in_same_quote_group(&blocks[end - 1], next))
  {
    return Err(invalid(
      "AI edit splits a blockquote group; regenerate with the complete outer quote group",
    ));
  }
  Ok(())
}

fn export_body(blocks: &[SourceBlock]) -> ApiResult<String> {
  // Pick a synthetic root that cannot shadow a supplied block ID.
  let mut root_id = String::from("ai-preview-root");
  while blocks.iter().any(|block| block.id == root_id) {
    root_id.push('_');
  }
  let mut snapshot_blocks = vec![Block {
    id: root_id.clone(),
    kind: "paragraph".to_string(),
    text: String::new(),
    data: Value::Null,
    children: blocks.iter().map(|block| block.id.clone()).collect(),
  }];
  snapshot_blocks.extend(blocks.iter().map(|block| Block {
    id: block.id.clone(),
    kind: block.kind.clone(),
    text: block.text.clone(),
    data: block.data.clone(),
    children: Vec::new(),
  }));
  export_markdown(&DocumentSnapshotPayload {
    schema_version: 1,
    root_block_id: root_id,
    blocks: snapshot_blocks,
  })
  .map_err(|_| invalid("Current page block range could not be exported as Markdown"))
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn request(response: Value) -> PreviewRequest {
    let blocks = vec![
      SourceBlock {
        id: "a".into(),
        kind: "paragraph".into(),
        text: "Keep before".into(),
        data: Value::Null,
        children: vec![],
      },
      SourceBlock {
        id: "b".into(),
        kind: "paragraph".into(),
        text: "Old body".into(),
        data: Value::Null,
        children: vec![],
      },
      SourceBlock {
        id: "c".into(),
        kind: "paragraph".into(),
        text: "Keep after".into(),
        data: Value::Null,
        children: vec![],
      },
    ];
    PreviewRequest {
      response: response.to_string(),
      blocks,
    }
  }

  fn request_from_markdown(markdown: &str, targets: &[usize], replacement: &str) -> PreviewRequest {
    let snapshot = import_markdown_fragment(markdown, "source-root");
    let source_ids = snapshot.blocks[0].children.clone();
    let block_ids: Vec<_> = targets
      .iter()
      .map(|&index| source_ids[index].clone())
      .collect();
    let blocks = source_ids
      .iter()
      .map(|id| {
        let block = snapshot
          .blocks
          .iter()
          .find(|block| &block.id == id)
          .unwrap();
        SourceBlock {
          id: block.id.clone(),
          kind: block.kind.clone(),
          text: block.text.clone(),
          data: block.data.clone(),
          children: block.children.clone(),
        }
      })
      .collect();
    PreviewRequest {
      response: json!({"edits":[{"block_ids":block_ids,"markdown":replacement}]}).to_string(),
      blocks,
    }
  }

  #[test]
  fn ai_edits_nested_list_requires_outer_parent_and_complete_children() {
    let source = "- parent\n  - child\n  - sibling\n\nAfter";
    assert!(build_preview(request_from_markdown(source, &[1], "  - revised")).is_err());
    assert!(
      build_preview(request_from_markdown(
        source,
        &[0, 1],
        "- parent\n  - revised"
      ))
      .is_err()
    );
    let preview = build_preview(request_from_markdown(
      source,
      &[0, 1, 2],
      "- parent\n  - revised\n  - sibling",
    ))
    .unwrap();
    assert_eq!(preview.edits[0].replacement.len(), 3);
    assert_eq!(preview.edits[0].replacement[1].text, "revised");
    assert_eq!(preview.edits[0].replacement[1].data["indent"], 1);
    assert_eq!(preview.edits[0].replacement[2].data["indent"], 1);
  }

  #[test]
  fn ai_edits_list_child_code_paragraph_and_quote_require_parent_context() {
    let source = "- parent\n\n  ```rust\n  old\n  ```\n\n  child paragraph\n\nAfter";
    let parsed = request_from_markdown(source, &[1], "```rust\nnew\n```");
    assert_eq!(parsed.blocks[1].kind, "code_block");
    assert_eq!(parsed.blocks[1].data["li"], 0);
    assert_eq!(parsed.blocks[2].kind, "paragraph");
    assert_eq!(parsed.blocks[2].data["li"], 0);
    assert!(build_preview(parsed).is_err());
    assert!(build_preview(request_from_markdown(source, &[2], "revised paragraph")).is_err());
    let preview = build_preview(request_from_markdown(
      source,
      &[0, 1, 2],
      "- parent\n\n  ```rust\n  new\n  ```\n\n  child paragraph",
    ))
    .unwrap();
    assert_eq!(preview.edits[0].replacement[1].data["li"], 0);
    assert_eq!(preview.edits[0].replacement[2].data["li"], 0);

    let quote_source = "- parent\n\n  > child quote\n\nAfter";
    let parsed = request_from_markdown(quote_source, &[1], "> revised quote");
    assert_eq!(parsed.blocks[1].data["li"], 0);
    assert!(build_preview(parsed).is_err());
    let preview = build_preview(request_from_markdown(
      quote_source,
      &[0, 1],
      "- parent\n\n  > revised quote",
    ))
    .unwrap();
    assert_eq!(preview.edits[0].replacement[1].data["li"], 0);
  }

  #[test]
  fn ai_edits_quote_group_requires_complete_context_and_keeps_external_boundary() {
    let source = "> first\n>\n> second\n\nAfter";
    let parsed = request_from_markdown(source, &[1], "> revised");
    assert_eq!(parsed.blocks[0].kind, "quote");
    assert_eq!(parsed.blocks[1].kind, "quote");
    assert!(build_preview(parsed).is_err());
    assert!(build_preview(request_from_markdown(source, &[0], "> revised")).is_err());
    let preview = build_preview(request_from_markdown(
      source,
      &[0, 1],
      "> first\n>\n> revised",
    ))
    .unwrap();
    assert_eq!(preview.edits[0].replacement[1].text, "revised");

    let separate = "> previous group\n\n> target group\n\nAfter";
    let parsed = request_from_markdown(separate, &[1], "> revised group");
    assert_eq!(parsed.blocks[1].data["qbreak"], true);
    let preview = build_preview(parsed).unwrap();
    assert_eq!(preview.edits[0].replacement[0].data["qbreak"], true);
    // The authoritative exporter emits a final bare quote marker for the
    // paragraph's blank terminator inside the quote.
    assert_eq!(preview.edits[0].after, "> revised group\n>");

    // Quote depth itself is absolute; it need not be guessed from indentation.
    let nested = ">> nested quote\n\nAfter";
    let preview = build_preview(request_from_markdown(
      nested,
      &[0],
      ">> revised nested quote",
    ))
    .unwrap();
    assert_eq!(preview.edits[0].replacement[0].data["quote"], 2);
  }

  #[test]
  fn ai_edits_local_replacement_has_authoritative_preview_and_no_root() {
    let payload =
      request(json!({"edits":[{"block_ids":["b"],"markdown":"## New body\n\n**Details**"}]}));
    let preview = build_preview(payload).unwrap();
    assert_eq!(preview.edits.len(), 1);
    assert_eq!(preview.edits[0].block_ids, ["b"]);
    assert_eq!(preview.edits[0].before, "Old body");
    assert_eq!(preview.edits[0].after, "## New body\n\n**Details**");
    assert_eq!(preview.edits[0].replacement.len(), 2);
    assert_eq!(preview.edits[0].replacement[0].kind, "heading");
    let output = serde_json::to_value(preview).unwrap();
    assert!(output["edits"][0]["replacement"][0].get("id").is_none());
    assert!(
      output["edits"][0]["replacement"][0]
        .get("children")
        .is_none()
    );
  }

  #[test]
  fn ai_edits_multiple_edits_are_sorted_and_multiblock_ranges_are_supported() {
    let preview = build_preview(request(json!({"edits":[
      {"block_ids":["c"],"markdown":"Last"},
      {"block_ids":["a","b"],"markdown":"Combined"}
    ]})))
    .unwrap();
    assert_eq!(preview.edits[0].block_ids, ["a", "b"]);
    assert_eq!(preview.edits[0].before, "Keep before\n\nOld body");
    assert_eq!(preview.edits[1].block_ids, ["c"]);
  }

  #[test]
  fn ai_edits_reject_unknown_empty_repeated_noncontiguous_reversed_or_overlapping_ids() {
    for edits in [
      json!([]),
      json!([{"block_ids":[],"markdown":"x"}]),
      json!([{"block_ids":[""],"markdown":"x"}]),
      json!([{"block_ids":["unknown"],"markdown":"x"}]),
      json!([{"block_ids":["a","a"],"markdown":"x"}]),
      json!([{"block_ids":["a","c"],"markdown":"x"}]),
      json!([{"block_ids":["b","a"],"markdown":"x"}]),
      json!([{"block_ids":["a","b"],"markdown":"x"},{"block_ids":["b"],"markdown":"y"}]),
    ] {
      assert!(build_preview(request(json!({"edits":edits}))).is_err());
    }
  }

  #[test]
  fn ai_edits_reject_duplicate_or_empty_source_ids_and_nested_source() {
    for mode in 0..3 {
      let mut payload = request(json!({"edits":[{"block_ids":["b"],"markdown":"New"}]}));
      match mode {
        0 => payload.blocks[0].id = "b".into(),
        1 => payload.blocks[0].id = " ".into(),
        _ => payload.blocks[0].children.push("b".into()),
      }
      assert!(build_preview(payload).is_err());
    }
  }

  #[test]
  fn ai_edits_fragment_dividers_do_not_change_page_properties() {
    let preview = build_preview(request(
      json!({"edits":[{"block_ids":["b"],"markdown":"---\n\nText\n\n---"}]}),
    ))
    .unwrap();
    assert_eq!(
      preview.edits[0]
        .replacement
        .iter()
        .map(|b| b.kind.as_str())
        .collect::<Vec<_>>(),
      ["divider", "paragraph", "divider"]
    );
    assert!(
      preview.edits[0]
        .replacement
        .iter()
        .all(|b| b.data.get("front_matter").is_none())
    );
  }

  #[test]
  fn ai_edits_allow_local_deletion_but_reject_deleting_the_entire_page() {
    let preview = build_preview(request(
      json!({"edits":[{"block_ids":["b"],"markdown":""}]}),
    ))
    .unwrap();
    assert!(preview.edits[0].replacement.is_empty());
    assert_eq!(preview.edits[0].after, "");
    for edits in [
      json!([{"block_ids":["a","b","c"],"markdown":""}]),
      json!([{"block_ids":["a"],"markdown":" "},{"block_ids":["b","c"],"markdown":""}]),
      // Markdown containing only definitions also yields no body blocks.
      json!([{"block_ids":["a","b","c"],"markdown":"[label]: https://example.com"}]),
    ] {
      assert!(build_preview(request(json!({"edits":edits}))).is_err());
    }
  }

  #[test]
  fn ai_edits_accept_whole_json_fence_and_reject_prose_truncation_and_unknown_fields() {
    let mut payload = request(json!({"edits":[{"block_ids":["b"],"markdown":"New"}]}));
    payload.response = format!("```json\n{}\n```", payload.response);
    assert!(build_preview(payload).is_ok());
    for response in [
      "not JSON",
      "{\"edits\":[",
      "Here you go: {\"edits\":[{\"block_ids\":[\"b\"],\"markdown\":\"New\"}]}",
      "{\"edits\":[{\"block_ids\":[\"b\"],\"markdown\":\"New\",\"extra\":true}]}",
      "{\"edits\":[{\"block_ids\":[\"b\"],\"markdown\":\"New\"}],\"extra\":true}",
      "```json\n{\"edits\":[]}\n```\nOther prose",
    ] {
      let mut payload = request(json!({}));
      payload.response = response.into();
      assert!(build_preview(payload).is_err(), "{response}");
    }
  }
}
