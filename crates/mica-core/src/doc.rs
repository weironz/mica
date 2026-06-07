//! The yrs document model.
//!
//! A document is one yrs [`Doc`] holding:
//! - `blocks: Map<block_id, Block(Map)>` — every block, flat (not nested).
//!   Each block map has `ty` (kind), `text` (a `Text` whose formatting = inline
//!   marks), `props` (block attrs, see note below) and `children` (`Array` of
//!   ids). Tree order is the `children` arrays from a root.
//! - `meta: Map` — `root` = the root block id.
//!
//! The Doc uses `OffsetKind::Utf16` so mark offsets equal Dart string indices.
//!
//! NOTE (M1 simplification): `props` is stored as a JSON string, not a nested
//! `MapRef`. That's last-write-wins per block, NOT field-level CRDT — fine for
//! single-device M1/M2, but must become a `MapRef` before multi-writer sync
//! (P2-M4). Marks + text already get proper character-level CRDT via `Text`.

use serde_json::Value;
use yrs::types::{Attrs, GetString};
use yrs::updates::decoder::Decode;
use yrs::{
    Any, Array, ArrayPrelim, ArrayRef, Doc, Map, MapPrelim, MapRef, OffsetKind, Options, Out,
    ReadTxn, StateVector, Text, TextPrelim, TextRef, Transact, TransactionMut, Update,
};

use crate::block::Block;
use crate::marks::{self, Mark};

const BLOCKS: &str = "blocks";
const META: &str = "meta";
const ROOT_KEY: &str = "root";

#[derive(thiserror::Error, Debug)]
pub enum DocError {
    #[error("failed to decode yrs update: {0}")]
    Decode(String),
    #[error("transaction failed: {0}")]
    Apply(String),
}

/// A Mica document backed by a yrs CRDT doc.
pub struct MicaDoc {
    doc: Doc,
}

impl MicaDoc {
    fn new_doc() -> Doc {
        Doc::with_options(Options {
            offset_kind: OffsetKind::Utf16,
            ..Default::default()
        })
    }

    /// Build a document from a flat block list rooted at `root_id`.
    pub fn from_blocks(root_id: &str, blocks: &[Block]) -> Self {
        let doc = Self::new_doc();
        {
            let blocks_map = doc.get_or_insert_map(BLOCKS);
            let meta = doc.get_or_insert_map(META);
            let mut txn = doc.transact_mut();
            meta.insert(&mut txn, ROOT_KEY, root_id.to_string());

            for b in blocks {
                write_block(&mut txn, &blocks_map, b);
            }
        }
        MicaDoc { doc }
    }

    /// The root block id (empty string if unset).
    pub fn root_block_id(&self) -> String {
        // Resolve the root type handle BEFORE opening the transaction — calling
        // `get_or_insert_map` while a txn is held re-locks the store and
        // deadlocks (yrs transactions are not reentrant).
        let meta = self.doc.get_or_insert_map(META);
        let txn = self.doc.transact();
        read_root(&meta, &txn)
    }

    /// Read the document back into a flat block list, in tree order (DFS from the
    /// root following `children`). Blocks unreachable from the root are appended
    /// afterwards so nothing is silently dropped.
    pub fn to_blocks(&self) -> Vec<Block> {
        // Resolve root type handles BEFORE the txn (see [`root_block_id`]).
        let blocks_map = self.doc.get_or_insert_map(BLOCKS);
        let meta = self.doc.get_or_insert_map(META);
        let txn = self.doc.transact();

        let root = read_root(&meta, &txn);

        // Collect every block id present.
        let mut all_ids: Vec<String> = blocks_map.keys(&txn).map(|k| k.to_string()).collect();
        all_ids.sort();
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();

        // DFS from root for stable tree order.
        let mut stack = if root.is_empty() { Vec::new() } else { vec![root] };
        while let Some(id) = stack.pop() {
            if !seen.insert(id.clone()) {
                continue;
            }
            if let Some(block) = self.read_block(&txn, &blocks_map, &id) {
                // push children in reverse so DFS visits them left-to-right
                for child in block.children.iter().rev() {
                    stack.push(child.clone());
                }
                out.push(block);
            }
        }
        // Append any orphans (not reachable from root), id-sorted for determinism.
        for id in all_ids {
            if !seen.contains(&id) {
                if let Some(block) = self.read_block(&txn, &blocks_map, &id) {
                    out.push(block);
                }
            }
        }
        out
    }

