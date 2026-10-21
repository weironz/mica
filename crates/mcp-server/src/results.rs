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
use rmcp::{
  ErrorData as McpError,
  model::{CallToolResult, Content},
};
use serde_json::{Value, json};


/// Return an API payload as MCP text content.
///
/// Deliberately NOT `Json<Value>`, which is what the tools used to return: rmcp
/// derives an `outputSchema` from the return type, and `Value` is "any", so it
/// emitted `{"title": "AnyValue"}` with no `"type": "object"`. The MCP spec
/// requires an object schema there, so a validating client REJECTS the whole
/// tools/list — Claude Code logged `tools/list failed … expected "object"` and
/// registered nothing, even though the handshake and every tool were fine.
/// `outputSchema` is optional; the honest move for a proxy returning whatever
/// the REST API said is to not declare one at all.
pub(crate) fn tool_result(result: Result<Value, McpError>) -> Result<CallToolResult, McpError> {
  match result {
    Ok(value) => {
      let text = serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
      Ok(CallToolResult::success(vec![Content::text(text)]))
    }
    // An API failure is a TOOL error, not a protocol error. The spec splits
    // them by who can act on them: JSON-RPC errors are for structural faults
    // the model cannot fix (unknown tool, malformed request), while anything
    // the model could react to — API failures, validation, business rules —
    // must come back as `isError: true` content so it can read the reason and
    // retry. Reporting a 403 or a bad-input message as a protocol error hides
    // it from the very reader it was written for.
    Err(error) => Ok(tool_error(error)),
  }
}

/// Turn an internal error into a tool-execution error the model can read.
pub(crate) fn tool_error(error: McpError) -> CallToolResult {
  CallToolResult::error(vec![Content::text(error.message.to_string())])
}

/// Answer a write with WHAT HAPPENED, not with the document itself.
///
/// A tool result is the model's context — that is MCP's whole design — and it is
/// re-sent on every later turn, so a fat result is not a one-off cost, it
/// compounds for the rest of the conversation. The REST API returns the full
/// snapshot because the EDITOR needs the blocks to render; a model needs the id
/// and nothing else. Forwarding it verbatim turned a 558-byte page into a 6.4KB
/// reply (~2k tokens), and a real page into far more — measured, not guessed.
///
/// Falls back to the whole payload when nothing is recognisable: if an endpoint
/// changes shape, the caller should see too much rather than a silent "ok".
pub(crate) fn write_ack(value: Value) -> Result<CallToolResult, McpError> {
  let mut ack = serde_json::Map::new();
  for (at, key) in [
    ("/document/id", "document_id"),
    ("/view/id", "view_id"),
    ("/view/name", "name"),
  ] {
    if let Some(found) = value.pointer(at).and_then(Value::as_str) {
      ack.insert(key.to_string(), json!(found));
    }
  }
  // A count, not the blocks: enough to confirm the write landed and roughly
  // how big it is, without shipping the tree.
  if let Some(blocks) = value
    .pointer("/snapshot/payload/blocks")
    .and_then(Value::as_array)
  {
    ack.insert("blocks".to_string(), json!(blocks.len()));
    // The last top-level block id — the natural anchor for the NEXT insert_at
    // after an append, so the caller chains edits without re-reading the doc.
    if let Some(root_id) = value
      .pointer("/snapshot/payload/root_block_id")
      .and_then(Value::as_str)
      && let Some(last) = blocks
        .iter()
        .find(|b| b.get("id").and_then(Value::as_str) == Some(root_id))
        .and_then(|root| root.get("children"))
        .and_then(Value::as_array)
        .and_then(|children| children.last())
        .and_then(Value::as_str)
    {
      ack.insert("last_block_id".to_string(), json!(last));
    }
  }
  if let Some(seq) = value.pointer("/update/seq").and_then(Value::as_i64) {
    ack.insert("seq".to_string(), json!(seq));
  }
  if ack.is_empty() {
    return tool_result(Ok(value));
  }
  ack.insert("ok".to_string(), json!(true));
  tool_result(Ok(Value::Object(ack)))
}

