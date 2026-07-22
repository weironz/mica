//! Structured page properties, parsed from a document's YAML front matter.
//!
//! # Where this sits in the data model
//!
//! Front matter is stored verbatim as the raw inner string on the document
//! root block's `data["front_matter"]` (see `split_front_matter` / the writer in
//! `lib.rs`). That raw string stays the SOLE authority — there is no second
//! table, no parsed copy that has to be kept in sync. This module is a *lazy
//! structured view over that string*: `parse_properties` reads it for display,
//! and `upsert_property` / `remove_property` edit it with SURGICAL text edits
//! that leave every untouched key (and its comments, order, quoting) byte-exact.
//!
//! # Round-trip invariant (deliberately relaxed — see docs/page-properties.md)
//!
//! Storing front matter as an opaque string gives byte-exact round-trip. The
//! moment we let a user structurally edit a property we cannot keep that: a
//! property panel that adds `tags: [a, b]` MUST re-emit YAML for that key. So the
//! invariant for the EDITED key relaxes from "byte-exact" to "normalized subset"
//! — the same principle mica's body markdown already uses (output is a
//! normalized subset, round-trip is a subset invariant). Surgical write-back
//! confines the normalization to the one key that actually changed; comparable
//! products (Obsidian) reserialize the WHOLE block and lose comments/order — we
//! do not. The invariant we hold and test:
//!   * editing key A never alters key B's raw bytes;
//!   * `parse` ∘ `render` is stable: re-emitting a parsed value and parsing it
//!     back yields the same [`PropertyValue`].
//!
//! # Scope
//!
//! The flat subset real front-matter properties use (what Obsidian calls
//! Properties): top-level `key: scalar`, flow lists `key: [a, b]`, and block
//! lists (`key:` then indented `- item` lines). Anything richer (nested maps,
//! multi-line block scalars, anchors) is left untouched and simply not surfaced
//! as an editable property — it round-trips as the opaque bytes it already was.
//! `tags` is not a distinct type: it is a `List`-valued property whose items the
//! caller feeds into the same page-reference index as body `[[` links.

use serde::{Deserialize, Serialize};

/// A typed front-matter value. The set is intentionally small and closed —
/// inferred from the YAML scalar shape, no per-key schema (matches Obsidian's
/// Text / Number / Checkbox / Date / List). `tags` is just a `List`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum PropertyValue {
    Text(String),
    Number(f64),
    Checkbox(bool),
    /// A `YYYY-MM-DD` date (shape-validated on parse; stored/rendered as-is).
    Date(String),
    List(Vec<String>),
}

/// One top-level front-matter key and its typed value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Property {
    pub key: String,
    pub value: PropertyValue,
}

/// Parse the flat, editable subset of a front-matter string into typed
/// properties, in source order. Best-effort: comments, blank lines, and any
/// structure this subset doesn't cover are skipped (they are preserved by the
/// writers, just not surfaced here). The input is the RAW INNER front matter
/// (no `---` fences), exactly as stored on `data["front_matter"]`.
pub fn parse_properties(front_matter: &str) -> Vec<Property> {
    let lines: Vec<&str> = front_matter.split('\n').collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let raw = lines[i];
        // Top-level keys are unindented; indented lines belong to the key above
        // (block-list items), and are consumed there — encountering one here
        // means it had no owning key, so skip it.
        if raw.starts_with(' ') || raw.starts_with('\t') {
            i += 1;
            continue;
        }
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            i += 1;
            continue;
        }
        let Some((key, rest)) = split_key(raw) else {
            i += 1;
            continue;
        };
        let value_text = rest.trim();
        if value_text.is_empty() {
            // A bare `key:` — scan the indented continuation block. All `- item`
            // lines → a block list; anything else indented (nested map, block
            // scalar) → out of the flat subset, so the key is unsurfaced but its
            // whole block is stepped over. No indented block → an empty value.
            let mut items = Vec::new();
            let mut all_list_items = true;
            let mut saw_indented = false;
            let mut j = i + 1;
            while j < lines.len() {
                let cont = lines[j];
                if !(cont.starts_with(' ') || cont.starts_with('\t')) {
                    break;
                }
                let ct = cont.trim();
                if ct.is_empty() {
                    break;
                }
                saw_indented = true;
                if let Some(item) = ct.strip_prefix('-') {
                    items.push(unquote_scalar(item.trim()));
                } else {
                    all_list_items = false;
                }
                j += 1;
            }
            if saw_indented {
                if all_list_items {
                    out.push(Property {
                        key,
                        value: PropertyValue::List(items),
                    });
                }
                // else: nested/complex map — left unsurfaced, preserved as bytes.
                i = j;
                continue;
            }
            // Bare key, no indented block → empty Text.
            out.push(Property {
                key,
                value: PropertyValue::Text(String::new()),
            });
            i += 1;
            continue;
        }
        out.push(Property {
            key,
            value: infer_scalar(value_text),
        });
        i += 1;
    }
    out
}

