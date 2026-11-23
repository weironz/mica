//! Crisp 1.5px stroke inline SVG icons + the real repo Mica brand mark
//! (`assets/mica-logo.svg`, blue-purple folded sheets).
//!
//! All glyphs use pure vector `<path>` / `<rect>` / `<circle>` / `<line>` primitives
//! (zero `<text>` nodes) so `resvg` renders razor-sharp icons identically across
//! Windows, macOS, Linux, and WebAssembly with zero system-font dependencies.

use nsh_core::{
  element::{ImageSrc, Svg, svg},
  geometry::px,
  prelude::*,
};

const MICA_LOGO_SVG: &str = include_str!("../../../assets/mica-logo.svg");

fn icon_svg(size_px: f32, body: String) -> Svg {
  svg(ImageSrc::svg_markup(&body)).w(px(size_px)).h(px(size_px))
}

fn stroked(size_px: f32, color: &str, inner: &str) -> Svg {
  let doc = format!(
    r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="none" stroke="{color}" stroke-width="1.65" stroke-linecap="round" stroke-linejoin="round">{inner}</svg>"##
  );
  icon_svg(size_px, doc)
}

/// Official Mica brand logo embedded directly from `assets/mica-logo.svg`.
pub fn mica_logo() -> Svg {
  icon_svg(20.0, MICA_LOGO_SVG.to_string())
}

pub fn sidebar_toggle(color: &str) -> Svg {
  stroked(
    16.0,
    color,
    r##"<rect x="2.5" y="3.5" width="15" height="13" rx="2"/><line x1="7.5" y1="3.5" x2="7.5" y2="16.5"/>"##,
  )
}

pub fn sidebar_toggle_right(color: &str) -> Svg {
  stroked(
    16.0,
    color,
    r##"<rect x="2.5" y="3.5" width="15" height="13" rx="2"/><line x1="12.5" y1="3.5" x2="12.5" y2="16.5"/>"##,
  )
}

