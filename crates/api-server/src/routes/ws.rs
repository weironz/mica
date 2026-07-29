use std::sync::Arc;

use axum::{
  extract::{
    Path, Query, State,
    ws::{Message, WebSocket, WebSocketUpgrade},
  },
  http::HeaderMap,
  http::header::AUTHORIZATION,
  response::Response,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use mica_app_core::{
  AppState, PresenceEntry, Room,
  documents::DocumentOperation,
  store::{self, AppliedUpdate},
  sync,
};
use mica_infra::{ApiError, ApiResult};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::broadcast::error::RecvError;
use uuid::Uuid;

use crate::routes::auth::user_id_from_token;
use crate::routes::documents::{
  DocumentPermissions, ensure_workspace_member, permissions_for_role, workspace_role,
};

/// The WS sync protocol this server speaks.
///
/// `1` = `sync.bootstrap` / `sync.pull` / `sync.push` with the optional state
/// vector. A client that sends nothing is `0`: it predates the parameter, which
/// in practice means it predates this gate.
///
/// Bump this only for a change an old client cannot survive. Everything the
/// protocol has grown so far has been additive by discipline (old clients never
/// send the new field, old servers ignore it), and additive changes must NOT
/// bump it — the number exists to mark the breaks, and a version that moves for
/// compatible changes would force upgrades nobody needed.
pub const WS_PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Deserialize)]
pub struct ConnectQuery {
  token: Option<String>,
  /// Client's protocol version (`v` in the query string). Absent = 0.
  v: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct ClientEnvelope {
  #[serde(default)]
  id: Option<String>,
  #[serde(rename = "type")]
  kind: String,
  #[serde(default)]
  payload: Value,
}

#[derive(Debug, Deserialize)]
struct UpdatePayload {
  operations: Vec<DocumentOperation>,
}

/// `GET /ws/workspaces/{workspace_id}/documents/{document_id}`
///
/// Authenticates the upgrade request (token via `Authorization` header or
/// `?token=` query), verifies workspace membership, then hands the socket to the
/// per-connection loop.
pub async fn document_socket(
  State(state): State<AppState>,
  Path((workspace_id, document_id)): Path<(Uuid, Uuid)>,
  Query(query): Query<ConnectQuery>,
  headers: HeaderMap,
  upgrade: WebSocketUpgrade,
) -> ApiResult<Response> {
  // Version gate first — before auth, and before the upgrade.
  //
  // Before the upgrade so a client too old to be handled gets a clean HTTP error
  // carrying a machine code, instead of a socket that opens and then misbehaves
  // in ways neither side can explain. Before AUTH because an old client whose
  // token also happens to have lapsed would otherwise be told `unauthorized` and
  // sent off to sign in again — it would sign in, reconnect, and be refused
  // exactly the same way, forever. The version is a public parameter and the
  // check reveals nothing, so there is no reason to make it wait behind a secret.
  let client_protocol = check_protocol(query.v, state.config.ws_min_protocol)?;

  let token = token_from_request(&headers, &query).ok_or(ApiError::Unauthorized)?;
  let user_id = user_id_from_token(&state, &token)?;
  if client_protocol < WS_PROTOCOL_VERSION {
    // The signal that decides when the floor can be raised. Logged only for
    // below-current clients, so ordinary traffic stays quiet and this line
    // appearing at all means "someone out there still needs the old behaviour".
    tracing::info!(
      client_protocol,
      server_protocol = WS_PROTOCOL_VERSION,
      "ws: client below current sync protocol"
    );
  }

  ensure_workspace_member(&state.db, workspace_id, user_id).await?;
  let role = workspace_role(&state.db, workspace_id, user_id)
    .await?
    .ok_or(ApiError::NotFound)?;
  let permissions = permissions_for_role(&role);

  // Confirm the document exists in this workspace before upgrading, so an
  // invalid id fails as a clean HTTP error rather than a dropped socket.
  store::fetch_document(&state.db, workspace_id, document_id)
    .await?
    .ok_or(ApiError::NotFound)?;

  Ok(upgrade.on_upgrade(move |socket| {
    run_connection(
      socket,
      state,
      workspace_id,
      document_id,
      user_id,
      permissions,
    )
  }))
}

/// Resolve the client's announced protocol version against the server's floor.
///
/// Absent means 0 — a client older than the parameter itself. `client_too_old`
/// is a stable machine code, deliberately distinct from an auth failure, so the
/// client can tell the user to update instead of to sign in again.
fn check_protocol(announced: Option<u32>, floor: u32) -> ApiResult<u32> {
  let client_protocol = announced.unwrap_or(0);
  if client_protocol < floor {
    return Err(ApiError::BadRequestCode(
      "client_too_old",
      format!("client speaks sync protocol {client_protocol}; this server requires at least {floor}"),
    ));
  }
  Ok(client_protocol)
}

fn token_from_request(headers: &HeaderMap, query: &ConnectQuery) -> Option<String> {
  if let Some(token) = headers
    .get(AUTHORIZATION)
    .and_then(|value| value.to_str().ok())
    .and_then(|value| value.strip_prefix("Bearer "))
  {
    return Some(token.to_string());
  }

  query
    .token
    .as_ref()
    .map(|token| token.trim().to_string())
    .filter(|token| !token.is_empty())
}

async fn run_connection(
  mut socket: WebSocket,
  state: AppState,
  workspace_id: Uuid,
  document_id: Uuid,
  user_id: Uuid,
  permissions: DocumentPermissions,
) {
  let connection_id = Uuid::new_v4();
  let room = state.hub.join(document_id);
  let mut events = room.subscribe();

  if send_bootstrap(
    &mut socket,
    &state,
    &room,
    workspace_id,
    document_id,
    connection_id,
    permissions,
  )
  .await
  .is_err()
  {
    return;
  }

  loop {
    tokio::select! {
      incoming = socket.recv() => {
        match incoming {
          Some(Ok(Message::Text(text))) => {
            let replies = handle_client_message(
              text.as_str(),
              &state,
              &room,
              connection_id,
              user_id,
              workspace_id,
              document_id,
              permissions,
            )
            .await;
            if send_all(&mut socket, replies).await.is_err() {
              break;
            }
          }
          Some(Ok(Message::Close(_))) | None => break,
          Some(Ok(_)) => {} // Ping/Pong handled by axum; Binary is unused.
          Some(Err(_)) => break,
        }
      }
      broadcast = events.recv() => {
        match broadcast {
          Ok(message) => {
            // Skip our own echoes; the originator gets a direct ack instead.
            if message.origin != connection_id
              && socket
                .send(Message::Text(message.text.to_string().into()))
                .await
                .is_err()
            {
              break;
            }
          }
          Err(RecvError::Lagged(_)) => {
            let notice = json!({
              "type": "error",
              "code": "client_out_of_date",
              "message": "missed updates; reload the document to resync",
            });
            if socket
              .send(Message::Text(notice.to_string().into()))
              .await
              .is_err()
            {
              break;
            }
          }
          Err(RecvError::Closed) => break,
        }
      }
    }
  }

  if room.remove_presence(connection_id).is_some() {
    let leave = json!({
      "type": "presence.leave",
      "document_id": document_id,
      "connection_id": connection_id,
      "user_id": user_id,
    });
    room.broadcast(connection_id, Arc::from(leave.to_string()));
  }
}

async fn send_bootstrap(
  socket: &mut WebSocket,
  state: &AppState,
  room: &Room,
  workspace_id: Uuid,
  document_id: Uuid,
  connection_id: Uuid,
  permissions: DocumentPermissions,
) -> Result<(), ()> {
  let document = store::fetch_document(&state.db, workspace_id, document_id)
    .await
    .map_err(|_| ())?
    .ok_or(())?;
  let snapshot = store::latest_snapshot(&state.db, document_id)
    .await
    .map_err(|_| ())?
    .ok_or(())?;

  let bootstrap = json!({
    "type": "document.bootstrap",
    "document_id": document_id,
    "connection_id": connection_id,
    "server_seq": document.current_seq,
    "snapshot": snapshot.payload,
    "permissions": permissions,
  });
  socket
    .send(Message::Text(bootstrap.to_string().into()))
    .await
    .map_err(|_| ())?;

  let presence_state = json!({
    "type": "presence.state",
    "document_id": document_id,
    "presences": room.presences(),
  });
  socket
    .send(Message::Text(presence_state.to_string().into()))
    .await
    .map_err(|_| ())
}

#[allow(clippy::too_many_arguments)]
async fn handle_client_message(
  raw: &str,
  state: &AppState,
  room: &Room,
  connection_id: Uuid,
  user_id: Uuid,
  workspace_id: Uuid,
  document_id: Uuid,
  permissions: DocumentPermissions,
) -> Vec<String> {
  let envelope = match serde_json::from_str::<ClientEnvelope>(raw) {
    Ok(envelope) => envelope,
    Err(_) => {
      return vec![error_message(
        None,
        "invalid_message",
        "message is not valid JSON",
      )];
    }
  };
  let ack_id = envelope.id.as_deref();

  match envelope.kind.as_str() {
    "ping" => vec![json!({ "type": "pong" }).to_string()],
    "presence.update" => {
      room.set_presence(PresenceEntry {
        connection_id,
        user_id,
        data: envelope.payload.clone(),
      });
      let event = json!({
        "type": "presence.update",
        "document_id": document_id,
        "connection_id": connection_id,
        "user_id": user_id,
        "data": envelope.payload,
      });
      room.broadcast(connection_id, Arc::from(event.to_string()));
      Vec::new()
    }
    "document.update" => {
      if !permissions.can_write {
        return vec![error_message(
          ack_id,
          "permission_denied",
          "you do not have permission to edit this document",
        )];
      }

      let payload = match serde_json::from_value::<UpdatePayload>(envelope.payload) {
        Ok(payload) => payload,
        Err(error) => {
          return vec![error_message(
            ack_id,
            "invalid_payload",
            &format!("invalid update payload: {error}"),
          )];
        }
      };
      if payload.operations.is_empty() {
        return vec![error_message(
          ack_id,
          "invalid_payload",
          "at least one operation is required",
        )];
      }

      match store::apply_document_operations(
        &state.db,
        workspace_id,
        document_id,
        user_id,
        &payload.operations,
      )
      .await
      {
        Ok(applied) => {
          // Fan out to the rest of the room, then ack the originator directly.
          room.broadcast(
            connection_id,
            Arc::from(accepted_event(&applied, None).to_string()),
          );
          vec![accepted_event(&applied, ack_id).to_string()]
        }
        Err(error) => vec![error_message(
          ack_id,
          error_code(&error),
          &error.to_string(),
        )],
      }
    }
    // ── yrs CRDT sync (P2-M4), parallel to the op path above ───────────────
    "sync.bootstrap" => {
      // Fast-forward base for a client opening the doc: full yrs state + the rid
      // it is current to. The base is built lazily from the op snapshot on first
      // access, so existing documents work without a migration pass. P4-3: a
      // client that already holds a replica may send its state vector (`sv`,
      // base64) — it then gets the minimal diff instead of the full state (same
      // `sync.base` shape; applying either is the same yrs operation).
      let client_sv = client_sv(&envelope.payload);
      match sync::bootstrap_base(&state.db, document_id).await {
        Ok(base) => vec![base_message(&base, client_sv.as_deref(), ack_id, document_id)],
        Err(error) => vec![error_message(ack_id, error_code(&error), &error.to_string())],
      }
    }
    "sync.pull" => {
      // Incremental catch-up: every update for this doc after the client's cursor
      // (cold start / offline reconnect). Rooms are per-document, so the cursor is
      // per-document too.
      let since_rid = envelope
        .payload
        .get("since_rid")
        .and_then(Value::as_i64)
        .unwrap_or(0);
      // P4-3: the optional client state vector turns a prune-forced re-bootstrap
      // into a minimal diff instead of a full-doc download.
      let client_sv = client_sv(&envelope.payload);
      match sync::catch_up_document(&state.db, document_id, since_rid, 1000).await {
        // Cursor fell behind the pruned window → re-bootstrap from the base.
        Ok(sync::CatchUp::Rebootstrap(base)) => {
          vec![base_message(&base, client_sv.as_deref(), ack_id, document_id)]
        }
        Ok(sync::CatchUp::Updates(updates)) => {
          let head = updates.last().map(|u| u.rid).unwrap_or(since_rid);
          let encoded: Vec<Value> = updates
            .iter()
            .map(|u| {
              json!({
                "rid": u.rid,
                "actor_id": u.actor_id,
                "update": STANDARD.encode(&u.payload),
              })
            })
            .collect();
          vec![
            json!({
              "type": "sync.updates",
              "ack_id": ack_id,
              "document_id": document_id,
              "updates": encoded,
              "head": head,
            })
            .to_string(),
          ]
        }
        Err(error) => vec![error_message(ack_id, error_code(&error), &error.to_string())],
      }
    }
    "sync.push" => {
      if !permissions.can_write {
        return vec![error_message(
          ack_id,
          "permission_denied",
          "you do not have permission to edit this document",
        )];
      }
      let update_b64 = match envelope.payload.get("update").and_then(Value::as_str) {
        Some(s) => s.to_string(),
        None => {
          return vec![error_message(
            ack_id,
            "invalid_payload",
            "missing `update` (base64 yrs update)",
          )];
        }
      };
      let update = match STANDARD.decode(update_b64.as_bytes()) {
        Ok(bytes) => bytes,
        Err(_) => {
          return vec![error_message(ack_id, "invalid_payload", "update is not valid base64")];
        }
      };
      match sync::push_update(&state.db, workspace_id, document_id, user_id, &update).await {
        Ok(rid) => {
          // Fan the update out to the rest of the room (already-have-it sender
          // gets only the rid in its ack, below).
          let event = json!({
            "type": "sync.update",
            "document_id": document_id,
            "rid": rid,
            "actor_id": user_id,
            "update": update_b64,
          });
          room.broadcast(connection_id, Arc::from(event.to_string()));
          vec![
            json!({
              "type": "sync.ack",
              "ack_id": ack_id,
              "document_id": document_id,
              "rid": rid,
            })
            .to_string(),
          ]
        }
        Err(error) => vec![error_message(ack_id, error_code(&error), &error.to_string())],
      }
    }
    other => vec![error_message(
      ack_id,
      "unknown_type",
      &format!("unsupported message type: {other}"),
    )],
  }
}

/// Notify connected WebSocket clients of an update accepted through any path.
/// REST writes call this with the nil connection id so every live socket
/// receives the change.
pub fn broadcast_applied_update(
  hub: &mica_app_core::DocumentHub,
  applied: &AppliedUpdate,
  origin: Uuid,
  ack_id: Option<&str>,
) {
  let text: Arc<str> = Arc::from(accepted_event(applied, ack_id).to_string());
  hub.broadcast_if_active(applied.document.id, origin, text);
  // The same change on the yrs channel: an editor open on this document speaks
  // yrs, not the op-model event above, so without this a REST/MCP write stayed
  // invisible in an open window until it rebootstrapped. Identical shape to a
  // peer's `sync.push` fan-out, so clients need no new handling.
  if let Some(yrs) = &applied.yrs {
    let event = json!({
      "type": "sync.update",
      "document_id": applied.document.id,
      "rid": yrs.rid,
      "actor_id": applied.update.actor_id,
      "update": STANDARD.encode(&yrs.update),
    });
    hub.broadcast_if_active(applied.document.id, origin, Arc::from(event.to_string()));
  }
}

fn accepted_event(applied: &AppliedUpdate, ack_id: Option<&str>) -> Value {
  json!({
    "type": "document.update.accepted",
    "document_id": applied.document.id,
    "server_seq": applied.update.seq,
    "actor_id": applied.update.actor_id,
    "ack_id": ack_id,
    // `block_operations` for every edit this server still writes. Historical
    // rows may also carry `restore_snapshot`: the op-model restore that wrote it
    // was deleted with the rest of the dead op-model history writers, so no new
    // ones appear — real restores go through the yrs-native history routes.
    // Clients that cannot apply a non-`block_operations` kind should rebootstrap.
    "kind": applied.update.update_kind,
    "payload": applied.update.payload,
  })
}

/// P4-3: the client's optional state vector (`sv`, base64) from a
/// sync.bootstrap / sync.pull payload. Undecodable → None (full base fallback).
fn client_sv(payload: &Value) -> Option<Vec<u8>> {
  payload
    .get("sv")
    .and_then(Value::as_str)
    .and_then(|s| STANDARD.decode(s.as_bytes()).ok())
}

/// A `sync.base` frame carrying either the minimal diff for `client_sv` (P4-3,
/// when present and computable) or the full base state. Both are yrs updates —
/// the client applies them identically; `delta` is observability only.
fn base_message(
  base: &sync::YrsBase,
  client_sv: Option<&[u8]>,
  ack_id: Option<&str>,
  document_id: Uuid,
) -> String {
  let diff = client_sv.and_then(|sv| sync::diff_from_base(base, sv));
  let delta = diff.is_some();
  json!({
    "type": "sync.base",
    "ack_id": ack_id,
    "document_id": document_id,
    "base": STANDARD.encode(diff.as_deref().unwrap_or(&base.state)),
    "base_rid": base.base_rid,
    "delta": delta,
  })
  .to_string()
}

fn error_message(ack_id: Option<&str>, code: &str, message: &str) -> String {
  json!({
    "type": "error",
    "ack_id": ack_id,
    "code": code,
    "message": message,
  })
  .to_string()
}

fn error_code(error: &ApiError) -> &'static str {
  match error {
    ApiError::BadRequest(_) => "invalid_operation",
    // Already carries a specific code; hand it through rather than flattening it
    // back to the generic one.
    ApiError::BadRequestCode(code, _) => code,
    ApiError::Unauthorized => "unauthorized",
    ApiError::Forbidden => "permission_denied",
    ApiError::NotFound => "not_found",
    ApiError::Conflict(_) => "conflict",
    ApiError::Unavailable(_) => "service_unavailable",
    ApiError::Database(_) | ApiError::Migration(_) | ApiError::Internal(_) => "internal",
  }
}

