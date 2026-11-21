//! Field-level CRDT representation of a board on `yrs::Doc` (behind `crdt` feature).
//! Uses dedicated root keys (`board_meta`, `board_nodes`, `board_edges`, `board_strokes`)
//! completely disjoint from `MicaDoc`'s `blocks` + `meta` keys.

use yrs::updates::decoder::Decode;
use yrs::{
  Any, ClientID, Doc, Map, MapPrelim, MapRef, Number, OffsetKind, Options, Out, ReadTxn,
  StateVector, Transact, Update,
};

use crate::model::{
  BOARD_SCHEMA_VERSION, BoardEdge, BoardNode, BoardSnapshot, BoardStroke, EdgeStyle, NodeStyle,
};

const BOARD_META: &str = "board_meta";
const BOARD_NODES: &str = "board_nodes";
const BOARD_EDGES: &str = "board_edges";
const BOARD_STROKES: &str = "board_strokes";

#[derive(Debug, thiserror::Error)]
pub enum BoardCrdtError {
  #[error("failed to decode yrs update: {0}")]
  Decode(String),
  #[error("failed to apply yrs update: {0}")]
  Apply(String),
}

pub struct BoardDoc {
  doc: Doc,
}

impl Default for BoardDoc {
  fn default() -> Self {
    Self::new(None)
  }
}

impl BoardDoc {
  pub fn new(client_id: Option<u64>) -> Self {
    let mut options = Options {
      offset_kind: OffsetKind::Utf16,
      ..Default::default()
    };
    if let Some(cid) = client_id {
      options.client_id = ClientID::new(cid & ((1u64 << 53) - 1));
    }
    Self {
      doc: Doc::with_options(options),
    }
  }

  pub fn from_snapshot(snap: &BoardSnapshot, client_id: Option<u64>) -> Self {
    let board = Self::new(client_id);
    board.replace_from_snapshot(snap);
    board
  }

  pub fn replace_from_snapshot(&self, snap: &BoardSnapshot) {
    let meta = self.doc.get_or_insert_map(BOARD_META);
    let nodes = self.doc.get_or_insert_map(BOARD_NODES);
    let edges = self.doc.get_or_insert_map(BOARD_EDGES);
    let strokes = self.doc.get_or_insert_map(BOARD_STROKES);

    let mut txn = self.doc.transact_mut();
    meta.insert(&mut txn, "version", snap.version as i64);
    meta.insert(&mut txn, "title", snap.title.as_str());
    meta.insert(&mut txn, "mode", snap.mode.as_str());

    let existing_node_keys: Vec<String> = nodes.keys(&txn).map(str::to_string).collect();
    for k in existing_node_keys {
      if !snap.nodes.iter().any(|n| n.id == k) {
        nodes.remove(&mut txn, &k);
      }
    }
    for n in &snap.nodes {
      let nm: MapRef = nodes.insert(&mut txn, n.id.as_str(), MapPrelim::default());
      nm.insert(&mut txn, "kind", n.kind.as_str());
      nm.insert(&mut txn, "x", n.x);
      nm.insert(&mut txn, "y", n.y);
      nm.insert(&mut txn, "w", n.w);
      nm.insert(&mut txn, "h", n.h);
      nm.insert(&mut txn, "rot", n.rot);
      nm.insert(&mut txn, "z", n.z as i64);
      nm.insert(&mut txn, "rank", n.rank as i64);
      nm.insert(&mut txn, "collapsed", n.collapsed);
      nm.insert(&mut txn, "text", n.text.as_str());
      if let Some(pid) = &n.parent_id {
        nm.insert(&mut txn, "parentId", pid.as_str());
      }
      if let Some(icon) = &n.icon {
        nm.insert(&mut txn, "icon", icon.as_str());
      }
      if let Ok(style_json) = serde_json::to_string(&n.style) {
        nm.insert(&mut txn, "style", style_json);
      }
    }

    let existing_edge_keys: Vec<String> = edges.keys(&txn).map(str::to_string).collect();
    for k in existing_edge_keys {
      if !snap.edges.iter().any(|e| e.id == k) {
        edges.remove(&mut txn, &k);
      }
    }
    for e in &snap.edges {
      if let Ok(json) = serde_json::to_string(e) {
        edges.insert(&mut txn, e.id.as_str(), json);
      }
    }

    let existing_stroke_keys: Vec<String> = strokes.keys(&txn).map(str::to_string).collect();
    for k in existing_stroke_keys {
      if !snap.strokes.iter().any(|s| s.id == k) {
        strokes.remove(&mut txn, &k);
      }
    }
    for s in &snap.strokes {
      if let Ok(json) = serde_json::to_string(s) {
        strokes.insert(&mut txn, s.id.as_str(), json);
      }
    }
  }

  /// Field-level update of a node's position (does not overwrite concurrent text/style edits).
  pub fn move_node(&self, node_id: &str, x: f64, y: f64) {
    let nodes = self.doc.get_or_insert_map(BOARD_NODES);
    let mut txn = self.doc.transact_mut();
    if let Some(Out::YMap(map)) = nodes.get(&txn, node_id) {
      map.insert(&mut txn, "x", x);
      map.insert(&mut txn, "y", y);
    }
  }