    fn read_block<T: ReadTxn>(&self, txn: &T, blocks_map: &MapRef, id: &str) -> Option<Block> {
        let bm: MapRef = blocks_map.get(txn, id)?.cast().ok()?;

        let kind = match bm.get(txn, "ty") {
            Some(Out::Any(Any::String(s))) => s.to_string(),
            _ => String::new(),
        };

        let (text, runs) = match bm.get(txn, "text") {
            Some(out) => match out.cast::<TextRef>() {
                Ok(t) => (t.get_string(txn), text_runs(txn, &t)),
                Err(_) => (String::new(), Vec::new()),
            },
            None => (String::new(), Vec::new()),
        };
        let block_marks: Vec<Mark> = marks::marks_from_runs(&runs);

        let props_str = match bm.get(txn, "props") {
            Some(Out::Any(Any::String(s))) => s.to_string(),
            _ => "null".to_string(),
        };
        let data = rebuild_data(&props_str, &block_marks);

        let children = match bm.get(txn, "children") {
            Some(out) => match out.cast::<ArrayRef>() {
                Ok(arr) => arr
                    .iter(txn)
                    .filter_map(|v| match v {
                        Out::Any(Any::String(s)) => Some(s.to_string()),
                        _ => None,
                    })
                    .collect(),
                Err(_) => Vec::new(),
            },
            None => Vec::new(),
        };

        Some(Block {
            id: id.to_string(),
            kind,
            text,
            data,
            children,
        })
    }

    /// Encode the whole document state as a yrs v1 update (the base snapshot).
    pub fn encode_state(&self) -> Vec<u8> {
        self.doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default())
    }

    /// Rebuild a document from an encoded v1 update.
    pub fn from_update(bytes: &[u8]) -> Result<Self, DocError> {
        let doc = Self::new_doc();
        let update = Update::decode_v1(bytes).map_err(|e| DocError::Decode(e.to_string()))?;
        {
            let mut txn = doc.transact_mut();
            txn.apply_update(update)
                .map_err(|e| DocError::Apply(e.to_string()))?;
        }
        Ok(MicaDoc { doc })
    }
}

fn read_root<T: ReadTxn>(meta: &MapRef, txn: &T) -> String {
    match meta.get(txn, ROOT_KEY) {
        Some(Out::Any(Any::String(s))) => s.to_string(),
        _ => String::new(),
    }
}

/// Read a `Text` as `(utf16_len, attrs)` runs in order, for [`marks::marks_from_runs`].
fn text_runs<T: ReadTxn>(txn: &T, text: &TextRef) -> Vec<(u32, Option<Attrs>)> {
    text.diff(txn, |_| ())
        .into_iter()
        .map(|d| {
            let len = match &d.insert {
                Out::Any(Any::String(s)) => s.encode_utf16().count() as u32,
                other => other.to_string().encode_utf16().count() as u32,
            };
            (len, d.attributes.map(|b| *b))
        })
        .collect()
}

fn props_without_marks(data: &Value) -> Value {
    match data {
        Value::Object(map) => {
            let mut m = map.clone();
            m.remove("marks");
            if m.is_empty() {
                Value::Null
            } else {
                Value::Object(m)
            }
        }
        _ => data.clone(),
    }
}

fn rebuild_data(props_str: &str, block_marks: &[Mark]) -> Value {
    let mut data: Value = serde_json::from_str(props_str).unwrap_or(Value::Null);
    if !block_marks.is_empty() {
        let marks_json = marks::marks_to_json(block_marks);
        match &mut data {
            Value::Object(m) => {
                m.insert("marks".into(), marks_json);
            }
            _ => {
                let mut m = serde_json::Map::new();
                m.insert("marks".into(), marks_json);
                data = Value::Object(m);
            }
        }
    }
    data
}

// ── shared write/read helpers ────────────────────────────────────────────────

/// Create (or overwrite) a block's map entry: `ty`, `text` (+marks as
/// formatting), `props` (data minus marks, as JSON), `children`.
fn write_block(txn: &mut TransactionMut, blocks_map: &MapRef, b: &Block) -> MapRef {
    let bm: MapRef = blocks_map.insert(txn, b.id.clone(), MapPrelim::default());
    bm.insert(txn, "ty", b.kind.clone());
    let text: TextRef = bm.insert(txn, "text", TextPrelim::new(b.text.clone()));
    for (start, len, attrs) in marks::marks_to_format_ops(&marks::marks_from_data(&b.data)) {
        text.format(txn, start, len, attrs);
    }
    let props = props_without_marks(&b.data);
    let props_str = serde_json::to_string(&props).unwrap_or_else(|_| "null".into());
    bm.insert(txn, "props", props_str);
    bm.insert(txn, "children", ArrayPrelim::from(b.children.clone()));
    bm
}