/// Ack for a batch that fully landed: how many edits applied, plus the final
/// `seq` and `last_block_id` (the end anchor to continue from) — the same
/// thrifty shape as [`write_ack`], never the document.
pub(crate) fn batch_ack(applied: usize, last: Option<Value>) -> Result<CallToolResult, McpError> {
  let mut ack = serde_json::Map::new();
  ack.insert("ok".to_string(), json!(true));
  ack.insert("applied".to_string(), json!(applied));
  if let Some(value) = last {
    if let Some(seq) = value.pointer("/update/seq").and_then(Value::as_i64) {
      ack.insert("seq".to_string(), json!(seq));
    }
    if let Some(blocks) = value
      .pointer("/snapshot/payload/blocks")
      .and_then(Value::as_array)
      && let Some(root_id) = value
        .pointer("/snapshot/payload/root_block_id")
        .and_then(Value::as_str)
      && let Some(last_block) = blocks
        .iter()
        .find(|b| b.get("id").and_then(Value::as_str) == Some(root_id))
        .and_then(|root| root.get("children"))
        .and_then(Value::as_array)
        .and_then(|children| children.last())
        .and_then(Value::as_str)
    {
      ack.insert("last_block_id".to_string(), json!(last_block));
    }
  }
  tool_result(Ok(Value::Object(ack)))
}

/// Answer a write whose endpoint replies with a LIST rather than the thing
/// written. Move and trash both return the workspace's remaining views, because
/// the sidebar re-renders from that — but the model already knows the id it
/// acted on (it supplied it), so echoing the tree back is pure cost: 6.4KB on a
/// 14-page workspace to report one deletion. `action` says which side of the
/// call happened, so "did it land?" needs no follow-up read.
pub(crate) fn action_ack(
  result: Result<Value, McpError>,
  action: &str,
  view_id: impl std::fmt::Display,
) -> Result<CallToolResult, McpError> {
  match result {
    Ok(_) => tool_result(Ok(json!({
        "ok": true,
        "action": action,
        "view_id": view_id.to_string(),
    }))),
    Err(error) => Ok(tool_error(error)),
  }
}

/// Keep only the fields a model navigates by; drop the storage bookkeeping.
///
/// The `/views` endpoint answers the Flutter client, which needs every column
/// (position for lexo-ordering, icon, timestamps, is_deleted…). A model needs
/// none of that — just enough to pick a page and hand its ids to another tool.
/// On a 14-page workspace the full list was 6.3KB, more than half of it
/// `workspace_id` (identical on every row, and supplied by the caller in the
/// first place), `created_by`, `position`, and timestamps. This is the read-side
/// twin of `write_ack`: the API stays whole and honest, the proxy fits the
/// answer to its audience.
pub(crate) fn slim_pages(value: Value) -> Value {
  let Some(views) = value.get("views").and_then(Value::as_array) else {
    return value;
  };
  let slim: Vec<Value> = views
    .iter()
    .map(|v| {
      let mut row = serde_json::Map::new();
      // id → the view id, for move/trash. object_id → the document id, for
      // read/outline/update. object_type → folder vs document. The rest is what
      // a human-facing tree needs and a model does not.
      // `state_bytes` rides along only when with_stats asked for it; the
      // endpoint omits the field entirely otherwise, so this costs nothing.
      for key in [
        "id",
        "object_id",
        "object_type",
        "name",
        "parent_view_id",
        "state_bytes",
      ] {
        if let Some(found) = v.get(key)
          && !found.is_null()
        {
          row.insert(key.to_string(), found.clone());
        }
      }
      // icon only when set — an emoji is a cheap, useful disambiguator; a null
      // is just noise.
      if let Some(icon) = v.get("icon").filter(|i| !i.is_null()) {
        row.insert("icon".to_string(), icon.clone());
      }
      Value::Object(row)
    })
    .collect();
  json!({ "pages": slim })
}

