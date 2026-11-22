//! flutter_rust_bridge surface for `mica-sync`: pure-Rust Sans-I/O
//! synchronization state machine (`SyncEngine`) over unified `SyncStorage`.

use std::sync::Mutex;

use flutter_rust_bridge::frb;

/// Stateful Sans-I/O document synchronization engine (desktop synchronous FFI handle).
#[frb(opaque)]
pub struct SyncEngineHandle {
    inner: Mutex<mica_sync::SyncEngine>,
}

impl SyncEngineHandle {
    /// Create a new Sans-I/O sync engine backed by in-memory mirror storage
    /// (or online-only when `use_memory_storage` is false).
    #[frb(sync)]
    pub fn new_session(
        document_id: String,
        client_id: Option<u64>,
        self_name: Option<String>,
        use_memory_storage: bool,
    ) -> Self {
        let storage: Option<Box<dyn mica_sync::SyncStorage>> = if use_memory_storage {
            Some(Box::new(mica_sync::MemoryMirrorStorage::new()))
        } else {
            None
        };
        let engine = mica_sync::SyncEngine::new(
            mica_sync::SyncConfig {
                client_id,
                self_name,
                document_id: Some(document_id),
                restore_unacked: Vec::new(),
            },
            storage,
        );
        Self {
            inner: Mutex::new(engine),
        }
    }

    /// Feed a JSON-encoded `SyncEvent` and receive a JSON array of `SyncEffect`s.
    #[frb(sync)]
    pub fn handle_event_json(&self, event_json: String) -> Result<String, String> {
        let event: mica_sync::SyncEvent =
            serde_json::from_str(&event_json).map_err(|e| e.to_string())?;
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let effects = guard.handle_event(event);
        serde_json::to_string(&effects).map_err(|e| e.to_string())
    }

    /// Current sync phase string (`"online"`, `"syncing"`, `"offline"`, etc.).
    #[frb(sync)]
    pub fn phase(&self) -> String {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        serde_json::to_string(&guard.phase())
            .unwrap_or_else(|_| "\"offline\"".into())
            .trim_matches('"')
            .to_string()
    }

    /// Current full block list JSON from the synchronized CRDT replica.
    #[frb(sync)]
    pub fn all_blocks_json(&self) -> String {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.all_blocks_json()
    }

    /// Whether the engine has hydrated or received its initial base state.
    #[frb(sync)]
    pub fn is_ready(&self) -> bool {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.is_ready()
    }
}
