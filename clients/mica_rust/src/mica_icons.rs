//! Crisp 1.5px stroke inline SVG icon set matching Mica's minimal Notion-like
//! visual language (sidebar, tab strip, formatting toolbar, and page actions).

use nsh_core::{
  element::{ImageSrc, svg},
  geometry::px,
  prelude::*,
};

fn stroke_icon(size: f32, color: &str, body: &str) -> impl IntoElement {
  let markup = format!(
    r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="{color}" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">{body}</svg>"#
  );
  svg(ImageSrc::svg_markup(&markup))
    .w(px(size))
    .h(px(size))
}

/// Signature teal Mica "M" app mark (top-left of the shell title bar).
pub fn mica_logo(color: &str) -> impl IntoElement {
  let markup = format!(
    r##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><rect x="2" y="2" width="20" height="20" rx="5" fill="{color}"/><path d="M6.5 16.5V7.5L12 13L17.5 7.5V16.5" stroke="#ffffff" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round"/></svg>"##
  );
  svg(ImageSrc::svg_markup(&markup)).w(px(18.0)).h(px(18.0))
}

pub fn sidebar_toggle(color: &str) -> impl IntoElement {
  stroke_icon(
    15.0,
    color,
    r#"<rect x="3" y="4" width="18" height="16" rx="2"/><line x1="9" y1="4" x2="9" y2="20"/>"#,
  )
}