  /// Field-level update of a node's text (does not overwrite concurrent position edits).
  pub fn set_node_text(&self, node_id: &str, text: &str) {
    let nodes = self.doc.get_or_insert_map(BOARD_NODES);
    let mut txn = self.doc.transact_mut();
    if let Some(Out::YMap(map)) = nodes.get(&txn, node_id) {
      map.insert(&mut txn, "text", text);
    }
  }

  pub fn encode_state_as_update(&self) -> Vec<u8> {
    let txn = self.doc.transact();
    txn.encode_state_as_update_v1(&StateVector::default())
  }

  pub fn apply_update(&self, bytes: &[u8]) -> Result<(), BoardCrdtError> {
    let update = Update::decode_v1(bytes).map_err(|e| BoardCrdtError::Decode(e.to_string()))?;
    let mut txn = self.doc.transact_mut();
    txn
      .apply_update(update)
      .map_err(|e| BoardCrdtError::Apply(e.to_string()))
  }

  pub fn to_snapshot(&self) -> BoardSnapshot {
    let txn = self.doc.transact();
    let meta = txn.get_map(BOARD_META);
    let title = meta
      .as_ref()
      .and_then(|m| read_str(&txn, m, "title"))
      .unwrap_or_else(|| "未命名画板".into());
    let mode = meta
      .as_ref()
      .and_then(|m| read_str(&txn, m, "mode"))
      .unwrap_or_else(|| "whiteboard".into());

    let mut nodes = Vec::new();
    if let Some(nmap) = txn.get_map(BOARD_NODES) {
      for (id, val) in nmap.iter(&txn) {
        if let Out::YMap(m) = val {
          let style = read_str(&txn, &m, "style")
            .and_then(|s| serde_json::from_str::<NodeStyle>(&s).ok())
            .unwrap_or_default();
          nodes.push(BoardNode {
            id: id.to_string(),
            kind: read_str(&txn, &m, "kind").unwrap_or_else(|| "rounded".into()),
            x: read_f64(&txn, &m, "x").unwrap_or(0.0),
            y: read_f64(&txn, &m, "y").unwrap_or(0.0),
            w: read_f64(&txn, &m, "w").unwrap_or(140.0),
            h: read_f64(&txn, &m, "h").unwrap_or(52.0),
            rot: read_f64(&txn, &m, "rot").unwrap_or(0.0),
            z: read_i64(&txn, &m, "z").unwrap_or(1) as i32,
            parent_id: read_str(&txn, &m, "parentId"),
            rank: read_i64(&txn, &m, "rank").unwrap_or(0) as i32,
            collapsed: read_bool(&txn, &m, "collapsed").unwrap_or(false),
            text: read_str(&txn, &m, "text").unwrap_or_default(),
            icon: read_str(&txn, &m, "icon"),
            style,
          });
        }
      }
    }
    nodes.sort_by(|a, b| a.z.cmp(&b.z).then_with(|| a.id.cmp(&b.id)));

    let mut edges = Vec::new();
    if let Some(emap) = txn.get_map(BOARD_EDGES) {
      for (_, val) in emap.iter(&txn) {
        if let Out::Any(Any::String(s)) = val {
          if let Ok(mut e) = serde_json::from_str::<BoardEdge>(&s) {
            if e.style.stroke.is_empty() {
              e.style = EdgeStyle::default();
            }
            edges.push(e);
          }
        }
      }
    }
    edges.sort_by(|a, b| a.id.cmp(&b.id));

    let mut strokes = Vec::new();
    if let Some(smap) = txn.get_map(BOARD_STROKES) {
      for (_, val) in smap.iter(&txn) {
        if let Out::Any(Any::String(s)) = val {
          if let Ok(st) = serde_json::from_str::<BoardStroke>(&s) {
            strokes.push(st);
          }
        }
      }
    }
    strokes.sort_by(|a, b| a.id.cmp(&b.id));

    BoardSnapshot {
      version: BOARD_SCHEMA_VERSION,
      title,
      mode,
      nodes,
      edges,
      strokes,
    }
  }
}

fn read_str<T: ReadTxn>(txn: &T, m: &MapRef, key: &str) -> Option<String> {
  match m.get(txn, key)? {
    Out::Any(Any::String(s)) => Some(s.to_string()),
    _ => None,
  }
}

fn read_f64<T: ReadTxn>(txn: &T, m: &MapRef, key: &str) -> Option<f64> {
  match m.get(txn, key)? {
    Out::Any(Any::Number(Number::Float(f))) => Some(f),
    Out::Any(Any::Number(Number::Int(i))) => Some(i as f64),
    _ => None,
  }
}

fn read_i64<T: ReadTxn>(txn: &T, m: &MapRef, key: &str) -> Option<i64> {
  match m.get(txn, key)? {
    Out::Any(Any::Number(Number::Int(i))) => Some(i),
    Out::Any(Any::Number(Number::Float(f))) => Some(f as i64),
    _ => None,
  }
}

fn read_bool<T: ReadTxn>(txn: &T, m: &MapRef, key: &str) -> Option<bool> {
  match m.get(txn, key)? {
    Out::Any(Any::Bool(b)) => Some(b),
    _ => None,
  }
}
