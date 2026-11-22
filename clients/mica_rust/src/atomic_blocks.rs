//! Atomic block renderer registry for Mica's pure-Rust L3 shell.
//!
//! Mirrors the `AtomicBlockRenderer` registry architecture (AGENTS.md):
//! specialized non-prose blocks (math LaTeX via RaTeX, Mermaid diagrams via
//! merman, embedded whiteboards via `mica-board`, code blocks, images, dividers)
//! register here instead of piling branches into `editor_view.rs`.

use nsh_base::theme;
use nsh_core::{
  element::{ImageSrc, svg},
  geometry::px,
  prelude::*,
};

/// Render descriptor passed to each atomic block renderer.
#[derive(Clone, Debug, PartialEq)]
pub struct AtomicBlockInput {
  pub id: String,
  pub kind: String,
  pub text: String,
  pub data: serde_json::Value,
}

/// Returns `true` for block kinds owned by the atomic block registry.
pub fn is_atomic_or_rich_block(kind: &str) -> bool {
  matches!(
    kind,
    "divider" | "code_block" | "math" | "mermaid" | "board" | "image" | "table"
  )
}

/// Renders an atomic block into an NSH element tree using L1 Rust renderers.
pub fn render_atomic_block(input: &AtomicBlockInput, dark_mode: bool) -> AnyElement {
  match input.kind.as_str() {
    "divider" => render_divider().into_element(),
    "code_block" => render_code_block(input, dark_mode),
    "math" => render_math_block(input, dark_mode).into_element(),
    "mermaid" => render_mermaid_block(input, dark_mode).into_element(),
    "board" => render_embedded_board_block(input).into_element(),
    "image" => render_image_block(input).into_element(),
    _ => view()
      .px_3()
      .py_2()
      .child(
        text(format!("[{}] {}", input.kind, input.text))
          .text_sm()
          .text_color(move || theme().muted_foreground),
      )
      .into_element(),
  }
}

fn render_divider() -> impl IntoElement {
  view()
    .w_full()
    .py_2()
    .child(
      text("────────────────────────")
        .text_xs()
        .text_color(move || theme().muted_foreground),
    )
}

fn render_code_block(input: &AtomicBlockInput, dark_mode: bool) -> AnyElement {
  let lang = input
    .data
    .get("language")
    .and_then(|v| v.as_str())
    .unwrap_or("plain")
    .to_string();

  // Check if fenced code block is actually a Mermaid diagram or embedded board.
  if lang.eq_ignore_ascii_case("mermaid") {
    return render_mermaid_card(&input.text, dark_mode).into_element();
  }
  if lang.eq_ignore_ascii_case("mica-board") {
    return render_board_card(&input.text).into_element();
  }

  let body = if input.text.is_empty() {
    "// empty code block".to_string()
  } else {
    input.text.clone()
  };

  view()
    .w_full()
    .p_3()
    .gap_1()
    .border_1()
    .border_color(move || theme().border)
    .rounded(move || theme().radius)
    .bg(move || theme().muted)
    .child(
      text(format!("``` {lang}"))
        .text_xs()
        .text_color(move || theme().muted_foreground),
    )
    .child(
      text(body)
        .text_sm()
        .text_color(move || theme().foreground),
    )
    .into_element()
}

fn render_math_block(input: &AtomicBlockInput, dark_mode: bool) -> impl IntoElement {
  let latex = if input.text.trim().is_empty() {
    r"E = mc^2"
  } else {
    input.text.trim()
  };
  let fg_hex = if dark_mode { "#f4f4f5" } else { "#18181b" };
  let rendered = mica_md::render::render_math_svg(latex, true, Some(fg_hex));
  let label = format!("Math: {latex}");

  let mut card = view()
    .w_full()
    .p_3()
    .gap_2()
    .border_1()
    .border_color(move || theme().border)
    .rounded(move || theme().radius)
    .bg(move || theme().card)
    .child(
      text(label)
        .text_xs()
        .text_color(move || theme().muted_foreground),
    );

  if let Some(math_svg) = rendered {
    let h = ((math_svg.height * 22.0).clamp(28.0, 140.0)) as f32;
    let w = ((math_svg.width * 22.0).clamp(60.0, 520.0)) as f32;
    card = card.child(
      svg(ImageSrc::svg_markup(&math_svg.svg))
        .w(px(w))
        .h(px(h)),
    );
  }
  card
}