/// Infer a typed value from a user's raw single-line input (what a property
/// editor commits): empty → empty text, otherwise the same bool/number/date/
/// list/text inference `parse_properties` uses. The Dart mirror's `inferValue`
/// must agree with this.
pub fn infer_value(raw: &str) -> PropertyValue {
    let t = raw.trim();
    if t.is_empty() {
        return PropertyValue::Text(String::new());
    }
    infer_scalar(t)
}

/// Insert or replace `key`'s value, editing only that key's line(s) and leaving
/// the rest of the front matter byte-exact. A new key is appended (with a
/// trailing newline discipline that matches the existing content). Returns the
/// new raw front-matter string.
pub fn upsert_property(front_matter: &str, key: &str, value: &PropertyValue) -> String {
    let lines: Vec<&str> = front_matter.split('\n').collect();
    // Find the key's line and the extent of its (possibly multi-line) value.
    if let Some((start, end)) = key_span(&lines, key) {
        // Preserve list *style*: if the existing entry was a block list, re-emit
        // as a block list; otherwise flow. Non-list edits are single-line.
        let was_block_list = end > start + 1;
        let mut rendered = render_property(key, value, was_block_list);
        let mut new_lines: Vec<String> = Vec::with_capacity(lines.len());
        new_lines.extend(lines[..start].iter().map(|s| s.to_string()));
        new_lines.append(&mut rendered);
        new_lines.extend(lines[end..].iter().map(|s| s.to_string()));
        return new_lines.join("\n");
    }
    // Append a new key. `false` → new lists render in flow style.
    let rendered = render_property(key, value, false).join("\n");
    if front_matter.is_empty() {
        rendered
    } else if front_matter.ends_with('\n') {
        format!("{front_matter}{rendered}")
    } else {
        format!("{front_matter}\n{rendered}")
    }
}

/// Remove `key` and its value lines, leaving the rest byte-exact. Unknown key →
/// unchanged. Returns the new raw front-matter string.
pub fn remove_property(front_matter: &str, key: &str) -> String {
    let lines: Vec<&str> = front_matter.split('\n').collect();
    let Some((start, end)) = key_span(&lines, key) else {
        return front_matter.to_string();
    };
    let mut kept: Vec<&str> = Vec::with_capacity(lines.len());
    kept.extend_from_slice(&lines[..start]);
    kept.extend_from_slice(&lines[end..]);
    kept.join("\n")
}

