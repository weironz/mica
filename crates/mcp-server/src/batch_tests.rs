//! Regression tests for `mica_apply_edits`: per-edit "where did it land"
//! echo in the success ack, and a partial-failure payload that names every
//! edit that DID apply. The old "applied N/M" string-only error left a
//! caller unable to say WHICH edits it had already made -- the ids live in
//! each update's `operations` all along; these pin that the batch surfaces
//! them.
use super::*;

/// A single write's API reply: snapshot+update plus the operations the write
/// derived. `batch_ack` builds from exactly this shape.
fn write_ops_value(ops: &[&str]) -> Value {
  json!({
      "document": {},
      "snapshot": { "payload": { "blocks": [], "root_block_id": "block_root" } },
       "update": {
           "seq": 7,
           "payload": {
               "operations": ops.iter().map(|id| json!({"block_id": id})).collect::<Vec<_>>(),
           },
       },
  })
}

fn result_json(result: &CallToolResult) -> Value {
  let text = result
    .content
    .iter()
    .find_map(|content| content.as_text())
    .expect("a text content block");
  serde_json::from_str(&text.text).expect("tool result text is JSON")
}

#[test]
fn a_full_batch_answers_with_each_edits_block_ids() {
  let steps = vec![
    json!({"mode": "append", "block_ids": ["block_a"]}),
    json!({"mode": "insert_at", "anchor": "block_a", "block_ids": ["block_b"]}),
  ];
  let result = batch_ack(2, Some(write_ops_value(&["block_b"])), steps).unwrap();
  let v = result_json(&result);
  assert_eq!(v["ok"], true);
  assert_eq!(v["applied"], 2);
  assert_eq!(v["edits"][0]["block_ids"][0], "block_a");
  assert_eq!(v["edits"][1]["anchor"], "block_a");
  assert_eq!(v["edits"][1]["block_ids"][0], "block_b");
  // The final seq still comes from the LAST write, for the next expected_seq.
  assert_eq!(v["seq"], 7);
}

#[test]
fn an_update_without_operations_omits_block_ids_honestly() {
  // Unknown ids are omitted, NOT reported as empty: a `block_ids: []` reads
  // as "touched nothing", and guessing which ids a write touched when the
  // API does not say is a guess the tool must not make.
  let value = json!({
      "snapshot": { "payload": { "blocks": [], "root_block_id": "block_root" } },
      "update": { "seq": 3 },
  });
  assert!(update_block_ids(&value).is_none());
  let with_ops = write_ops_value(&[]);
  assert_eq!(update_block_ids(&with_ops), Some(vec![]));
}

#[test]
fn a_partial_failure_keeps_is_error_and_names_landed_edits() {
  // The isError result keeps the isError flag (the batch is NOT a success)
  // while carrying the landed steps in the error details, so the caller can
  // see which edits it already made without a blind re-read.
  let steps = vec![json!({"mode": "append", "block_ids": ["block_a"]})];
  let result = batch_failure(
    1,
    Some(write_ops_value(&["block_a"])),
    steps,
    "find_replace",
    "applied 1/2 edits; edit #1 (find_replace) failed: unique match not unique".to_string(),
  )
  .unwrap();
  assert_eq!(result.is_error, Some(true));
  let text = result
    .content
    .iter()
    .find_map(|content| content.as_text())
    .expect("a text content block");
  assert!(text.text.contains("applied 1/2"));
  let v = result_json(&result);
  assert_eq!(v["ok"], false);
  assert_eq!(v["applied"], 1);
  assert_eq!(v["seq"], 7);
  assert_eq!(v["edits"][0]["block_ids"][0], "block_a");
  assert_eq!(v["failed_index"], 1);
}

#[test]
fn inserted_updated_and_deleted_ids_follow_real_operation_shapes() {
  let value = json!({"update":{"payload":{"operations":[
    {"type":"insert_block","block":{"id":"new"},"parent_id":"root"},
    {"type":"update_block","block_id":"new"},
    {"type":"delete_block","block_id":"gone"}
  ]}}});
  assert_eq!(
    update_block_ids(&value),
    Some(vec!["new".into(), "gone".into()])
  );
}

type Requests = std::sync::Arc<std::sync::Mutex<Vec<(String, Value)>>>;

