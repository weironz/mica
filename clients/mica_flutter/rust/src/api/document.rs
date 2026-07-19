//! flutter_rust_bridge surface for the offline document model (`mica-core`).
//!
//! `MicaDocument` is an opaque handle to a yrs-backed [`MicaDoc`]; Dart holds it
//! and calls edit operations. Blocks cross the boundary as JSON arrays — the
//! shape the editor already uses — so the editor binding (P2-M3) can adopt this
//! incrementally without a parallel block model.

use std::sync::Mutex;

use flutter_rust_bridge::frb;
use mica_core::{marks_from_data, Block, Mark, MicaDoc};

#[frb(opaque)]
pub struct MicaDocument {
    // crate-visible so the local store (api::store) can save/load it.
    pub(crate) inner: Mutex<MicaDoc>,
}

/// One on-device image blob to bundle into a page ZIP export: the block's
/// `file_id` plus the bytes Dart read from the local blob CAS.
pub struct ZipAsset {
    pub file_id: String,
    pub bytes: Vec<u8>,
}

/// Make a unique `assets/` filename, appending `-1`, `-2`… on collision —
/// mirrors the server's `unique_asset_name` so local ZIPs match cloud ones.
fn unique_asset_name(name: &str, used: &mut std::collections::HashSet<String>) -> String {
    if used.insert(name.to_string()) {
        return name.to_string();
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) => (s.to_string(), format!(".{e}")),
        None => (name.to_string(), String::new()),
    };
    let mut n = 1;
    loop {
        let candidate = format!("{stem}-{n}{ext}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        n += 1;
    }
}

/// Sanitize a page title into a ZIP-safe `.md` base name (drop path separators
/// and reserved chars); empty → "document", matching the cloud export.
fn safe_base(name: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| if "/\\:*?\"<>|\r\n\t".contains(c) { '_' } else { c })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').trim().to_string();
    if cleaned.is_empty() {
        "document".to_string()
    } else {
        cleaned
    }
}

impl MicaDocument {
    /// Build a document from a root id and a JSON array of blocks.
    #[frb(sync)]
    pub fn from_blocks_json(root_id: String, blocks_json: String) -> MicaDocument {
        let blocks: Vec<Block> = serde_json::from_str(&blocks_json).unwrap_or_default();
        MicaDocument {
            inner: Mutex::new(MicaDoc::from_blocks(&root_id, &blocks)),
        }
    }

    /// Build a document by parsing Markdown with the authoritative engine
    /// (CommonMark + GFM). Used by local vault import (S-tier): the file stays the
    /// user's, parsing stays in Rust (and round-trips with `export_markdown`). A
    /// fresh root id is minted; `mica_markdown::Block` mirrors `mica_core::Block`
    /// field-for-field, so no schema translation is needed.
    #[frb(sync)]
    pub fn from_markdown(markdown: String) -> MicaDocument {
        let root_id = format!("block_{}", uuid::Uuid::new_v4());
        let payload = mica_markdown::import_markdown(&markdown, &root_id);
        let blocks: Vec<Block> = payload
            .blocks
            .into_iter()
            .map(|b| Block {
                id: b.id,
                kind: b.kind,
                text: b.text,
                data: b.data,
                children: b.children,
            })
            .collect();
        MicaDocument {
            inner: Mutex::new(MicaDoc::from_blocks(&payload.root_block_id, &blocks)),
        }
    }

    /// Rebuild from an encoded yrs state (the local snapshot). Returns null if
    /// the bytes don't decode.
    #[frb(sync)]
    pub fn from_state(bytes: Vec<u8>) -> Option<MicaDocument> {
        MicaDoc::from_update(&bytes)
            .ok()
            .map(|d| MicaDocument { inner: Mutex::new(d) })
    }