/// The `[start, end)` line range a top-level `key` owns: its `key:` line plus any
/// immediately-following indented continuation lines (block-list items). `None`
/// if the key is absent.
fn key_span(lines: &[&str], key: &str) -> Option<(usize, usize)> {
    for (idx, raw) in lines.iter().enumerate() {
        if raw.starts_with(' ') || raw.starts_with('\t') {
            continue;
        }
        if let Some((k, _)) = split_key(raw) {
            if k == key {
                let mut end = idx + 1;
                while end < lines.len()
                    && (lines[end].starts_with(' ') || lines[end].starts_with('\t'))
                    && !lines[end].trim().is_empty()
                {
                    end += 1;
                }
                return Some((idx, end));
            }
        }
    }
    None
}

/// Render `key: value` as one or more YAML lines. `block_list` picks block vs
/// flow style for a `List` (single-line values ignore it).
fn render_property(key: &str, value: &PropertyValue, block_list: bool) -> Vec<String> {
    match value {
        PropertyValue::Text(s) => vec![format!("{key}: {}", quote_if_needed(s))],
        PropertyValue::Number(n) => vec![format!("{key}: {}", format_number(*n))],
        PropertyValue::Checkbox(b) => vec![format!("{key}: {b}")],
        PropertyValue::Date(s) => vec![format!("{key}: {s}")],
        PropertyValue::List(items) => {
            if items.is_empty() {
                vec![format!("{key}: []")]
            } else if block_list {
                let mut out = vec![format!("{key}:")];
                out.extend(items.iter().map(|it| format!("  - {}", quote_if_needed(it))));
                out
            } else {
                let inner = items
                    .iter()
                    .map(|it| quote_if_needed(it))
                    .collect::<Vec<_>>()
                    .join(", ");
                vec![format!("{key}: [{inner}]")]
            }
        }
    }
}

/// Split `key: rest` at the first top-level colon. The key must be a plain
/// scalar (no spaces, not quoted) for the flat subset; returns `None` otherwise.
fn split_key(raw: &str) -> Option<(String, &str)> {
    let colon = raw.find(':')?;
    let key = &raw[..colon];
    if key.is_empty() || key.contains(char::is_whitespace) || key.contains(['"', '\'', '#']) {
        return None;
    }
    Some((key.to_string(), &raw[colon + 1..]))
}

/// Infer a typed value from a single-line scalar (already trimmed, non-empty).
/// Precedence: bool → number → date → flow-list → text.
fn infer_scalar(text: &str) -> PropertyValue {
    match text {
        "true" | "True" | "TRUE" => return PropertyValue::Checkbox(true),
        "false" | "False" | "FALSE" => return PropertyValue::Checkbox(false),
        _ => {}
    }
    // A flow list `[a, b, c]` (possibly empty `[]`).
    if let Some(inner) = text.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        let items = if inner.trim().is_empty() {
            Vec::new()
        } else {
            split_flow_items(inner)
        };
        return PropertyValue::List(items);
    }
    // Numbers: only unquoted, finite, and byte-identical to their reformat (so
    // `007` or `1.0` stay Text — reformatting them would lose the source shape,
    // breaking the parse∘render stability we test).
    if !text.starts_with(['"', '\'']) {
        if let Ok(n) = text.parse::<f64>() {
            if n.is_finite() && format_number(n) == text {
                return PropertyValue::Number(n);
            }
        }
    }
    if is_iso_date(text) {
        return PropertyValue::Date(text.to_string());
    }
    PropertyValue::Text(unquote_scalar(text))
}

/// `YYYY-MM-DD`, digits and month/day in range. Shape only — not a calendar.
fn is_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    if !b
        .iter()
        .enumerate()
        .all(|(i, &c)| i == 4 || i == 7 || c.is_ascii_digit())
    {
        return false;
    }
    let month = &s[5..7];
    let day = &s[8..10];
    ("01"..="12").contains(&month) && ("01"..="31").contains(&day)
}

