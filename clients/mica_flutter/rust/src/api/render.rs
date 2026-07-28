//! flutter_rust_bridge surface for headless rendering shared with HTML/PDF
//! export. Currently: Mermaid → SVG via the merman engine (the `render` feature
//! is enabled for this crate), so the editor's live diagram preview and the
//! export path use ONE Rust engine — no separate Dart mermaid package.

/// The host palette handed to merman, mirrored to Dart.
///
/// Declared HERE rather than re-exported from `mica_markdown`: a type from
/// another crate crosses the bridge as an opaque handle, which Dart can hold but
/// cannot build — and building it in Dart is the entire point. The values come
/// from `MicaTokens`, the one place the app's palette is defined; a copy on this
/// side would be a second answer to the same question.
pub struct MermaidTheme {
    pub dark: bool,
    pub canvas: String,
    pub surface: String,
    pub surface_alt: String,
    pub text: String,
    pub subtle_text: String,
    pub border: String,
    pub line: String,
    pub error: String,
    pub warning: String,
    pub success: String,
}

impl From<&MermaidTheme> for mica_markdown::MermaidHostTheme {
    fn from(t: &MermaidTheme) -> Self {
        Self {
            dark: t.dark,
            canvas: t.canvas.clone(),
            surface: t.surface.clone(),
            surface_alt: t.surface_alt.clone(),
            text: t.text.clone(),
            subtle_text: t.subtle_text.clone(),
            border: t.border.clone(),
            line: t.line.clone(),
            error: t.error.clone(),
            warning: t.warning.clone(),
            success: t.success.clone(),
        }
    }
}

/// Render Mermaid `source` to a self-contained SVG string, or `None` on a
/// syntax / render error. Async by default (runs off the Dart isolate); the
/// editor rasterizes the returned SVG for its inline preview.
///
/// `theme` absent keeps merman's own default (light) — which is what the export
/// path wants, because an exported file is not inside anyone's editor.
pub fn render_mermaid_svg(source: String, theme: Option<MermaidTheme>) -> Option<String> {
    let host = theme.as_ref().map(mica_markdown::MermaidHostTheme::from);
    mica_markdown::render_mermaid_svg(&source, host.as_ref())
}