pub fn chevron_left(color: &str) -> Svg {
  stroked(14.0, color, r##"<path d="M12 5L7 10L12 15"/>"##)
}

pub fn chevron_right(color: &str) -> Svg {
  stroked(14.0, color, r##"<path d="M8 5L13 10L8 15"/>"##)
}

pub fn chevron_down(color: &str) -> Svg {
  stroked(12.0, color, r##"<path d="M5 7.5L10 12.5L15 7.5"/>"##)
}

pub fn plus(color: &str) -> Svg {
  stroked(15.0, color, r##"<path d="M10 4.5V15.5M4.5 10H15.5"/>"##)
}

pub fn close_sm(color: &str) -> Svg {
  stroked(13.0, color, r##"<path d="M5.5 5.5L14.5 14.5M14.5 5.5L5.5 14.5"/>"##)
}

pub fn check_icon(color: &str) -> Svg {
  stroked(14.0, color, r##"<path d="M4.5 10.5L8 14L15.5 6.5"/>"##)
}

pub fn cloud(color: &str) -> Svg {
  stroked(
    16.0,
    color,
    r##"<path d="M6.5 15H14.5A3 3 0 0 0 15 9.1A4.5 4.5 0 0 0 6.3 8A3.2 3.2 0 0 0 6.5 15Z"/>"##,
  )
}

pub fn chevrons_up_down(color: &str) -> Svg {
  stroked(13.0, color, r##"<path d="M7 8L10 5L13 8M7 12L10 15L13 12"/>"##)
}

pub fn search(color: &str) -> Svg {
  stroked(
    14.0,
    color,
    r##"<circle cx="8.8" cy="8.8" r="5.3"/><path d="M13 13L16.5 16.5"/>"##,
  )
}

pub fn folder(color: &str) -> Svg {
  stroked(
    15.0,
    color,
    r##"<path d="M2.5 5.5A1.5 1.5 0 0 1 4 4H7.6L9.2 6H16A1.5 1.5 0 0 1 17.5 7.5V14.5A1.5 1.5 0 0 1 16 16H4A1.5 1.5 0 0 1 2.5 14.5Z"/>"##,
  )
}

pub fn folder_plus(color: &str) -> Svg {
  stroked(
    15.0,
    color,
    r##"<path d="M2.5 5.5A1.5 1.5 0 0 1 4 4H7.6L9.2 6H16A1.5 1.5 0 0 1 17.5 7.5V14.5A1.5 1.5 0 0 1 16 16H4A1.5 1.5 0 0 1 2.5 14.5Z"/><path d="M10 9V13M8 11H12"/>"##,
  )
}

pub fn file_doc(color: &str) -> Svg {
  stroked(
    15.0,
    color,
    r##"<path d="M5 3.5H11.5L15 7V16.5H5Z"/><path d="M11.5 3.5V7H15"/><path d="M7.5 10.5H12.5M7.5 13.5H11"/>"##,
  )
}

pub fn settings_gear(color: &str) -> Svg {
  stroked(
    16.0,
    color,
    r##"<circle cx="10" cy="10" r="2.7"/><path d="M10 2.8V4.6M10 15.4V17.2M2.8 10H4.6M15.4 10H17.2M4.9 4.9L6.2 6.2M13.8 13.8L15.1 15.1M15.1 4.9L13.8 6.2M6.2 13.8L4.9 15.1"/>"##,
  )
}

pub fn graph_nodes(color: &str) -> Svg {
  stroked(
    15.0,
    color,
    r##"<circle cx="10" cy="5" r="2"/><circle cx="5" cy="14.5" r="2"/><circle cx="15" cy="14.5" r="2"/><path d="M8.9 6.8L6.1 12.7M11.1 6.8L13.9 12.7M7 14.5H13"/>"##,
  )
}

pub fn whiteboard(color: &str) -> Svg {
  stroked(
    17.0,
    color,
    r##"<rect x="3" y="4" width="14" height="10" rx="1.5"/><path d="M6.5 10.5L9 8L11 10L13.5 7.5M10 14V17M7 17H13"/>"##,
  )
}

pub fn undo(color: &str) -> Svg {
  stroked(17.0, color, r##"<path d="M7 6L3.5 9.5L7 13"/><path d="M4 9.5H12.5A3.5 3.5 0 0 1 16 13A3.5 3.5 0 0 1 12.5 16.5H10"/>"##)
}

pub fn redo(color: &str) -> Svg {
  stroked(17.0, color, r##"<path d="M13 6L16.5 9.5L13 13"/><path d="M16 9.5H7.5A3.5 3.5 0 0 0 4 13A3.5 3.5 0 0 0 7.5 16.5H10"/>"##)
}

pub fn align_text(color: &str) -> Svg {
  stroked(17.0, color, r##"<path d="M3.5 5.5H16.5M3.5 10H16.5M3.5 14.5H11.5"/>"##)
}

/// Pure-vector rounded box with crisp `<path>` digits `1`, `2`, `3` (zero `<text>` dependency).
pub fn heading_box(level: &str, color: &str) -> Svg {
  let glyph = match level {
    "1" => r##"<path d="M8.7 7.6L10.3 6.3V13.7M8.5 13.7H12"/>"##,
    "2" => r##"<path d="M7.8 8.0C7.8 6.8 8.7 6.2 10.0 6.2C11.3 6.2 12.2 6.9 12.2 8.0C12.2 9.0 11.4 9.7 10.1 10.7L7.8 13.6H12.4"/>"##,
    _ => r##"<path d="M7.9 7.5C8.2 6.7 9.0 6.2 10.1 6.2C11.4 6.2 12.2 6.9 12.2 7.9C12.2 8.9 11.4 9.6 10.2 9.8C11.5 10.0 12.4 10.8 12.4 11.9C12.4 13.0 11.4 13.8 10.0 13.8C8.9 13.8 8.1 13.3 7.7 12.5"/>"##,
  };
  stroked(
    17.0,
    color,
    &format!(r##"<rect x="3.2" y="3.2" width="13.6" height="13.6" rx="2.4"/>{glyph}"##),
  )
}

pub fn bold_icon(color: &str) -> Svg {
  stroked(
    17.0,
    color,
    r##"<path stroke-width="2" d="M6.5 4.5H11A2.7 2.7 0 0 1 11 10H6.5ZM6.5 10H11.8A2.8 2.8 0 0 1 11.8 15.5H6.5Z"/>"##,
  )
}

pub fn italic_icon(color: &str) -> Svg {
  stroked(17.0, color, r##"<path d="M8.5 4.5H14.5M5.5 15.5H11.5M11.8 4.5L8.2 15.5"/>"##)
}

pub fn strikethrough_icon(color: &str) -> Svg {
  stroked(
    17.0,
    color,
    r##"<path d="M3.5 10H16.5M13.5 6.8C13.1 5.4 11.8 4.6 10 4.6C8.1 4.6 6.8 5.6 6.8 7.0C6.8 8.2 7.8 8.9 9.5 9.3M10.8 10.7C12.4 11.2 13.4 11.9 13.4 13.2C13.4 14.6 12.0 15.6 10.0 15.6C8.1 15.6 6.8 14.7 6.4 13.4"/>"##,
  )
}

pub fn code_icon(color: &str) -> Svg {
  stroked(17.0, color, r##"<path d="M7.5 6.5L4 10L7.5 13.5M12.5 6.5L16 10L12.5 13.5"/>"##)
}

pub fn list_bullet(color: &str) -> Svg {
  stroked(
    17.0,
    color,
    r##"<path d="M8 6H16.5M8 10H16.5M8 14H16.5"/><circle cx="4.5" cy="6" r="0.9" fill="currentColor"/><circle cx="4.5" cy="10" r="0.9" fill="currentColor"/><circle cx="4.5" cy="14" r="0.9" fill="currentColor"/>"##,
  )
}

pub fn list_ordered(color: &str) -> Svg {
  stroked(
    17.0,
    color,
    r##"<path d="M8.5 6H16.5M8.5 10H16.5M8.5 14H16.5M4.5 5V7.5M3.8 10H5.4L3.8 11.5H5.4M3.8 13.5H5.4"/>"##,
  )
}

pub fn todo_box(color: &str) -> Svg {
  stroked(
    17.0,
    color,
    r##"<rect x="3.5" y="3.5" width="13" height="13" rx="2.5"/><path d="M7 10.2L9.2 12.4L13.5 7.8"/>"##,
  )
}

pub fn quote_icon(color: &str) -> Svg {
  stroked(
    17.0,
    color,
    r##"<path d="M4.5 9.5H8V13.5H4.5ZM12 9.5H15.5V13.5H12ZM8 9.5C8 7.5 6.8 6.2 5.2 5.8M15.5 9.5C15.5 7.5 14.3 6.2 12.7 5.8"/>"##,
  )
}

pub fn divider_icon(color: &str) -> Svg {
  stroked(17.0, color, r##"<path d="M3.5 10H16.5"/>"##)
}

pub fn table_icon(color: &str) -> Svg {
  stroked(
    17.0,
    color,
    r##"<rect x="3.5" y="4" width="13" height="12" rx="1.8"/><path d="M3.5 8.5H16.5M9.5 4V16"/>"##,
  )
}

pub fn copy_icon(color: &str) -> Svg {
  stroked(
    16.0,
    color,
    r##"<rect x="7" y="7" width="9" height="9" rx="1.5"/><path d="M13 7V5.5A1.5 1.5 0 0 0 11.5 4H5.5A1.5 1.5 0 0 0 4 5.5V11.5A1.5 1.5 0 0 0 5.5 13H7"/>"##,
  )
}

pub fn link_icon(color: &str) -> Svg {
  stroked(
    17.0,
    color,
    r##"<path d="M8.5 11.5L11.5 8.5M7 13.5L5.5 15A2.8 2.8 0 0 1 1.5 11L4.5 8A2.8 2.8 0 0 1 8.5 8M13 6.5L14.5 5A2.8 2.8 0 0 1 18.5 9L15.5 12A2.8 2.8 0 0 1 11.5 12"/>"##,
  )
}

pub fn trash_icon(color: &str) -> Svg {
  stroked(
    16.0,
    color,
    r##"<path d="M4.5 6H15.5M7.5 6V4.5H12.5V6M6 6L6.7 15.5H13.3L14 6"/>"##,
  )
}

pub fn star_icon(color: &str) -> Svg {
  stroked(
    17.0,
    color,
    r##"<path d="M10 3.2L12.1 7.4L16.7 8.1L13.3 11.4L14.1 16L10 13.8L5.9 16L6.7 11.4L3.3 8.1L7.9 7.4Z"/>"##,
  )
}

pub fn star_filled(color: &str) -> Svg {
  let doc = format!(
    r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="{color}" stroke="{color}" stroke-width="1.2" stroke-linejoin="round"><path d="M10 3.2L12.1 7.4L16.7 8.1L13.3 11.4L14.1 16L10 13.8L5.9 16L6.7 11.4L3.3 8.1L7.9 7.4Z"/></svg>"##
  );
  icon_svg(17.0, doc)
}

pub fn more_dots(color: &str) -> Svg {
  stroked(
    17.0,
    color,
    r##"<circle cx="5" cy="10" r="1.2" fill="currentColor"/><circle cx="10" cy="10" r="1.2" fill="currentColor"/><circle cx="15" cy="10" r="1.2" fill="currentColor"/>"##,
  )
}

pub fn sun_icon(color: &str) -> Svg {
  stroked(
    15.0,
    color,
    r##"<circle cx="10" cy="10" r="3.2"/><path d="M10 3V4.5M10 15.5V17M3 10H4.5M15.5 10H17M5 5L6.1 6.1M13.9 13.9L15 15M15 5L13.9 6.1M6.1 13.9L5 15"/>"##,
  )
}

pub fn moon_icon(color: &str) -> Svg {
  stroked(15.0, color, r##"<path d="M15.2 12.4A5.8 5.8 0 0 1 7.6 4.8A5.8 5.8 0 1 0 15.2 12.4Z"/>"##)
}

pub fn outline_icon(color: &str) -> Svg {
  stroked(15.0, color, r##"<path d="M3.5 5.5H16.5M6.5 10H16.5M6.5 14.5H13.5"/>"##)
}

pub fn history_icon(color: &str) -> Svg {
  stroked(
    15.0,
    color,
    r##"<path d="M4.2 10A5.8 5.8 0 1 0 6 5.8L3.8 7.8M3.8 4.5V7.8H7.1M10 7V10.2L12.3 11.6"/>"##,
  )
}

pub fn database_icon(color: &str) -> Svg {
  stroked(
    15.0,
    color,
    r##"<ellipse cx="10" cy="5.5" rx="6" ry="2.3"/><path d="M4 5.5V14.5C4 15.8 6.7 16.8 10 16.8C13.3 16.8 16 15.8 16 14.5V5.5M4 10C4 11.3 6.7 12.3 10 12.3C13.3 12.3 16 11.3 16 10"/>"##,
  )
}

pub fn sparkles_icon(color: &str) -> Svg {
  stroked(
    15.0,
    color,
    r##"<path d="M10 3.2L11.4 7.6L15.8 9L11.4 10.4L10 14.8L8.6 10.4L4.2 9L8.6 7.6Z"/>"##,
  )
}

pub fn bar_chart_icon(color: &str) -> Svg {
  stroked(
    15.0,
    color,
    r##"<path d="M4 16H16M6.2 16V10.5M10 16V5.5M13.8 16V8.5"/>"##,
  )
}

pub fn refresh_icon(color: &str) -> Svg {
  stroked(
    14.0,
    color,
    r##"<path d="M15.5 10A5.5 5.5 0 1 1 13.9 6.1L16 8.2M16 4.8V8.2H12.6"/>"##,
  )
}

pub fn keyboard_icon(color: &str) -> Svg {
  stroked(
    15.0,
    color,
    r##"<rect x="2.8" y="5" width="14.4" height="10" rx="2"/><path d="M6 8.5H6.5M9.5 8.5H10.5M13.5 8.5H14M6.5 12H13.5"/>"##,
  )
}

pub fn info_icon(color: &str) -> Svg {
  stroked(
    15.0,
    color,
    r##"<circle cx="10" cy="10" r="6.5"/><path d="M10 9.5V13.5M10 6.8H10.05"/>"##,
  )
}
