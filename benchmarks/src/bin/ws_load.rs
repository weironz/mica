//! Mica Real-Protocol Server Collaborative WebSocket Load Tester (`mica-ws-load`).
//!
//! Speaks the real `/ws/workspaces/{ws}/documents/{doc}?v=1` yrs CRDT protocol
//! (`sync.bootstrap`, `sync.push`, `sync.pull`, `sync.update`, `sync.ack`) against
//! a live `mica-api-server` + PostgreSQL instance, then scrapes `/metrics`.
//!
//! Scenarios:
//!   1. Hot Room — N concurrent peers editing the SAME document simultaneously
//!   2. Multi-Room — M concurrent peers spread across R active rooms
//!   3. Reconnect Storm — K peers disconnect, accumulate offline CRDT edits, and
//!      reconnect simultaneously (sync.pull + sync.push burst)
//!
//! Usage:
//!   cargo run -p mica-bench --release --bin mica-ws-load -- --base http://127.0.0.1:18088

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use futures_util::{SinkExt, StreamExt};
use mica_core::{Block, MicaDoc};
use reqwest::Client;
use serde_json::{Value, json};
use tokio::sync::{Barrier, Mutex};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use uuid::Uuid;

struct Config {
  base: String,
  email: String,
  password: String,
  hot_peers: usize,
  hot_pushes: usize,
  multi_rooms: usize,
  multi_peers_per_room: usize,
  multi_pushes: usize,
  storm_peers: usize,
  storm_offline_ops: usize,
  json_mode: bool,
}

impl Default for Config {
  fn default() -> Self {
    Self {
      base: "http://127.0.0.1:18088".to_string(),
      email: "wsload@mica.dev".to_string(),
      password: "Mica#PerfTest99!".to_string(),
      hot_peers: 30,
      hot_pushes: 8,
      multi_rooms: 10,
      multi_peers_per_room: 5,
      multi_pushes: 6,
      storm_peers: 30,
      storm_offline_ops: 5,
      json_mode: false,
    }
  }
}

fn parse_args() -> Config {
  let mut cfg = Config::default();
  let mut args = std::env::args().skip(1);
  while let Some(arg) = args.next() {
    match arg.as_str() {
      "--base" => {
        if let Some(v) = args.next() {
          cfg.base = v.trim_end_matches('/').to_string();
        }
      }
      "--email" => {
        if let Some(v) = args.next() {
          cfg.email = v;
        }
      }
      "--password" => {
        if let Some(v) = args.next() {
          cfg.password = v;
        }
      }
      "--hot-peers" => {
        if let Some(v) = args.next().and_then(|s| s.parse().ok()) {
          cfg.hot_peers = v;
        }
      }
      "--hot-pushes" => {
        if let Some(v) = args.next().and_then(|s| s.parse().ok()) {
          cfg.hot_pushes = v;
        }
      }
      "--multi-rooms" => {
        if let Some(v) = args.next().and_then(|s| s.parse().ok()) {
          cfg.multi_rooms = v;
        }
      }
      "--multi-peers-per-room" => {
        if let Some(v) = args.next().and_then(|s| s.parse().ok()) {
          cfg.multi_peers_per_room = v;
        }
      }
      "--storm-peers" => {
        if let Some(v) = args.next().and_then(|s| s.parse().ok()) {
          cfg.storm_peers = v;
        }
      }
      "--json" => cfg.json_mode = true,
      "--help" | "-h" => {
        println!(
          "Usage: mica-ws-load [--base URL] [--email E] [--password P] \
           [--hot-peers N] [--hot-pushes N] [--multi-rooms N] [--multi-peers-per-room N] \
           [--storm-peers N] [--json]"
        );
        std::process::exit(0);
      }
      other => {
        eprintln!("Unknown argument: {other}");
        std::process::exit(2);
      }
    }
  }
  cfg
}

struct Stats {
  p50_ms: f64,
  p95_ms: f64,
  p99_ms: f64,
  max_ms: f64,
}

fn summarize_ms(mut samples: Vec<Duration>) -> Stats {
  if samples.is_empty() {
    return Stats {
      p50_ms: 0.0,
      p95_ms: 0.0,
      p99_ms: 0.0,
      max_ms: 0.0,
    };
  }
  samples.sort_unstable();
  let pct = |p: f64| -> f64 {
    let idx = (((samples.len() - 1) as f64) * p).round() as usize;
    samples[idx.min(samples.len() - 1)].as_secs_f64() * 1000.0
  };
  Stats {
    p50_ms: pct(0.50),
    p95_ms: pct(0.95),
    p99_ms: pct(0.99),
    max_ms: samples.last().unwrap().as_secs_f64() * 1000.0,
  }
}

