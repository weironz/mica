use super::*;
use mica_infra::Mailer;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct CaptureMailer(Mutex<Vec<Mail>>, bool);
#[async_trait::async_trait]
impl Mailer for CaptureMailer {
  async fn send(&self, mail: &Mail) -> anyhow::Result<()> {
    if self.1 {
      anyhow::bail!("test provider failure");
    }
    self.0.lock().unwrap().push(mail.clone());
    Ok(())
  }
}

fn config() -> mica_infra::AppConfig {
  mica_infra::AppConfig {
    environment: mica_infra::Environment::Test,
    http_addr: "127.0.0.1:0".parse().unwrap(),
    database_url: String::new(),
    database_max_connections: 1,
    jwt_secret: "resend-timing-test-secret".to_string(),
    ws_min_protocol: 1,
    access_token_ttl_seconds: 60,
    refresh_token_ttl_seconds: 60,
    cors_allowed_origins: Vec::new(),
    seed_test_user: None,
    registration_enabled: false,
    workspace_quota_bytes: 0,
    owner_quota_bytes: 0,
    search_index_mlock: false,
    sync_tuning: Default::default(),
    app_base_url: "http://localhost:8080".to_string(),
  }
}

#[tokio::test]
async fn terminal_job_mail_is_owned_opt_out_aware_and_claimed_once() {
  let Some(url) = mica_infra::test_database_url() else {
    return;
  };
  let db = sqlx::PgPool::connect(&url).await.unwrap();
  mica_infra::run_migrations(&db).await.unwrap();
  let user = Uuid::new_v4();
  let job = Uuid::new_v4();
  let email = format!("{user}@import.test");
  sqlx::query("INSERT INTO users(id,email,display_name,password_hash) VALUES($1,$2,'test','h')")
    .bind(user)
    .bind(&email)
    .execute(&db)
    .await
    .unwrap();
  sqlx::query("INSERT INTO import_jobs(id,user_id,status,total,done) VALUES($1,$2,'running',3,2)")
    .bind(job)
    .bind(user)
    .execute(&db)
    .await
    .unwrap();
  let capture = Arc::new(CaptureMailer::default());
  let state = AppState::new(config(), db.clone(), capture.clone());
  notify(&state, user, job).await;
  assert!(
    capture.0.lock().unwrap().is_empty(),
    "running is not completion"
  );
  sqlx::query("UPDATE import_jobs SET status='error', error='<script>unsafe</script>' WHERE id=$1")
    .bind(job)
    .execute(&db)
    .await
    .unwrap();
  notify(&state, Uuid::new_v4(), job).await;
  assert!(
    capture.0.lock().unwrap().is_empty(),
    "wrong owner cannot send"
  );
  sqlx::query("UPDATE users SET import_completion_email_enabled=false WHERE id=$1")
    .bind(user)
    .execute(&db)
    .await
    .unwrap();
  notify(&state, user, job).await;
  assert!(
    capture.0.lock().unwrap().is_empty(),
    "opt-out is authoritative"
  );
  sqlx::query("UPDATE users SET import_completion_email_enabled=true WHERE id=$1")
    .bind(user)
    .execute(&db)
    .await
    .unwrap();
  tokio::join!(notify(&state, user, job), notify(&state, user, job));
  {
    let messages = capture.0.lock().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].to, email);
    assert!(
      messages[0]
        .html_body
        .contains("成功页面：2；失败或未完成页面：1")
    );
    assert!(messages[0].html_body.contains("http://localhost:8080"));
    assert!(!messages[0].html_body.contains("<script>"));
  }
  let status: String =
    sqlx::query_scalar("SELECT completion_email_state FROM import_jobs WHERE id=$1")
      .bind(job)
      .fetch_one(&db)
      .await
      .unwrap();
  assert_eq!(status, "sent");
  sqlx::query("UPDATE import_jobs SET completion_email_state=NULL WHERE id=$1")
    .bind(job)
    .execute(&db)
    .await
    .unwrap();
  let failed = AppState::new(
    config(),
    db.clone(),
    Arc::new(CaptureMailer(Mutex::default(), true)),
  );
  notify(&failed, user, job).await;
  let status: String =
    sqlx::query_scalar("SELECT completion_email_state FROM import_jobs WHERE id=$1")
      .bind(job)
      .fetch_one(&db)
      .await
      .unwrap();
  assert_eq!(
    status, "failed",
    "provider errors are never recorded as delivered"
  );
  sqlx::query("DELETE FROM users WHERE id=$1")
    .bind(user)
    .execute(&db)
    .await
    .unwrap();
}