    /// Like [`Self::from_state`] but pins the yrs actor to this device's stable
    /// `client_id` (from the local store identity) — so all of a device's edits
    /// share one actor across sessions, which cloud sync (P2-M4.5) relies on.
    #[frb(sync)]
    pub fn from_state_with_client_id(bytes: Vec<u8>, client_id: u64) -> Option<MicaDocument> {
        MicaDoc::from_update_with_client_id(&bytes, Some(client_id))
            .ok()
            .map(|d| MicaDocument { inner: Mutex::new(d) })
    }

    /// The document as a JSON array of blocks (tree order).
    #[frb(sync)]
    pub fn to_blocks_json(&self) -> String {
        serde_json::to_string(&self.inner.lock().unwrap().to_blocks())
            .unwrap_or_else(|_| "[]".into())
    }

    /// Export this page as a self-contained HTML document, through the same Rust
    /// engine the server uses — so a LOCAL page's export matches a cloud page's
    /// byte-for-byte. `image_srcs` maps image `file_id`s to `data:` URIs the Dart
    /// side has already read from the on-device blob CAS; images with no entry
    /// keep their url. Local export otherwise had no path (the ZIP/Markdown
    /// exports are server endpoints), so this also closes that gap.
    #[frb(sync)]
    pub fn export_html(
        &self,
        title: String,
        image_srcs: std::collections::HashMap<String, String>,
        content_width: u32,
    ) -> String {
        let doc = self.inner.lock().unwrap();
        let root_block_id = doc.root_block_id();
        // mica_core::Block and mica_markdown::Block mirror each other field-for-
        // field (see from_markdown); translate to the engine's type.
        let blocks = doc
            .to_blocks()
            .into_iter()
            .map(|b| mica_markdown::Block {
                id: b.id,
                kind: b.kind,
                text: b.text,
                data: b.data,
                children: b.children,
            })
            .collect();
        let mut payload = mica_markdown::DocumentSnapshotPayload {
            schema_version: 1,
            root_block_id,
            blocks,
        };
        let srcs: std::collections::BTreeMap<String, String> = image_srcs.into_iter().collect();
        mica_markdown::set_image_srcs(&mut payload, &srcs);
        mica_markdown::export_html_document(&payload, &title, content_width).unwrap_or_default()
    }

    /// The doc as the engine's snapshot payload — shared by the markdown exports.
    /// mica_core::Block and mica_markdown::Block mirror each other field-for-field.
    fn snapshot(&self) -> mica_markdown::DocumentSnapshotPayload {
        let doc = self.inner.lock().unwrap();
        let root_block_id = doc.root_block_id();
        let blocks = doc
            .to_blocks()
            .into_iter()
            .map(|b| mica_markdown::Block {
                id: b.id,
                kind: b.kind,
                text: b.text,
                data: b.data,
                children: b.children,
            })
            .collect();
        mica_markdown::DocumentSnapshotPayload {
            schema_version: 1,
            root_block_id,
            blocks,
        }
    }

    /// Export this page's body as Markdown through the SAME engine the cloud uses
    /// (`mica_markdown::export_markdown`), so a local page's `.md` matches a cloud
    /// page's byte-for-byte. Image blocks keep their stored ref — use
    /// [`MicaDocument::export_markdown_zip`] when bundling image bytes. Closes the
    /// local-page md-export gap (previously only HTML/PDF existed locally).
    #[frb(sync)]
    pub fn export_markdown(&self) -> String {
        mica_markdown::export_markdown(&self.snapshot()).unwrap_or_default()
    }