/// Split a flow-list body (`a, "b, c", d` — the text inside `[...]`) on
/// top-level commas, respecting `"…"` / `'…'` so a quoted item containing a
/// comma stays one item. Each item is then unquoted.
fn split_flow_items(inner: &str) -> Vec<String> {
    let mut items: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_double = false;
    let mut in_single = false;
    let mut escaped = false;
    for c in inner.chars() {
        if in_double {
            cur.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_double = false;
            }
        } else if in_single {
            cur.push(c);
            if c == '\'' {
                in_single = false;
            }
        } else {
            match c {
                '"' => {
                    in_double = true;
                    cur.push(c);
                }
                '\'' => {
                    in_single = true;
                    cur.push(c);
                }
                ',' => {
                    items.push(std::mem::take(&mut cur));
                }
                _ => cur.push(c),
            }
        }
    }
    items.push(cur);
    items.iter().map(|s| unquote_scalar(s.trim())).collect()
}

/// Strip one layer of matching quotes from a scalar, unescaping a double-quoted
/// string's `\"` and `\\`. A bare scalar returns as-is.
fn unquote_scalar(s: &str) -> String {
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        let inner = &s[1..s.len() - 1];
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('"') => out.push('"'),
                    Some('\\') => out.push('\\'),
                    Some('n') => out.push('\n'),
                    Some(other) => {
                        out.push('\\');
                        out.push(other);
                    }
                    None => out.push('\\'),
                }
            } else {
                out.push(c);
            }
        }
        out
    } else if s.len() >= 2 && s.starts_with('\'') && s.ends_with('\'') {
        // YAML single-quote: `''` is a literal `'`.
        s[1..s.len() - 1].replace("''", "'")
    } else {
        s.to_string()
    }
}

/// Double-quote a string when leaving it bare would change its meaning on
/// re-parse (empty, whitespace edges, a leading indicator char, an embedded
/// `: ` / `, ` / `]`, or a value that would infer as bool/number/date/list).
/// Conservative: quoting is always safe, so quote whenever in doubt.
fn quote_if_needed(s: &str) -> String {
    let needs = s.is_empty()
        || s.starts_with([' ', '\t'])
        || s.ends_with([' ', '\t'])
        || s.starts_with([
            '[', ']', '{', '}', '#', '&', '*', '!', '|', '>', '%', '@', '`', '"', '\'', '-', '?',
            ':', ',',
        ])
        || s.contains(": ")
        || s.contains(", ")
        || s.contains(['\n', ']'])
        || !matches!(infer_scalar(s), PropertyValue::Text(ref t) if t == s);
    if needs {
        let escaped = s
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n");
        format!("\"{escaped}\"")
    } else {
        s.to_string()
    }
}

