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

/// The MINIMAL format ops that turn the formatting currently in the text
/// (`runs`, as [`crate::doc`] reads them) into `target`.
///
/// This is the delta↔marks mapping, and it is what makes concurrent FORMATTING
/// merge instead of overwrite. The coarse version it replaces was
/// `format(0, len, clear_all)` followed by a replay of every mark: correct for
/// one writer, and for two writers a guaranteed clobber, because "clear
/// everything" is an operation over the WHOLE text — B's bold on words 6-10 was
/// deleted by A's re-format even though A only touched word 1. Emitting only the
/// ranges that actually differ puts the two writers on disjoint ranges, exactly
/// as the minimal text splice does for characters.
///
/// Removals become expressible for the first time: a key that `runs` has and
/// `target` does not becomes an explicit `Null` over just that range, which is
/// how "this got un-bolded" is said without also saying "and nothing else is
/// bold either".
///
/// Segments are cut at every run boundary AND every mark edge, so within a
/// segment a mark either covers all of it or none of it. Adjacent segments with
/// an identical delta are merged, which keeps a whole-block bold to one op
/// rather than one per pre-existing run.
///
/// Mirrored in Dart (`web/mica_ydoc.dart`, `_marksDiffFormatOps`) — the two
/// engines have to agree on what an edit MEANS, not just on what it renders to.
pub fn marks_diff_format_ops(
    runs: &[(u32, Option<Attrs>)],
    target: &[Mark],
) -> Vec<(u32, u32, Attrs)> {
    // Current formatting as (end_offset, attrs), plus the total length.
    let mut cur: Vec<(u32, Option<&Attrs>)> = Vec::with_capacity(runs.len());
    let mut total: u32 = 0;
    for (len, attrs) in runs {
        total += len;
        cur.push((total, attrs.as_ref()));
    }
    if total == 0 {
        return Vec::new();
    }

    let mut cuts: Vec<u32> = vec![0, total];
    cuts.extend(cur.iter().map(|(end, _)| *end));
    for m in target.iter().filter(|m| m.end > m.start) {
        cuts.push(m.start.min(total));
        cuts.push(m.end.min(total));
    }
    cuts.sort_unstable();
    cuts.dedup();

    let attrs_at = |p: u32| -> Option<&Attrs> {
        cur.iter().find(|(end, _)| p < *end).and_then(|(_, a)| *a)
    };

    let mut ops: Vec<(u32, u32, Attrs)> = Vec::new();
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        if a >= b {
            continue;
        }
        // What `target` says this segment should carry. Cuts land on every mark
        // edge, so an overlap here is total coverage.
        let mut want: Attrs = HashMap::new();
        for m in target.iter().filter(|m| m.start <= a && m.end >= b) {
            want.insert(Arc::from(m.ty.as_str()), m.attr_value());
        }
        let now = attrs_at(a);
        let mut delta: Attrs = HashMap::new();
        for (k, v) in &want {
            // `Null` in the current run means the attribute is absent, not that
            // it is present with a null value.
            let same = now
                .and_then(|n| n.get(k))
                .is_some_and(|n| n != &Any::Null && n == v);
            if !same {
                delta.insert(k.clone(), v.clone());
            }
        }
        if let Some(now) = now {
            for (k, v) in now {
                if v != &Any::Null && !want.contains_key(k) {
                    delta.insert(k.clone(), Any::Null);
                }
            }
        }
        if delta.is_empty() {
            continue;
        }
        match ops.last_mut() {
            Some((start, len, prev)) if *start + *len == a && *prev == delta => *len += b - a,
            _ => ops.push((a, b - a, delta)),
        }
    }
    ops
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

#[cfg(test)]
mod diff_tests {
    use super::*;

    fn attrs(pairs: &[(&str, Any)]) -> Attrs {
        pairs
            .iter()
            .map(|(k, v)| (Arc::from(*k), v.clone()))
            .collect()
    }

    fn target(v: serde_json::Value) -> Vec<Mark> {
        crate::marks_from_data(&serde_json::json!({ "marks": v }))
    }

    /// Flatten ops to something readable and order-independent per op.
    fn flat(ops: Vec<(u32, u32, Attrs)>) -> Vec<(u32, u32, Vec<(String, String)>)> {
        ops.into_iter()
            .map(|(s, l, a)| {
                let mut kv: Vec<(String, String)> = a
                    .iter()
                    .map(|(k, v)| (k.to_string(), format!("{v:?}")))
                    .collect();
                kv.sort();
                (s, l, kv)
            })
            .collect()
    }

    /// **The same table exists in Dart** (`test/marks_diff_test.dart`). Rust is
    /// the authority and Dart is the mirror; a case added here belongs there.

    #[test]
    fn an_unchanged_block_emits_nothing() {
        let runs = vec![(5, Some(attrs(&[("bold", Any::Bool(true))]))), (6, None)];
        let ops = marks_diff_format_ops(&runs, &target(serde_json::json!([
            {"start": 0, "end": 5, "type": "bold"}
        ])));
        assert!(ops.is_empty(), "{ops:?}");
    }

    #[test]
    fn adding_a_mark_touches_only_its_own_range() {
        let runs = vec![(5, Some(attrs(&[("bold", Any::Bool(true))]))), (6, None)];
        let ops = marks_diff_format_ops(&runs, &target(serde_json::json!([
            {"start": 0, "end": 5, "type": "bold"},
            {"start": 6, "end": 11, "type": "italic"},
        ])));
        assert_eq!(
            flat(ops),
            vec![(6, 5, vec![("italic".into(), "Bool(true)".into())])]
        );
    }

    #[test]
    fn removing_a_mark_is_a_null_over_just_that_range() {
        let runs = vec![(11, Some(attrs(&[("bold", Any::Bool(true))])))];
        let ops = marks_diff_format_ops(&runs, &target(serde_json::json!([
            {"start": 6, "end": 11, "type": "bold"}
        ])));
        assert_eq!(
            flat(ops),
            vec![(0, 6, vec![("bold".into(), "Null".into())])],
            "only the un-bolded half is written"
        );
    }

    #[test]
    fn one_op_spans_runs_that_need_the_same_delta() {
        // Three runs, none bold; bolding all of it is ONE op, not three.
        let runs = vec![(3, None), (4, None), (4, None)];
        let ops = marks_diff_format_ops(&runs, &target(serde_json::json!([
            {"start": 0, "end": 11, "type": "bold"}
        ])));
        assert_eq!(
            flat(ops),
            vec![(0, 11, vec![("bold".into(), "Bool(true)".into())])]
        );
    }

    #[test]
    fn a_changed_href_rewrites_the_link_attribute() {
        let runs = vec![(
            5,
            Some(attrs(&[(
                "link",
                Any::Map(Arc::new(
                    [("href".to_string(), Any::String(Arc::from("http://a")))]
                        .into_iter()
                        .collect(),
                )),
            )])),
        )];
        let ops = marks_diff_format_ops(&runs, &target(serde_json::json!([
            {"start": 0, "end": 5, "type": "link", "href": "http://b"}
        ])));
        assert_eq!(ops.len(), 1, "{ops:?}");
        assert_eq!((ops[0].0, ops[0].1), (0, 5));
    }

    #[test]
    fn an_empty_text_emits_nothing() {
        assert!(marks_diff_format_ops(&[], &target(serde_json::json!([
            {"start": 0, "end": 5, "type": "bold"}
        ]))).is_empty());
    }
}
