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
use mica_app_core::{
  AppState, PresenceEntry, Room,
  documents::DocumentOperation,
  store::{self, AppliedUpdate},
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

#[derive(Debug, Deserialize)]
pub struct ConnectQuery {
  token: Option<String>,
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
  let token = token_from_request(&headers, &query).ok_or(ApiError::Unauthorized)?;
  let user_id = user_id_from_token(&state, &token)?;

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
}

fn accepted_event(applied: &AppliedUpdate, ack_id: Option<&str>) -> Value {
  json!({
    "type": "document.update.accepted",
    "document_id": applied.document.id,
    "server_seq": applied.update.seq,
    "actor_id": applied.update.actor_id,
    "ack_id": ack_id,
    // `block_operations` for ordinary edits, `restore_snapshot` for a restore;
    // clients that cannot apply the latter incrementally should rebootstrap.
    "kind": applied.update.update_kind,
    "payload": applied.update.payload,
  })
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

  #[test]
  fn token_prefers_authorization_header() {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, "Bearer header-token".parse().unwrap());
    let query = ConnectQuery {
      token: Some("query-token".to_string()),
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
    };
    assert_eq!(
      token_from_request(&headers, &query),
      Some("query-token".to_string())
    );
  }

  #[test]
  fn token_missing_is_none() {
    let headers = HeaderMap::new();
    let query = ConnectQuery { token: None };
    assert_eq!(token_from_request(&headers, &query), None);
  }

  #[test]
  fn invalid_client_message_is_rejected() {
    let envelope = serde_json::from_str::<ClientEnvelope>("not json");
    assert!(envelope.is_err());
  }
}