fn render_mermaid_block(input: &AtomicBlockInput, dark_mode: bool) -> impl IntoElement {
  render_mermaid_card(&input.text, dark_mode)
}

fn render_mermaid_card(source: &str, dark_mode: bool) -> impl IntoElement {
  let src = if source.trim().is_empty() {
    "graph LR\n  A[Mica Core] --> B[Pure Rust Shell]"
  } else {
    source.trim()
  };
  let host_theme = mica_md::MermaidHostTheme {
    dark: dark_mode,
    canvas: if dark_mode { "#18181b" } else { "#ffffff" }.into(),
    surface: if dark_mode { "#27272a" } else { "#f4f4f5" }.into(),
    surface_alt: if dark_mode { "#3f3f46" } else { "#e4e4e7" }.into(),
    text: if dark_mode { "#f4f4f5" } else { "#18181b" }.into(),
    subtle_text: if dark_mode { "#a1a1aa" } else { "#71717a" }.into(),
    border: if dark_mode { "#52525b" } else { "#d4d4d8" }.into(),
    line: if dark_mode { "#a1a1aa" } else { "#52525b" }.into(),
    error: "#ef4444".into(),
    warning: "#f59e0b".into(),
    success: "#22c55e".into(),
  };
  let rendered = mica_md::render_mermaid_svg(src, Some(&host_theme));

  let mut card = view()
    .w_full()
    .p_3()
    .gap_2()
    .border_1()
    .border_color(move || theme().border)
    .rounded(move || theme().radius)
    .bg(move || theme().card)
    .child(
      text("Mermaid Diagram")
        .text_xs()
        .text_color(move || theme().muted_foreground),
    );

  if let Some(svg_markup) = rendered {
    card = card.child(
      svg(ImageSrc::svg_markup(&svg_markup))
        .w(px(460.0))
        .h(px(180.0)),
    );
  } else {
    card = card.child(
      text(src.to_string())
        .text_xs()
        .text_color(move || theme().foreground),
    );
  }
  card
}

fn render_embedded_board_block(input: &AtomicBlockInput) -> impl IntoElement {
  render_board_card(&input.text)
}

fn render_board_card(json_src: &str) -> impl IntoElement {
  let payload = if json_src.trim().is_empty() {
    mica_board::starter_board_json("mindmap")
  } else {
    json_src.to_string()
  };
  let rendered = mica_md::render_board_svg(&payload);

  let mut card = view()
    .w_full()
    .p_3()
    .gap_2()
    .border_1()
    .border_color(move || theme().border)
    .rounded(move || theme().radius)
    .bg(move || theme().card)
    .child(
      text("Embedded Whiteboard")
        .text_xs()
        .text_color(move || theme().muted_foreground),
    );

  if let Some(svg_markup) = rendered {
    card = card.child(
      svg(ImageSrc::svg_markup(&svg_markup))
        .w(px(460.0))
        .h(px(180.0)),
    );
  }
  card
}

fn render_image_block(input: &AtomicBlockInput) -> impl IntoElement {
  let alt = input
    .data
    .get("alt")
    .and_then(|v| v.as_str())
    .unwrap_or("Image")
    .to_string();
  let url = input
    .data
    .get("url")
    .and_then(|v| v.as_str())
    .unwrap_or("")
    .to_string();

  view()
    .w_full()
    .p_3()
    .gap_1()
    .border_1()
    .border_color(move || theme().border)
    .rounded(move || theme().radius)
    .bg(move || theme().muted)
    .child(
      text(format!("[Image] {alt}"))
        .text_sm()
        .text_color(move || theme().foreground),
    )
    .child(
      text(url)
        .text_xs()
        .text_color(move || theme().muted_foreground),
    )
}
