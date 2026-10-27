//! The block-document schema (docs/single-source/02-architecture.md §6), as
//! constants. Normative: a kind / data key / mark type the engine emits but
//! this file does not list is a bug, and tests/schema_sync.rs fails on it.

/// Block kinds (`Block::kind`).
pub mod kinds {
  /// Root block kind (editor/root only; the parser currently emits `paragraph`
  /// for the root — kept as a schema value for future callers).
  pub const PAGE: &str = "page";
  /// Paragraph.
  pub const PARAGRAPH: &str = "paragraph";
  /// ATX / setext heading.
  pub const HEADING: &str = "heading";
  /// Bulleted list item.
  pub const BULLETED_LIST: &str = "bulleted_list";
  /// Numbered list item.
  pub const NUMBERED_LIST: &str = "numbered_list";
  /// Task list item.
  pub const TODO: &str = "todo";
  /// Block quote / GFM alert.
  pub const QUOTE: &str = "quote";
  /// Fenced, indented or raw-HTML code block.
  pub const CODE_BLOCK: &str = "code_block";
  /// Display math block.
  pub const MATH_BLOCK: &str = "math_block";
  /// Image block.
  pub const IMAGE: &str = "image";
  /// GFM table block.
  pub const TABLE: &str = "table";
  /// Thematic break.
  pub const DIVIDER: &str = "divider";
  /// Fenced footnote definition.
  pub const FOOTNOTE_DEF: &str = "footnote_def";

  /// Every kind the engine may emit or accept.
  pub const ALL: &[&str] = &[
    PAGE,
    PARAGRAPH,
    HEADING,
    BULLETED_LIST,
    NUMBERED_LIST,
    TODO,
    QUOTE,
    CODE_BLOCK,
    MATH_BLOCK,
    IMAGE,
    TABLE,
    DIVIDER,
    FOOTNOTE_DEF,
  ];

  /// Read-only aliases the exporters accept (never emitted by the parser).
  pub const ALIASES: &[(&str, &str)] = &[
    ("bullet_list", BULLETED_LIST),
    ("number_list", NUMBERED_LIST),
    ("code", CODE_BLOCK),
  ];
}

/// Inline mark types (`Mark::kind`).
pub mod marks {
  /// Bold / strong.
  pub const BOLD: &str = "bold";
  /// Italic / emphasis.
  pub const ITALIC: &str = "italic";
  /// Strikethrough.
  pub const STRIKE: &str = "strike";
  /// Inline code.
  pub const CODE: &str = "code";
  /// Link, including zero-width labels.
  pub const LINK: &str = "link";
  /// Inline image.
  pub const IMAGE: &str = "image";
  /// Inline formula.
  pub const MATH: &str = "math";
  /// Raw inline HTML.
  pub const HTML: &str = "html";
  /// Footnote reference.
  pub const FOOTNOTE: &str = "footnote";

  /// Every mark type.
  pub const ALL: &[&str] = &[
    BOLD, ITALIC, STRIKE, CODE, LINK, IMAGE, MATH, HTML, FOOTNOTE,
  ];
}

/// Keys the engine reads from or writes to `Block::data`.
pub mod data_keys {
  /// Raw YAML front matter on the root block.
  pub const FRONT_MATTER: &str = "front_matter";
  /// Page title on the root block (written by the Mica adapter).
  pub const TITLE: &str = "title";
  /// Inline marks array.
  pub const MARKS: &str = "marks";
  /// List indentation level.
  pub const INDENT: &str = "indent";
  /// Blank line inside a list (loose list).
  pub const LOOSE: &str = "loose";
  /// Bullet marker character.
  pub const MARKER: &str = "marker";
  /// Ordered-list start number.
  pub const START: &str = "start";
  /// Todo checked state.
  pub const CHECKED: &str = "checked";
  /// Quote depth.
  pub const QUOTE: &str = "quote";
  /// Quote-group break.
  pub const QBREAK: &str = "qbreak";
  /// GFM alert type.
  pub const ALERT: &str = "alert";
  /// Container-child list level.
  pub const LI: &str = "li";
  /// Heading level 1–6.
  pub const LEVEL: &str = "level";
  /// Code fence language.
  pub const LANGUAGE: &str = "language";
  /// Raw HTML code block (no fences on export).
  pub const RAW: &str = "raw";
  /// Image destination.
  pub const URL: &str = "url";
  /// Image title.
  pub const IMAGE_TITLE: &str = "title";
  /// Mica content-addressed asset id.
  pub const FILE_ID: &str = "file_id";
  /// Image display name.
  pub const NAME: &str = "name";
  /// Table rows (cell storage shape).
  pub const ROWS: &str = "rows";
  /// Table header cells.
  pub const HEADER: &str = "header";
  /// Table column alignment.
  pub const ALIGN: &str = "align";
  /// Plural alignment alias.
  pub const ALIGNS: &str = "aligns";
  /// Editor-only column widths.
  pub const WIDTHS: &str = "widths";
  /// Editor-only column width.
  pub const WIDTH: &str = "width";
  /// Footnote definition label.
  pub const LABEL: &str = "label";

  /// Every data key the engine reads or writes.
  pub const ALL: &[&str] = &[
    FRONT_MATTER,
    TITLE,
    MARKS,
    INDENT,
    LOOSE,
    MARKER,
    START,
    CHECKED,
    QUOTE,
    QBREAK,
    ALERT,
    LI,
    LEVEL,
    LANGUAGE,
    RAW,
    URL,
    FILE_ID,
    NAME,
    ROWS,
    HEADER,
    ALIGN,
    ALIGNS,
    WIDTHS,
    WIDTH,
    LABEL,
  ];
}
