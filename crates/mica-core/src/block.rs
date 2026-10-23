//! The flat block DTO — a re-export of `mica_markdown::Block`, the single
//! definition, so the yrs document model and the markdown/REST/interchange
//! model cannot drift apart (they were field-identical mirrors before, with
//! hand-written conversions at every boundary).

pub use mica_markdown::Block;

/// The `mica://page/<viewId>` targets referenced by a block's link marks.
///
/// The single definition of "what counts as a page link", for BOTH worlds: the
/// cloud derives `document_yrs_base.link_targets` from it and the local store
/// derives `doc_snapshot.link_targets` from it. It lived in api-server first,
/// then app-core; each move was the same discovery — a copy on the other side is
/// a second answer to a question that must have one, and the two indexes would
/// disagree about what a backlink is.
pub fn page_link_targets(data: &serde_json::Value) -> Vec<String> {
  const SCHEME: &str = "mica://page/";
  let Some(marks) = data.get("marks").and_then(|m| m.as_array()) else {
    return Vec::new();
  };
  marks
    .iter()
    .filter_map(|mark| {
      mark
        .get("href")
        .and_then(|h| h.as_str())
        .and_then(|href| href.strip_prefix(SCHEME))
        .map(str::to_string)
    })
    .collect()
}