async fn send_all(socket: &mut WebSocket, messages: Vec<String>) -> Result<(), ()> {
  for message in messages {
    socket
      .send(Message::Text(message.into()))
      .await
      .map_err(|_| ())?;
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use chrono::Utc;
  use mica_app_core::store::{DocumentRecord, SnapshotRecord, UpdateRecord};
  use serde_json::json;

  /// The gate ships INERT: the default floor is 0, so every client — including
  /// ones that predate the `v` parameter entirely — still connects. That is the
  /// whole point of landing it early; a gate that started rejecting on the day
  /// it shipped would break every desktop install that had not updated yet.
  #[test]
  fn the_default_floor_rejects_nobody() {
    assert_eq!(check_protocol(None, 0).unwrap(), 0, "a pre-parameter client");
    assert_eq!(check_protocol(Some(1), 0).unwrap(), 1);
    assert_eq!(check_protocol(Some(99), 0).unwrap(), 99, "a client newer than us");
  }

  /// Once the floor is raised (op-model retirement S4 removes the REST fallback
  /// that quietly caught old clients), a client below it must be turned away
  /// with a code it can act on — not left to fail later in ways neither side
  /// can explain.
  #[test]
  fn a_raised_floor_turns_old_clients_away_by_code() {
    let error = check_protocol(None, 1).unwrap_err();
    match error {
      ApiError::BadRequestCode(code, _) => assert_eq!(code, "client_too_old"),
      other => panic!("expected a machine code the client can branch on, got {other:?}"),
    }
    assert!(check_protocol(Some(1), 1).is_ok(), "exactly at the floor is fine");
  }

  /// The client announces the same number the server calls current. They live in
  /// two languages and drift silently; this is the cheapest place to notice.
  #[test]
  fn the_current_version_is_one() {
    assert_eq!(
      WS_PROTOCOL_VERSION, 1,
      "bump kSyncProtocolVersion in sync_client.dart in the same commit"
    );
  }

  fn sample_applied() -> AppliedUpdate {
    let now = Utc::now();
    let document_id = Uuid::from_u128(1);
    AppliedUpdate {
      document: DocumentRecord {
        id: document_id,
        workspace_id: Uuid::from_u128(2),
        root_block_id: "root".to_string(),
        current_seq: 5,
        created_by: Uuid::from_u128(3),
        created_at: now,
        updated_at: now,
      },
      snapshot: SnapshotRecord {
        id: Uuid::from_u128(4),
        document_id,
        version_seq: 5,
        schema_version: 1,
        payload: json!({ "schema_version": 1 }),
        created_at: now,
      },
      update: UpdateRecord {
        id: Uuid::from_u128(5),
        document_id,
        seq: 5,
        actor_id: Uuid::from_u128(3),
        update_kind: "block_operations".to_string(),
        payload: json!({ "operations": [] }),
        created_at: now,
      },
      // These tests cover the op-model `accepted_event` shape only; the yrs
      // half is exercised against a real database in app-core's sync_pg.
      yrs: None,
    }
  }

  #[test]
  fn accepted_event_carries_seq_and_ack() {
    let event = accepted_event(&sample_applied(), Some("client-1"));
    assert_eq!(event["type"], "document.update.accepted");
    assert_eq!(event["server_seq"], 5);
    assert_eq!(event["ack_id"], "client-1");
    assert_eq!(event["kind"], "block_operations");
    assert_eq!(event["payload"]["operations"], json!([]));
  }

  // P4-3: the sv glue between the WS layer and sync::diff_from_base. The
  // reconciliation math itself is covered in app-core; here we pin the frame
  // shape + fallback decisions the client contract depends on.
  #[test]
  fn client_sv_decodes_base64_and_rejects_garbage() {
    use base64::Engine;
    let sv = STANDARD.encode([1u8, 2, 3]);
    assert_eq!(
      client_sv(&json!({ "sv": sv })),
      Some(vec![1u8, 2, 3])
    );
    // Absent / non-string / non-base64 → None (full-base path).
    assert_eq!(client_sv(&json!({})), None);
    assert_eq!(client_sv(&json!({ "sv": 5 })), None);
    assert_eq!(client_sv(&json!({ "sv": "!!!not base64!!!" })), None);
  }

  #[test]
  fn base_message_sends_delta_only_when_sv_yields_one() {
    use mica_core::{Block, MicaDoc};
    let doc = MicaDoc::from_blocks(
      "r",
      &[
        Block::new("r", "page").with_children(vec!["a".into()]),
        Block::new("a", "paragraph").with_text("hello world"),
      ],
    );
    let base = sync::YrsBase {
      state: doc.encode_state(),
      state_vector: doc.state_vector(),
      base_rid: 7,
    };
    let doc_id = Uuid::from_u128(1);

    // No sv → full base, delta=false, base bytes are the full state.
    let full: Value =
      serde_json::from_str(&base_message(&base, None, Some("c1"), doc_id)).unwrap();
    assert_eq!(full["type"], "sync.base");
    assert_eq!(full["delta"], false);
    assert_eq!(full["base_rid"], 7);
    assert_eq!(full["ack_id"], "c1");
    let full_bytes = STANDARD.decode(full["base"].as_str().unwrap()).unwrap();
    assert_eq!(full_bytes, base.state);

    // A brand-new client's empty sv → the diff equals the full state (nothing to
    // trim), delta=true. A warm client that already holds the state → a strictly
    // smaller diff. Either way the client applies it as an ordinary yrs update.
    let empty_sv = MicaDoc::from_blocks("r", &[]).state_vector();
    let delta: Value =
      serde_json::from_str(&base_message(&base, Some(&empty_sv), None, doc_id)).unwrap();
    assert_eq!(delta["delta"], true);
    let delta_bytes = STANDARD.decode(delta["base"].as_str().unwrap()).unwrap();
    let mut applied = MicaDoc::from_update(&empty_sv).unwrap_or_else(|_| MicaDoc::from_blocks("r", &[]));
    applied.apply_update(&delta_bytes).unwrap();
    assert_eq!(applied.to_blocks(), doc.to_blocks());

    // A warm client that already has everything → the diff is smaller than the
    // full base (the P4-3 win).
    let warm_sv = base.state_vector.clone();
    let warm: Value =
      serde_json::from_str(&base_message(&base, Some(&warm_sv), None, doc_id)).unwrap();
    let warm_bytes = STANDARD.decode(warm["base"].as_str().unwrap()).unwrap();
    assert!(warm_bytes.len() < base.state.len());

    // Garbage sv → diff_from_base returns None → full-base fallback (delta=false).
    let bad: Value =
      serde_json::from_str(&base_message(&base, Some(b"not a state vector"), None, doc_id))
        .unwrap();
    assert_eq!(bad["delta"], false);
  }

  #[test]
  fn token_prefers_authorization_header() {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, "Bearer header-token".parse().unwrap());
    let query = ConnectQuery {
      token: Some("query-token".to_string()),
      v: None,
    };
    assert_eq!(
      token_from_request(&headers, &query),
      Some("header-token".to_string())
    );
  }

  #[test]
  fn token_falls_back_to_query() {
    let headers = HeaderMap::new();
    let query = ConnectQuery {
      token: Some("query-token".to_string()),
      v: None,
    };
    assert_eq!(
      token_from_request(&headers, &query),
      Some("query-token".to_string())
    );
  }

  #[test]
  fn token_missing_is_none() {
    let headers = HeaderMap::new();
    let query = ConnectQuery { token: None, v: None };
    assert_eq!(token_from_request(&headers, &query), None);
  }

  #[test]
  fn invalid_client_message_is_rejected() {
    let envelope = serde_json::from_str::<ClientEnvelope>("not json");
    assert!(envelope.is_err());
  }
}
