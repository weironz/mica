//! Web bindings to the Markdown engine. Preview builds export only a renderer;
//! the default Markdown build never pulls either renderer into startup.
#[cfg(feature = "markdown")]
mod markdown;
#[cfg(feature = "markdown")]
pub use markdown::*;

#[cfg(any(feature = "preview-math", feature = "preview-mermaid"))]
mod preview;