pub fn chevron_left(color: &str) -> impl IntoElement {
  stroke_icon(14.0, color, r#"<polyline points="15 18 9 12 15 6"/>"#)
}

pub fn chevron_right(color: &str) -> impl IntoElement {
  stroke_icon(14.0, color, r#"<polyline points="9 18 15 12 9 6"/>"#)
}

pub fn chevron_down(color: &str) -> impl IntoElement {
  stroke_icon(12.0, color, r#"<polyline points="6 9 12 15 18 9"/>"#)
}

pub fn plus(color: &str) -> impl IntoElement {
  stroke_icon(
    14.0,
    color,
    r#"<line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/>"#,
  )
}

pub fn close_sm(color: &str) -> impl IntoElement {
  stroke_icon(
    11.0,
    color,
    r#"<line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>"#,
  )
}

pub fn cloud(color: &str) -> impl IntoElement {
  stroke_icon(
    15.0,
    color,
    r#"<path d="M17.5 19H9a7 7 0 1 1 6.71-9h1.79a4.5 4.5 0 1 1 0 9Z"/>"#,
  )
}

pub fn chevrons_up_down(color: &str) -> impl IntoElement {
  stroke_icon(
    12.0,
    color,
    r#"<path d="m7 15 5 5 5-5"/><path d="m7 9 5-5 5 5"/>"#,
  )
}

pub fn search(color: &str) -> impl IntoElement {
  stroke_icon(
    14.0,
    color,
    r#"<circle cx="11" cy="11" r="7"/><line x1="20" y1="20" x2="16.65" y2="16.65"/>"#,
  )
}

pub fn folder(color: &str) -> impl IntoElement {
  stroke_icon(
    15.0,
    color,
    r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>"#,
  )
}

pub fn folder_plus(color: &str) -> impl IntoElement {
  stroke_icon(
    14.0,
    color,
    r#"<path d="M12 10v6"/><path d="M9 13h6"/><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>"#,
  )
}

pub fn file_doc(color: &str) -> impl IntoElement {
  stroke_icon(
    15.0,
    color,
    r#"<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><polyline points="14 2 14 8 20 8"/><line x1="8" y1="13" x2="16" y2="13"/><line x1="8" y1="17" x2="13" y2="17"/>"#,
  )
}

pub fn settings_gear(color: &str) -> impl IntoElement {
  stroke_icon(
    15.0,
    color,
    r#"<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9c.26.6.85 1 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1Z"/>"#,
  )
}

pub fn graph_nodes(color: &str) -> impl IntoElement {
  stroke_icon(
    15.0,
    color,
    r#"<circle cx="18" cy="5" r="2.5"/><circle cx="6" cy="12" r="2.5"/><circle cx="18" cy="19" r="2.5"/><line x1="8.3" y1="10.8" x2="15.7" y2="6.2"/><line x1="8.3" y1="13.2" x2="15.7" y2="17.8"/>"#,
  )
}

pub fn whiteboard(color: &str) -> impl IntoElement {
  stroke_icon(
    15.0,
    color,
    r#"<rect x="3" y="4" width="18" height="14" rx="2"/><path d="M8 12l2.5-2.5L13 12l3-3"/><line x1="8" y1="21" x2="16" y2="21"/><line x1="12" y1="18" x2="12" y2="21"/>"#,
  )
}

pub fn undo(color: &str) -> impl IntoElement {
  stroke_icon(
    15.0,
    color,
    r#"<path d="M3 7v6h6"/><path d="M21 17a9 9 0 0 0-9-9 9 9 0 0 0-6 2.3L3 13"/>"#,
  )
}

pub fn redo(color: &str) -> impl IntoElement {
  stroke_icon(
    15.0,
    color,
    r#"<path d="M21 7v6h-6"/><path d="M3 17a9 9 0 0 1 9-9 9 9 0 0 1 6 2.3L21 13"/>"#,
  )
}

pub fn align_text(color: &str) -> impl IntoElement {
  stroke_icon(
    15.0,
    color,
    r#"<line x1="4" y1="6" x2="20" y2="6"/><line x1="4" y1="12" x2="16" y2="12"/><line x1="4" y1="18" x2="20" y2="18"/>"#,
  )
}

pub fn heading_box(num: &str, color: &str) -> impl IntoElement {
  let markup = format!(
    r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none"><rect x="4" y="4" width="16" height="16" rx="3" stroke="{color}" stroke-width="1.6"/><text x="12" y="16" text-anchor="middle" font-family="Segoe UI, sans-serif" font-size="11" font-weight="600" fill="{color}">{num}</text></svg>"#
  );
  svg(ImageSrc::svg_markup(&markup)).w(px(16.0)).h(px(16.0))
}

pub fn bold_icon(color: &str) -> impl IntoElement {
  stroke_icon(
    14.0,
    color,
    r#"<path d="M6 4h8a4 4 0 0 1 4 4 4 4 0 0 1-4 4H6z"/><path d="M6 12h9a4 4 0 0 1 4 4 4 4 0 0 1-4 4H6z"/>"#,
  )
}

pub fn italic_icon(color: &str) -> impl IntoElement {
  stroke_icon(
    14.0,
    color,
    r#"<line x1="19" y1="4" x2="10" y2="4"/><line x1="14" y1="20" x2="5" y2="20"/><line x1="15" y1="4" x2="9" y2="20"/>"#,
  )
}

pub fn code_icon(color: &str) -> impl IntoElement {
  stroke_icon(
    15.0,
    color,
    r#"<polyline points="16 18 22 12 16 6"/><polyline points="8 6 2 12 8 18"/>"#,
  )
}

pub fn list_bullet(color: &str) -> impl IntoElement {
  stroke_icon(
    15.0,
    color,
    r#"<line x1="9" y1="6" x2="20" y2="6"/><line x1="9" y1="12" x2="20" y2="12"/><line x1="9" y1="18" x2="20" y2="18"/><circle cx="4.5" cy="6" r="1"/><circle cx="4.5" cy="12" r="1"/><circle cx="4.5" cy="18" r="1"/>"#,
  )
}

pub fn todo_box(color: &str) -> impl IntoElement {
  stroke_icon(
    15.0,
    color,
    r#"<rect x="4" y="4" width="16" height="16" rx="3"/><polyline points="8.5 12 11 14.5 15.5 9.5"/>"#,
  )
}

pub fn quote_icon(color: &str) -> impl IntoElement {
  stroke_icon(
    15.0,
    color,
    r#"<path d="M10 11H6a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h3a1 1 0 0 1 1 1v6c0 2.667-1.333 4.333-4 5"/><path d="M19 11h-4a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h3a1 1 0 0 1 1 1v6c0 2.667-1.333 4.333-4 5"/>"#,
  )
}

pub fn divider_icon(color: &str) -> impl IntoElement {
  stroke_icon(15.0, color, r#"<line x1="4" y1="12" x2="20" y2="12"/>"#)
}

pub fn table_icon(color: &str) -> impl IntoElement {
  stroke_icon(
    15.0,
    color,
    r#"<rect x="3" y="4" width="18" height="16" rx="2"/><line x1="3" y1="10" x2="21" y2="10"/><line x1="9" y1="4" x2="9" y2="20"/><line x1="15" y1="4" x2="15" y2="20"/>"#,
  )
}

pub fn copy_icon(color: &str) -> impl IntoElement {
  stroke_icon(
    13.0,
    color,
    r#"<rect x="9" y="9" width="12" height="12" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/>"#,
  )
}

pub fn trash_icon(color: &str) -> impl IntoElement {
  stroke_icon(
    14.0,
    color,
    r#"<polyline points="3 6 5 6 21 6"/><path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6"/><path d="M10 11v6"/><path d="M14 11v6"/>"#,
  )
}

pub fn star_icon(color: &str) -> impl IntoElement {
  stroke_icon(
    15.0,
    color,
    r#"<polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2"/>"#,
  )
}

pub fn link_icon(color: &str) -> impl IntoElement {
  stroke_icon(
    14.0,
    color,
    r#"<path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/>"#,
  )
}

pub fn more_dots(color: &str) -> impl IntoElement {
  stroke_icon(
    15.0,
    color,
    r#"<circle cx="5" cy="12" r="1.2"/><circle cx="12" cy="12" r="1.2"/><circle cx="19" cy="12" r="1.2"/>"#,
  )
}
