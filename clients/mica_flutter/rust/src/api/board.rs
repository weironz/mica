//! flutter_rust_bridge surface for `mica-board`: headless SVG rendering,
//! mind map auto-layout, orthogonal obstacle-avoiding connector routing,
//! draw.io import, Markdown outline conversion, and 3-way semantic merge.

pub struct BoardTheme {
    pub dark: bool,
    pub canvas: String,
    pub surface: String,
    pub text: String,
    pub subtle_text: String,
    pub border: String,
    pub accent: String,
}

impl From<&BoardTheme> for mica_board::BoardSvgTheme {
    fn from(t: &BoardTheme) -> Self {
        Self {
            dark: t.dark,
            canvas: t.canvas.clone(),
            surface: t.surface.clone(),
            text: t.text.clone(),
            subtle_text: t.subtle_text.clone(),
            border: t.border.clone(),
            accent: t.accent.clone(),
        }
    }
}

/// Render a `mica-board` JSON string to standalone SVG, or `None` if invalid.
pub fn render_board_svg(source: String, theme: Option<BoardTheme>) -> Option<String> {
    let host = theme.as_ref().map(mica_board::BoardSvgTheme::from);
    mica_board::render_board_json_svg(&source, host.as_ref())
}

/// Re-layout mind map trees inside a board JSON string.
pub fn layout_mindmap_board(source: String) -> Option<String> {
    mica_board::layout_mindmap_json(&source)
}

/// Compute obstacle-avoiding routed polylines for every edge in a board JSON string.
/// Returns JSON object `{ "<edge_id>": [[x, y], ...] }`.
pub fn route_board_edges(source: String) -> Option<String> {
    mica_board::route_board_edges_json(&source)
}

/// Import `.drawio`, `.xml`, `.drawio.svg`, or `.drawio.png` bytes into board JSON.
pub fn import_drawio_to_board(bytes: Vec<u8>) -> Result<String, String> {
    mica_board::import_drawio_bytes(&bytes)
        .map(|snap| snap.to_compact_json())
        .map_err(|e| e.to_string())
}

/// 3-way field-level semantic merge of concurrent board JSON edits.
pub fn merge_board_json(base: String, local: String, remote: String) -> String {
    mica_board::merge_board_json(&base, &local, &remote)
}

/// Convert a Markdown heading/bullet outline into a laid-out mind map JSON string.
pub fn markdown_to_mindmap_board(markdown: String, title: Option<String>) -> String {
    mica_board::markdown_to_mindmap(&markdown, title.as_deref()).to_compact_json()
}

/// Convert a mind map board JSON string back into a Markdown outline.
pub fn mindmap_board_to_markdown(source: String) -> Option<String> {
    let snap = mica_board::BoardSnapshot::parse_json(&source).ok()?;
    Some(mica_board::mindmap_to_markdown(&snap))
}

/// Return a starter board JSON for `"whiteboard"`, `"mindmap"`, or `"flowchart"`.
pub fn starter_board_json(template: String) -> String {
    mica_board::starter_board_json(&template)
}

/// Extract searchable plain text from a board JSON string.
pub fn extract_board_plain_text(source: String) -> String {
    mica_board::extract_board_text_json(&source)
}