fn get_block_map<T: ReadTxn>(txn: &T, blocks_map: &MapRef, id: &str) -> Option<MapRef> {
    blocks_map.get(txn, id)?.cast().ok()
}

fn get_children<T: ReadTxn>(txn: &T, bm: &MapRef) -> Option<ArrayRef> {
    bm.get(txn, "children")?.cast().ok()
}

fn get_text<T: ReadTxn>(txn: &T, bm: &MapRef) -> Option<TextRef> {
    bm.get(txn, "text")?.cast().ok()
}

fn out_string(out: Out) -> Option<String> {
    match out {
        Out::Any(Any::String(s)) => Some(s.to_string()),
        _ => None,
    }
}

/// The id at `index` of a children array, if it's a string.
fn array_get_string<T: ReadTxn>(txn: &T, arr: &ArrayRef, index: u32) -> Option<String> {
    out_string(arr.get(txn, index)?)
}

/// Find the parent block of `child_id` and the child's index within it.
fn find_parent<T: ReadTxn>(
    txn: &T,
    blocks_map: &MapRef,
    child_id: &str,
) -> Option<(MapRef, u32)> {
    let keys: Vec<String> = blocks_map.keys(txn).map(|k| k.to_string()).collect();
    for key in keys {
        let bm = match get_block_map(txn, blocks_map, &key) {
            Some(m) => m,
            None => continue,
        };
        if let Some(children) = get_children(txn, &bm) {
            for (i, v) in children.iter(txn).enumerate() {
                if out_string(v).as_deref() == Some(child_id) {
                    return Some((bm, i as u32));
                }
            }
        }
    }
    None
}

/// Split a UTF-8 string at UTF-16 offset `at` into `(left, right)`. If `at` lands
/// inside a surrogate-pair character it snaps to after that char.
fn utf16_split(s: &str, at: u32) -> (String, String) {
    let mut count = 0u32;
    for (byte_idx, ch) in s.char_indices() {
        if count == at {
            return (s[..byte_idx].to_string(), s[byte_idx..].to_string());
        }
        count += ch.len_utf16() as u32;
        if count > at {
            let end = byte_idx + ch.len_utf8();
            return (s[..end].to_string(), s[end..].to_string());
        }
    }
    (s.to_string(), String::new())
}

/// Split marks at offset `at`: those fully before go left, those fully after go
/// right (shifted by `-at`), straddling ones are cut at `at`.
fn split_marks(all: &[Mark], at: u32) -> (Vec<Mark>, Vec<Mark>) {
    let mut a = Vec::new();
    let mut b = Vec::new();
    for m in all {
        if m.end <= at {
            a.push(m.clone());
        } else if m.start >= at {
            b.push(Mark { start: m.start - at, end: m.end - at, ..m.clone() });
        } else {
            a.push(Mark { start: m.start, end: at, ..m.clone() });
            b.push(Mark { start: 0, end: m.end - at, ..m.clone() });
        }
    }
    (a, b)
}

fn set_text_and_marks(txn: &mut TransactionMut, bm: &MapRef, text: &str, block_marks: &[Mark]) {
    if let Some(t) = get_text(txn, bm) {
        let len = t.len(txn);
        if len > 0 {
            t.remove_range(txn, 0, len);
        }
        t.insert(txn, 0, text);
        for (start, l, attrs) in marks::marks_to_format_ops(block_marks) {
            t.format(txn, start, l, attrs);
        }
    }
}

// ── editor-intent operations (each one yrs transaction) ──────────────────────

impl MicaDoc {
    /// Insert `block` as a child of `parent_id` at `index` (clamped to the end).
    pub fn insert_block(&mut self, parent_id: &str, index: usize, block: &Block) {
        let blocks_map = self.doc.get_or_insert_map(BLOCKS);
        let mut txn = self.doc.transact_mut();
        write_block(&mut txn, &blocks_map, block);
        if let Some(parent) = get_block_map(&txn, &blocks_map, parent_id) {
            if let Some(children) = get_children(&txn, &parent) {
                let i = (index as u32).min(children.len(&txn));
                children.insert(&mut txn, i, block.id.clone());
            }
        }
    }

