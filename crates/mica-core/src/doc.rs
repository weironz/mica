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
    ReadTxn, StateVector, Text, TextPrelim, TextRef, Transact, Update,
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
                let bm: MapRef = blocks_map.insert(&mut txn, b.id.clone(), MapPrelim::default());
                bm.insert(&mut txn, "ty", b.kind.clone());

                let text: TextRef = bm.insert(&mut txn, "text", TextPrelim::new(b.text.clone()));
                let block_marks = marks::marks_from_data(&b.data);
                for (start, len, attrs) in marks::marks_to_format_ops(&block_marks) {
                    text.format(&mut txn, start, len, attrs);
                }

                let props = props_without_marks(&b.data);
                let props_str = serde_json::to_string(&props).unwrap_or_else(|_| "null".into());
                bm.insert(&mut txn, "props", props_str);

                bm.insert(&mut txn, "children", ArrayPrelim::from(b.children.clone()));
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