/// Real HTTP request -> background Rust importer -> persisted job -> signed
/// DirectMail HTTP request received by a local mailbox. No real email is sent.
#[tokio::test]
async fn an_http_import_delivers_a_completion_message_to_a_local_mailbox() {
  use axum::{Router, body::Bytes, routing::post};
  use mica_interchange::zip::{ZipEntry, build_zip};
  use std::time::Duration;

  let Some(url) = mica_infra::test_database_url() else {
    return;
  };
  let db = sqlx::PgPool::connect(&url).await.unwrap();
  mica_infra::run_migrations(&db).await.unwrap();
  let user = Uuid::new_v4();
  let email = format!("{user}@capture.test");
  sqlx::query("INSERT INTO users(id,email,display_name,password_hash) VALUES($1,$2,'T','h')")
    .bind(user)
    .bind(&email)
    .execute(&db)
    .await
    .unwrap();
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let origin = format!("http://{}", listener.local_addr().unwrap());
  let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
  let mut config = config();
  config.app_base_url = origin.clone();
  let secret = config.jwt_secret.clone();
  let mailer = Arc::new(crate::mail::DirectMailMailer::local_capture(format!(
    "{origin}/capture"
  )));
  let state = AppState::new(config, db.clone(), mailer);
  let router = Router::new()
    .nest("/api", crate::routes::layered_api(&state))
    .with_state(state)
    .route(
      "/capture",
      post(move |body: Bytes| {
        let tx = tx.clone();
        async move {
          tx.send(String::from_utf8(body.to_vec()).unwrap()).unwrap();
          "accepted"
        }
      }),
    );
  let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
  struct Server(tokio::task::JoinHandle<()>);
  impl Drop for Server {
    fn drop(&mut self) {
      self.0.abort();
    }
  }
  let _server = Server(server);
  let claims = serde_json::json!({"sub":user.to_string(),"exp":(chrono::Utc::now()+chrono::Duration::hours(1)).timestamp(),"iat":chrono::Utc::now().timestamp()});
  let token = jsonwebtoken::encode(
    &jsonwebtoken::Header::default(),
    &claims,
    &jsonwebtoken::EncodingKey::from_secret(secret.as_bytes()),
  )
  .unwrap();
  let client = reqwest::Client::new();
  let headers = || format!("Bearer {token}");
  // The preference is a real authenticated endpoint, not a device-only toggle.
  let preference = client
    .get(format!("{origin}/api/auth/me/import-notifications"))
    .header("authorization", headers())
    .send()
    .await
    .unwrap();
  assert!(preference.status().is_success());
  assert_eq!(
    preference.json::<serde_json::Value>().await.unwrap()["enabled"],
    true
  );
  for invalid in [
    serde_json::json!({}),
    serde_json::json!({"enabled": null}),
    serde_json::json!({"enabled": "false"}),
  ] {
    let response = client
      .put(format!("{origin}/api/auth/me/import-notifications"))
      .header("authorization", headers())
      .json(&invalid)
      .send()
      .await
      .unwrap();
    assert!(
      response.status().is_client_error(),
      "invalid preference must not be accepted"
    );
  }
  let archive = build_zip(&[ZipEntry {
    name: "Hello.md".into(),
    data: b"# Hello\n\nImported body".to_vec(),
  }]);
  let response = client
    .post(format!(
      "{origin}/api/workspaces/import?rehost_external=false"
    ))
    .header("authorization", headers())
    .header("content-type", "application/zip")
    .body(archive)
    .send()
    .await
    .unwrap();
  assert!(
    response.status().is_success(),
    "{}",
    response.text().await.unwrap()
  );
  let job = response.json::<serde_json::Value>().await.unwrap()["job_id"]
    .as_str()
    .unwrap()
    .to_owned();
  let message = tokio::time::timeout(Duration::from_secs(20), rx.recv())
    .await
    .unwrap()
    .unwrap();
  let decoded = reqwest::Url::parse(&format!("http://capture/?{message}")).unwrap();
  let params: std::collections::HashMap<_, _> = decoded.query_pairs().into_owned().collect();
  assert_eq!(params["ToAddress"], email);
  assert!(params["Subject"].contains("导入完成"));
  assert!(params["HtmlBody"].contains("成功页面：1；失败或未完成页面：0"));
  assert!(params["HtmlBody"].contains(&origin));
  assert!(params.contains_key("Signature"));
  let id = Uuid::parse_str(&job).unwrap();
  let terminal: (String, i32, i32) =
    sqlx::query_as("SELECT status,total,done FROM import_jobs WHERE id=$1")
      .bind(id)
      .fetch_one(&db)
      .await
      .unwrap();
  assert_eq!(terminal, ("done".into(), 1, 1));
  let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
  loop {
    let delivery: Option<String> =
      sqlx::query_scalar("SELECT completion_email_state FROM import_jobs WHERE id=$1")
        .bind(id)
        .fetch_one(&db)
        .await
        .unwrap();
    if delivery.as_deref() == Some("sent") {
      break;
    }
    assert!(
      tokio::time::Instant::now() < deadline,
      "mail transport did not finish"
    );
    tokio::task::yield_now().await;
  }
  // An ordinary appearance-blob replacement cannot reset the account opt-out.
  let off = client
    .put(format!("{origin}/api/auth/me/import-notifications"))
    .header("authorization", headers())
    .json(&serde_json::json!({"enabled":false}))
    .send()
    .await
    .unwrap();
  assert!(off.status().is_success());
  let appearance = client
    .put(format!("{origin}/api/auth/me/settings"))
    .header("authorization", headers())
    .json(&serde_json::json!({"settings":{"themeMode":"dark"}}))
    .send()
    .await
    .unwrap();
  assert!(appearance.status().is_success());
  let enabled: bool =
    sqlx::query_scalar("SELECT import_completion_email_enabled FROM users WHERE id=$1")
      .bind(user)
      .fetch_one(&db)
      .await
      .unwrap();
  assert!(!enabled);
  sqlx::query("DELETE FROM workspaces WHERE owner_id=$1")
    .bind(user)
    .execute(&db)
    .await
    .unwrap();
  sqlx::query("DELETE FROM users WHERE id=$1")
    .bind(user)
    .execute(&db)
    .await
    .unwrap();
}
