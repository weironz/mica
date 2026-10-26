//! #118: the import summary must use the server's true totals and must surface
//! external-image failures instead of reporting a clean "done".
//!
//! The job-status route is the only thing the CLI reads; the fake server plays
//! both the upload start and the polling endpoint.

mod common;

use common::{FakeServer, Reply, run_cli, temp_file};

const WS: &str = "22222222-2222-4222-8222-222222222222";
const DOC: &str = "33333333-3333-4333-8333-333333333333";
const JOB: &str = "44444444-4444-4444-8444-444444444444";

fn status(skipped_total: usize, image_failure: bool) -> String {
  let skipped: Vec<String> = (1..=50)
    .map(|i| format!("\"unused/asset-{i:02}.txt\""))
    .collect();
  let failures = if image_failure {
    format!(
      r#"[{{"url":"https://source.example/image.png","page":"Sample",
           "document_id":"{}","block_id":"image-block",
           "reason":"connection timed out","attempted":true}}]"#,
      DOC
    )
  } else {
    "[]".to_string()
  };
  format!(
    r#"{{"status":"done","total":1,"done":1,"workspace_id":"{}","error":null,
       "skipped":[{}],"skipped_total":{},
       "image_failures":{},"image_failures_total":{}}}"#,
    WS,
    skipped.join(","),
    skipped_total,
    failures,
    u8::from(image_failure)
  )
}

fn api(job: String) -> FakeServer {
  FakeServer::start(move |request| {
    if request.starts_with("POST ") && request.contains("/workspaces/import") {
      return Reply::json(200, format!(r#"{{"job_id":"{}"}}"#, JOB));
    }
    if request.starts_with("GET ") && request.contains(&format!("/import/jobs/{}", JOB)) {
      return Reply::json(200, job.clone());
    }
    Reply::json(404, r#"{"error":"unexpected route"}"#)
  })
}

fn sample_zip() -> std::path::PathBuf {
  temp_file(
    "sample.zip",
    "PK\u{3}\u{4} not a real archive; the server never reads it",
  )
}

#[test]
fn json_keeps_the_true_total_and_the_structured_image_failures() {
  let server = api(status(51, true));
  let zip = sample_zip();
  let run = run_cli(&server, &["--json", "import", zip.to_str().unwrap()]);

  let results: serde_json::Value =
    serde_json::from_str(&run.stdout).expect("stdout is the JSON result array");
  let entry = &results[0];
  assert_eq!(entry["status"], "done");
  assert_eq!(entry["skipped"], 50, "the listed sample count is preserved");
  assert_eq!(
    entry["skipped_total"], 51,
    "the server's real total is used"
  );
  assert_eq!(entry["image_failures_total"], 1);
  let failure = &entry["image_failures"][0];
  assert_eq!(failure["url"], "https://source.example/image.png");
  assert_eq!(failure["page"], "Sample");
  assert_eq!(failure["document_id"], DOC);
  assert_eq!(failure["block_id"], "image-block");
  assert_eq!(failure["reason"], "connection timed out");
  assert_eq!(failure["attempted"], true);
}

#[test]
fn human_output_shows_the_true_total_and_warns_about_external_images() {
  let server = api(status(51, true));
  let zip = sample_zip();
  let run = run_cli(&server, &["import", zip.to_str().unwrap()]);

  // Human wording is allowed to change; the facts are not: the server's true
  // total (51), the truncated sample (50), and the failed image named.
  assert!(
    run.stdout.contains("51"),
    "must report 51, not the 50-entry sample: {:?}",
    run.stdout
  );
  assert!(
    run.stdout.contains("50"),
    "must mention the 50-entry sample: {:?}",
    run.stdout
  );
  assert!(
    run.stdout.contains("source.example/image.png"),
    "the failed image must be named: {:?}",
    run.stdout
  );
  assert_eq!(
    run.status, 0,
    "the import itself finished; stderr {:?}",
    run.stderr
  );
}

#[test]
fn a_clean_import_has_no_extra_warnings() {
  let server = api(status(50, false));
  let zip = sample_zip();
  let run = run_cli(&server, &["import", zip.to_str().unwrap()]);

  assert!(
    run.stdout.contains("50"),
    "true total, equal to the sample: {:?}",
    run.stdout
  );
  assert!(
    !run.stdout.contains("outside Mica") && !run.stdout.contains("WARNING"),
    "a clean import must not warn: {:?}",
    run.stdout
  );
}