/// Format an `f64` without a trailing `.0` for integers (so `3.0` → `3`), so a
/// whole number round-trips as the integer literal a user typed.
fn format_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> PropertyValue {
        PropertyValue::Text(s.to_string())
    }
    fn list(items: &[&str]) -> PropertyValue {
        PropertyValue::List(items.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn parses_the_flat_typed_subset() {
        let fm = "title: My Page\n\
                  tags: [work, urgent]\n\
                  count: 3\n\
                  ratio: 1.5\n\
                  done: true\n\
                  due: 2026-07-22\n\
                  authors:\n  - Alice\n  - Bob";
        let props = parse_properties(fm);
        assert_eq!(
            props,
            vec![
                Property { key: "title".into(), value: text("My Page") },
                Property { key: "tags".into(), value: list(&["work", "urgent"]) },
                Property { key: "count".into(), value: PropertyValue::Number(3.0) },
                Property { key: "ratio".into(), value: PropertyValue::Number(1.5) },
                Property { key: "done".into(), value: PropertyValue::Checkbox(true) },
                Property { key: "due".into(), value: PropertyValue::Date("2026-07-22".into()) },
                Property { key: "authors".into(), value: list(&["Alice", "Bob"]) },
            ]
        );
    }

    #[test]
    fn comments_blanks_and_unknown_structure_are_skipped_not_surfaced() {
        let fm = "# a comment\n\
                  title: Hi\n\
                  \n\
                  nested:\n  child: 1\n\
                  after: ok";
        let props = parse_properties(fm);
        // `nested` has an indented non-list child → out of subset, unsurfaced.
        assert_eq!(
            props,
            vec![
                Property { key: "title".into(), value: text("Hi") },
                Property { key: "after".into(), value: text("ok") },
            ]
        );
    }

    #[test]
    fn ambiguous_scalars_stay_text_to_keep_parse_render_stable() {
        // Leading-zero and `1.0` would lose their shape if treated as Number.
        assert_eq!(infer_scalar("007"), text("007"));
        assert_eq!(infer_scalar("1.0"), text("1.0"));
        // A quoted number is text.
        assert_eq!(infer_scalar("\"3\""), text("3"));
    }

    #[test]
    fn upsert_edits_only_the_target_key_leaving_others_byte_exact() {
        let fm = "# keep me\n\
                  title: Old  # trailing comment kept\n\
                  tags: [a, b]\n\
                  note: 'single quoted'";
        let out = upsert_property(fm, "tags", &list(&["a", "b", "c"]));
        // Only the tags line changed; the comment line, title (with its inline
        // comment), and the single-quoted note are untouched byte-for-byte.
        assert_eq!(
            out,
            "# keep me\n\
             title: Old  # trailing comment kept\n\
             tags: [a, b, c]\n\
             note: 'single quoted'"
        );
    }

    #[test]
    fn upsert_preserves_block_list_style() {
        let fm = "authors:\n  - Alice\ntitle: X";
        let out = upsert_property(fm, "authors", &list(&["Alice", "Bob"]));
        assert_eq!(out, "authors:\n  - Alice\n  - Bob\ntitle: X");
    }

    #[test]
    fn upsert_appends_a_new_key() {
        assert_eq!(
            upsert_property("title: X", "done", &PropertyValue::Checkbox(true)),
            "title: X\ndone: true"
        );
        assert_eq!(upsert_property("", "title", &text("First")), "title: First");
        assert_eq!(
            upsert_property("a: 1\n", "b", &PropertyValue::Number(2.0)),
            "a: 1\nb: 2"
        );
    }

    #[test]
    fn remove_deletes_the_key_and_its_block_leaving_the_rest() {
        let fm = "title: X\nauthors:\n  - Alice\n  - Bob\ndone: true";
        assert_eq!(remove_property(fm, "authors"), "title: X\ndone: true");
        assert_eq!(remove_property(fm, "missing"), fm);
    }

    #[test]
    fn parse_then_render_is_stable_for_every_type() {
        // Re-emitting a parsed value and parsing it back yields the same value —
        // the normalized-subset round-trip invariant (per type).
        let cases = vec![
            ("s", text("hello world")),
            ("s", text("")),
            ("s", text("needs: quoting")),
            ("s", text("true")), // string that looks like a bool → must re-parse as text
            ("s", text("42")),   // string that looks like a number → text
            ("n", PropertyValue::Number(3.0)),
            ("n", PropertyValue::Number(-2.5)),
            ("b", PropertyValue::Checkbox(false)),
            ("d", PropertyValue::Date("2026-01-09".into())),
            ("l", list(&["x", "y, z", "true"])),
            ("l", list(&[])),
        ];
        for (key, value) in cases {
            let rendered = render_property(key, &value, false).join("\n");
            let reparsed = parse_properties(&rendered);
            assert_eq!(
                reparsed,
                vec![Property { key: key.into(), value: value.clone() }],
                "parse∘render unstable for {value:?} → rendered {rendered:?}"
            );
        }
    }

    #[test]
    fn tags_are_just_a_list_valued_property() {
        let props = parse_properties("tags: [rust, crdt]");
        assert_eq!(
            props,
            vec![Property { key: "tags".into(), value: list(&["rust", "crdt"]) }]
        );
    }
}