fn report(json_mode: bool, scenario: &str, metric: &str, summary: &str, data: Value) {
  if json_mode {
    println!(
      "{}",
      json!({ "scenario": scenario, "metric": metric, "summary": summary, "data": data })
    );
  } else {
    println!("  {metric:<36} {summary}");
  }
}

fn http_to_ws(base: &str, path: &str) -> String {
  if let Some(rest) = base.strip_prefix("https://") {
    format!("wss://{rest}{path}")
  } else if let Some(rest) = base.strip_prefix("http://") {
    format!("ws://{rest}{path}")
  } else {
    format!("ws://{base}{path}")
  }
}

async fn ensure_auth_and_workspace(http: &Client, cfg: &Config) -> Result<(String, Uuid), String> {
  let try_login = || async {
    http
      .post(format!("{}/api/auth/login", cfg.base))
      .json(&json!({ "email": cfg.email, "password": cfg.password }))
      .send()
      .await
  };
  let mut login = try_login()
    .await
    .map_err(|e| format!("login request failed: {e}"))?;
  if !login.status().is_success() {
    let reg = http
      .post(format!("{}/api/auth/register", cfg.base))
      .json(&json!({
        "email": cfg.email,
        "password": cfg.password,
        "display_name": "Load Tester"
      }))
      .send()
      .await
      .map_err(|e| format!("register request failed: {e}"))?;
    let st = reg.status().as_u16();
    if ![200, 201, 409].contains(&st) {
      let body = reg.text().await.unwrap_or_default();
      return Err(format!("register HTTP {st}: {body}"));
    }
    login = try_login()
      .await
      .map_err(|e| format!("login request failed: {e}"))?;
  }
  if !login.status().is_success() {
    let st = login.status();
    let body = login.text().await.unwrap_or_default();
    return Err(format!("login HTTP {st}: {body}"));
  }
  let login_json: Value = login.json().await.map_err(|e| format!("login json: {e}"))?;
  let token = login_json
    .get("access_token")
    .and_then(Value::as_str)
    .ok_or("missing access_token in login response")?
    .to_string();

  let create_ws = http
    .post(format!("{}/api/workspaces", cfg.base))
    .bearer_auth(&token)
    .json(&json!({ "name": format!("WS-Load-{}", Uuid::new_v4().simple()) }))
    .send()
    .await
    .map_err(|e| format!("create workspace failed: {e}"))?;
  if !create_ws.status().is_success() {
    let st = create_ws.status();
    let body = create_ws.text().await.unwrap_or_default();
    return Err(format!("create workspace HTTP {st}: {body}"));
  }
  let ws_json: Value = create_ws
    .json()
    .await
    .map_err(|e| format!("workspace json: {e}"))?;
  let ws_id_str = ws_json
    .get("workspace")
    .and_then(|w| w.get("id"))
    .or_else(|| ws_json.get("id"))
    .and_then(Value::as_str)
    .ok_or("missing workspace id")?;
  let ws_id = Uuid::parse_str(ws_id_str).map_err(|e| format!("parse ws_id: {e}"))?;
  Ok((token, ws_id))
}

async fn create_seeded_document(
  http: &Client,
  base: &str,
  token: &str,
  ws_id: Uuid,
  name: &str,
) -> Result<(Uuid, String), String> {
  let resp = http
    .post(format!("{base}/api/workspaces/{ws_id}/documents"))
    .bearer_auth(token)
    .json(&json!({ "name": name }))
    .send()
    .await
    .map_err(|e| format!("create doc failed: {e}"))?;
  if !resp.status().is_success() {
    let st = resp.status();
    let body = resp.text().await.unwrap_or_default();
    return Err(format!("create doc HTTP {st}: {body}"));
  }
  let v: Value = resp.json().await.map_err(|e| format!("doc json: {e}"))?;
  let doc_id_str = v
    .get("document")
    .and_then(|d| d.get("id"))
    .or_else(|| v.get("id"))
    .and_then(Value::as_str)
    .ok_or_else(|| format!("missing document id in {v}"))?;
  let doc_id = Uuid::parse_str(doc_id_str).map_err(|e| format!("parse doc_id: {e}"))?;

  // Seed one initial paragraph block via WS bootstrap + push so all peers have a shared target block.
  let (mut ws, doc, _rid) = open_and_bootstrap(base, token, ws_id, doc_id, 900_000).await?;
  let mut doc = doc;
  let blocks = doc.to_blocks();
  let target_block_id = if let Some(b) = blocks.iter().find(|b| b.kind == "paragraph") {
    b.id.clone()
  } else {
    let root_id = {
      let r = doc.root_block_id();
      if r.is_empty() { "root".to_string() } else { r }
    };
    let bid = "p-shared-0".to_string();
    let sv0 = doc.state_vector();
    if !doc.has_block(&root_id) {
      doc.set_blocks(
        &root_id,
        &[
          Block {
            id: root_id.clone(),
            kind: "page".into(),
            text: String::new(),
            data: Value::Null,
            children: vec![bid.clone()],
          },
          Block {
            id: bid.clone(),
            kind: "paragraph".into(),
            text: "init ".into(),
            data: Value::Null,
            children: vec![],
          },
        ],
      );
    } else {
      doc.insert_block(
        &root_id,
        0,
        &Block {
          id: bid.clone(),
          kind: "paragraph".into(),
          text: "init ".into(),
          data: Value::Null,
          children: vec![],
        },
      );
    }
    let delta = doc
      .encode_diff(&sv0)
      .map_err(|e| format!("encode_diff: {e}"))?;
    let _ = push_and_wait_ack(&mut ws, "seed-0", &delta).await?;
    bid
  };
  let _ = ws.close(None).await;
  Ok((doc_id, target_block_id))
}