    /// Export this page as a portable ZIP (`<base>.md` + `assets/<name>` image
    /// bytes), byte-compatible with the cloud page ZIP export: same naming +
    /// dedup + Markdown rewrite via `export_markdown_with_assets`. [`assets`]
    /// supplies the on-device blob bytes per `file_id` (Dart reads the local
    /// CAS); external-URL images are left untouched.
    #[frb(sync)]
    pub fn export_markdown_zip(&self, base: String, assets: Vec<ZipAsset>) -> Vec<u8> {
        let payload = self.snapshot();
        let bytes_by_id: std::collections::HashMap<String, Vec<u8>> =
            assets.into_iter().map(|a| (a.file_id, a.bytes)).collect();
        // Mirror the server's collect_assets: image blocks in document order,
        // name from data.name (default "image"), first occurrence wins.
        let mut wanted: Vec<(String, String)> = Vec::new();
        for block in &payload.blocks {
            if block.kind != "image" {
                continue;
            }
            if let Some(id) = block.data.get("file_id").and_then(|v| v.as_str()) {
                if bytes_by_id.contains_key(id) && !wanted.iter().any(|(w, _)| w == id) {
                    let name = block
                        .data
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("image")
                        .to_string();
                    wanted.push((id.to_string(), name));
                }
            }
        }
        let mut entries: Vec<mica_interchange::ZipEntry> = Vec::new();
        let mut used = std::collections::HashSet::new();
        let mut map = std::collections::BTreeMap::new();
        for (file_id, name) in wanted {
            let asset = unique_asset_name(&name, &mut used);
            entries.push(mica_interchange::ZipEntry {
                name: format!("assets/{asset}"),
                data: bytes_by_id.get(&file_id).cloned().unwrap_or_default(),
            });
            map.insert(file_id, format!("assets/{asset}"));
        }
        let markdown =
            mica_markdown::export_markdown_with_assets(&payload, &map).unwrap_or_default();
        entries.insert(
            0,
            mica_interchange::ZipEntry {
                name: format!("{}.md", safe_base(&base)),
                data: markdown.into_bytes(),
            },
        );
        mica_interchange::build_zip(&entries)
    }

    /// Encode the full document state (the base snapshot to persist locally).
    #[frb(sync)]
    pub fn encode_state(&self) -> Vec<u8> {
        self.inner.lock().unwrap().encode_state()
    }

    // ── sync primitives (P2-M4.5): let Dart compute diffs to push + apply
    //    remote updates, for cloud CRDT sync. ─────────────────────────────────

    /// This replica's state vector — capture it before an edit batch, then
    /// [`Self::encode_diff_since`] after to get just that batch's update to push.
    #[frb(sync)]
    pub fn state_vector(&self) -> Vec<u8> {
        self.inner.lock().unwrap().state_vector()
    }

    /// The minimal update carrying everything added since `state_vector` was
    /// taken — the bytes to push to the cloud. Empty on a malformed vector.
    #[frb(sync)]
    pub fn encode_diff_since(&self, state_vector: Vec<u8>) -> Vec<u8> {
        self.inner
            .lock()
            .unwrap()
            .encode_diff(&state_vector)
            .unwrap_or_default()
    }

    /// Merge a remote yrs update into this doc (CRDT merge). Returns false if the
    /// bytes don't decode (caller should resync rather than trust local state).
    #[frb(sync)]
    pub fn apply_update(&self, update: Vec<u8>) -> bool {
        self.inner.lock().unwrap().apply_update(&update).is_ok()
    }

    #[frb(sync)]
    pub fn root_block_id(&self) -> String {
        self.inner.lock().unwrap().root_block_id()
    }

    #[frb(sync)]
    pub fn insert_block_json(&self, parent_id: String, index: u32, block_json: String) {
        if let Ok(b) = serde_json::from_str::<Block>(&block_json) {
            self.inner
                .lock()
                .unwrap()
                .insert_block(&parent_id, index as usize, &b);
        }
    }

    #[frb(sync)]
    pub fn update_block_kind(&self, id: String, kind: String) {
        self.inner.lock().unwrap().update_block_kind(&id, &kind);
    }

