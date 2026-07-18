//! flutter_rust_bridge surface for headless rendering shared with HTML/PDF
//! export. Currently: Mermaid → SVG via the merman engine (the `render` feature
//! is enabled for this crate), so the editor's live diagram preview and the
//! export path use ONE Rust engine — no separate Dart mermaid package.

/// Render Mermaid `source` to a self-contained SVG string, or `None` on a
/// syntax / render error. Async by default (runs off the Dart isolate); the
/// editor rasterizes the returned SVG for its inline preview.
pub fn render_mermaid_svg(source: String) -> Option<String> {
    mica_markdown::render_mermaid_svg(&source)
}
