//! flutter_rust_bridge surface for `mica-editor`: headless block editor kernel
//! (`EditorSession`) over single-authority `mica-core::MicaDoc`.

use std::collections::BTreeMap;
use std::sync::Mutex;

use flutter_rust_bridge::frb;

/// Stateful headless block editor session (desktop synchronous FFI handle).
#[frb(opaque)]
pub struct EditorSessionHandle {
    inner: Mutex<mica_editor::EditorSession>,
}

impl EditorSessionHandle {
    /// Open an editor session parsed from Markdown through single-source `mica-md`.
    #[frb(sync)]
    pub fn from_markdown(markdown: String, client_id: Option<u64>) -> Self {
        Self {
            inner: Mutex::new(mica_editor::EditorSession::from_markdown_with_client_id(
                &markdown,
                client_id,
            )),
        }
    }

    /// Open an editor session from a persisted yrs v1 CRDT state.
    #[frb(sync)]
    pub fn from_crdt_state(state: Vec<u8>, client_id: Option<u64>) -> Result<Self, String> {
        let doc = mica_core::MicaDoc::from_update_with_client_id(&state, client_id)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            inner: Mutex::new(mica_editor::EditorSession::from_doc(doc)),
        })
    }

    /// Dispatch a JSON-encoded `EditorCommand` and return a JSON-encoded `EditorPatch`.
    #[frb(sync)]
    pub fn dispatch_json(&self, cmd_json: String) -> Result<String, String> {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.dispatch_json(&cmd_json)
    }

    /// Snapshot current flat blocks as JSON.
    #[frb(sync)]
    pub fn blocks_json(&self) -> String {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        serde_json::to_string(guard.blocks()).unwrap_or_else(|_| "[]".into())
    }

    /// Export live Markdown through `mica-md`.
    #[frb(sync)]
    pub fn export_markdown(&self) -> String {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.export_markdown(&BTreeMap::new())
    }

    /// Encode full yrs v1 CRDT state bytes.
    #[frb(sync)]
    pub fn encode_state(&self) -> Vec<u8> {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.encode_state()
    }

    /// Current content revision counter.
    #[frb(sync)]
    pub fn content_revision(&self) -> u64 {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.content_revision()
    }
}