    /// Mirror the editor's coarse `update_block` op: apply any subset of
    /// kind/text/data to a block in one call. Inline marks travel *inside* the
    /// editor's `data` (`data["marks"]`) — when `text` is given they are applied
    /// to the (replaced) text as yrs formatting; `set_block_data` then stores the
    /// non-marks props. This is the single chokepoint the desktop op stream funnels
    /// through, so the on-device yrs doc tracks every edit (P2-M3).
    #[frb(sync)]
    pub fn update_block(
        &self,
        id: String,
        kind: Option<String>,
        text: Option<String>,
        data_json: Option<String>,
    ) {
        let mut doc = self.inner.lock().unwrap();
        if let Some(k) = kind {
            doc.update_block_kind(&id, &k);
        }
        let data: Option<serde_json::Value> =
            data_json.as_deref().and_then(|s| serde_json::from_str(s).ok());
        if let Some(t) = text {
            // Text changed: set text + its marks together.
            let marks = data.as_ref().map(marks_from_data).unwrap_or_default();
            doc.set_block_text(&id, &t, &marks);
        } else if let Some(d) = &data {
            // Data-only update (e.g. a turn-into resetting data): reconcile the
            // marks to whatever `data` now says — clearing them if it has none.
            doc.set_block_marks(&id, &marks_from_data(d));
        }
        if let Some(d) = &data {
            doc.set_block_data(&id, d);
        }
    }

    #[frb(sync)]
    pub fn set_block_data_json(&self, id: String, data_json: String) {
        if let Ok(data) = serde_json::from_str(&data_json) {
            self.inner.lock().unwrap().set_block_data(&id, &data);
        }
    }

    #[frb(sync)]
    pub fn text_insert(&self, id: String, at: u32, text: String) {
        self.inner.lock().unwrap().text_insert(&id, at, &text);
    }

    #[frb(sync)]
    pub fn text_delete(&self, id: String, at: u32, len: u32) {
        self.inner.lock().unwrap().text_delete(&id, at, len);
    }

    #[frb(sync)]
    pub fn text_format(
        &self,
        id: String,
        start: u32,
        end: u32,
        ty: String,
        href: Option<String>,
        title: Option<String>,
    ) {
        let mark = Mark {
            start,
            end,
            ty,
            href,
            title,
        };
        self.inner.lock().unwrap().text_format(&id, &mark);
    }

    #[frb(sync)]
    pub fn delete_block(&self, id: String, bring_children: bool) {
        self.inner.lock().unwrap().delete_block(&id, bring_children);
    }

    #[frb(sync)]
    pub fn move_block(&self, id: String, new_parent: String, index: u32) {
        self.inner
            .lock()
            .unwrap()
            .move_block(&id, &new_parent, index as usize);
    }

    #[frb(sync)]
    pub fn split_block(&self, id: String, at: u32, new_id: String, new_kind: String) {
        self.inner
            .lock()
            .unwrap()
            .split_block(&id, at, &new_id, &new_kind);
    }

    #[frb(sync)]
    pub fn join_into_prev(&self, id: String) {
        self.inner.lock().unwrap().join_into_prev(&id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_markdown_parses_headings_and_marks() {
        let doc = MicaDocument::from_markdown("# Title\n\nHello **world**".to_string());
        let blocks: Vec<serde_json::Value> =
            serde_json::from_str(&doc.to_blocks_json()).unwrap();
        assert!(
            blocks
                .iter()
                .any(|b| b["type"] == "heading" && b["text"] == "Title"),
            "heading imported: {blocks:?}"
        );
        // Plain text is clean; the bold is a mark inside `data`, not in `text`.
        assert!(
            blocks
                .iter()
                .any(|b| b["type"] == "paragraph" && b["text"] == "Hello world"),
            "paragraph imported with clean text: {blocks:?}"
        );
    }

    #[test]
    fn from_empty_markdown_still_builds_a_doc() {
        let doc = MicaDocument::from_markdown(String::new());
        assert!(!doc.root_block_id().is_empty());
        assert!(!doc.encode_state().is_empty());
    }
}
