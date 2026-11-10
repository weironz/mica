//! Optional rich rendering: math → SVG and Mermaid → SVG.
//!
//! The module exists unconditionally so `MermaidHostTheme` and the "feature
//! off returns None" signatures stay stable; only the function bodies switch
//! on the corresponding render features.

/// The host's palette, handed to merman so a diagram is drawn in the SAME
/// colours as the page around it. Values are CSS colour strings supplied by the
/// client (`MicaTokens`), not copied here.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct MermaidHostTheme {
  /// Dark appearance?
  pub dark: bool,
  /// Page background.
  pub canvas: String,
  /// Surface colour.
  pub surface: String,
  /// Alternate surface colour.
  pub surface_alt: String,
  /// Primary text colour.
  pub text: String,
  /// Secondary text colour.
  pub subtle_text: String,
  /// Border colour.
  pub border: String,
  /// Line/edge colour.
  pub line: String,
  /// Error colour.
  pub error: String,
  /// Warning colour.
  pub warning: String,
  /// Success colour.
  pub success: String,
}

/// A typeset formula with the original RaTeX view box and dimensions in em.
#[derive(Debug, serde::Serialize)]
pub struct MathSvg {
  /// Standalone SVG, with glyph outlines where available.
  pub svg: String,
  /// Width in em.
  pub width: f64,
  /// Total height in em.
  pub height: f64,
  /// Baseline measured from the top, in em.
  pub baseline: f64,
}

/// Typeset LaTeX for both editor previews and HTML export. A missing color
/// uses black. Embedded LaTeX colors are applied by RaTeX.
#[cfg(feature = "render-math")]
pub fn render_math_svg(latex: &str, display: bool, color: Option<&str>) -> Option<MathSvg> {
  use ratex_layout::{LayoutOptions, layout, to_display_list};
  use ratex_svg::{SvgOptions, render_to_svg};
  use ratex_types::color::Color;
  use ratex_types::math_style::MathStyle;

  let nodes = ratex_parser::parse(latex).ok()?;
  let opts = LayoutOptions {
    style: if display {
      MathStyle::Display
    } else {
      MathStyle::Text
    },
    color: color.and_then(Color::from_hex).unwrap_or(Color::BLACK),
    ..Default::default()
  };
  let list = to_display_list(&layout(&nodes, &opts));
  let svg = render_to_svg(
    &list,
    &SvgOptions {
      embed_glyphs: true,
      padding: 0.0,
      ..Default::default()
    },
  );
  Some(MathSvg {
    svg,
    width: list.width,
    height: list.height + list.depth,
    baseline: list.height,
  })
}

/// Feature-off math renderer.
#[cfg(not(feature = "render-math"))]
pub fn render_math_svg(_latex: &str, _display: bool, _color: Option<&str>) -> Option<MathSvg> {
  None
}

pub(crate) fn render_math(latex: &str, display: bool) -> Option<String> {
  let math = render_math_svg(latex, display, None)?;
  let height_em = math.height;
  let style = if display {
    format!("height:{height_em:.4}em")
  } else {
    format!(
      "height:{height_em:.4}em;vertical-align:-{:.4}em",
      math.height - math.baseline
    )
  };
  Some(restyle_math_svg(&math.svg, &style, display))
}

/// Rewrite RaTeX's `<svg>` opening tag: drop fixed `pt` size, add class/style.
fn restyle_math_svg(svg: &str, style: &str, display: bool) -> String {
  let class = if display { "math-block" } else { "math-inline" };
  let view_box = svg
    .find("viewBox=\"")
    .and_then(|i| {
      let rest = &svg[i + 9..];
      rest.find('"').map(|j| &rest[..j])
    })
    .unwrap_or("0 0 1 1");
  let body_start = svg.find('>').map(|i| i + 1).unwrap_or(0);
  let body_end = svg.rfind("</svg>").unwrap_or(svg.len());
  let body = &svg[body_start..body_end];
  format!(
    "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{view_box}\" \
     class=\"{class}\" style=\"{style}\" role=\"math\">{body}</svg>"
  )
}

/// Mermaid diagram → self-contained inline SVG, or `None`.
#[cfg(feature = "render-mermaid")]
pub fn render_mermaid_svg_with_id(
  source: &str,
  id: &str,
  theme: Option<&MermaidHostTheme>,
) -> Option<String> {
  use merman::render::{
    HeadlessRenderer, HostThemeAppearance, HostThemeProfile, HostThemeRoles, SvgPipeline,
  };
  let pipeline = SvgPipeline::resvg_safe();
  let renderer = HeadlessRenderer::new()
    .with_strict_parsing()
    .with_diagram_id(id);
  let profile = theme.map(|t| {
    HostThemeProfile::builder()
      .appearance(if t.dark {
        HostThemeAppearance::Dark
      } else {
        HostThemeAppearance::Light
      })
      .roles(HostThemeRoles {
        canvas: Some(t.canvas.clone()),
        surface: Some(t.surface.clone()),
        surface_alt: Some(t.surface_alt.clone()),
        text: Some(t.text.clone()),
        subtle_text: Some(t.subtle_text.clone()),
        border: Some(t.border.clone()),
        line: Some(t.line.clone()),
        error: Some(t.error.clone()),
        warning: Some(t.warning.clone()),
        success: Some(t.success.clone()),
        ..Default::default()
      })
      .build()
  });
  let renderer = match profile.as_ref() {
    Some(p) => renderer.with_host_theme(p),
    None => renderer,
  };
  renderer
    .render_svg_with_pipeline_sync(source, &pipeline)
    .ok()
    .flatten()
}

/// Feature-off Mermaid renderer.
#[cfg(not(feature = "render-mermaid"))]
pub fn render_mermaid_svg_with_id(
  _source: &str,
  _id: &str,
  _theme: Option<&MermaidHostTheme>,
) -> Option<String> {
  None
}

/// Single-diagram convenience with a stable id.
pub fn render_mermaid_svg(source: &str, theme: Option<&MermaidHostTheme>) -> Option<String> {
  render_mermaid_svg_with_id(source, "mica-mermaid", theme)
}

/// A merman diagram id from a block id: `[A-Za-z0-9_-]`, leading with a letter.
pub(crate) fn mermaid_id(block_id: &str) -> String {
  let mut s = String::from("m");
  for c in block_id.chars() {
    if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
      s.push(c);
    }
  }
  s
}
