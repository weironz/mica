use std::{
  collections::HashMap,
  sync::{Arc, Mutex, Weak},
};

use serde::Serialize;
use serde_json::Value;
use tokio::sync::broadcast;
use uuid::Uuid;

/// Capacity of each room's broadcast channel. Slow receivers that fall behind
/// this many messages observe a lag error and rebootstrap.
const ROOM_CHANNEL_CAPACITY: usize = 256;

/// A message fanned out to every live connection in a document room.
#[derive(Clone, Debug)]
pub struct RoomMessage {
  /// Connection that produced the message, so a socket can skip its own echoes.
  pub origin: Uuid,
  /// Pre-serialized server message JSON.
  pub text: Arc<str>,
}

/// Ephemeral presence for one connection (cursor, selection, display metadata).
#[derive(Clone, Debug, Serialize)]
pub struct PresenceEntry {
  pub connection_id: Uuid,
  pub user_id: Uuid,
  pub data: Value,
}

/// A live editing room for a single document.
///
/// Held by an `Arc` for as long as at least one connection is joined; the hub
/// keeps only a `Weak` reference, so an empty room drops automatically.
pub struct Room {
  document_id: Uuid,
  sender: broadcast::Sender<RoomMessage>,
  presence: Mutex<HashMap<Uuid, PresenceEntry>>,
}

impl Room {
  pub fn document_id(&self) -> Uuid {
    self.document_id
  }

  pub fn subscribe(&self) -> broadcast::Receiver<RoomMessage> {
    self.sender.subscribe()
  }

  /// Fan a message out to every joined connection. Returns the number of
  /// receivers it reached (zero is normal when the sender is alone).
  pub fn broadcast(&self, origin: Uuid, text: Arc<str>) -> usize {
    self.sender.send(RoomMessage { origin, text }).unwrap_or(0)
  }

  pub fn set_presence(&self, entry: PresenceEntry) {
    self
      .presence
      .lock()
      .expect("room presence mutex poisoned")
      .insert(entry.connection_id, entry);
  }

  pub fn remove_presence(&self, connection_id: Uuid) -> Option<PresenceEntry> {
    self
      .presence
      .lock()
      .expect("room presence mutex poisoned")
      .remove(&connection_id)
  }

  pub fn presences(&self) -> Vec<PresenceEntry> {
    self
      .presence
      .lock()
      .expect("room presence mutex poisoned")
      .values()
      .cloned()
      .collect()
  }
}

/// Registry of active document rooms, shared across all connections.
#[derive(Clone, Default)]
pub struct DocumentHub {
  rooms: Arc<Mutex<HashMap<Uuid, Weak<Room>>>>,
}

impl DocumentHub {
  pub fn new() -> Self {
    Self::default()
  }

  /// Join (or create) the room for a document. The returned `Arc<Room>` keeps
  /// the room alive; drop it on disconnect to release the room.
  pub fn join(&self, document_id: Uuid) -> Arc<Room> {
    let mut rooms = self.rooms.lock().expect("document hub mutex poisoned");

    if let Some(room) = rooms.get(&document_id).and_then(Weak::upgrade) {
      return room;
    }

    let (sender, _receiver) = broadcast::channel(ROOM_CHANNEL_CAPACITY);
    let room = Arc::new(Room {
      document_id,
      sender,
      presence: Mutex::new(HashMap::new()),
    });
    rooms.insert(document_id, Arc::downgrade(&room));
    room
  }

  /// Broadcast to a room only if it currently has live connections. Used by
  /// REST writes so changes reach anyone editing over WebSocket, without
  /// spinning up a room that nobody is watching.
  pub fn broadcast_if_active(&self, document_id: Uuid, origin: Uuid, text: Arc<str>) -> usize {
    let room = self
      .rooms
      .lock()
      .expect("document hub mutex poisoned")
      .get(&document_id)
      .and_then(Weak::upgrade);

    match room {
      Some(room) => room.broadcast(origin, text),
      None => 0,
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn join_reuses_live_room() {
    let hub = DocumentHub::new();
    let document_id = Uuid::nil();

    let first = hub.join(document_id);
    let second = hub.join(document_id);

    assert!(Arc::ptr_eq(&first, &second));
  }

  #[test]
  fn room_drops_when_all_handles_released() {
    let hub = DocumentHub::new();
    let document_id = Uuid::nil();

    let room = hub.join(document_id);
    let _subscriber = room.subscribe();
    drop(room);

    // No live handle remains, so an inactive broadcast reaches nobody and the
    // next join allocates a fresh room.
    let reached = hub.broadcast_if_active(document_id, Uuid::nil(), Arc::from("{}"));
    assert_eq!(reached, 0);
  }

  #[test]
  fn presence_round_trips() {
    let hub = DocumentHub::new();
    let room = hub.join(Uuid::nil());
    let connection_id = Uuid::from_u128(1);

    room.set_presence(PresenceEntry {
      connection_id,
      user_id: Uuid::from_u128(2),
      data: json!({ "name": "Alice" }),
    });

    assert_eq!(room.presences().len(), 1);
    let removed = room
      .remove_presence(connection_id)
      .expect("presence exists");
    assert_eq!(removed.user_id, Uuid::from_u128(2));
    assert!(room.presences().is_empty());
  }

  #[tokio::test]
  async fn broadcast_reaches_other_subscribers() {
    let hub = DocumentHub::new();
    let room = hub.join(Uuid::nil());
    let mut receiver = room.subscribe();

    let origin = Uuid::from_u128(9);
    room.broadcast(origin, Arc::from("{\"type\":\"pong\"}"));

    let message = receiver.recv().await.expect("message delivered");
    assert_eq!(message.origin, origin);
    assert_eq!(&*message.text, "{\"type\":\"pong\"}");
  }
}