type WsStream =
  tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn open_ws(base: &str, token: &str, ws_id: Uuid, doc_id: Uuid) -> Result<WsStream, String> {
  let url = http_to_ws(
    base,
    &format!("/ws/workspaces/{ws_id}/documents/{doc_id}?v=1"),
  );
  let mut req = url
    .as_str()
    .into_client_request()
    .map_err(|e| format!("ws request build: {e}"))?;
  req.headers_mut().insert(
    "Authorization",
    HeaderValue::from_str(&format!("Bearer {token}")).map_err(|e| format!("auth header: {e}"))?,
  );
  let (ws, _resp) = connect_async(req)
    .await
    .map_err(|e| format!("ws connect {url}: {e}"))?;
  Ok(ws)
}

async fn open_and_bootstrap(
  base: &str,
  token: &str,
  ws_id: Uuid,
  doc_id: Uuid,
  client_id: u64,
) -> Result<(WsStream, MicaDoc, i64), String> {
  let mut ws = open_ws(base, token, ws_id, doc_id).await?;
  ws.send(Message::Text(
    json!({
      "id": "boot",
      "type": "sync.bootstrap",
      "payload": {}
    })
    .to_string()
    .into(),
  ))
  .await
  .map_err(|e| format!("send bootstrap: {e}"))?;

  loop {
    let msg = tokio::time::timeout(Duration::from_secs(15), ws.next())
      .await
      .map_err(|_| "timeout waiting for sync.base".to_string())?
      .ok_or("ws closed before sync.base")?
      .map_err(|e| format!("ws read error: {e}"))?;
    if let Message::Text(txt) = msg {
      let v: Value = serde_json::from_str(&txt).map_err(|e| format!("json parse: {e}"))?;
      match v.get("type").and_then(Value::as_str) {
        Some("sync.base") => {
          let state_b64 = v
            .get("base")
            .or_else(|| v.get("state"))
            .and_then(Value::as_str)
            .unwrap_or("");
          let head = v
            .get("base_rid")
            .or_else(|| v.get("head"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
          let doc = if state_b64.is_empty() {
            MicaDoc::from_blocks_with_client_id("root", &[], Some(client_id))
          } else {
            let bytes = B64
              .decode(state_b64.as_bytes())
              .map_err(|e| format!("b64 decode base: {e}"))?;
            MicaDoc::from_update_with_client_id(&bytes, Some(client_id))
              .map_err(|e| format!("from_update: {e}"))?
          };
          return Ok((ws, doc, head));
        }
        Some("error") => {
          return Err(format!("server error during bootstrap: {v}"));
        }
        _ => {}
      }
    }
  }
}

async fn push_and_wait_ack(
  ws: &mut WsStream,
  ack_id: &str,
  delta: &[u8],
) -> Result<(i64, Vec<Vec<u8>>), String> {
  ws.send(Message::Text(
    json!({
      "id": ack_id,
      "type": "sync.push",
      "payload": { "update": B64.encode(delta) }
    })
    .to_string()
    .into(),
  ))
  .await
  .map_err(|e| format!("send push: {e}"))?;

  let mut incoming_updates = Vec::new();
  loop {
    let msg = tokio::time::timeout(Duration::from_secs(30), ws.next())
      .await
      .map_err(|_| format!("timeout waiting for ack {ack_id}"))?
      .ok_or("ws closed before ack")?
      .map_err(|e| format!("ws recv: {e}"))?;
    if let Message::Text(txt) = msg {
      let v: Value = serde_json::from_str(&txt).map_err(|e| format!("json: {e}"))?;
      match v.get("type").and_then(Value::as_str) {
        Some("sync.ack") if v.get("ack_id").and_then(Value::as_str) == Some(ack_id) => {
          let rid = v.get("rid").and_then(Value::as_i64).unwrap_or(0);
          return Ok((rid, incoming_updates));
        }
        Some("sync.update") => {
          if let Some(u) = v.get("update").and_then(Value::as_str)
            && let Ok(bytes) = B64.decode(u.as_bytes())
          {
            incoming_updates.push(bytes);
          }
        }
        Some("error") => {
          return Err(format!("server error on push {ack_id}: {v}"));
        }
        _ => {}
      }
    }
  }
}

struct RoomBenchOutcome {
  ack_latencies: Vec<Duration>,
  broadcast_latencies: Vec<Duration>,
  broadcasts_received: usize,
  lag_notices: usize,
  errors: usize,
  converged: bool,
  elapsed: Duration,
}

#[allow(clippy::too_many_arguments)]
async fn run_room_concurrent(
  base: String,
  token: String,
  ws_id: Uuid,
  doc_id: Uuid,
  block_id: String,
  peers: usize,
  pushes_per_peer: usize,
  client_id_offset: u64,
) -> RoomBenchOutcome {
  let start_barrier = Arc::new(Barrier::new(peers));
  let done_barrier = Arc::new(Barrier::new(peers));
  let send_times: Arc<Mutex<HashMap<i64, Instant>>> = Arc::new(Mutex::new(HashMap::new()));
  let expected_total_pushes = peers * pushes_per_peer;

  let t_all = Instant::now();
  let mut handles = Vec::with_capacity(peers);

  for peer_idx in 0..peers {
    let base = base.clone();
    let token = token.clone();
    let block_id = block_id.clone();
    let start_barrier = Arc::clone(&start_barrier);
    let done_barrier = Arc::clone(&done_barrier);
    let send_times = Arc::clone(&send_times);

    handles.push(tokio::spawn(async move {
      let mut ack_lats = Vec::with_capacity(pushes_per_peer);
      let mut bcast_lats = Vec::new();
      let mut bcast_count = 0usize;
      let mut lags = 0usize;
      let mut errs = 0usize;

      let client_id = client_id_offset + peer_idx as u64 + 1;
      let (ws, mut doc, _head) =
        match open_and_bootstrap(&base, &token, ws_id, doc_id, client_id).await {
          Ok(x) => x,
          Err(_) => {
            start_barrier.wait().await;
            done_barrier.wait().await;
            return (
              ack_lats,
              bcast_lats,
              bcast_count,
              lags,
              1usize,
              String::new(),
            );
          }
        };

      let (mut sink, mut stream) = ws.split();
      start_barrier.wait().await;

      // Each peer sends `pushes_per_peer` valid CRDT updates while continuously draining broadcasts.
      let mut pending_ack: Option<(String, Instant)> = None;
      let mut sent = 0usize;
      let mut peer_remote_needed = expected_total_pushes - pushes_per_peer;

      // Kick off first push immediately (staggered slightly by peer_idx % 8 ms to avoid single-tick spike).
      let stagger = Duration::from_millis((peer_idx % 8) as u64);
      if !stagger.is_zero() {
        tokio::time::sleep(stagger).await;
      }

      while sent < pushes_per_peer || pending_ack.is_some() {
        if pending_ack.is_none() && sent < pushes_per_peer {
          let sv = doc.state_vector();
          let token_str = format!("P{peer_idx}:{sent} ");
          doc.text_insert(&block_id, 0, &token_str);
          if let Ok(delta) = doc.encode_diff(&sv) {
            let aid = format!("p{peer_idx}-{sent}");
            let t0 = Instant::now();
            let frame = json!({
              "id": aid,
              "type": "sync.push",
              "payload": { "update": B64.encode(&delta) }
            })
            .to_string();
            if sink.send(Message::Text(frame.into())).await.is_err() {
              errs += 1;
              break;
            }
            pending_ack = Some((aid, t0));
            sent += 1;
          } else {
            errs += 1;
            break;
          }
        }

        match tokio::time::timeout(Duration::from_secs(20), stream.next()).await {
          Ok(Some(Ok(Message::Text(txt)))) => {
            if let Ok(v) = serde_json::from_str::<Value>(&txt) {
              match v.get("type").and_then(Value::as_str) {
                Some("sync.ack") => {
                  if let Some((ref expected, t0)) = pending_ack
                    && v.get("ack_id").and_then(Value::as_str) == Some(expected.as_str())
                  {
                    let elapsed = t0.elapsed();
                    ack_lats.push(elapsed);
                    if let Some(rid) = v.get("rid").and_then(Value::as_i64) {
                      send_times.lock().await.insert(rid, t0);
                    }
                    pending_ack = None;
                  }
                }
                Some("sync.update") => {
                  let now = Instant::now();
                  if let Some(u) = v.get("update").and_then(Value::as_str)
                    && let Ok(bytes) = B64.decode(u.as_bytes())
                  {
                    let _ = doc.apply_update(&bytes);
                    bcast_count += 1;
                    peer_remote_needed = peer_remote_needed.saturating_sub(1);
                    if let Some(rid) = v.get("rid").and_then(Value::as_i64)
                      && let Some(t_send) = send_times.lock().await.get(&rid).copied()
                    {
                      bcast_lats.push(now.saturating_duration_since(t_send));
                    }
                  }
                }
                Some("error") => {
                  if v.get("code").and_then(Value::as_str) == Some("client_out_of_date") {
                    lags += 1;
                  } else {
                    errs += 1;
                  }
                }
                _ => {}
              }
            }
          }
          _ => {
            errs += 1;
            break;
          }
        }
      }

      // Drain remaining remote broadcasts until converged or short idle timeout.
      let drain_deadline = Instant::now() + Duration::from_secs(8);
      while peer_remote_needed > 0 && Instant::now() < drain_deadline {
        match tokio::time::timeout(Duration::from_millis(800), stream.next()).await {
          Ok(Some(Ok(Message::Text(txt)))) => {
            if let Ok(v) = serde_json::from_str::<Value>(&txt) {
              match v.get("type").and_then(Value::as_str) {
                Some("sync.update") => {
                  let now = Instant::now();
                  if let Some(u) = v.get("update").and_then(Value::as_str)
                    && let Ok(bytes) = B64.decode(u.as_bytes())
                  {
                    let _ = doc.apply_update(&bytes);
                    bcast_count += 1;
                    peer_remote_needed = peer_remote_needed.saturating_sub(1);
                    if let Some(rid) = v.get("rid").and_then(Value::as_i64)
                      && let Some(t_send) = send_times.lock().await.get(&rid).copied()
                    {
                      bcast_lats.push(now.saturating_duration_since(t_send));
                    }
                  }
                }
                Some("error")
                  if v.get("code").and_then(Value::as_str) == Some("client_out_of_date") =>
                {
                  lags += 1;
                }
                _ => {}
              }
            }
          }
          _ => break,
        }
      }

      done_barrier.wait().await;
      let final_text = doc
        .to_blocks()
        .into_iter()
        .find(|b| b.id == block_id)
        .map(|b| b.text)
        .unwrap_or_default();
      let _ = sink.close().await;
      (ack_lats, bcast_lats, bcast_count, lags, errs, final_text)
    }));
  }

  let mut all_acks = Vec::new();
  let mut all_bcasts = Vec::new();
  let mut total_bcast = 0usize;
  let mut total_lags = 0usize;
  let mut total_errs = 0usize;
  let mut texts = Vec::with_capacity(peers);

  for h in handles {
    if let Ok((a, b, bc, l, e, t)) = h.await {
      all_acks.extend(a);
      all_bcasts.extend(b);
      total_bcast += bc;
      total_lags += l;
      total_errs += e;
      texts.push(t);
    }
  }
  let converged = !texts.is_empty() && !texts[0].is_empty() && texts.iter().all(|t| t == &texts[0]);

  RoomBenchOutcome {
    ack_latencies: all_acks,
    broadcast_latencies: all_bcasts,
    broadcasts_received: total_bcast,
    lag_notices: total_lags,
    errors: total_errs,
    converged,
    elapsed: t_all.elapsed(),
  }
}

async fn run_reconnect_storm(
  base: String,
  token: String,
  ws_id: Uuid,
  doc_id: Uuid,
  block_id: String,
  peers: usize,
  offline_ops: usize,
) -> (Vec<Duration>, Duration, usize, bool) {
  // Phase 1: All peers bootstrap, then disconnect.
  let mut offline_replicas: Vec<(MicaDoc, i64, Vec<Vec<u8>>)> = Vec::with_capacity(peers);
  for i in 0..peers {
    let (mut ws, mut doc, head) =
      open_and_bootstrap(&base, &token, ws_id, doc_id, 500_000 + i as u64)
        .await
        .expect("storm initial bootstrap");
    let _ = ws.close(None).await;

    // Accumulate offline edits locally while disconnected.
    let mut queued = Vec::with_capacity(offline_ops);
    for step in 0..offline_ops {
      let sv = doc.state_vector();
      doc.text_insert(&block_id, 0, &format!("OFF{i}:{step} "));
      let delta = doc.encode_diff(&sv).expect("offline delta");
      queued.push(delta);
    }
    offline_replicas.push((doc, head, queued));
  }

  // Phase 2: All peers reconnect simultaneously, pull + push queued updates.
  let barrier = Arc::new(Barrier::new(peers));
  let t_storm = Instant::now();
  let mut handles = Vec::with_capacity(peers);
  for (i, (mut doc, head, queued)) in offline_replicas.into_iter().enumerate() {
    let base = base.clone();
    let token = token.clone();
    let barrier = Arc::clone(&barrier);
    handles.push(tokio::spawn(async move {
      barrier.wait().await;
      let t0 = Instant::now();
      let mut errs = 0usize;
      let mut ws = match open_ws(&base, &token, ws_id, doc_id).await {
        Ok(w) => w,
        Err(_) => return (t0.elapsed(), 1usize, doc),
      };
      // Catch up via sync.pull
      let sv_b64 = B64.encode(doc.state_vector());
      let pull = json!({
        "id": format!("pull-{i}"),
        "type": "sync.pull",
        "payload": { "since_rid": head, "sv": sv_b64 }
      })
      .to_string();
      if ws.send(Message::Text(pull.into())).await.is_err() {
        return (t0.elapsed(), 1usize, doc);
      }
      // Wait for sync.updates or sync.base
      loop {
        match tokio::time::timeout(Duration::from_secs(20), ws.next()).await {
          Ok(Some(Ok(Message::Text(txt)))) => {
            if let Ok(v) = serde_json::from_str::<Value>(&txt) {
              match v.get("type").and_then(Value::as_str) {
                Some("sync.updates") => {
                  if let Some(arr) = v.get("updates").and_then(Value::as_array) {
                    for u in arr {
                      if let Some(b64) = u.get("update").and_then(Value::as_str)
                        && let Ok(bytes) = B64.decode(b64.as_bytes())
                      {
                        let _ = doc.apply_update(&bytes);
                      }
                    }
                  }
                  break;
                }
                Some("sync.base") => {
                  if let Some(b64) = v
                    .get("base")
                    .or_else(|| v.get("state"))
                    .and_then(Value::as_str)
                    && let Ok(bytes) = B64.decode(b64.as_bytes())
                  {
                    let _ = doc.apply_update(&bytes);
                  }
                  break;
                }
                Some("error") => {
                  errs += 1;
                  break;
                }
                _ => {}
              }
            }
          }
          _ => {
            errs += 1;
            break;
          }
        }
      }
      // Push all offline updates
      for (k, delta) in queued.iter().enumerate() {
        match push_and_wait_ack(&mut ws, &format!("storm-{i}-{k}"), delta).await {
          Ok((_rid, incoming)) => {
            for inc in incoming {
              let _ = doc.apply_update(&inc);
            }
          }
          Err(_) => {
            errs += 1;
            break;
          }
        }
      }
      let lat = t0.elapsed();
      let _ = ws.close(None).await;
      (lat, errs, doc)
    }));
  }

  let mut lats = Vec::with_capacity(peers);
  let mut total_errs = 0usize;
  for h in handles {
    if let Ok((l, e, _doc)) = h.await {
      lats.push(l);
      total_errs += e;
    }
  }
  let storm_total = t_storm.elapsed();

  // Verify server converged state contains EVERY offline peer's edits.
  let (mut check_ws, server_doc, _head) = open_and_bootstrap(&base, &token, ws_id, doc_id, 999_999)
    .await
    .expect("storm verify bootstrap");
  let _ = check_ws.close(None).await;
  let final_text = server_doc
    .to_blocks()
    .into_iter()
    .find(|b| b.id == block_id)
    .map(|b| b.text)
    .unwrap_or_default();
  let mut all_present = true;
  for i in 0..peers {
    for step in 0..offline_ops {
      if !final_text.contains(&format!("OFF{i}:{step}")) {
        all_present = false;
        break;
      }
    }
  }
  (lats, storm_total, total_errs, all_present)
}

async fn scrape_key_metrics(http: &Client, base: &str) -> HashMap<String, String> {
  let mut out = HashMap::new();
  let Ok(resp) = http.get(format!("{base}/metrics")).send().await else {
    return out;
  };
  let Ok(body) = resp.text().await else {
    return out;
  };
  let interesting = [
    "mica_ws_connections",
    "mica_crdt_lag_notices_total",
    "mica_crdt_integrity_failures_total",
    "mica_crdt_push_rejected_total",
    "mica_crdt_push_bytes_total",
    "mica_crdt_push_duration_seconds_count",
    "mica_crdt_push_duration_seconds_sum",
    "mica_db_pool_connections",
  ];
  for line in body.lines() {
    if line.starts_with('#') || line.trim().is_empty() {
      continue;
    }
    for prefix in interesting {
      if line.starts_with(prefix)
        && let Some((k, v)) = line.rsplit_once(' ')
      {
        out.insert(k.to_string(), v.to_string());
      }
    }
  }
  out
}

#[tokio::main]
async fn main() {
  let cfg = parse_args();
  if !cfg.json_mode {
    println!(
      "Mica Real-Protocol WebSocket Collaboration Load Test ({})",
      cfg.base
    );
  }
  let http = Client::builder()
    .timeout(Duration::from_secs(30))
    .build()
    .expect("http client");

  let (token, ws_id) = ensure_auth_and_workspace(&http, &cfg)
    .await
    .expect("auth + workspace setup");

  // ── Scenario 1: Hot Room ──────────────────────────────────────────────────
  if !cfg.json_mode {
    println!(
      "\n=== [1/3] Hot Room ({} concurrent peers x {} pushes on 1 doc) ===",
      cfg.hot_peers, cfg.hot_pushes
    );
  }
  let (hot_doc, hot_block) =
    create_seeded_document(&http, &cfg.base, &token, ws_id, "Hot Room Doc")
      .await
      .expect("seed hot room doc");
  let hot = run_room_concurrent(
    cfg.base.clone(),
    token.clone(),
    ws_id,
    hot_doc,
    hot_block,
    cfg.hot_peers,
    cfg.hot_pushes,
    10_000,
  )
  .await;
  let hot_ack = summarize_ms(hot.ack_latencies.clone());
  let hot_bcast = summarize_ms(hot.broadcast_latencies.clone());
  let hot_throughput = hot.ack_latencies.len() as f64 / hot.elapsed.as_secs_f64().max(1e-6);
  report(
    cfg.json_mode,
    "hot_room",
    "push_to_ack_latency",
    &format!(
      "p50={:.2} ms  p95={:.2} ms  p99={:.2} ms  max={:.2} ms ({:.1} pushes/s)",
      hot_ack.p50_ms, hot_ack.p95_ms, hot_ack.p99_ms, hot_ack.max_ms, hot_throughput
    ),
    json!({
      "p50_ms": hot_ack.p50_ms, "p95_ms": hot_ack.p95_ms,
      "p99_ms": hot_ack.p99_ms, "max_ms": hot_ack.max_ms,
      "pushes": hot.ack_latencies.len(), "pushes_per_sec": hot_throughput
    }),
  );
  report(
    cfg.json_mode,
    "hot_room",
    "peer_to_peer_broadcast_latency",
    &format!(
      "p50={:.2} ms  p95={:.2} ms  p99={:.2} ms ({} fan-outs, lags={}, errs={}, converged={})",
      hot_bcast.p50_ms,
      hot_bcast.p95_ms,
      hot_bcast.p99_ms,
      hot.broadcasts_received,
      hot.lag_notices,
      hot.errors,
      hot.converged
    ),
    json!({
      "p50_ms": hot_bcast.p50_ms, "p95_ms": hot_bcast.p95_ms, "p99_ms": hot_bcast.p99_ms,
      "broadcasts": hot.broadcasts_received, "lags": hot.lag_notices,
      "errors": hot.errors, "converged": hot.converged
    }),
  );

  // ── Scenario 2: Multi-Room Concurrent Connections ─────────────────────────
  let total_multi_peers = cfg.multi_rooms * cfg.multi_peers_per_room;
  if !cfg.json_mode {
    println!(
      "\n=== [2/3] Multi-Room ({} rooms x {} peers = {} concurrent WS x {} pushes) ===",
      cfg.multi_rooms, cfg.multi_peers_per_room, total_multi_peers, cfg.multi_pushes
    );
  }
  let mut room_specs = Vec::with_capacity(cfg.multi_rooms);
  for r in 0..cfg.multi_rooms {
    let (doc_id, block_id) =
      create_seeded_document(&http, &cfg.base, &token, ws_id, &format!("Multi Room {r}"))
        .await
        .expect("seed multi room doc");
    room_specs.push((doc_id, block_id));
  }
  let t_multi = Instant::now();
  let mut room_tasks = Vec::with_capacity(cfg.multi_rooms);
  for (r, (doc_id, block_id)) in room_specs.into_iter().enumerate() {
    let base = cfg.base.clone();
    let token = token.clone();
    let peers = cfg.multi_peers_per_room;
    let pushes = cfg.multi_pushes;
    room_tasks.push(tokio::spawn(async move {
      run_room_concurrent(
        base,
        token,
        ws_id,
        doc_id,
        block_id,
        peers,
        pushes,
        100_000 + (r as u64) * 1_000,
      )
      .await
    }));
  }
  let mut multi_acks = Vec::new();
  let mut multi_bcasts = Vec::new();
  let mut multi_bcast_count = 0usize;
  let mut multi_lags = 0usize;
  let mut multi_errs = 0usize;
  let mut all_rooms_converged = true;
  for task in room_tasks {
    let res = task.await.expect("multi room join");
    multi_acks.extend(res.ack_latencies);
    multi_bcasts.extend(res.broadcast_latencies);
    multi_bcast_count += res.broadcasts_received;
    multi_lags += res.lag_notices;
    multi_errs += res.errors;
    all_rooms_converged &= res.converged;
  }
  let multi_elapsed = t_multi.elapsed();
  let m_ack = summarize_ms(multi_acks.clone());
  let m_bcast = summarize_ms(multi_bcasts);
  let m_tput = multi_acks.len() as f64 / multi_elapsed.as_secs_f64().max(1e-6);
  report(
    cfg.json_mode,
    "multi_room",
    "push_to_ack_latency",
    &format!(
      "p50={:.2} ms  p95={:.2} ms  p99={:.2} ms ({:.1} pushes/s, {} total pushes)",
      m_ack.p50_ms,
      m_ack.p95_ms,
      m_ack.p99_ms,
      m_tput,
      multi_acks.len()
    ),
    json!({
      "p50_ms": m_ack.p50_ms, "p95_ms": m_ack.p95_ms, "p99_ms": m_ack.p99_ms,
      "pushes": multi_acks.len(), "pushes_per_sec": m_tput
    }),
  );
  report(
    cfg.json_mode,
    "multi_room",
    "broadcast_and_convergence",
    &format!(
      "p50={:.2} ms  p95={:.2} ms  p99={:.2} ms ({} fan-outs, lags={}, errs={}, all_converged={})",
      m_bcast.p50_ms,
      m_bcast.p95_ms,
      m_bcast.p99_ms,
      multi_bcast_count,
      multi_lags,
      multi_errs,
      all_rooms_converged
    ),
    json!({
      "p50_ms": m_bcast.p50_ms, "p95_ms": m_bcast.p95_ms, "p99_ms": m_bcast.p99_ms,
      "broadcasts": multi_bcast_count, "lags": multi_lags,
      "errors": multi_errs, "all_converged": all_rooms_converged
    }),
  );

  // ── Scenario 3: Reconnect Storm ───────────────────────────────────────────
  if !cfg.json_mode {
    println!(
      "\n=== [3/3] Reconnect Storm ({} peers x {} offline edits reconnecting simultaneously) ===",
      cfg.storm_peers, cfg.storm_offline_ops
    );
  }
  let (storm_doc, storm_block) =
    create_seeded_document(&http, &cfg.base, &token, ws_id, "Reconnect Storm Doc")
      .await
      .expect("seed storm doc");
  let (storm_lats, storm_total, storm_errs, storm_ok) = run_reconnect_storm(
    cfg.base.clone(),
    token.clone(),
    ws_id,
    storm_doc,
    storm_block,
    cfg.storm_peers,
    cfg.storm_offline_ops,
  )
  .await;
  let s_stats = summarize_ms(storm_lats);
  report(
    cfg.json_mode,
    "reconnect_storm",
    "reconcile_latency",
    &format!(
      "total={:.1} ms | per-peer p50={:.2} ms  p95={:.2} ms  p99={:.2} ms (errs={}, all_edits_preserved={})",
      storm_total.as_secs_f64() * 1000.0,
      s_stats.p50_ms,
      s_stats.p95_ms,
      s_stats.p99_ms,
      storm_errs,
      storm_ok
    ),
    json!({
      "total_ms": storm_total.as_secs_f64() * 1000.0,
      "p50_ms": s_stats.p50_ms, "p95_ms": s_stats.p95_ms, "p99_ms": s_stats.p99_ms,
      "errors": storm_errs, "all_edits_preserved": storm_ok
    }),
  );

  // ── Prometheus /metrics Snapshot ──────────────────────────────────────────
  let metrics = scrape_key_metrics(&http, &cfg.base).await;
  if !cfg.json_mode {
    println!("\n=== Server /metrics Snapshot ===");
    let mut keys: Vec<&String> = metrics.keys().collect();
    keys.sort();
    for k in keys {
      println!("  {k:<44} {}", metrics[k]);
    }
    println!("\nWS load test completed.");
  } else {
    println!(
      "{}",
      json!({ "scenario": "metrics", "metric": "prometheus_snapshot", "data": metrics })
    );
  }
}
