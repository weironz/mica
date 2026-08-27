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

impl MicaDocument {
    /// The doc lock, recovering from poisoning instead of propagating it.
    ///
    /// Every FFI entry point below goes through here, and each one used to be
    /// `.lock().unwrap()`. That turned ONE panic anywhere in the Rust core into
    /// permanent data loss: the mutex poisons, and from then on every read —
    /// `read_blocks` included — panics too, so the page renders blank for the
    /// rest of the process. A user reported exactly that (PanicException /
    /// PoisonError, "页面数据看不见了"); the PoisonError is the *second*
    /// failure, and it is the one that does the damage.
    ///
    /// Recovering is sound here because a yrs `TransactionMut` commits in its
    /// `Drop`, so an unwind still leaves a well-formed CRDT document — at worst
    /// a half-applied edit. Trading "maybe half an edit" for "this document is
    /// unreadable until restart" is not a close call.
    ///
    /// This does NOT fix whatever panics first; it stops that panic from being
    /// amplified. The root cause needs the FIRST `panicked at` line from the
    /// logs — see docs/code-review-2026-07-20.md.
    pub(crate) fn doc(&self) -> std::sync::MutexGuard<'_, MicaDoc> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
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

    /// [`Self::from_markdown`], but rewiring image references to on-device blobs.
    ///
    /// The local vault import kept only `.md` entries and dropped every other
    /// file on the floor, so a folder whose pages referenced `assets/x.png`
    /// imported as pages whose images were all dead links — silently, with no
    /// error and nothing in the result to say so.
    ///
    /// [`from_path`] is where this page sits inside the imported tree, and
    /// [`asset_ids`] maps every non-Markdown entry's path to the blob id Dart
    /// already stored it under (the local CAS keys by sha256). Resolution goes
    /// through `mica_interchange::resolve_ref` — the SAME function the server
    /// import uses — so relative paths, `..`, percent-encoding and the
    /// unique-basename fallback behave identically on both sides. A second
    /// implementation here is exactly how one rule becomes two that drift.
    ///
    /// A reference that does not resolve keeps its original `url`: an external
    /// link stays external, and a genuinely missing file stays visibly missing
    /// rather than being silently repointed at the wrong bytes.
    #[frb(sync)]
    pub fn from_markdown_with_assets(
        markdown: String,
        from_path: String,
        asset_ids: std::collections::HashMap<String, String>,
    ) -> MicaDocument {
        // A panic here ABORTS THE PROCESS — it unwinds into C and the app just
        // vanishes. Seen on a real vault: 155 of ~840 pages imported, the
        // window closed, and Windows logged 0xc0000409 against this .dll twice
        // at the same offset. Whatever one page does to the parser, it has to
        // cost that page, not the other 839 and the user's whole import.
        //
        // Caught here rather than "fixed" upstream because the guarantee worth
        // having is not "no markdown can ever panic" — unprovable against
        // arbitrary input — but "one bad page cannot take the import down".
        let recovered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Self::build_with_assets(&markdown, &from_path, &asset_ids)
        }));
        match recovered {
            Ok(doc) => doc,
            // Fall back to the page WITHOUT its images. The text is the part
            // the user cannot recreate, and a page with dead image links beats
            // no page at all. If the plain path panics too, that is a different
            // bug and the abort is the honest outcome — better than silently
            // storing an empty document in its place.
            Err(_) => Self::from_markdown(markdown),
        }
    }

    fn build_with_assets(
        markdown: &str,
        from_path: &str,
        asset_ids: &std::collections::HashMap<String, String>,
    ) -> MicaDocument {
        let root_id = format!("block_{}", uuid::Uuid::new_v4());
        let payload = mica_markdown::import_markdown(markdown, &root_id);
        let paths: std::collections::HashSet<String> = asset_ids.keys().cloned().collect();

        let blocks: Vec<Block> = payload
            .blocks
            .into_iter()
            .map(|b| {
                // Nested rather than a let-chain: this crate is not on the 2024
                // edition, where those became legal.
                let mut data = b.data;
                if b.kind == "image" {
                    if let Some(url) = data.get("url").and_then(|v| v.as_str()) {
                        if let Some(hit) =
                            mica_interchange::resolve_ref(from_path, url, &paths)
                        {
                            if let Some(file_id) = asset_ids.get(&hit) {
                                // The shape the cloud import produces and the
                                // local blob store expects: id + a readable
                                // name, and no url.
                                let name =
                                    hit.rsplit('/').next().unwrap_or(&hit).to_string();
                                data = serde_json::json!({
                                    "file_id": file_id,
                                    "name": name,
                                });
                            }
                        }
                    }
                }
                Block {
                    id: b.id,
                    kind: b.kind,
                    text: b.text,
                    data,
                    children: b.children,
                }
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
        serde_json::to_string(&self.doc().to_blocks())
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
        // `snapshot()` takes the lock and gives it back; RENDERING MUST NOT HOLD
        // IT. This used to bind the guard here and keep it alive across
        // `export_html_document` below, which — with the `render` feature on —
        // runs the whole markdown engine plus merman, a third-party mermaid
        // renderer, over arbitrary user diagram source. Any panic in there
        // poisoned this document's mutex, and from then on every read panicked
        // too: the page went blank until the app restarted. `unwrap_or_default`
        // catches the Err arm; a panic goes straight past it.
        let mut payload = self.snapshot();
        let srcs: std::collections::BTreeMap<String, String> = image_srcs.into_iter().collect();
        mica_markdown::set_image_srcs(&mut payload, &srcs);
        mica_markdown::export_html_document(&payload, &title, content_width).unwrap_or_default()
    }

    /// The doc as the engine's snapshot payload — shared by the markdown exports
    /// and the local folder-tree export (`api::store`).
    /// mica_core::Block and mica_markdown::Block mirror each other field-for-field.
    pub(crate) fn snapshot(&self) -> mica_markdown::DocumentSnapshotPayload {
        let doc = self.doc();
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

    /// Write the page's title into the document's root block.
    ///
    /// The LOCAL counterpart of the server's `sync::set_document_title`, and it
    /// goes through the same narrow primitive for the same reason: rebuilding
    /// the blocks (`set_blocks`) would give every block a brand-new text object
    /// and invalidate every comment anchor on the page. See
    /// `docs/page-title-plan.md` §5.1.
    ///
    /// Returns false when nothing changed — the title already said this, or the
    /// document has no root — so the caller can skip persisting. Comparing first
    /// matters: a yrs map insert is an operation whether or not the value moved,
    /// so writing unconditionally would grow the document on every rename to the
    /// same name.
    #[frb(sync)]
    pub fn set_title(&self, title: String) -> bool {
        let title = title.trim();
        // ONE lock for the whole read-compare-write. `doc()` is a MutexGuard, so
        // taking it twice in one expression would deadlock this thread against
        // itself — `snapshot()` takes it too, which is why the comparison reads
        // the blocks through the guard already in hand instead of calling it.
        let mut doc = self.doc();
        let root = doc.root_block_id();
        if root.is_empty() {
            return false;
        }
        let current = doc
            .to_blocks()
            .into_iter()
            .find(|b| b.id == root)
            .and_then(|b| {
                b.data
                    .get("title")
                    .and_then(|t| t.as_str())
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .map(str::to_string)
            });
        if current.as_deref() == Some(title) {
            return false;
        }
        doc.set_block_prop(&root, "title", &serde_json::json!(title))
    }

    /// Like [`Self::export_markdown`], with `# <title>` leading the text (after
    /// any front matter).
    ///
    /// For the two LOCAL paths that hand a human a copy of the page — "export as
    /// .md" and "copy page content" — where the page name is otherwise lost the
    /// moment the text leaves Mica. The cloud does the same thing through
    /// `?title=true` on its read endpoint; the rule itself lives in one place
    /// (`mica_markdown::with_page_title`) so the two worlds cannot drift.
    ///
    /// [`title`] is the view's name; a document that carries its own title
    /// (`root.data['title']`) wins over it. See `docs/page-title-plan.md`.
    #[frb(sync)]
    pub fn export_markdown_titled(&self, title: String) -> String {
        let payload = self.snapshot();
        let body = mica_markdown::export_markdown(&payload).unwrap_or_default();
        let title = mica_markdown::document_title(&payload).unwrap_or(title.as_str());
        mica_markdown::with_page_title(&body, title)
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
        let body = mica_markdown::export_markdown_with_assets(&payload, &map).unwrap_or_default();
        // The title leads the text as well as naming the file — the same rule
        // the cloud ZIP follows (`docs/page-title-plan.md`). `base` is the raw
        // page name here; only the FILE name gets sanitized, so it doubles as
        // the title with no extra parameter.
        let title = mica_markdown::document_title(&payload).unwrap_or(base.as_str());
        entries.insert(
            0,
            mica_interchange::ZipEntry {
                name: format!("{}.md", safe_base(&base)),
                data: mica_markdown::with_page_title(&body, title).into_bytes(),
            },
        );
        mica_interchange::build_zip(&entries)
    }

    /// Encode the full document state (the base snapshot to persist locally).
    #[frb(sync)]
    pub fn encode_state(&self) -> Vec<u8> {
        self.doc().encode_state()
    }

    // ── sync primitives (P2-M4.5): let Dart compute diffs to push + apply
    //    remote updates, for cloud CRDT sync. ─────────────────────────────────

    /// This replica's state vector — capture it before an edit batch, then
    /// [`Self::encode_diff_since`] after to get just that batch's update to push.
    #[frb(sync)]
    pub fn state_vector(&self) -> Vec<u8> {
        self.doc().state_vector()
    }

    /// The minimal update carrying everything added since `state_vector` was
    /// taken — the bytes to push to the cloud. Empty on a malformed vector.
    #[frb(sync)]
    pub fn encode_diff_since(&self, state_vector: Vec<u8>) -> Vec<u8> {
        self.doc()
            .encode_diff(&state_vector)
            .unwrap_or_default()
    }

    /// Merge a remote yrs update into this doc (CRDT merge). Returns false if the
    /// bytes don't decode (caller should resync rather than trust local state).
    #[frb(sync)]
    pub fn apply_update(&self, update: Vec<u8>) -> bool {
        self.doc().apply_update(&update).is_ok()
    }

    #[frb(sync)]
    pub fn root_block_id(&self) -> String {
        self.doc().root_block_id()
    }

    #[frb(sync)]
    pub fn insert_block_json(&self, parent_id: String, index: u32, block_json: String) {
        if let Ok(b) = serde_json::from_str::<Block>(&block_json) {
            self.doc()
                .insert_block(&parent_id, index as usize, &b);
        }
    }

    #[frb(sync)]
    pub fn update_block_kind(&self, id: String, kind: String) {
        self.doc().update_block_kind(&id, &kind);
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
        let mut doc = self.doc();
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
            self.doc().set_block_data(&id, &data);
        }
    }

    #[frb(sync)]
    pub fn text_insert(&self, id: String, at: u32, text: String) {
        self.doc().text_insert(&id, at, &text);
    }

    #[frb(sync)]
    pub fn text_delete(&self, id: String, at: u32, len: u32) {
        self.doc().text_delete(&id, at, len);
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
        self.doc().text_format(&id, &mark);
    }

    #[frb(sync)]
    pub fn delete_block(&self, id: String, bring_children: bool) {
        self.doc().delete_block(&id, bring_children);
    }

    #[frb(sync)]
    pub fn move_block(&self, id: String, new_parent: String, index: u32) {
        self.doc()
            .move_block(&id, &new_parent, index as usize);
    }

    #[frb(sync)]
    pub fn split_block(&self, id: String, at: u32, new_id: String, new_kind: String) {
        self.doc()
            .split_block(&id, at, &new_id, &new_kind);
    }

    #[frb(sync)]
    pub fn join_into_prev(&self, id: String) {
        self.doc().join_into_prev(&id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bug the user hit: a panic elsewhere poisons the mutex, and every
    /// later read panics too, so the page goes blank until the app restarts.
    /// With `.lock().unwrap()` this test panics on the LAST line, not the
    /// simulated one — which is precisely the amplification being fixed.
    #[test]
    fn a_poisoned_lock_still_reads() {
        let doc = std::sync::Arc::new(MicaDocument::from_markdown("hello".to_string()));

        let other = std::sync::Arc::clone(&doc);
        let crashed = std::thread::spawn(move || {
            let _guard = other.doc();
            panic!("simulated panic while holding the doc lock");
        })
        .join();
        assert!(crashed.is_err(), "the helper thread must actually panic");

        // The lock is poisoned now. Reading must still work.
        let json = doc.to_blocks_json();
        assert!(
            json.contains("hello"),
            "document still readable after poisoning: {json}"
        );
    }

    /// Walk a real vault and report which page, if any, takes the importer
    /// down. Ignored by default — it needs a directory that only exists on the
    /// machine reproducing the bug. Run with:
    ///
    ///   MICA_VAULT=C:\path\to\vault cargo test -- --ignored --nocapture
    ///
    /// Kept in the tree rather than thrown away after the fix: "one page kills
    /// the import" is the failure mode worth being able to re-check, and the
    /// next such archive will not be this one.
    #[test]
    #[ignore]
    fn every_page_in_a_real_vault_imports_without_aborting() {
        let Ok(root) = std::env::var("MICA_VAULT") else {
            eprintln!("set MICA_VAULT to a vault directory");
            return;
        };
        let root = std::path::PathBuf::from(root);
        let mut checked = 0usize;
        let mut failed: Vec<String> = Vec::new();

        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            let Ok(rd) = std::fs::read_dir(dir) else { return };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().is_some_and(|x| x == "md") {
                    out.push(p);
                }
            }
        }
        let mut files = Vec::new();
        walk(&root, &mut files);
        files.sort();
        eprintln!("{} markdown files under {}", files.len(), root.display());

        for path in files {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let rel = path
                .strip_prefix(&root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            // Printed BEFORE the call and flushed: a stack overflow cannot be
            // caught, so the last line standing is the file that did it.
            eprintln!("-> {rel}");
            use std::io::Write;
            let _ = std::io::stderr().flush();
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = MicaDocument::from_markdown_with_assets(
                    text.clone(),
                    rel.clone(),
                    std::collections::HashMap::new(),
                );
            }));
            checked += 1;
            if res.is_err() {
                eprintln!("PANIC on {rel}");
                failed.push(rel);
            }
        }
        eprintln!("checked {checked}, panicked {}", failed.len());
        assert!(failed.is_empty(), "pages that abort the import: {failed:?}");
    }

    /// Build the blocks a page imports to, so the assertions below read as
    /// "what happened to this image reference".
    fn image_data(markdown: &str, from: &str, assets: &[(&str, &str)]) -> serde_json::Value {
        let map: std::collections::HashMap<String, String> = assets
            .iter()
            .map(|(p, id)| (p.to_string(), id.to_string()))
            .collect();
        let doc = MicaDocument::from_markdown_with_assets(
            markdown.to_string(),
            from.to_string(),
            map,
        );
        let blocks: Vec<serde_json::Value> =
            serde_json::from_str(&doc.to_blocks_json()).unwrap();
        // `to_blocks_json` spells the kind `type`, not `kind`.
        blocks
            .iter()
            .find(|b| b["type"] == "image")
            .map(|b| b["data"].clone())
            .unwrap_or_else(|| {
                panic!(
                    "no image block; blocks were: {}",
                    serde_json::to_string(&blocks).unwrap()
                )
            })
    }

    /// The reported failure: a vault import kept the pages and dropped every
    /// image, leaving `![](assets/…)` pointing at nothing.
    #[test]
    fn an_archive_relative_image_is_repointed_at_its_blob() {
        let data = image_data(
            "# Page\n\n![shot](assets/diagram.png)\n",
            "guide/page.md",
            &[("guide/assets/diagram.png", "sha256-aaa")],
        );
        assert_eq!(data["file_id"], "sha256-aaa");
        assert_eq!(data["name"], "diagram.png");
        assert!(data.get("url").is_none(), "the dead url must not survive");
    }

    /// Resolution is `resolve_ref`, so `..` and root-relative forms work the
    /// same way they do on the server — this is the point of not writing a
    /// second path resolver.
    #[test]
    fn parent_relative_paths_resolve_like_the_server() {
        let data = image_data(
            "![](../assets/x.png)",
            "guide/deep/page.md",
            &[("guide/assets/x.png", "sha256-bbb")],
        );
        assert_eq!(data["file_id"], "sha256-bbb");
    }

    /// An http(s) image is somebody else's URL, not an archive entry. Leaving
    /// it alone is what keeps external images working.
    #[test]
    fn an_external_url_is_left_untouched() {
        let data = image_data(
            "![](https://example.com/x.png)",
            "page.md",
            &[("assets/x.png", "sha256-ccc")],
        );
        assert_eq!(data["url"], "https://example.com/x.png");
        assert!(data.get("file_id").is_none());
    }

    /// A reference to something the archive does not contain stays visibly
    /// broken. Silently repointing it at a same-named file elsewhere would be
    /// worse than the dead link: it would be the WRONG image, and look right.
    #[test]
    fn an_unresolvable_reference_keeps_its_url() {
        let data = image_data(
            "![](assets/missing.png)",
            "page.md",
            &[("assets/other.png", "sha256-ddd")],
        );
        assert_eq!(data["url"], "assets/missing.png");
        assert!(data.get("file_id").is_none());
    }

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
