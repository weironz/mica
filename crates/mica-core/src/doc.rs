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

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;
use yrs::types::{Attrs, GetString};
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{
    Any, Array, ArrayPrelim, ArrayRef, ClientID, Doc, Map, MapPrelim, MapRef, OffsetKind, Options,
    Out, ReadTxn, StateVector, Text, TextPrelim, TextRef, Transact, TransactionMut, Update,
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
    /// Create the backing yrs doc. UTF-16 offsets (to match Dart string indices);
    /// a fixed `client_id` (the persisted on-device identity) keeps CRDT
    /// authorship stable across restarts — never random per launch (§6).
    fn new_doc_with(client_id: Option<u64>) -> Doc {
        let mut options = Options {
            offset_kind: OffsetKind::Utf16,
            ..Default::default()
        };
        if let Some(cid) = client_id {
            // ClientID is a 53-bit value (Yjs-compatible); callers pass a masked
            // u64 (see the store's identity minting).
            options.client_id = ClientID::new(cid & ((1 << 53) - 1));
        }
        Doc::with_options(options)
    }

    /// The yrs client id (53-bit u64) of this doc's local actor.
    pub fn client_id(&self) -> u64 {
        self.doc.client_id().get()
    }

    /// Build a document from a flat block list rooted at `root_id`.
    pub fn from_blocks(root_id: &str, blocks: &[Block]) -> Self {
        Self::from_blocks_with_client_id(root_id, blocks, None)
    }

    /// Like [`Self::from_blocks`] but with a fixed yrs client id (the persisted
    /// device identity) so all this device's edits share one stable actor.
    pub fn from_blocks_with_client_id(
        root_id: &str,
        blocks: &[Block],
        client_id: Option<u64>,
    ) -> Self {
        let doc = Self::new_doc_with(client_id);
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

        let data = merge_marks(read_props(txn, &bm), &block_marks);

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

    // ── sync primitives (P2-M4) ──────────────────────────────────────────────

    /// This replica's state vector (what it has observed), v1-encoded. A peer
    /// sends its own to ask us for the minimal update it is missing.
    pub fn state_vector(&self) -> Vec<u8> {
        self.doc.transact().state_vector().encode_v1()
    }

    /// Encode the update carrying everything THIS doc has that a peer with
    /// `remote_sv` (their v1-encoded [`Self::state_vector`]) lacks — the minimal
    /// diff to send them. Passing an empty/`StateVector::default` SV yields the
    /// full state (same as [`Self::encode_state`]).
    pub fn encode_diff(&self, remote_sv: &[u8]) -> Result<Vec<u8>, DocError> {
        let sv = StateVector::decode_v1(remote_sv).map_err(|e| DocError::Decode(e.to_string()))?;
        Ok(self.doc.transact().encode_state_as_update_v1(&sv))
    }

    /// Merge a peer's v1-encoded update into this doc (CRDT merge — commutative,
    /// idempotent, order-independent). No-op-safe on already-seen updates.
    pub fn apply_update(&mut self, bytes: &[u8]) -> Result<(), DocError> {
        let update = Update::decode_v1(bytes).map_err(|e| DocError::Decode(e.to_string()))?;
        let mut txn = self.doc.transact_mut();
        txn.apply_update(update)
            .map_err(|e| DocError::Apply(e.to_string()))?;
        Ok(())
    }

    /// Rebuild a document from an encoded v1 update.
    pub fn from_update(bytes: &[u8]) -> Result<Self, DocError> {
        Self::from_update_with_client_id(bytes, None)
    }

    /// Like [`Self::from_update`] but with a fixed yrs client id (see
    /// [`Self::from_blocks_with_client_id`]).
    pub fn from_update_with_client_id(
        bytes: &[u8],
        client_id: Option<u64>,
    ) -> Result<Self, DocError> {
        let doc = Self::new_doc_with(client_id);
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

/// `data` with the inline `marks` key dropped (marks live on the text, not in
/// props). Returns the remaining object, or `Null` if there's nothing else.
fn data_without_marks(data: &Value) -> Value {
    match data {
        Value::Object(o) => {
            let mut m = o.clone();
            m.remove("marks");
            if m.is_empty() {
                Value::Null
            } else {
                Value::Object(m)
            }
        }
        _ => Value::Null,
    }
}

/// serde_json::Value → yrs `Any` (the embeddable JSON-like value type). Integers
/// stay integers (`BigInt`), so block props like `indent: 1` round-trip exactly.
fn json_to_any(v: &Value) -> Any {
    match v {
        Value::Null => Any::Null,
        Value::Bool(b) => Any::Bool(*b),
        Value::Number(n) => match n.as_i64() {
            Some(i) => Any::BigInt(i),
            None => Any::Number(n.as_f64().unwrap_or(0.0)),
        },
        Value::String(s) => Any::String(s.as_str().into()),
        Value::Array(a) => Any::Array(a.iter().map(json_to_any).collect::<Vec<_>>().into()),
        Value::Object(o) => Any::Map(Arc::new(
            o.iter().map(|(k, v)| (k.clone(), json_to_any(v))).collect(),
        )),
    }
}

/// yrs `Any` → serde_json::Value (inverse of [`json_to_any`]).
fn any_to_json(a: &Any) -> Value {
    match a {
        Any::Null | Any::Undefined => Value::Null,
        Any::Bool(b) => Value::Bool(*b),
        // JS (yjs) has no int/float split, so an integer-valued prop like
        // `level: 2` can arrive as a float. Normalise it back to an int so it
        // matches what the desktop writes (and what the editor expects).
        Any::Number(f) if f.is_finite() && f.fract() == 0.0 && f.abs() < 9.007e15 => {
            Value::Number((*f as i64).into())
        }
        Any::Number(f) => serde_json::Number::from_f64(*f)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        Any::BigInt(i) => Value::Number((*i).into()),
        Any::String(s) => Value::String(s.to_string()),
        Any::Buffer(_) => Value::Null,
        Any::Array(arr) => Value::Array(arr.iter().map(any_to_json).collect()),
        Any::Map(m) => Value::Object(m.iter().map(|(k, v)| (k.clone(), any_to_json(v))).collect()),
    }
}

/// Read a block's `props` into a data object, handling BOTH the field-level
/// `MapRef` form (P2-M4.7) and the legacy JSON-string form (pre-M4.7 data, read
/// for migration). Inline marks are NOT here — they're reconstructed from text.
fn read_props<T: ReadTxn>(txn: &T, bm: &MapRef) -> Value {
    match bm.get(txn, "props") {
        // Legacy: props stored as a JSON string.
        Some(Out::Any(Any::String(s))) => serde_json::from_str(&s).unwrap_or(Value::Null),
        // Field-level: props stored as a nested map (one entry per top-level key).
        Some(Out::YMap(m)) => {
            let mut obj = serde_json::Map::new();
            let keys: Vec<String> = m.keys(txn).map(|k| k.to_string()).collect();
            for k in keys {
                if let Some(Out::Any(a)) = m.get(txn, &k) {
                    obj.insert(k, any_to_json(&a));
                }
            }
            if obj.is_empty() {
                Value::Null
            } else {
                Value::Object(obj)
            }
        }
        _ => Value::Null,
    }
}

/// Write a block's `data` (minus inline `marks`) into a nested `props` MapRef so
/// concurrent edits to DIFFERENT props keys converge (field-level CRDT) instead
/// of last-write-wins on the whole blob. Reconciles in place — sets desired
/// keys, drops absent ones — migrating a legacy string `props` to a map on the
/// first write.
fn set_props(txn: &mut TransactionMut, bm: &MapRef, data: &Value) {
    let props: MapRef = match bm.get(txn, "props") {
        Some(Out::YMap(m)) => m,
        _ => bm.insert(txn, "props", MapPrelim::default()),
    };
    let desired: HashMap<&str, &Value> = match data {
        Value::Object(o) => o
            .iter()
            .filter(|(k, _)| k.as_str() != "marks")
            .map(|(k, v)| (k.as_str(), v))
            .collect(),
        _ => HashMap::new(),
    };
    let existing: Vec<String> = props.keys(txn).map(|k| k.to_string()).collect();
    for k in existing {
        if !desired.contains_key(k.as_str()) {
            props.remove(txn, &k);
        }
    }
    for (k, v) in desired {
        props.insert(txn, k.to_string(), json_to_any(v));
    }
}


fn merge_marks(mut data: Value, block_marks: &[Mark]) -> Value {
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
    set_props(txn, &bm, &b.data);
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

    /// Replace a block's inline marks while keeping its text, by clearing all
    /// known formatting over the whole text and re-applying `block_marks`. Use
    /// this when the editor's authoritative marks (in `data`) change without a
    /// text edit — e.g. a turn-into that resets `data` and so should drop marks.
    pub fn set_block_marks(&mut self, id: &str, block_marks: &[Mark]) {
        let blocks_map = self.doc.get_or_insert_map(BLOCKS);
        let mut txn = self.doc.transact_mut();
        if let Some(bm) = get_block_map(&txn, &blocks_map, id) {
            if let Some(t) = get_text(&txn, &bm) {
                let len = t.len(&txn);
                if len > 0 {
                    t.format(&mut txn, 0, len, marks::clear_all_attrs());
                    for (start, l, attrs) in marks::marks_to_format_ops(block_marks) {
                        t.format(&mut txn, start, l, attrs);
                    }
                }
            }
        }
    }

    /// Set a block's attrs (`props`). Inline marks live on the text, so a
    /// `marks` key in `data` is ignored here.
    pub fn set_block_data(&mut self, id: &str, data: &Value) {
        let blocks_map = self.doc.get_or_insert_map(BLOCKS);
        let mut txn = self.doc.transact_mut();
        if let Some(bm) = get_block_map(&txn, &blocks_map, id) {
            set_props(&mut txn, &bm, data);
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
    /// Read-modify-write: the tail is re-inserted as new text, so it does NOT
    /// keep the moved characters' CRDT item identity — a *simultaneous* edit by
    /// another writer to the exact tail being split off can be lost (it still
    /// CONVERGES; no divergence/corruption). This is an ACCEPTED limitation, not
    /// a TODO: yrs/Yjs cannot transplant text items across `Text` instances (an
    /// item's parent is fixed at integration), so identity-preserving split is
    /// impossible in a block-owns-its-own-`Text` model without a doc-wide
    /// single-sequence redesign. The closest analogs deliberately accept the same
    /// trade — AppFlowy (`appflowy-editor` `insertNewLine`/`mergeText`, same
    /// yrs+Flutter+per-block-`TextRef` stack) and BlockSuite/AFFiNE
    /// (`Text.split`/`join`) both do read-modify-write. (P2: researched, decided.)
    pub fn split_block(&mut self, id: &str, at: u32, new_id: &str, new_kind: &str) {
        let blocks_map = self.doc.get_or_insert_map(BLOCKS);
        let mut txn = self.doc.transact_mut();
        let orig = match self.read_block(&txn, &blocks_map, id) {
            Some(b) => b,
            None => return,
        };
        let (text_a, text_b) = utf16_split(&orig.text, at);
        let (marks_a, marks_b) = split_marks(&marks::marks_from_data(&orig.data), at);
        let tail_props = data_without_marks(&orig.data);

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
            data: merge_marks(tail_props, &marks_b),
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
