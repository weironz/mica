//! #112: `rehost-images` must exit non-zero when any request failed, while
//! still processing the remaining items and printing the full summary.
//!
//! Binary-level: the exit status and the stdout/stderr split are the contract.

mod common;

use common::{FakeServer, Reply, run_cli, temp_file};

const WS: &str = "11111111-1111-4111-8111-111111111111";
const DOC: &str = "22222222-2222-4222-8222-222222222222";
const FAIL_URL: &str = "https://dead.example/fail.png";
const OK_URL: &str = "https://dead.example/ok.png";

fn document_views() -> String {
  format!(
    r#"{{"views":[
      {{"id":"{}","object_id":"{}","object_type":"document","name":"Doc"}}
    ]}}"#,
    WS, DOC
  )
}

fn bootstrap(blocks: &str) -> String {
  format!(r#"{{"snapshot":{{"payload":{{"blocks":[{blocks}]}}}}}}"#)
}

/// The 409 comes back for the block whose url is the first drop target. The
/// filter deliberately does not depend on which connection arrives first.
fn api(blocks: String) -> FakeServer {
  FakeServer::start(move |request| {
    if request.contains("/drop-image") {
      if request.contains("fail.png") {
        return Reply::json(409, r#"{"error":"image block already changed"}"#);
      }
      return Reply::json(200, "{}");
    }
    if request.starts_with("GET ") && request.contains("/bootstrap") {
      return Reply::json(200, bootstrap(&blocks));
    }
    if request.starts_with("GET ") && request.contains("/views") {
      return Reply::json(200, document_views());
    }
    Reply::json(404, r#"{"error":"unexpected route"}"#)
  })
}

const BOTH_IMAGES: &str = "\
      {\"id\":\"block_fail\",\"type\":\"image\",\"data\":{\"url\":\"https://dead.example/fail.png\"}},\
      {\"id\":\"block_ok\",\"type\":\"image\",\"data\":{\"url\":\"https://dead.example/ok.png\"}}";
const OK_IMAGE: &str =
  "{\"id\":\"block_ok\",\"type\":\"image\",\"data\":{\"url\":\"https://dead.example/ok.png\"}}";

#[test]
fn any_failed_image_exits_non_zero_and_still_processes_the_rest() {
  let server = api(BOTH_IMAGES.to_string());
  let drop = temp_file("drop-nonzero.txt", &format!("{FAIL_URL}\n{OK_URL}\n"));

  let run = run_cli(
    &server,
    &[
      "--json",
      "rehost-images",
      "--ws",
      WS,
      "--drop",
      drop.to_str().unwrap(),
    ],
  );

  // Both blocks were attempted: a failure must not abort the remaining work.
  let drop_requests: Vec<_> = server
    .requests()
    .into_iter()
    .filter(|r| r.contains("/drop-image"))
    .collect();
  assert_eq!(
    drop_requests.len(),
    2,
    "both drop requests must be sent, got {drop_requests:?}"
  );

  // The summary keeps its JSON structure on stdout.
  let summary: serde_json::Value =
    serde_json::from_str(&run.stdout).expect("stdout is the JSON summary");
  assert_eq!(summary["dropped"], 1);
  assert_eq!(summary["failed"], 1);
  assert_eq!(summary["rehosted"], 0);

  assert_ne!(
    run.status, 0,
    "a failed image must be visible to scripts; stdout was {}",
    run.stdout
  );
  assert!(
    run.stderr.contains("fail.png"),
    "stderr names the failed image, got: {:?}",
    run.stderr
  );
}

#[test]
fn a_clean_run_still_exits_zero() {
  let server = api(OK_IMAGE.to_string());
  let drop = temp_file("drop-clean.txt", &format!("{OK_URL}\n"));

  let run = run_cli(
    &server,
    &[
      "--json",
      "rehost-images",
      "--ws",
      WS,
      "--drop",
      drop.to_str().unwrap(),
    ],
  );

  let summary: serde_json::Value =
    serde_json::from_str(&run.stdout).expect("stdout is the JSON summary");
  assert_eq!(summary["dropped"], 1);
  assert_eq!(summary["failed"], 0);
  assert_eq!(run.status, 0, "stderr was {:?}", run.stderr);
  assert!(run.stderr.is_empty(), "stderr was {:?}", run.stderr);
}
