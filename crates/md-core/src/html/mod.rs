//! HTML rendering pipeline.

pub(crate) mod clipboard;

// The file name is the S1 layout contract (html/html.rs), so the module name
// intentionally matches its container.
#[allow(clippy::module_inception)]
pub(crate) mod html;
pub(crate) use html::*;