    /// Change a block's kind/flavour.
    pub fn update_block_kind(&mut self, id: &str, kind: &str) {
        let blocks_map = self.doc.get_or_insert_map(BLOCKS);
        let mut txn = self.doc.transact_mut();
        if let Some(bm) = get_block_map(&txn, &blocks_map, id) {
            bm.insert(&mut txn, "ty", kind.to_string());
        }
    }

    /// Replace a block's whole text + marks (coarse; for fine edits use
    /// [`Self::text_insert`]/[`Self::text_delete`]/[`Self::text_format`]).
    pub fn set_block_text(&mut self, id: &str, text: &str, block_marks: &[Mark]) {
        let blocks_map = self.doc.get_or_insert_map(BLOCKS);
        let mut txn = self.doc.transact_mut();
        if let Some(bm) = get_block_map(&txn, &blocks_map, id) {
            set_text_and_marks(&mut txn, &bm, text, block_marks);
        }
    }

    /// Set a block's attrs (`props`). Inline marks live on the text, so a
    /// `marks` key in `data` is ignored here.
    pub fn set_block_data(&mut self, id: &str, data: &Value) {
        let blocks_map = self.doc.get_or_insert_map(BLOCKS);
        let mut txn = self.doc.transact_mut();
        if let Some(bm) = get_block_map(&txn, &blocks_map, id) {
            let props = props_without_marks(data);
            let props_str = serde_json::to_string(&props).unwrap_or_else(|_| "null".into());
            bm.insert(&mut txn, "props", props_str);
        }
    }

    /// Insert `s` into a block's text at UTF-16 offset `at` (character-level CRDT).
    pub fn text_insert(&mut self, id: &str, at: u32, s: &str) {
        let blocks_map = self.doc.get_or_insert_map(BLOCKS);
        let mut txn = self.doc.transact_mut();
        if let Some(bm) = get_block_map(&txn, &blocks_map, id) {
            if let Some(t) = get_text(&txn, &bm) {
                let at = at.min(t.len(&txn));
                t.insert(&mut txn, at, s);
            }
        }
    }

    /// Delete `len` UTF-16 units from a block's text starting at `at`.
    pub fn text_delete(&mut self, id: &str, at: u32, len: u32) {
        let blocks_map = self.doc.get_or_insert_map(BLOCKS);
        let mut txn = self.doc.transact_mut();
        if let Some(bm) = get_block_map(&txn, &blocks_map, id) {
            if let Some(t) = get_text(&txn, &bm) {
                let total = t.len(&txn);
                let at = at.min(total);
                let len = len.min(total - at);
                if len > 0 {
                    t.remove_range(&mut txn, at, len);
                }
            }
        }
    }

    /// Apply an inline mark over `[mark.start, mark.end)` of a block's text.
    pub fn text_format(&mut self, id: &str, mark: &Mark) {
        if mark.end <= mark.start {
            return;
        }
        let blocks_map = self.doc.get_or_insert_map(BLOCKS);
        let mut txn = self.doc.transact_mut();
        if let Some(bm) = get_block_map(&txn, &blocks_map, id) {
            if let Some(t) = get_text(&txn, &bm) {
                for (start, l, attrs) in marks::marks_to_format_ops(std::slice::from_ref(mark)) {
                    t.format(&mut txn, start, l, attrs);
                }
            }
        }
    }

    /// Delete a block. With `bring_children_to_parent`, its children are spliced
    /// into the parent at the block's old position; otherwise they become
    /// unreachable (M1: their map entries stay as orphans). The block's own map
    /// entry is removed either way.
    pub fn delete_block(&mut self, id: &str, bring_children_to_parent: bool) {
        let blocks_map = self.doc.get_or_insert_map(BLOCKS);
        let mut txn = self.doc.transact_mut();
        let child_ids: Vec<String> = get_block_map(&txn, &blocks_map, id)
            .and_then(|bm| get_children(&txn, &bm))
            .map(|c| c.iter(&txn).filter_map(out_string).collect())
            .unwrap_or_default();

        if let Some((parent, idx)) = find_parent(&txn, &blocks_map, id) {
            if let Some(pchildren) = get_children(&txn, &parent) {
                pchildren.remove(&mut txn, idx);
                if bring_children_to_parent {
                    for (k, cid) in child_ids.iter().enumerate() {
                        pchildren.insert(&mut txn, idx + k as u32, cid.clone());
                    }
                }
            }
        }
        blocks_map.remove(&mut txn, id);
    }