async fn api(replies: Vec<(u16, Value)>) -> (String, Requests) {
  use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let base = format!("http://{}", listener.local_addr().unwrap());
  let requests = Requests::default();
  let seen = requests.clone();
  tokio::spawn(async move {
    for (status, reply) in replies {
      let (socket, _) = listener.accept().await.unwrap();
      let mut reader = BufReader::new(socket);
      let mut request = String::new();
      reader.read_line(&mut request).await.unwrap();
      let mut len = 0;
      loop {
        let mut header = String::new();
        reader.read_line(&mut header).await.unwrap();
        if header == "\r\n" {
          break;
        }
        if let Some((key, value)) = header.split_once(':')
          && key.eq_ignore_ascii_case("content-length")
        {
          len = value.trim().parse().unwrap();
        }
      }
      let mut body = vec![0; len];
      reader.read_exact(&mut body).await.unwrap();
      seen.lock().unwrap().push((
        request,
        serde_json::from_slice(&body).unwrap_or(Value::Null),
      ));
      let body = reply.to_string();
      let response = format!(
        "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
      );
      reader
        .get_mut()
        .write_all(response.as_bytes())
        .await
        .unwrap();
    }
  });
  (base, requests)
}

#[tokio::test]
async fn legacy_api_is_refused_before_sending_new_edit_semantics() {
  let (base, seen) = api(vec![(200, json!({"block_ids":[], "seq":3}))]).await;
  let server = MicaMcp::new(base, "test".into(), false);
  let error = server
    .write_markdown(
      Uuid::nil(),
      Uuid::nil(),
      "find_replace",
      None,
      None,
      Some("old"),
      Some("`new`"),
      "markdown",
      true,
      None,
    )
    .await
    .unwrap_err();
  assert!(error.message.contains("updated Mica API"));
  let requests = seen.lock().unwrap();
  assert_eq!(requests.len(), 1);
  assert!(requests[0].0.starts_with("GET "));
}

#[tokio::test]
async fn find_replace_sends_unique_and_format_to_the_authoritative_api() {
  let (base, seen) = api(vec![
    (200, json!({"blocks":[], "seq":3})),
    (200, write_ops_value(&["block_a"])),
  ])
  .await;
  let server = MicaMcp::new(base, "test".into(), false);
  server
    .write_markdown(
      Uuid::nil(),
      Uuid::nil(),
      "find_replace",
      None,
      None,
      Some("old"),
      Some("`new`"),
      "plain_text",
      true,
      Some(3),
    )
    .await
    .unwrap();
  let requests = seen.lock().unwrap();
  assert_eq!(requests.len(), 2);
  assert!(requests[1].0.starts_with("PATCH "));
  assert_eq!(requests[1].1["unique"], true);
  assert_eq!(requests[1].1["replacement_format"], "plain_text");
  assert_eq!(requests[1].1["expected_seq"], 3);
}

#[tokio::test]
async fn actual_batch_partial_failure_returns_landed_ids_and_seq() {
  let inserted = json!({"update":{"seq":4,"payload":{"operations":[
    {"type":"insert_block","block":{"id":"block_inserted"},"parent_id":"root"}
  ]}}});
  let (base, seen) = api(vec![
    (200, inserted),
    (400, json!({"error":"no such block"})),
  ])
  .await;
  let server = MicaMcp::new(base, "test".into(), false);
  let args = serde_json::from_value(json!({
    "workspace_id":Uuid::nil(), "document_id":Uuid::nil(), "expected_seq":3,
    "edits":[{"mode":"append","markdown":"new"},{"mode":"delete","anchor":"block_missing"}]
  }))
  .unwrap();
  let result = server.mica_apply_edits(Parameters(args)).await.unwrap();
  assert_eq!(result.is_error, Some(true));
  let result = result_json(&result);
  assert_eq!(result["applied"], 1);
  assert_eq!(result["edits"][0]["block_ids"], json!(["block_inserted"]));
  assert_eq!(result["seq"], 4);
  assert_eq!(result["failed_index"], 1);
  assert!(result["message"].as_str().unwrap().contains("applied 1/2"));
  let requests = seen.lock().unwrap();
  assert_eq!(requests[0].1["expected_seq"], 3);
}
