//! Inline marks ↔ yrs `Y.Text` formatting attributes.
//!
//! Mica stores inline rich text as "marks over plain text": the block text is
//! clean and `data.marks` carries `{start,end,type[,href,title]}` ranges in Dart
//! UTF-16 string offsets. In yrs we model the same text as a `TextRef` whose
//! formatting attributes ARE the marks — one attribute key per mark type. This
//! module converts both ways. The owning [`crate::doc::MicaDoc`] uses
//! `OffsetKind::Utf16`, so offsets line up with Dart string indices exactly.

use std::collections::HashMap;
use std::sync::Arc;

use yrs::types::Attrs;
use yrs::Any;

/// The inline mark types Mica supports (mirrors the Dart `marks.dart` set).
/// Used to wipe a text range's formatting before re-applying a block's
/// authoritative marks.
pub const MARK_TYPES: [&str; 6] = ["bold", "italic", "code", "strike", "link", "footnote"];

/// An [`Attrs`] that unsets every known mark attribute (each key → `Null`). yrs
/// treats a `Null` formatting value as "remove this attribute", so applying this
/// over a range clears all of Mica's inline marks there.
pub fn clear_all_attrs() -> Attrs {
    MARK_TYPES
        .iter()
        .map(|k| (Arc::from(*k), Any::Null))
        .collect()
}

/// A single inline mark over `[start, end)` in UTF-16 offsets.
#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    pub start: u32,
    pub end: u32,
    pub ty: String,
    pub href: Option<String>,
    pub title: Option<String>,
}

impl Mark {
    /// The yrs attribute value: bare `true` for a simple mark, or a
    /// `{href?, title?}` map when it carries link metadata.
    fn attr_value(&self) -> Any {
        if self.href.is_some() || self.title.is_some() {
            let mut m = HashMap::new();
            if let Some(h) = &self.href {
                m.insert("href".to_string(), Any::String(h.as_str().into()));
            }
            if let Some(t) = &self.title {
                m.insert("title".to_string(), Any::String(t.as_str().into()));
            }
            Any::Map(Arc::new(m))
        } else {
            Any::Bool(true)
        }
    }
}

/// Parse marks from a block's `data` JSON (`data.marks`), skipping malformed or
/// empty-range entries — mirrors the Dart `marksFromData`.
pub fn marks_from_data(data: &serde_json::Value) -> Vec<Mark> {
    let Some(arr) = data.get("marks").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for m in arr {
        let (Some(start), Some(end), Some(ty)) = (
            m.get("start").and_then(|v| v.as_u64()),
            m.get("end").and_then(|v| v.as_u64()),
            m.get("type").and_then(|v| v.as_str()),
        ) else {
            continue;
        };
        if end <= start {
            continue;
        }
        out.push(Mark {
            start: start as u32,
            end: end as u32,
            ty: ty.to_string(),
            href: m.get("href").and_then(|v| v.as_str()).map(String::from),
            title: m.get("title").and_then(|v| v.as_str()).map(String::from),
        });
    }
    out
}

/// Serialize marks back into the JSON array stored under `data.marks`.
pub fn marks_to_json(marks: &[Mark]) -> serde_json::Value {
    let arr: Vec<serde_json::Value> = marks
        .iter()
        .map(|m| {
            let mut o = serde_json::Map::new();
            o.insert("start".into(), m.start.into());
            o.insert("end".into(), m.end.into());
            o.insert("type".into(), m.ty.clone().into());
            if let Some(h) = &m.href {
                o.insert("href".into(), h.clone().into());
            }
            if let Some(t) = &m.title {
                o.insert("title".into(), t.clone().into());
            }
            serde_json::Value::Object(o)
        })
        .collect();
    serde_json::Value::Array(arr)
}

/// One `Text::format(start, len, attrs)` op per mark. yrs merges overlapping
/// attributes correctly, so applying each mark independently is sound.
pub fn marks_to_format_ops(marks: &[Mark]) -> Vec<(u32, u32, Attrs)> {
    marks
        .iter()
        .filter(|m| m.end > m.start)
        .map(|m| {
            let mut attrs: Attrs = HashMap::new();
            attrs.insert(Arc::from(m.ty.as_str()), m.attr_value());
            (m.start, m.end - m.start, attrs)
        })
        .collect()
}

/// Per-type metadata carried by a mark attribute (href/title for links).
type Meta = (Option<String>, Option<String>);

fn meta_of(value: &Any) -> Meta {
    match value {
        Any::Map(m) => (
            m.get("href").and_then(any_str),
            m.get("title").and_then(any_str),
        ),
        _ => (None, None),
    }
}

fn any_str(v: &Any) -> Option<String> {
    match v {
        Any::String(s) => Some(s.to_string()),
        _ => None,
    }
}

/// Rebuild marks from a yrs text delta given as `(run_utf16_len, attrs)` runs in
/// text order. A mark type stays open across consecutive runs that carry it with
/// the SAME metadata, and is closed (and re-opened) when it disappears or its
/// metadata changes — so split runs (caused by overlapping marks) recombine into
/// the original ranges.
pub fn marks_from_runs(runs: &[(u32, Option<Attrs>)]) -> Vec<Mark> {
    let mut marks: Vec<Mark> = Vec::new();
    // ty -> (start_offset, meta)
    let mut open: HashMap<String, (u32, Meta)> = HashMap::new();
    let mut offset: u32 = 0;

    for (len, attrs) in runs {
        let here: HashMap<String, Meta> = match attrs {
            // A `Null` value means the attribute was cleared (yrs reports removed
            // formatting as `key: Null`), so treat it as absent — not a mark.
            Some(a) => a
                .iter()
                .filter(|(_, v)| !matches!(v, Any::Null))
                .map(|(k, v)| (k.to_string(), meta_of(v)))
                .collect(),
            None => HashMap::new(),
        };

        // Close marks absent here, or whose metadata changed.
        let to_close: Vec<String> = open
            .iter()
            .filter(|(ty, (_, meta))| here.get(*ty).map(|m| m != meta).unwrap_or(true))
            .map(|(ty, _)| ty.clone())
            .collect();
        for ty in to_close {
            let (start, (href, title)) = open.remove(&ty).unwrap();
            marks.push(Mark { start, end: offset, ty, href, title });
        }
        // Open marks newly present (or just re-opened after a metadata change).
        for (ty, meta) in here {
            open.entry(ty).or_insert((offset, meta));
        }
        offset += len;
    }

    // Close whatever's still open at the end of the text.
    let mut tail: Vec<(String, (u32, Meta))> = open.into_iter().collect();
    tail.sort_by(|a, b| a.0.cmp(&b.0));
    for (ty, (start, (href, title))) in tail {
        marks.push(Mark { start, end: offset, ty, href, title });
    }

    marks.sort_by(|a, b| (a.start, a.end, a.ty.as_str()).cmp(&(b.start, b.end, b.ty.as_str())));
    marks
}
