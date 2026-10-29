//! Markdown block-document engine: CommonMark 0.31.2 + GFM + Mica dialect.
//!
//! The single implementation behind native (server, CLI, MCP, desktop FFI) and
//! Web (wasm) callers. The model is a flat block table plus UTF-16 inline marks;
//! Markdown and HTML exist only at this boundary.
//!
//! ```
//! use mica_md::{ParseMode, SerializeOptions, parse_body, serialize_body};
//! let blocks = parse_body("# Title\n\nSome **bold** text.", ParseMode::Document);
//! assert_eq!(blocks[0].kind, "heading");
//! let md = serialize_body(&blocks, &SerializeOptions::default());
//! assert_eq!(md, "# Title\n\nSome **bold** text.");
//! ```

#![deny(missing_docs)]

mod api;
pub mod compat;
mod html;
mod ids;
mod model;
mod parse;
pub mod render;
mod serialize;

pub mod properties;
pub mod schema;
pub mod wire;

pub use api::{
  HtmlOptions, HtmlProfile, ParseMode, ParseOptions, SerializeOptions, decode_cell, encode_cell,
  math_run_spans, parse, parse_body, parse_inline, parse_inline_math, render_html,
  render_html_body, serialize, serialize_body, serialize_inline,
};
pub use ids::BlockIds;
pub use model::{
  Block, Document, DocumentSnapshotPayload, InlineText, Mark, MdError, SCHEMA_VERSION,
};
pub use render::{MermaidHostTheme, render_mermaid_svg, render_mermaid_svg_with_id};

// The historical error/result names used by the moved modules. The facade maps
// these onto its own `DocumentOperationError`.
pub(crate) use model::MdError as DocumentOperationError;
pub(crate) type DocumentOperationResult<T> = Result<T, MdError>;

// Module aliases so the moved files keep their original `crate::…` paths.
pub(crate) use crate::parse::block;
pub(crate) use crate::parse::block_handlers;
pub(crate) use crate::parse::entities;
pub(crate) use crate::parse::inline;
pub(crate) use crate::parse::scan_memo;
pub(crate) use crate::render::{mermaid_id, render_math};
pub(crate) use crate::serialize::export_md;
pub(crate) use crate::serialize::span_walk;