    /// Move a block to be a child of `new_parent` at `index`.
    pub fn move_block(&mut self, id: &str, new_parent: &str, index: usize) {
        let blocks_map = self.doc.get_or_insert_map(BLOCKS);
        let mut txn = self.doc.transact_mut();
        if let Some((old, oidx)) = find_parent(&txn, &blocks_map, id) {
            if let Some(oc) = get_children(&txn, &old) {
                oc.remove(&mut txn, oidx);
            }
        }
        if let Some(np) = get_block_map(&txn, &blocks_map, new_parent) {
            if let Some(nc) = get_children(&txn, &np) {
                let i = (index as u32).min(nc.len(&txn));
                nc.insert(&mut txn, i, id.to_string());
            }
        }
    }

    /// Split block `id` at UTF-16 offset `at` into two: the original keeps
    /// `[..at]`, a new block `new_id` (kind `new_kind`) takes `[at..]` plus the
    /// original's children, inserted as the next sibling.
    ///
    /// Read-modify-write — it does NOT preserve character identity across the
    /// split, so a concurrent edit of the moved tail could be lost. Fine for
    /// single-writer M1; revisit for CRDT fidelity before multi-writer sync (M4).
    pub fn split_block(&mut self, id: &str, at: u32, new_id: &str, new_kind: &str) {
        let blocks_map = self.doc.get_or_insert_map(BLOCKS);
        let mut txn = self.doc.transact_mut();
        let orig = match self.read_block(&txn, &blocks_map, id) {
            Some(b) => b,
            None => return,
        };
        let (text_a, text_b) = utf16_split(&orig.text, at);
        let (marks_a, marks_b) = split_marks(&marks::marks_from_data(&orig.data), at);
        let props_str =
            serde_json::to_string(&props_without_marks(&orig.data)).unwrap_or_else(|_| "null".into());

        // Original keeps the head and loses its children (they go to the tail).
        if let Some(bm) = get_block_map(&txn, &blocks_map, id) {
            set_text_and_marks(&mut txn, &bm, &text_a, &marks_a);
            if let Some(c) = get_children(&txn, &bm) {
                let cl = c.len(&txn);
                if cl > 0 {
                    c.remove_range(&mut txn, 0, cl);
                }
            }
        }

        // New tail block.
        let tail = Block {
            id: new_id.to_string(),
            kind: new_kind.to_string(),
            text: text_b,
            data: rebuild_data(&props_str, &marks_b),
            children: orig.children.clone(),
        };
        write_block(&mut txn, &blocks_map, &tail);

        if let Some((parent, idx)) = find_parent(&txn, &blocks_map, id) {
            if let Some(pc) = get_children(&txn, &parent) {
                pc.insert(&mut txn, idx + 1, new_id.to_string());
            }
        }
    }

    /// Join block `id` into its previous sibling: append `id`'s text (marks
    /// shifted) to the previous sibling, move `id`'s children after, and delete
    /// `id`. No-op if `id` has no previous sibling.
    pub fn join_into_prev(&mut self, id: &str) {
        let blocks_map = self.doc.get_or_insert_map(BLOCKS);
        let mut txn = self.doc.transact_mut();
        let (parent, idx) = match find_parent(&txn, &blocks_map, id) {
            Some(x) => x,
            None => return,
        };
        if idx == 0 {
            return;
        }
        let prev_id = match get_children(&txn, &parent).and_then(|c| array_get_string(&txn, &c, idx - 1)) {
            Some(s) => s,
            None => return,
        };
        let cur = match self.read_block(&txn, &blocks_map, id) {
            Some(b) => b,
            None => return,
        };
        let prev = match self.read_block(&txn, &blocks_map, &prev_id) {
            Some(b) => b,
            None => return,
        };

        let prev_len = prev.text.encode_utf16().count() as u32;
        let merged_text = format!("{}{}", prev.text, cur.text);
        let mut merged_marks = marks::marks_from_data(&prev.data);
        for m in marks::marks_from_data(&cur.data) {
            merged_marks.push(Mark { start: m.start + prev_len, end: m.end + prev_len, ..m });
        }

        if let Some(pbm) = get_block_map(&txn, &blocks_map, &prev_id) {
            set_text_and_marks(&mut txn, &pbm, &merged_text, &merged_marks);
            if let Some(pc) = get_children(&txn, &pbm) {
                let base = pc.len(&txn);
                for (k, cid) in cur.children.iter().enumerate() {
                    pc.insert(&mut txn, base + k as u32, cid.clone());
                }
            }
        }
        if let Some(pc) = get_children(&txn, &parent) {
            pc.remove(&mut txn, idx);
        }
        blocks_map.remove(&mut txn, id);
    }
}
