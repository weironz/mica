//! Custom `nsh_base::rich_editor::view::NodeView` implementations for Mica's
//! pure-Rust L3 shell (1:1 parity with Flutter Mica's Notion-inspired editor):
//! - `code_block` / `codeBlock`: borderless warm-tint (`#F7F6F3` / `#202020`)
//!   editable code block via `nsh_base::editor()` with floating hover-only
//!   toolbar (segmented `[预览 | 代码]` for `mermaid`/`mica-board`, searchable
//!   `LanguagePicker`, Ask AI menu, Copy with 1600ms green check, `⋯` overflow
//!   menu for wrap/line-numbers/title/delete) and outside bottom caption.
//! - `math`: borderless tinted (`#F5F3FF` / `surface_raised`) RaTeX SVG preview
//!   with hover edit/copy icons and click-to-edit LaTeX input.
//! - `image`: real `nsh_core::prelude::image(src)` with graceful fallback,
//!   hover alignment/caption/delete toolbar, and centered bottom caption.
//! - `quote` / `blockquote`: editable content slot with `3px` left bar, plus
//!   GFM Callout (`note` / `tip` / `important` / `warning` / `caution`) accent
//!   bar, `6%` tint wash, and `22px` icon header.
//! - `mermaid`, `board`, `divider`, `todo`, `bulleted_list`, `numbered_list`.

use std::{rc::Rc, sync::Arc, time::Duration};

use nsh_base::{
  EditorScrollBeyond, EditorStyles, button, editor, input,
  rich_editor::{
    model::{Fragment, Node},
    state::Decoration,
    view::{EditorSession, NodeView, NodeViewCx, NodeViewEvent, NodeViews},
  },
  text_view::PartStyle,
};
use nsh_core::{
  Color, PropertyValue, StyleDecl, StyleKey, SubmitBehavior,
  color::Rgba,
  element::{ImageSrc, Style, Svg, set_clipboard, svg},
  geometry::px,
  prelude::*,
};
use serde_json::Value;

use crate::{
  atomic_blocks::{AtomicBlockInput, render_atomic_block},
  mica_icons,
  mica_theme::palette,
};

const TRANSPARENT_RGBA: Rgba = Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
const MATH_LIGHT_BG: Rgba = Rgba {
  r: 0xF5 as f32 / 255.0,
  g: 0xF3 as f32 / 255.0,
  b: 0xFF as f32 / 255.0,
  a: 1.0,
};

// ---------------------------------------------------------------------------
// Public context callbacks
// ---------------------------------------------------------------------------

/// Legacy context handle allowing embedded `board` node views to open the
/// workspace's full whiteboard view while keeping every `NodeView: Send + Sync`.
#[derive(Clone)]
pub struct OpenBoardCallback(pub Rc<dyn Fn()>);

/// Block-aware context handle passing `(block_pos, board_json)` when opening an
/// embedded `board` node for 3-way merge editing in the full whiteboard view.
#[derive(Clone)]
pub struct OpenBoardWithBlockCallback(pub Rc<dyn Fn(usize, String)>);

/// Preset actions offered by a code block's floating "Ask AI" menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeAiAction {
  Explain,
  Improve,
  Fix,
  Custom,
}

/// Optional context handle invoked when the user triggers a code-block AI action.
/// Receives `(action, block_pos, code_source)`.
#[derive(Clone)]
pub struct CodeAiCallback(pub Rc<dyn Fn(CodeAiAction, usize, String)>);

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// Builds the full `NodeViews` registry for Mica's rich document editor.
pub fn build_mica_node_views(dark_mode: Signal<bool>) -> NodeViews {
  let code_view: Arc<dyn NodeView> = Arc::new(CodeBlockNodeView { dark_mode });
  let math_view: Arc<dyn NodeView> = Arc::new(MathNodeView { dark_mode });
  let mermaid_view: Arc<dyn NodeView> = Arc::new(MermaidNodeView { dark_mode });
  let board_view: Arc<dyn NodeView> = Arc::new(BoardNodeView { dark_mode });
  let image_view: Arc<dyn NodeView> = Arc::new(ImageNodeView { dark_mode });
  let quote_view: Arc<dyn NodeView> = Arc::new(QuoteNodeView { dark_mode });
  let divider_view: Arc<dyn NodeView> = Arc::new(DividerNodeView { dark_mode });
  let todo_view: Arc<dyn NodeView> = Arc::new(TodoNodeView { dark_mode });
  let bullet_view: Arc<dyn NodeView> = Arc::new(FlatListNodeView { dark_mode, ordered: false });
  let ordered_view: Arc<dyn NodeView> = Arc::new(FlatListNodeView { dark_mode, ordered: true });

  let mut views = NodeViews::new();
  views
    .insert("code_block", code_view.clone())
    .insert("codeBlock", code_view)
    .insert("math", math_view)
    .insert("mermaid", mermaid_view)
    .insert("board", board_view)
    .insert("image", image_view)
    .insert("quote", quote_view.clone())
    .insert("blockquote", quote_view)
    .insert("divider", divider_view.clone())
    .insert("horizontalRule", divider_view)
    .insert("todo", todo_view.clone())
    .insert("taskItem", todo_view)
    .insert("bulleted_list", bullet_view)
    .insert("numbered_list", ordered_view);
  views
}

/// Registers all Mica custom `NodeView`s onto an existing `EditorSession`.
pub fn register_mica_node_views(session: &EditorSession, dark_mode: Signal<bool>) {
  session.set_node_views(build_mica_node_views(dark_mode));
}

// ---------------------------------------------------------------------------
// Attribute, transaction, language & copy helpers
// ---------------------------------------------------------------------------

fn has_schema_attr(node: &Node, key: &str) -> bool {
  node.ty().spec().attrs.iter().any(|(k, _)| k == key)
}

fn read_bool_attr(node: &Node, key: &str, default: bool) -> bool {
  node.attrs().get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn read_str_attr(node: &Node, key: &str) -> String {
  node.attrs().get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn read_f32_attr(node: &Node, key: &str) -> Option<f32> {
  node.attrs().get(key).and_then(Value::as_f64).filter(|&v| v > 0.0).map(|v| v as f32)
}

fn read_node_source(node: &Node) -> String {
  if !node.is_leaf() {
    let tc = node.text_content();
    if !tc.is_empty() {
      return tc;
    }
  }
  for key in ["source", "text", "code"] {
    if let Some(s) = node.attrs().get(key).and_then(Value::as_str) {
      return s.to_string();
    }
  }
  node.text_content()
}

fn read_code_language(node: &Node) -> String {
  node.attrs().get("language").and_then(Value::as_str).map(str::trim).unwrap_or("").to_string()
}

/// Pinned choice shown as checked inside `LanguagePicker` (`"auto"` when unset).
pub fn picker_current_language(raw: &str) -> String {
  let trimmed = raw.trim();
  if trimmed.is_empty() {
    "auto".to_string()
  } else {
    mica_md::highlight::canonical_code_language(trimmed)
  }
}

/// Chip label displayed on a code block's floating toolbar.
pub fn display_code_language(raw: &str, source: &str) -> String {
  let pinned = raw.trim();
  if pinned.eq_ignore_ascii_case("mica-board") {
    return "mica-board".to_string();
  }
  if !pinned.is_empty() && !pinned.eq_ignore_ascii_case("auto") {
    return mica_md::highlight::canonical_code_language(pinned);
  }
  let resolved = mica_md::highlight::resolve_code_language(source, Some("auto"));
  if resolved.is_empty() || resolved == "plaintext" {
    "auto".to_string()
  } else {
    resolved
  }
}

/// Returns `"auto"` pinned at the top followed by all supported highlight
/// languages sorted alphabetically, filtered by case-insensitive substring `query`.
pub fn filter_code_languages(query: &str) -> Vec<String> {
  let q = query.trim().to_lowercase();
  let mut rest: Vec<String> = mica_md::highlight::code_languages()
    .iter()
    .filter(|l| l.as_str() != "auto")
    .cloned()
    .collect();
  rest.sort_unstable();
  rest.dedup();
  let mut all = Vec::with_capacity(rest.len() + 1);
  all.push("auto".to_string());
  all.extend(rest);
  if q.is_empty() {
    all
  } else {
    all.into_iter().filter(|lang| lang.to_lowercase().contains(&q)).collect()
  }
}

fn editor_language_for(lang: &str) -> &str {
  if lang.is_empty()
    || lang.eq_ignore_ascii_case("auto")
    || lang.eq_ignore_ascii_case("text")
    || lang.eq_ignore_ascii_case("plain")
    || lang.eq_ignore_ascii_case("plaintext")
    || lang.eq_ignore_ascii_case("mermaid")
  {
    ""
  } else if lang.eq_ignore_ascii_case("mica-board") {
    "json"
  } else {
    lang
  }
}

fn dispatch_node_source_update(
  session: &EditorSession,
  get_pos: &Rc<dyn Fn() -> Option<usize>>,
  new_text: &str,
) {
  let Some(pos) = get_pos() else { return };
  let state = session.state();
  let Some(node) = state.doc().node_at(pos) else { return };
  if read_node_source(&node) == new_text {
    return;
  }
  let mut tr = state.tr();
  if node.is_leaf() || !node.inline_content() {
    let key = ["source", "text", "code"]
      .into_iter()
      .find(|k| has_schema_attr(&node, k))
      .unwrap_or("source");
    if tr.set_node_attribute(pos, key, Value::from(new_text)).is_ok() {
      let _ = session.dispatch(tr);
    }
  } else {
    let from = pos + 1;
    let to = pos + node.node_size().saturating_sub(1);
    if from <= to {
      let content = if new_text.is_empty() {
        Fragment::empty()
      } else {
        Fragment::from(state.schema().text(new_text, Vec::new()))
      };
      if tr.replace_with(from, to, content).is_ok() {
        let _ = session.dispatch(tr);
      }
    }
  }
}

fn dispatch_node_attr(
  session: &EditorSession,
  get_pos: &Rc<dyn Fn() -> Option<usize>>,
  attr: &str,
  value: Value,
) {
  let Some(pos) = get_pos() else { return };
  let state = session.state();
  let Some(node) = state.doc().node_at(pos) else { return };
  if !has_schema_attr(&node, attr) {
    return;
  }
  let mut tr = state.tr();
  if tr.set_node_attribute(pos, attr, value).is_ok() {
    let _ = session.dispatch(tr);
  }
}

fn dispatch_delete_node(session: &EditorSession, get_pos: &Rc<dyn Fn() -> Option<usize>>) {
  let Some(pos) = get_pos() else { return };
  let state = session.state();
  let Some(node) = state.doc().node_at(pos) else { return };
  let mut tr = state.tr();
  if tr.delete(pos, pos + node.node_size()).is_ok() {
    let _ = session.dispatch(tr);
  }
}

fn copy_with_reset(text: String, copied: Signal<bool>, copy_gen: Signal<u64>) {
  set_clipboard(text);
  copied.set(true);
  let gen_id = copy_gen.get_untracked().wrapping_add(1);
  copy_gen.set(gen_id);
  spawn(async move {
    timer(Duration::from_millis(1600)).await;
    if copy_gen.get_untracked() == gen_id {
      copied.set(false);
    }
  });
}

fn copy_done_hex(dark_mode: bool) -> &'static str {
  if dark_mode { "#56d364" } else { "#16a34a" }
}

// ---------------------------------------------------------------------------
// Inline SVG helpers & reusable UI primitives
// ---------------------------------------------------------------------------

fn nv_icon(size_px: f32, color: &str, body: &str) -> Svg {
  let doc = format!(
    r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="none" stroke="{color}" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">{body}</svg>"##
  );
  svg(ImageSrc::svg_markup(&doc)).w(px(size_px)).h(px(size_px))
}

fn icon_wrap_text(c: &str) -> Svg {
  nv_icon(14.0, c, r##"<path d="M3.5 5.5H16.5M3.5 10H13.5A2.5 2.5 0 0 1 13.5 15H10.5M12.5 13L10.5 15L12.5 17M3.5 14.5H7.5"/>"##)
}
fn icon_title_t(c: &str) -> Svg { nv_icon(14.0, c, r##"<path d="M4.5 5.5H15.5M10 5.5V15.5"/>"##) }
fn icon_pencil(c: &str) -> Svg { nv_icon(13.5, c, r##"<path d="M13.5 4.2L15.8 6.5L7.2 15.1L4.2 15.8L4.9 12.8Z"/>"##) }
fn icon_align_left(c: &str) -> Svg { nv_icon(14.0, c, r##"<path d="M3.5 5.5H16.5M3.5 10H12.5M3.5 14.5H15"/>"##) }
fn icon_align_center(c: &str) -> Svg { nv_icon(14.0, c, r##"<path d="M3.5 5.5H16.5M5.5 10H14.5M4.5 14.5H15.5"/>"##) }
fn icon_align_right(c: &str) -> Svg { nv_icon(14.0, c, r##"<path d="M3.5 5.5H16.5M7.5 10H16.5M5 14.5H16.5"/>"##) }
fn icon_image_frame(c: &str) -> Svg {
  nv_icon(22.0, c, r##"<rect x="3" y="4" width="14" height="12" rx="2"/><circle cx="7.5" cy="8" r="1.2"/><path d="M3.5 14L7.8 10L11 13L13.5 10.8L16.5 14"/>"##)
}
fn icon_lightbulb(c: &str) -> Svg {
  nv_icon(14.0, c, r##"<path d="M7.5 14.5H12.5M8.3 16.8H11.7M10 3.5A4.5 4.5 0 0 0 7 11.4C7.5 11.9 7.8 12.5 7.8 13.2H12.2C12.2 12.5 12.5 11.9 13 11.4A4.5 4.5 0 0 0 10 3.5Z"/>"##)
}
fn icon_bug(c: &str) -> Svg {
  nv_icon(14.0, c, r##"<rect x="6.5" y="6.5" width="7" height="8.5" rx="3.5"/><path d="M7.5 6.5V5.5A2.5 2.5 0 0 1 12.5 5.5V6.5M3.5 9.5H6.5M13.5 9.5H16.5M4 13H6.5M13.5 13H16M6.5 10.5H13.5"/>"##)
}
fn icon_chat(c: &str) -> Svg {
  nv_icon(14.0, c, r##"<path d="M4 5.5A1.5 1.5 0 0 1 5.5 4H14.5A1.5 1.5 0 0 1 16 5.5V12.5A1.5 1.5 0 0 1 14.5 14H8.2L5 16.5V14H5.5A1.5 1.5 0 0 1 4 12.5Z"/>"##)
}
fn icon_warning_triangle(c: &str) -> Svg {
  nv_icon(14.0, c, r##"<path d="M10 3.6L16.8 15.6H3.2Z"/><path d="M10 8.2V11.6M10 13.8H10.05"/>"##)
}
fn icon_caution_octagon(c: &str) -> Svg {
  nv_icon(14.0, c, r##"<path d="M7 3.5H13L16.5 7V13L13 16.5H7L3.5 13V7Z"/><path d="M10 7.2V11M10 13.3H10.05"/>"##)
}
fn icon_important_badge(c: &str) -> Svg {
  nv_icon(14.0, c, r##"<path d="M4 5A1.5 1.5 0 0 1 5.5 3.5H14.5A1.5 1.5 0 0 1 16 5V12A1.5 1.5 0 0 1 14.5 13.5H8.5L5.5 16.5V13.5H5.5A1.5 1.5 0 0 1 4 12Z"/><path d="M10 6.5V9.5M10 11.4H10.05"/>"##)
}

#[allow(clippy::too_many_arguments)]
fn inline_prompt_dialog(
  dialog_id: &'static str,
  input_id: &'static str,
  title: &'static str,
  placeholder: &'static str,
  draft: Signal<String>,
  dark_mode: Signal<bool>,
  on_cancel: impl Fn() + 'static,
  on_save: Rc<dyn Fn(String)>,
) -> impl IntoElement {
  let on_save_sub = on_save.clone();
  view()
    .test_id(dialog_id)
    .w_full()
    .mt_2()
    .p_3()
    .gap_2()
    .rounded(px(8.0))
    .bg(move || palette(dark_mode.get()).surface_overlay)
    .border_1()
    .border_color(move || palette(dark_mode.get()).border_normal)
    .child(
      text(title)
        .text_size(px(13.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(move || palette(dark_mode.get()).text_primary),
    )
    .child(
      input()
        .test_id(input_id)
        .auto_focus(true)
        .submit_behavior(SubmitBehavior::Submit)
        .placeholder(placeholder)
        .w_full()
        .px_2()
        .py_1()
        .rounded(px(5.0))
        .border_1()
        .border_color(move || palette(dark_mode.get()).border_normal)
        .bg(move || palette(dark_mode.get()).surface_base)
        .text_size(px(13.0))
        .text_color(move || palette(dark_mode.get()).text_primary)
        .value(move || draft.get())
        .on_change(move |v: &str| draft.set(v.to_string()))
        .on_submit(move |v: &str| (on_save_sub)(v.trim().to_string())),
    )
    .child(
      view()
        .w_full()
        .flex_row()
        .justify_end()
        .gap_2()
        .child(
          button()
            .px_3()
            .py_1()
            .rounded(px(5.0))
            .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
            .on_press(on_cancel)
            .child(
              text("取消")
                .text_size(px(12.5))
                .text_color(move || palette(dark_mode.get()).text_muted),
            ),
        )
        .child(
          button()
            .px_3()
            .py_1()
            .rounded(px(5.0))
            .bg(move || palette(dark_mode.get()).accent)
            .on_press(move || (on_save)(draft.get_untracked().trim().to_string()))
            .child(
              text("保存")
                .text_size(px(12.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || palette(dark_mode.get()).on_primary),
            ),
        ),
    )
}

fn decl_style(decls: impl IntoIterator<Item = (StyleKey, PropertyValue)>) -> Style {
  Style::from_decls(decls.into_iter().map(|(k, v)| StyleDecl::Set(k, v)))
}
fn num_prop(key: StyleKey, n: f32) -> (StyleKey, PropertyValue) {
  (key, key.number_value(n).unwrap_or(PropertyValue::Default))
}
fn color_prop(key: StyleKey, c: impl Into<Color>) -> (StyleKey, PropertyValue) {
  (key, PropertyValue::Color(Some(c.into())))
}

fn code_editor_styles(dark_mode: bool) -> EditorStyles {
  let p = palette(dark_mode);
  let mut styles = EditorStyles::default();
  styles.gutter = Some(PartStyle(decl_style([
    color_prop(StyleKey::BackgroundColor, p.code_bg),
    num_prop(StyleKey::PaddingLeft, 0.0),
    num_prop(StyleKey::PaddingRight, 10.0),
  ])));
  styles.line_number = Some(PartStyle(decl_style([color_prop(StyleKey::Color, p.text_ghost)])));
  styles.active_line_number = Some(PartStyle(decl_style([color_prop(StyleKey::Color, p.text_muted)])));
  styles.content = Some(PartStyle(decl_style([
    color_prop(StyleKey::BackgroundColor, p.code_bg),
    color_prop(StyleKey::Color, p.text_primary),
    num_prop(StyleKey::PaddingTop, 0.0),
    num_prop(StyleKey::PaddingBottom, 0.0),
    num_prop(StyleKey::PaddingLeft, 0.0),
    num_prop(StyleKey::PaddingRight, 0.0),
  ])));
  styles.active_line = Some(PartStyle(decl_style([color_prop(StyleKey::BackgroundColor, TRANSPARENT_RGBA)])));
  styles.selection = Some(PartStyle(decl_style([color_prop(StyleKey::BackgroundColor, p.selection_bg)])));
  styles.caret = Some(PartStyle(decl_style([color_prop(StyleKey::BackgroundColor, p.text_primary)])));
  styles.placeholder = Some(PartStyle(decl_style([color_prop(StyleKey::Color, p.text_ghost)])));
  styles
}

// ---------------------------------------------------------------------------
// 1. CodeBlockNodeView (1:1 Flutter borderless code block + floating toolbar)
// ---------------------------------------------------------------------------

struct CodeBlockNodeView {
  dark_mode: Signal<bool>,
}

impl NodeView for CodeBlockNodeView {
  fn create(&self, cx: &NodeViewCx, initial_node: &Node) -> AnyElement {
    let dark_mode = self.dark_mode;
    let node_sig = cx.node();
    let ai_cb = use_context::<CodeAiCallback>();

    let hovered = signal(false);
    let copied = signal(false);
    let copy_gen = signal(0u64);
    let show_preview = signal(true);

    let lang_picker_open = signal(false);
    let lang_query = signal(String::new());
    let more_menu_open = signal(false);
    let ai_menu_open = signal(false);
    let title_dialog_open = signal(false);
    let title_draft = signal(String::new());
    let ai_notice: Signal<Option<String>> = signal(None);
    let ai_notice_gen = signal(0u64);

    // Local fallback signals keep wrap / lineNumbers / title working even before
    // schema extensions declare those optional attributes.
    let local_wrap = signal(read_bool_attr(initial_node, "wrap", false));
    let local_line_numbers = signal(read_bool_attr(initial_node, "lineNumbers", false));
    let local_title = signal(read_str_attr(initial_node, "title"));

    let effective_wrap = computed(move || {
      let node = node_sig.get();
      if has_schema_attr(&node, "wrap") { read_bool_attr(&node, "wrap", false) } else { local_wrap.get() }
    });
    let effective_line_numbers = computed(move || {
      let node = node_sig.get();
      if has_schema_attr(&node, "lineNumbers") { read_bool_attr(&node, "lineNumbers", false) } else { local_line_numbers.get() }
    });
    let effective_title = computed(move || {
      let node = node_sig.get();
      if has_schema_attr(&node, "title") { read_str_attr(&node, "title") } else { local_title.get() }
    });
    let is_previewable = computed(move || {
      let lang = read_code_language(&node_sig.get());
      lang.eq_ignore_ascii_case("mermaid") || lang.eq_ignore_ascii_case("mica-board")
    });
    let toolbar_visible = computed(move || {
      hovered.get() || copied.get() || lang_picker_open.get() || more_menu_open.get() || ai_menu_open.get()
    });
    let show_editor = computed(move || !is_previewable.get() || !show_preview.get());

    let session_ed = cx.session.clone();
    let get_pos_ed = cx.get_pos();
    let session_lang = cx.session.clone();
    let get_pos_lang = cx.get_pos();
    let session_more = cx.session.clone();
    let get_pos_more = cx.get_pos();
    let session_title = cx.session.clone();
    let get_pos_title = cx.get_pos();
    let get_pos_ai = cx.get_pos();

    let floating_toolbar = when(
      move || toolbar_visible.get(),
      move || {
        view()
          .test_id("code-hover-toolbar")
          .absolute().top(px(4.0)).right(px(8.0)).z_index(20)
          .flex_row().items_center().gap(px(4.0))
          .child(when(
            move || is_previewable.get(),
            move || {
              view()
                .test_id("code-preview-toggle")
                .flex_row().items_center().rounded(px(6.0)).p(px(2.0))
                .bg(move || palette(dark_mode.get()).border_normal)
                .child(
                  button()
                    .test_id("code-segment-preview")
                    .px(px(6.0)).py(px(1.5)).rounded(px(4.0))
                    .bg(move || if show_preview.get() { palette(dark_mode.get()).surface_base } else { TRANSPARENT_RGBA })
                    .on_press(move || show_preview.set(true))
                    .child(
                      text("预览").text_size(px(11.0)).font_weight(FontWeight::MEDIUM)
                        .text_color(move || { let p = palette(dark_mode.get()); if show_preview.get() { p.text_primary } else { p.text_muted } }),
                    ),
                )
                .child(
                  button()
                    .test_id("code-segment-code")
                    .px(px(6.0)).py(px(1.5)).rounded(px(4.0))
                    .bg(move || if !show_preview.get() { palette(dark_mode.get()).surface_base } else { TRANSPARENT_RGBA })
                    .on_press(move || show_preview.set(false))
                    .child(
                      text("代码").text_size(px(11.0)).font_weight(FontWeight::MEDIUM)
                        .text_color(move || { let p = palette(dark_mode.get()); if !show_preview.get() { p.text_primary } else { p.text_muted } }),
                    ),
                )
            },
          ))
          .child(
            button()
              .test_id("code-lang-chip").accessibility_label("选择代码语言")
              .h(px(22.0)).px(px(7.0)).rounded(px(5.0)).items_center().justify_center()
              .bg(move || palette(dark_mode.get()).border_normal)
              .hover(move |s| s.bg(palette(dark_mode.get_untracked()).border_strong))
              .on_press(move || {
                let next = !lang_picker_open.get_untracked();
                more_menu_open.set(false);
                ai_menu_open.set(false);
                if next { lang_query.set(String::new()); }
                lang_picker_open.set(next);
              })
              .child(
                text(move || {
                  let node = node_sig.get();
                  format!("{} ▾", display_code_language(&read_code_language(&node), &read_node_source(&node)))
                })
                .text_size(px(11.0))
                .text_color(move || palette(dark_mode.get()).text_muted),
              ),
          )
          .child(
            button()
              .test_id("code-ai-btn").accessibility_label("AI 助手")
              .w(px(22.0)).h(px(22.0)).rounded(px(5.0)).items_center().justify_center()
              .hover(move |s| s.bg(palette(dark_mode.get_untracked()).border_normal))
              .on_press(move || {
                let next = !ai_menu_open.get_untracked();
                lang_picker_open.set(false);
                more_menu_open.set(false);
                ai_menu_open.set(next);
              })
              .child(dynamic(move || mica_icons::sparkles_icon(palette(dark_mode.get()).text_muted_hex))),
          )
          .child(
            button()
              .test_id("code-copy-btn").accessibility_label("复制代码块内容")
              .w(px(22.0)).h(px(22.0)).rounded(px(5.0)).items_center().justify_center()
              .hover(move |s| s.bg(palette(dark_mode.get_untracked()).border_normal))
              .on_press(move || copy_with_reset(read_node_source(&node_sig.get_untracked()), copied, copy_gen))
              .child(dynamic(move || {
                let dark = dark_mode.get();
                if copied.get() { mica_icons::check_icon(copy_done_hex(dark)) } else { mica_icons::copy_icon(palette(dark).text_muted_hex) }
              })),
          )
          .child(
            button()
              .test_id("code-more-btn").accessibility_label("更多代码块操作")
              .w(px(22.0)).h(px(22.0)).rounded(px(5.0)).items_center().justify_center()
              .hover(move |s| s.bg(palette(dark_mode.get_untracked()).border_normal))
              .on_press(move || {
                let next = !more_menu_open.get_untracked();
                lang_picker_open.set(false);
                ai_menu_open.set(false);
                more_menu_open.set(next);
              })
              .child(dynamic(move || mica_icons::more_dots(palette(dark_mode.get()).text_muted_hex))),
          )
      },
    );

    let language_picker_popover = when(
      move || lang_picker_open.get(),
      move || {
        let session_p = session_lang.clone();
        let get_pos_p = get_pos_lang.clone();
        let session_s = session_p.clone();
        let get_pos_s = get_pos_p.clone();
        view()
          .test_id("code-lang-picker")
          .absolute().top(px(30.0)).right(px(8.0)).z_index(40)
          .w(px(240.0)).h(px(312.0)).p(px(8.0)).gap(px(6.0))
          .rounded(px(8.0))
          .bg(move || palette(dark_mode.get()).surface_overlay)
          .border_1().border_color(move || palette(dark_mode.get()).border_normal)
          .child(
            view()
              .w_full().h(px(30.0)).px(px(8.0)).flex_row().items_center().gap(px(6.0))
              .rounded(px(6.0)).border_1()
              .border_color(move || palette(dark_mode.get()).border_normal)
              .bg(move || palette(dark_mode.get()).surface_base)
              .child(dynamic(move || mica_icons::search(palette(dark_mode.get()).text_faint_hex)))
              .child(
                input()
                  .test_id("code-lang-search").auto_focus(true)
                  .submit_behavior(SubmitBehavior::Submit).placeholder("搜索语言…")
                  .flex_1().text_size(px(12.5))
                  .text_color(move || palette(dark_mode.get()).text_primary)
                  .value(move || lang_query.get())
                  .on_change(move |q: &str| lang_query.set(q.to_string()))
                  .on_submit(move |q: &str| {
                    if let Some(first) = filter_code_languages(q).into_iter().next() {
                      dispatch_node_attr(&session_s, &get_pos_s, "language", Value::String(first));
                    }
                    lang_picker_open.set(false);
                  }),
              ),
          )
          .child(
            scroll().flex_1().w_full().child(dynamic(move || {
              let items = filter_code_languages(&lang_query.get());
              let current = picker_current_language(&read_code_language(&node_sig.get()));
              let p = palette(dark_mode.get());
              if items.is_empty() {
                return view()
                  .w_full().py_6().items_center().justify_center()
                  .child(text("无匹配结果").text_size(px(12.5)).text_color(p.text_faint))
                  .into_element();
              }
              let mut list = view().w_full();
              for lang in items {
                let is_cur = lang == current;
                let lang_click = lang.clone();
                let s_item = session_p.clone();
                let gp_item = get_pos_p.clone();
                list = list.child(
                  button()
                    .test_id(&format!("code-lang-{lang}"))
                    .w_full().h(px(34.0)).px(px(10.0)).rounded(px(5.0)).flex_row().items_center()
                    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                    .on_press(move || {
                      dispatch_node_attr(&s_item, &gp_item, "language", Value::String(lang_click.clone()));
                      lang_picker_open.set(false);
                    })
                    .child(
                      view().w(px(20.0)).items_center().justify_center()
                        .children(is_cur.then(|| mica_icons::check_icon(p.text_primary_hex))),
                    )
                    .child(
                      text(lang).text_size(px(13.0))
                        .font_weight(if is_cur { FontWeight::MEDIUM } else { FontWeight::NORMAL })
                        .text_color(p.text_primary),
                    ),
                );
              }
              list.into_element()
            })),
          )
      },
    );

    let ai_menu_popover = when(
      move || ai_menu_open.get(),
      move || {
        let ai_cb_m = ai_cb.clone();
        let get_pos_m = get_pos_ai.clone();
        let trigger_ai = move |action: CodeAiAction| {
          ai_menu_open.set(false);
          let src = read_node_source(&node_sig.get_untracked());
          let pos = get_pos_m().unwrap_or(0);
          if let Some(cb) = &ai_cb_m {
            (cb.0)(action, pos, src);
          } else {
            ai_notice.set(Some("请先在设置中配置 AI 模型".to_string()));
            let gen_id = ai_notice_gen.get_untracked().wrapping_add(1);
            ai_notice_gen.set(gen_id);
            spawn(async move {
              timer(Duration::from_millis(2200)).await;
              if ai_notice_gen.get_untracked() == gen_id {
                ai_notice.set(None);
              }
            });
          }
        };
        let items: [(CodeAiAction, &str, &str); 4] = [
          (CodeAiAction::Explain, "解释代码", "code-ai-explain"),
          (CodeAiAction::Improve, "优化代码", "code-ai-improve"),
          (CodeAiAction::Fix, "查找问题", "code-ai-fix"),
          (CodeAiAction::Custom, "自定义提问", "code-ai-custom"),
        ];
        let mut menu = view()
          .test_id("code-ai-menu")
          .absolute().top(px(30.0)).right(px(8.0)).z_index(40)
          .w(px(160.0)).py_1().px_1().rounded(px(8.0))
          .bg(move || palette(dark_mode.get()).surface_overlay)
          .border_1().border_color(move || palette(dark_mode.get()).border_normal);
        for (action, label, tid) in items {
          let trigger = trigger_ai.clone();
          menu = menu.child(
            button()
              .test_id(tid).w_full().h(px(30.0)).px(px(10.0)).rounded(px(5.0))
              .flex_row().items_center().gap(px(8.0))
              .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
              .on_press(move || trigger(action))
              .child(dynamic(move || {
                let c = palette(dark_mode.get()).text_muted_hex;
                match action {
                  CodeAiAction::Explain => icon_lightbulb(c),
                  CodeAiAction::Improve => mica_icons::sparkles_icon(c),
                  CodeAiAction::Fix => icon_bug(c),
                  CodeAiAction::Custom => icon_chat(c),
                }
              }))
              .child(text(label).text_size(px(12.5)).text_color(move || palette(dark_mode.get()).text_primary)),
          );
        }
        menu
      },
    );

    let more_menu_popover = when(
      move || more_menu_open.get(),
      move || {
        let (sw, gpw) = (session_more.clone(), get_pos_more.clone());
        let (sln, gpln) = (session_more.clone(), get_pos_more.clone());
        let (st, gpt) = (session_more.clone(), get_pos_more.clone());
        let (sdel, gpdel) = (session_more.clone(), get_pos_more.clone());

        view()
          .test_id("code-more-menu")
          .absolute().top(px(30.0)).right(px(8.0)).z_index(40)
          .w(px(164.0)).py_1().px_1().rounded(px(8.0))
          .bg(move || palette(dark_mode.get()).surface_overlay)
          .border_1().border_color(move || palette(dark_mode.get()).border_normal)
          .child(
            button()
              .test_id("code-menu-wrap").w_full().h(px(30.0)).px(px(10.0)).rounded(px(5.0))
              .flex_row().items_center().gap(px(8.0))
              .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
              .on_press(move || {
                let next = !effective_wrap.get_untracked();
                local_wrap.set(next);
                dispatch_node_attr(&sw, &gpw, "wrap", Value::Bool(next));
                more_menu_open.set(false);
              })
              .child(dynamic(move || icon_wrap_text(palette(dark_mode.get()).text_muted_hex)))
              .child(
                text(move || if effective_wrap.get() { "取消换行" } else { "自动换行" })
                  .text_size(px(12.5)).text_color(move || palette(dark_mode.get()).text_primary),
              ),
          )
          .child(
            button()
              .test_id("code-menu-line-numbers").w_full().h(px(30.0)).px(px(10.0)).rounded(px(5.0))
              .flex_row().items_center().gap(px(8.0))
              .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
              .on_press(move || {
                let next = !effective_line_numbers.get_untracked();
                local_line_numbers.set(next);
                dispatch_node_attr(&sln, &gpln, "lineNumbers", Value::Bool(next));
                more_menu_open.set(false);
              })
              .child(dynamic(move || mica_icons::list_ordered(palette(dark_mode.get()).text_muted_hex)))
              .child(
                text(move || if effective_line_numbers.get() { "隐藏行号" } else { "显示行号" })
                  .text_size(px(12.5)).text_color(move || palette(dark_mode.get()).text_primary),
              ),
          )
          .child(
            button()
              .test_id("code-menu-title").w_full().h(px(30.0)).px(px(10.0)).rounded(px(5.0))
              .flex_row().items_center().gap(px(8.0))
              .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
              .on_press(move || {
                more_menu_open.set(false);
                if effective_title.get_untracked().trim().is_empty() {
                  title_draft.set(String::new());
                  title_dialog_open.set(true);
                } else {
                  local_title.set(String::new());
                  dispatch_node_attr(&st, &gpt, "title", Value::String(String::new()));
                }
              })
              .child(dynamic(move || icon_title_t(palette(dark_mode.get()).text_muted_hex)))
              .child(
                text(move || if effective_title.get().trim().is_empty() { "添加标题" } else { "移除标题" })
                  .text_size(px(12.5)).text_color(move || palette(dark_mode.get()).text_primary),
              ),
          )
          .child(
            button()
              .test_id("code-menu-delete").w_full().h(px(30.0)).px(px(10.0)).rounded(px(5.0))
              .flex_row().items_center().gap(px(8.0))
              .hover(move |s| s.bg(palette(dark_mode.get_untracked()).danger_wash))
              .on_press(move || {
                more_menu_open.set(false);
                dispatch_delete_node(&sdel, &gpdel);
              })
              .child(dynamic(move || mica_icons::trash_icon(palette(dark_mode.get()).danger_hex)))
              .child(text("删除代码块").text_size(px(12.5)).text_color(move || palette(dark_mode.get()).danger)),
          )
      },
    );

    let title_dialog = when(
      move || title_dialog_open.get(),
      move || {
        let s_save = session_title.clone();
        let gp_save = get_pos_title.clone();
        let on_save: Rc<dyn Fn(String)> = Rc::new(move |cleaned| {
          local_title.set(cleaned.clone());
          dispatch_node_attr(&s_save, &gp_save, "title", Value::String(cleaned));
          title_dialog_open.set(false);
        });
        inline_prompt_dialog(
          "code-title-dialog",
          "code-title-input",
          "添加标题",
          "输入代码块标题…",
          title_draft,
          dark_mode,
          move || title_dialog_open.set(false),
          on_save,
        )
      },
    );

    view()
      .test_id("node-view-code-block")
      .relative().w_full().my_2()
      .child(
        // Borderless 6px rounded warm-tinted code box (NO header bar, NO 1px border)
        view()
          .relative().w_full().rounded(px(6.0))
          .bg(move || palette(dark_mode.get()).code_bg)
          .px(px(14.0)).py(px(12.0))
          .on_mouse_enter(move |_: &MouseEvent| hovered.set(true))
          .on_mouse_move(move |_: &MouseEvent| {
            if !hovered.get_untracked() {
              hovered.set(true);
            }
          })
          .on_mouse_leave(move |_: &MouseEvent| hovered.set(false))
          .child(when(
            move || show_editor.get(),
            move || {
              let session_c = session_ed.clone();
              let get_pos_c = get_pos_ed.clone();
              editor()
                .test_id("code-block-editor").accessibility_label("代码块编辑器")
                .w_full()
                .value(move || Some(SharedString::from(read_node_source(&node_sig.get()))))
                .language(move || Some(SharedString::from(editor_language_for(&read_code_language(&node_sig.get())).to_string())))
                .placeholder("输入代码...")
                .line_numbers(move || effective_line_numbers.get())
                .soft_wrap(move || effective_wrap.get())
                .folding(false)
                .auto_indent(true).auto_close(true)
                .font_size(px(14.0)).line_height(px(22.0))
                .min_rows(Some(1u32)).max_rows(Some(32u32))
                .scroll_beyond_last_line(EditorScrollBeyond::None)
                .styles(move || code_editor_styles(dark_mode.get()))
                .on_change(move |val: &SharedString| {
                  copied.set(false);
                  dispatch_node_source_update(&session_c, &get_pos_c, val.as_ref());
                })
            },
          ))
          .child(when(
            move || is_previewable.get() && show_preview.get(),
            move || {
              view().w_full().py_1().child(dynamic(move || {
                let node = node_sig.get();
                render_atomic_block(
                  &AtomicBlockInput {
                    id: "code-preview".into(),
                    kind: "code_block".into(),
                    text: read_node_source(&node),
                    data: serde_json::json!({ "language": read_code_language(&node) }),
                  },
                  dark_mode.get(),
                )
              }))
            },
          ))
          .child(floating_toolbar)
          .child(language_picker_popover)
          .child(ai_menu_popover)
          .child(more_menu_popover)
          .child(when(
            move || ai_notice.get().is_some(),
            move || {
              view()
                .absolute().bottom(px(8.0)).right(px(8.0)).z_index(30)
                .px_2().py_1().rounded(px(5.0))
                .bg(move || palette(dark_mode.get()).surface_overlay)
                .border_1().border_color(move || palette(dark_mode.get()).border_normal)
                .child(
                  text(move || ai_notice.get().unwrap_or_default())
                    .text_size(px(11.5)).text_color(move || palette(dark_mode.get()).text_muted),
                )
            },
          )),
      )
      .child(title_dialog)
      // Bottom caption rendered OUTSIDE & BELOW the rounded code box (24px centered)
      .child(when(
        move || !effective_title.get().trim().is_empty(),
        move || {
          button()
            .test_id("code-block-caption")
            .w_full().h(px(24.0)).items_center().justify_center()
            .on_press(move || {
              title_draft.set(effective_title.get_untracked());
              title_dialog_open.set(true);
            })
            .child(
              text(move || effective_title.get())
                .text_size(px(12.0)).text_color(move || palette(dark_mode.get()).text_muted),
            )
        },
      ))
      .into_element()
  }

  fn update(&self, node: &Node, _decorations: &[Decoration]) -> bool {
    matches!(node.ty().name(), "code_block" | "codeBlock")
  }

  fn handle_event(&self, _event: &NodeViewEvent) -> bool {
    true
  }
}

// ---------------------------------------------------------------------------
// 2. MathNodeView (borderless 6px tinted RaTeX preview + hover actions)
// ---------------------------------------------------------------------------

struct MathNodeView {
  dark_mode: Signal<bool>,
}

impl NodeView for MathNodeView {
  fn create(&self, cx: &NodeViewCx, _node: &Node) -> AnyElement {
    let dark_mode = self.dark_mode;
    let node_sig = cx.node();
    let editing = signal(false);
    let hovered = signal(false);
    let copied = signal(false);
    let copy_gen = signal(0u64);
    let session_in = cx.session.clone();
    let get_pos_in = cx.get_pos();

    view()
      .test_id("node-view-math")
      .relative().w_full().my_2().rounded(px(6.0))
      .bg(move || if dark_mode.get() { palette(true).surface_raised } else { MATH_LIGHT_BG })
      .on_mouse_enter(move |_: &MouseEvent| hovered.set(true))
      .on_mouse_leave(move |_: &MouseEvent| hovered.set(false))
      .child(
        button()
          .test_id("math-preview-btn").accessibility_label("点击编辑数学公式")
          .w_full().px(px(16.0)).py(px(14.0)).items_center().justify_center()
          .on_press(move || editing.set(!editing.get_untracked()))
          .child(dynamic(move || {
            let p = palette(dark_mode.get());
            let raw = read_node_source(&node_sig.get());
            let latex = raw.trim();
            if latex.is_empty() {
              return text("点击输入 LaTeX 公式…").text_size(px(13.0)).text_color(p.text_faint).into_element();
            }
            if let Some(math_svg) = mica_md::render::render_math_svg(latex, true, Some(p.text_primary_hex)) {
              let h = (math_svg.height * 22.0).clamp(26.0, 180.0) as f32;
              let w = (math_svg.width * 22.0).clamp(48.0, 640.0) as f32;
              svg(ImageSrc::svg_markup(&math_svg.svg)).w(px(w)).h(px(h)).into_element()
            } else {
              text(latex.to_string()).text_size(px(13.5)).text_color(p.text_primary).into_element()
            }
          })),
      )
      .child(when(
        move || hovered.get() || editing.get() || copied.get(),
        move || {
          view()
            .absolute().top(px(6.0)).right(px(8.0)).z_index(20)
            .flex_row().items_center().gap(px(4.0))
            .child(
              button()
                .test_id("math-copy-btn").accessibility_label("复制 LaTeX 源码")
                .w(px(22.0)).h(px(22.0)).rounded(px(5.0)).items_center().justify_center()
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).border_normal))
                .on_press(move || copy_with_reset(read_node_source(&node_sig.get_untracked()), copied, copy_gen))
                .child(dynamic(move || {
                  let dark = dark_mode.get();
                  if copied.get() { mica_icons::check_icon(copy_done_hex(dark)) } else { mica_icons::copy_icon(palette(dark).text_muted_hex) }
                })),
            )
            .child(
              button()
                .test_id("math-toggle-edit").accessibility_label("切换公式编辑模式")
                .w(px(22.0)).h(px(22.0)).rounded(px(5.0)).items_center().justify_center()
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).border_normal))
                .on_press(move || editing.set(!editing.get_untracked()))
                .child(dynamic(move || {
                  let p = palette(dark_mode.get());
                  if editing.get() { mica_icons::check_icon(p.accent_hex) } else { icon_pencil(p.text_muted_hex) }
                })),
            )
        },
      ))
      .child(when(
        move || editing.get(),
        move || {
          let s_c = session_in.clone();
          let gp_c = get_pos_in.clone();
          view().w_full().px_3().pb_3().child(
            input()
              .test_id("math-source-input").auto_focus(true)
              .submit_behavior(SubmitBehavior::Submit)
              .placeholder("输入 LaTeX 公式，例如 E = mc^2")
              .w_full().px_2().py_1().rounded(px(5.0))
              .border_1().border_color(move || palette(dark_mode.get()).border_normal)
              .text_size(px(13.0))
              .text_color(move || palette(dark_mode.get()).text_primary)
              .bg(move || palette(dark_mode.get()).surface_base)
              .value(move || read_node_source(&node_sig.get()))
              .on_change(move |v: &str| dispatch_node_source_update(&s_c, &gp_c, v))
              .on_submit(move |_: &str| editing.set(false)),
          )
        },
      ))
      .into_element()
  }

  fn update(&self, node: &Node, _decorations: &[Decoration]) -> bool { node.ty().name() == "math" }
  fn handle_event(&self, _event: &NodeViewEvent) -> bool { true }
}

// ---------------------------------------------------------------------------
// 3. ImageNodeView (real `image(src)`, fallback, hover toolbar, caption)
// ---------------------------------------------------------------------------

struct ImageNodeView {
  dark_mode: Signal<bool>,
}

fn read_image_src(node: &Node) -> String {
  ["src", "url"]
    .into_iter()
    .map(|k| read_str_attr(node, k))
    .find(|v| !v.trim().is_empty())
    .map(|v| v.trim().to_string())
    .unwrap_or_default()
}

fn read_image_alt(node: &Node) -> String {
  ["alt", "caption"]
    .into_iter()
    .map(|k| read_str_attr(node, k))
    .find(|v| !v.trim().is_empty())
    .unwrap_or_default()
}

impl NodeView for ImageNodeView {
  fn create(&self, cx: &NodeViewCx, initial_node: &Node) -> AnyElement {
    let dark_mode = self.dark_mode;
    let node_sig = cx.node();
    let hovered = signal(false);
    let load_failed = signal(false);
    let caption_dialog_open = signal(false);
    let caption_draft = signal(String::new());

    let local_align = signal({
      let a = read_str_attr(initial_node, "align");
      if a.is_empty() { "left".to_string() } else { a }
    });
    let local_alt = signal(read_image_alt(initial_node));

    let effective_align = computed(move || {
      let node = node_sig.get();
      if has_schema_attr(&node, "align") {
        let a = read_str_attr(&node, "align");
        if a.is_empty() { "left".to_string() } else { a }
      } else {
        local_align.get()
      }
    });
    let effective_alt = computed(move || {
      let node = node_sig.get();
      if has_schema_attr(&node, "alt") || has_schema_attr(&node, "caption") {
        read_image_alt(&node)
      } else {
        local_alt.get()
      }
    });

    let session_align = cx.session.clone();
    let get_pos_align = cx.get_pos();
    let session_cap = cx.session.clone();
    let get_pos_cap = cx.get_pos();
    let session_del = cx.session.clone();
    let get_pos_del = cx.get_pos();

    let bar_bg = Rgba { r: 0x19 as f32 / 255.0, g: 0x19 as f32 / 255.0, b: 0x19 as f32 / 255.0, a: 0.92 };
    let bar_hover = Rgba { r: 1.0, g: 1.0, b: 1.0, a: 0.14 };

    view()
      .test_id("node-view-image")
      .w_full().my_2()
      .child(dynamic(move || {
        let align = effective_align.get();
        let src = read_image_src(&node_sig.get());
        let p = palette(dark_mode.get());

        let picture = if !src.is_empty() && !load_failed.get() {
          image(ImageSrc::from(src.clone()))
            .test_id("image-node-picture")
            .max_w(relative(1.0)).min_h(px(64.0)).rounded(px(6.0))
            .on_error(move |_: &str| load_failed.set(true))
            .into_element()
        } else {
          view()
            .test_id("image-node-fallback")
            .w(px(320.0)).max_w(relative(1.0)).h(px(140.0)).rounded(px(6.0))
            .bg(p.surface_raised).items_center().justify_center().gap_2()
            .child(icon_image_frame(p.text_faint_hex))
            .child(
              text(if src.is_empty() { "暂无图片地址".to_string() } else { format!("图片加载失败 · {src}") })
                .text_size(px(12.0)).text_color(p.text_faint),
            )
            .into_element()
        };

        let (sa, gpa) = (session_align.clone(), get_pos_align.clone());
        let (sd, gpd) = (session_del.clone(), get_pos_del.clone());

        let img_box = view()
          .relative().max_w(relative(1.0)).rounded(px(6.0)).overflow_hidden()
          .on_mouse_enter(move |_: &MouseEvent| hovered.set(true))
          .on_mouse_move(move |_: &MouseEvent| {
            if !hovered.get_untracked() {
              hovered.set(true);
            }
          })
          .on_mouse_leave(move |_: &MouseEvent| hovered.set(false))
          .child(picture)
          .child(when(
            move || hovered.get(),
            move || {
              let (s1, gp1) = (sa.clone(), gpa.clone());
              let (s2, gp2) = (sa.clone(), gpa.clone());
              let (s3, gp3) = (sa.clone(), gpa.clone());
              let (sdel, gpdel) = (sd.clone(), gpd.clone());
              let pill_btn = |tid: &'static str, label: &'static str| {
                button()
                  .test_id(tid).accessibility_label(label)
                  .w(px(26.0)).h(px(26.0)).rounded(px(4.0))
                  .items_center().justify_center()
                  .hover(move |s| s.bg(bar_hover))
              };
              view()
                .test_id("image-hover-toolbar")
                .absolute().top(px(6.0)).right(px(6.0)).z_index(20)
                .flex_row().items_center().gap(px(2.0)).p(px(4.0)).rounded(px(7.0)).bg(bar_bg)
                .child(
                  pill_btn("image-align-left", "左对齐")
                    .on_press(move || { local_align.set("left".into()); dispatch_node_attr(&s1, &gp1, "align", Value::String("left".into())); })
                    .child(icon_align_left("#d4d4d4")),
                )
                .child(
                  pill_btn("image-align-center", "居中")
                    .on_press(move || { local_align.set("center".into()); dispatch_node_attr(&s2, &gp2, "align", Value::String("center".into())); })
                    .child(icon_align_center("#d4d4d4")),
                )
                .child(
                  pill_btn("image-align-right", "右对齐")
                    .on_press(move || { local_align.set("right".into()); dispatch_node_attr(&s3, &gp3, "align", Value::String("right".into())); })
                    .child(icon_align_right("#d4d4d4")),
                )
                .child(
                  pill_btn("image-edit-caption", "编辑说明")
                    .on_press(move || { caption_draft.set(effective_alt.get_untracked()); caption_dialog_open.set(true); })
                    .child(icon_title_t("#d4d4d4")),
                )
                .child(
                  pill_btn("image-delete-btn", "删除")
                    .on_press(move || dispatch_delete_node(&sdel, &gpdel))
                    .child(mica_icons::trash_icon("#fca5a5")),
                )
            },
          ));

        let container = match align.as_str() {
          "center" => view().w_full().items_center(),
          "right" => view().w_full().items_end(),
          _ => view().w_full().items_start(),
        };
        container.child(img_box).into_element()
      }))
      .child(when(
        move || caption_dialog_open.get(),
        move || {
          let s_save = session_cap.clone();
          let gp_save = get_pos_cap.clone();
          let on_save: Rc<dyn Fn(String)> = Rc::new(move |cleaned| {
            local_alt.set(cleaned.clone());
            dispatch_node_attr(&s_save, &gp_save, "alt", Value::String(cleaned));
            caption_dialog_open.set(false);
          });
          inline_prompt_dialog(
            "image-caption-dialog",
            "image-caption-input",
            "编辑说明",
            "输入图片说明文字…",
            caption_draft,
            dark_mode,
            move || caption_dialog_open.set(false),
            on_save,
          )
        },
      ))
      .child(when(
        move || !effective_alt.get().trim().is_empty(),
        move || {
          button()
            .test_id("image-caption")
            .w_full().h(px(24.0)).items_center().justify_center()
            .on_press(move || {
              caption_draft.set(effective_alt.get_untracked());
              caption_dialog_open.set(true);
            })
            .child(
              text(move || effective_alt.get())
                .text_size(px(12.0)).text_color(move || palette(dark_mode.get()).text_muted),
            )
        },
      ))
      .into_element()
  }

  fn update(&self, node: &Node, _decorations: &[Decoration]) -> bool { node.ty().name() == "image" }
  fn handle_event(&self, _event: &NodeViewEvent) -> bool { true }
}

// ---------------------------------------------------------------------------
// 4. QuoteNodeView (normal 3px quote bar + GFM Callout `note`/`tip`/...)
// ---------------------------------------------------------------------------

/// Metadata for a recognized GFM Callout alert type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalloutKind {
  Note,
  Tip,
  Important,
  Warning,
  Caution,
}

impl CalloutKind {
  /// Parses a case-insensitive GFM alert keyword (`note`, `tip`, `important`,
  /// `warning`, `caution`).
  pub fn parse(raw: &str) -> Option<Self> {
    match raw.trim().to_ascii_lowercase().as_str() {
      "note" => Some(Self::Note),
      "tip" => Some(Self::Tip),
      "important" => Some(Self::Important),
      "warning" => Some(Self::Warning),
      "caution" => Some(Self::Caution),
      _ => None,
    }
  }

  /// Standard GitHub English label shown in the 22px callout header strip.
  pub const fn label(self) -> &'static str {
    match self {
      Self::Note => "Note",
      Self::Tip => "Tip",
      Self::Important => "Important",
      Self::Warning => "Warning",
      Self::Caution => "Caution",
    }
  }

  fn accent(self, dark_mode: bool) -> (Rgba, &'static str) {
    let p = palette(dark_mode);
    match (self, dark_mode) {
      (Self::Note, false) => (p.alert_note, "#0969da"),
      (Self::Note, true) => (p.alert_note, "#58a6ff"),
      (Self::Tip, false) => (p.alert_tip, "#1a7f37"),
      (Self::Tip, true) => (p.alert_tip, "#3fb950"),
      (Self::Important, false) => (p.alert_important, "#8250df"),
      (Self::Important, true) => (p.alert_important, "#a371f7"),
      (Self::Warning, false) => (p.alert_warning, "#9a6700"),
      (Self::Warning, true) => (p.alert_warning, "#d29922"),
      (Self::Caution, false) => (p.alert_caution, "#cf222e"),
      (Self::Caution, true) => (p.alert_caution, "#f85149"),
    }
  }
}

struct QuoteNodeView {
  dark_mode: Signal<bool>,
}

impl NodeView for QuoteNodeView {
  fn create(&self, cx: &NodeViewCx, _node: &Node) -> AnyElement {
    let dark_mode = self.dark_mode;
    let node_sig = cx.node();
    let content = cx.take_content();
    let callout_kind = computed(move || CalloutKind::parse(&read_str_attr(&node_sig.get(), "alert")));

    view()
      .test_id("node-view-quote")
      .w_full().my_1()
      .border_l(px(3.0))
      .border_l_color(move || {
        let dark = dark_mode.get();
        match callout_kind.get() {
          Some(kind) => kind.accent(dark).0,
          None => palette(dark).quote_bar,
        }
      })
      .bg(move || {
        let dark = dark_mode.get();
        match callout_kind.get() {
          Some(kind) => Rgba { a: 0.06, ..kind.accent(dark).0 },
          None => TRANSPARENT_RGBA,
        }
      })
      .rounded_r(move || Length::from(px(if callout_kind.get().is_some() { 6.0 } else { 0.0 })))
      .pl(px(14.0))
      .pr(move || Length::from(px(if callout_kind.get().is_some() { 12.0 } else { 0.0 })))
      .py(move || Length::from(px(if callout_kind.get().is_some() { 8.0 } else { 2.0 })))
      .child(when(
        move || callout_kind.get().is_some(),
        move || {
          view()
            .test_id("quote-callout-header")
            .w_full().h(px(22.0)).mb(px(4.0))
            .flex_row().items_center().gap(px(6.0))
            .child(dynamic(move || {
              let kind = callout_kind.get().unwrap_or(CalloutKind::Note);
              let (_, hex) = kind.accent(dark_mode.get());
              match kind {
                CalloutKind::Note => mica_icons::info_icon(hex),
                CalloutKind::Tip => icon_lightbulb(hex),
                CalloutKind::Important => icon_important_badge(hex),
                CalloutKind::Warning => icon_warning_triangle(hex),
                CalloutKind::Caution => icon_caution_octagon(hex),
              }
            }))
            .child(
              text(move || callout_kind.get().map(CalloutKind::label).unwrap_or("Note"))
                .text_size(px(13.0)).font_weight(FontWeight::SEMIBOLD)
                .text_color(move || callout_kind.get().unwrap_or(CalloutKind::Note).accent(dark_mode.get()).0),
            )
        },
      ))
      .child(view().w_full().min_w(px(0.0)).children(content))
      .into_element()
  }

  fn update(&self, node: &Node, _decorations: &[Decoration]) -> bool {
    matches!(node.ty().name(), "quote" | "blockquote")
  }
  fn editable_content_slot(&self) -> bool { true }
}

// ---------------------------------------------------------------------------
// 5. MermaidNodeView (Merman SVG preview + hover toggle to source editor)
// ---------------------------------------------------------------------------

struct MermaidNodeView {
  dark_mode: Signal<bool>,
}

impl NodeView for MermaidNodeView {
  fn create(&self, cx: &NodeViewCx, _node: &Node) -> AnyElement {
    let dark_mode = self.dark_mode;
    let node_sig = cx.node();
    let editing = signal(false);
    let hovered = signal(false);
    let copied = signal(false);
    let copy_gen = signal(0u64);
    let session_ed = cx.session.clone();
    let get_pos_ed = cx.get_pos();

    view()
      .test_id("node-view-mermaid")
      .relative().w_full().my_2().rounded(px(6.0))
      .bg(move || palette(dark_mode.get()).surface_raised)
      .on_mouse_enter(move |_: &MouseEvent| hovered.set(true))
      .on_mouse_leave(move |_: &MouseEvent| hovered.set(false))
      .child(
        button()
          .test_id("mermaid-preview-btn").accessibility_label("点击编辑 Mermaid 图表")
          .w_full().p_3()
          .on_press(move || editing.set(!editing.get_untracked()))
          .child(dynamic(move || {
            render_atomic_block(
              &AtomicBlockInput {
                id: "mermaid-node".into(),
                kind: "mermaid".into(),
                text: read_node_source(&node_sig.get()),
                data: serde_json::json!({}),
              },
              dark_mode.get(),
            )
          })),
      )
      .child(when(
        move || hovered.get() || editing.get() || copied.get(),
        move || {
          view()
            .absolute().top(px(6.0)).right(px(8.0)).z_index(20)
            .flex_row().items_center().gap(px(4.0))
            .child(
              button()
                .test_id("mermaid-copy-btn").accessibility_label("复制 Mermaid 源码")
                .w(px(22.0)).h(px(22.0)).rounded(px(5.0)).items_center().justify_center()
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).border_normal))
                .on_press(move || copy_with_reset(read_node_source(&node_sig.get_untracked()), copied, copy_gen))
                .child(dynamic(move || {
                  let dark = dark_mode.get();
                  if copied.get() { mica_icons::check_icon(copy_done_hex(dark)) } else { mica_icons::copy_icon(palette(dark).text_muted_hex) }
                })),
            )
            .child(
              button()
                .test_id("mermaid-toggle-edit").accessibility_label("切换 Mermaid 源码编辑器")
                .px_2().h(px(22.0)).rounded(px(5.0)).items_center().justify_center()
                .bg(move || palette(dark_mode.get()).border_normal)
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).border_strong))
                .on_press(move || editing.set(!editing.get_untracked()))
                .child(
                  text(move || if editing.get() { "收起源码" } else { "编辑源码" })
                    .text_size(px(11.0)).font_weight(FontWeight::MEDIUM)
                    .text_color(move || palette(dark_mode.get()).text_primary),
                ),
            )
        },
      ))
      .child(when(
        move || editing.get(),
        move || {
          let s_c = session_ed.clone();
          let gp_c = get_pos_ed.clone();
          view()
            .w_full().px(px(14.0)).py(px(10.0))
            .border_t_1().border_color(move || palette(dark_mode.get()).border_subtle)
            .bg(move || palette(dark_mode.get()).code_bg)
            .child(
              editor()
                .test_id("mermaid-source-editor").accessibility_label("Mermaid 源码编辑器")
                .w_full()
                .value(move || Some(SharedString::from(read_node_source(&node_sig.get()))))
                .placeholder("graph LR\n  A[Mica] --> B[Pure Rust]")
                .line_numbers(false).soft_wrap(false).folding(false)
                .auto_indent(true).auto_close(true)
                .font_size(px(13.0)).line_height(px(20.0))
                .min_rows(Some(4u32)).max_rows(Some(18u32))
                .scroll_beyond_last_line(EditorScrollBeyond::None)
                .styles(move || code_editor_styles(dark_mode.get()))
                .on_change(move |v: &SharedString| dispatch_node_source_update(&s_c, &gp_c, v.as_ref())),
            )
        },
      ))
      .into_element()
  }

  fn update(&self, node: &Node, _decorations: &[Decoration]) -> bool { node.ty().name() == "mermaid" }
  fn handle_event(&self, _event: &NodeViewEvent) -> bool { true }
}

// ---------------------------------------------------------------------------
// 6. BoardNodeView (embedded `mica-board` SVG preview + block-aware open)
// ---------------------------------------------------------------------------

struct BoardNodeView {
  dark_mode: Signal<bool>,
}

impl NodeView for BoardNodeView {
  fn create(&self, cx: &NodeViewCx, initial_node: &Node) -> AnyElement {
    let dark_mode = self.dark_mode;
    let node_sig = cx.node();
    let selected_sig = cx.selected();
    let open_board = use_context::<OpenBoardCallback>();
    let open_board_block = use_context::<OpenBoardWithBlockCallback>();
    let get_pos = cx.get_pos();
    let session = cx.session.clone();

    let hovered = signal(false);
    let container_w = signal(720.0_f32);
    let local_width = signal(read_f32_attr(initial_node, "width"));
    let local_height = signal(read_f32_attr(initial_node, "height"));
    let resizing_size: Signal<Option<(f32, f32)>> = signal(None);
    let drag_start = signal((0.0_f32, 0.0_f32, 720.0_f32, 360.0_f32));
    let last_handle_click_ms = signal(0.0_f64);

    let effective_width = computed(move || {
      let node = node_sig.get();
      let max_w = container_w.get().max(240.0);
      if let Some((w, _)) = resizing_size.get() {
        return w.clamp(220.0, max_w);
      }
      local_width
        .get()
        .or_else(|| read_f32_attr(&node, "width"))
        .map(|w| w.clamp(220.0, max_w))
        .unwrap_or(max_w)
    });

    let effective_height = computed(move || {
      let node = node_sig.get();
      if let Some((_, h)) = resizing_size.get() {
        return h.clamp(180.0, 960.0);
      }
      if let Some(h) = local_height.get().or_else(|| read_f32_attr(&node, "height")) {
        return h.clamp(180.0, 960.0);
      }
      let w = effective_width.get();
      let aspect = crate::board_view::board_natural_aspect(&read_node_source(&node));
      (w * aspect).clamp(320.0, 540.0)
    });

    let has_custom_size = computed(move || {
      let node = node_sig.get();
      local_width.get().is_some()
        || local_height.get().is_some()
        || read_f32_attr(&node, "width").is_some()
        || read_f32_attr(&node, "height").is_some()
    });

    let open_board_click: Rc<dyn Fn()> = {
      let open_board_block = open_board_block.clone();
      let open_board = open_board.clone();
      let get_pos = get_pos.clone();
      Rc::new(move || {
        if let (Some(cb), Some(pos)) = (&open_board_block, get_pos()) {
          (cb.0)(pos, read_node_source(&node_sig.get_untracked()));
        } else if let Some(cb) = &open_board {
          (cb.0)();
        }
      })
    };
    let open_board_btn = open_board_click.clone();

    let commit_size: Rc<dyn Fn(Option<f32>, Option<f32>)> = {
      let s = session.clone();
      let gp = get_pos.clone();
      Rc::new(move |w_opt, h_opt| {
        local_width.set(w_opt);
        local_height.set(h_opt);
        dispatch_node_attr(
          &s,
          &gp,
          "width",
          w_opt.map(|v| Value::from(v.round() as i64)).unwrap_or(Value::Null),
        );
        dispatch_node_attr(
          &s,
          &gp,
          "height",
          h_opt.map(|v| Value::from(v.round() as i64)).unwrap_or(Value::Null),
        );
      })
    };
    let commit_reset = commit_size.clone();
    let commit_bottom = commit_size.clone();
    let commit_right = commit_size.clone();
    let commit_corner = commit_size;

    view()
      .test_id("node-view-board")
      .w_full()
      .my_2()
      .on_layout(move |b: &Bounds<Points>| {
        let w = b.size.width.0;
        if w > 120.0 && (container_w.get_untracked() - w).abs() > 2.0 {
          container_w.set(w);
        }
      })
      .child(
        // Single borderless-header card sized to `effective_width × effective_height`
        // (no permanent gray header strip; hover-only edit pill + draggable resize handles).
        view()
          .relative()
          .w(move || {
            let max_w = container_w.get().max(240.0);
            let ew = effective_width.get();
            if (max_w - ew).abs() <= 2.0 {
              Length::from(relative(1.0))
            } else {
              Length::from(px(ew))
            }
          })
          .h(move || Length::from(px(effective_height.get())))
          .rounded(px(8.0))
          .border_1()
          .border_color(move || {
            let p = palette(dark_mode.get());
            if selected_sig.get() || resizing_size.get().is_some() {
              p.accent
            } else {
              p.border_normal
            }
          })
          .bg(move || palette(dark_mode.get()).surface_base)
          .overflow_hidden()
          .on_mouse_enter(move |_: &MouseEvent| hovered.set(true))
          .on_mouse_leave(move |_: &MouseEvent| hovered.set(false))
          .child(
            // Edge-to-edge clickable canvas preview (no top gray strip, no double border).
            button()
              .test_id("board-preview-open-btn")
              .accessibility_label("点击画板进入编辑")
              .w_full()
              .h_full()
              .on_press(move || open_board_click())
              .child(dynamic(move || {
                render_atomic_block(
                  &AtomicBlockInput {
                    id: "board-node".into(),
                    kind: "board".into(),
                    text: read_node_source(&node_sig.get()),
                    data: serde_json::json!({}),
                  },
                  dark_mode.get(),
                )
              })),
          )
          // Hover-only top-right floating controls (`编辑白板` + optional `重置尺寸`)
          .child(when(
            move || hovered.get() || selected_sig.get(),
            move || {
              let open_btn = open_board_btn.clone();
              let reset_cb = commit_reset.clone();
              view()
                .absolute()
                .top(px(8.0))
                .right(px(8.0))
                .z_index(20)
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(when(move || has_custom_size.get(), {
                  let rcb = reset_cb.clone();
                  move || {
                    let rcb = rcb.clone();
                    button()
                      .test_id("board-reset-size-btn")
                      .accessibility_label("重置画板尺寸为自动适配")
                      .px_2()
                      .py_1()
                      .rounded(px(6.0))
                      .bg(move || palette(dark_mode.get()).surface_overlay)
                      .border_1()
                      .border_color(move || palette(dark_mode.get()).border_normal)
                      .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                      .on_press(move || rcb(None, None))
                      .child(
                        text("自适应")
                          .text_size(px(12.0))
                          .text_color(move || palette(dark_mode.get()).text_muted),
                      )
                  }
                }))
                .child(
                  button()
                    .test_id("board-open-full-btn")
                    .accessibility_label("编辑内嵌白板")
                    .px_2()
                    .py_1()
                    .rounded(px(6.0))
                    .flex_row()
                    .items_center()
                    .gap(px(5.0))
                    .bg(move || palette(dark_mode.get()).surface_overlay)
                    .border_1()
                    .border_color(move || palette(dark_mode.get()).border_normal)
                    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                    .on_press(move || open_btn())
                    .child(dynamic(move || {
                      mica_icons::whiteboard(palette(dark_mode.get()).text_muted_hex)
                    }))
                    .child(
                      text("编辑白板")
                        .text_size(px(12.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || palette(dark_mode.get()).text_primary),
                    ),
                )
            },
          ))
          // Bottom edge resize handle (drag up/down to stretch/shrink height; double-click resets)
          .child({
            let cb_b = commit_bottom.clone();
            view()
              .test_id("board-resize-bottom")
              .absolute()
              .left(px(16.0))
              .right(px(16.0))
              .bottom(px(0.0))
              .h(px(10.0))
              .z_index(25)
              .items_center()
              .justify_center()
              .cursor(Cursor::RowResize)
              .claim_touches(TouchClaim::on_start())
              .keep_touches(true)
              .on_responder_grant(move |e: &ResponderEvent| {
                let w = effective_width.get_untracked();
                let h = effective_height.get_untracked();
                drag_start.set((f32::from(e.page.x), f32::from(e.page.y), w, h));
                resizing_size.set(Some((w, h)));
              })
              .on_responder_move(move |e: &ResponderEvent| {
                let (_, sy, sw, sh) = drag_start.get_untracked();
                let dy = f32::from(e.page.y) - sy;
                resizing_size.set(Some((sw, (sh + dy).clamp(180.0, 960.0))));
              })
              .on_responder_release(move |e: &ResponderEvent| {
                let (_, sy, _, sh) = drag_start.get_untracked();
                let moved = (f32::from(e.page.y) - sy).abs();
                let fin = resizing_size.get_untracked();
                resizing_size.set(None);
                if moved < 4.0 {
                  let now = e.timestamp;
                  let prev = last_handle_click_ms.get_untracked();
                  if prev > 0.0 && now >= prev && (now - prev) <= 350.0 {
                    last_handle_click_ms.set(0.0);
                    cb_b(None, None);
                  } else {
                    last_handle_click_ms.set(now);
                  }
                  return;
                }
                if let Some((_, nh)) = fin && (nh - sh).abs() > 1.0 {
                  cb_b(local_width.get_untracked(), Some(nh));
                }
              })
              .on_responder_terminate(move |_: &ResponderEvent| resizing_size.set(None))
              .child(
                view()
                  .w(px(40.0))
                  .h(px(4.0))
                  .rounded(px(2.0))
                  .bg(move || {
                    let p = palette(dark_mode.get());
                    if resizing_size.get().is_some() {
                      p.accent
                    } else if hovered.get() {
                      p.border_strong
                    } else {
                      TRANSPARENT_RGBA
                    }
                  }),
              )
          })
          // Right edge resize handle (drag left/right to scale width & height proportionally)
          .child({
            let cb_r = commit_right.clone();
            view()
              .test_id("board-resize-right")
              .absolute()
              .top(px(16.0))
              .bottom(px(16.0))
              .right(px(0.0))
              .w(px(10.0))
              .z_index(25)
              .items_center()
              .justify_center()
              .cursor(Cursor::ColResize)
              .claim_touches(TouchClaim::on_start())
              .keep_touches(true)
              .on_responder_grant(move |e: &ResponderEvent| {
                let w = effective_width.get_untracked();
                let h = effective_height.get_untracked();
                drag_start.set((f32::from(e.page.x), f32::from(e.page.y), w, h));
                resizing_size.set(Some((w, h)));
              })
              .on_responder_move(move |e: &ResponderEvent| {
                let (sx, _, sw, sh) = drag_start.get_untracked();
                let dx = f32::from(e.page.x) - sx;
                let max_w = container_w.get_untracked().max(240.0);
                let nw = (sw + dx).clamp(220.0, max_w);
                let aspect = (sh / sw.max(1.0)).clamp(0.25, 2.5);
                let nh = (nw * aspect).clamp(180.0, 960.0);
                resizing_size.set(Some((nw, nh)));
              })
              .on_responder_release(move |e: &ResponderEvent| {
                let (sx, _, sw, _) = drag_start.get_untracked();
                let moved = (f32::from(e.page.x) - sx).abs();
                let fin = resizing_size.get_untracked();
                resizing_size.set(None);
                if moved < 4.0 {
                  let now = e.timestamp;
                  let prev = last_handle_click_ms.get_untracked();
                  if prev > 0.0 && now >= prev && (now - prev) <= 350.0 {
                    last_handle_click_ms.set(0.0);
                    cb_r(None, None);
                  } else {
                    last_handle_click_ms.set(now);
                  }
                  return;
                }
                if let Some((nw, nh)) = fin && (nw - sw).abs() > 1.0 {
                  let max_w = container_w.get_untracked().max(240.0);
                  let persisted_w = if (max_w - nw).abs() <= 4.0 { None } else { Some(nw) };
                  cb_r(persisted_w, Some(nh));
                }
              })
              .on_responder_terminate(move |_: &ResponderEvent| resizing_size.set(None))
              .child(
                view()
                  .w(px(4.0))
                  .h(px(36.0))
                  .rounded(px(2.0))
                  .bg(move || {
                    let p = palette(dark_mode.get());
                    if resizing_size.get().is_some() {
                      p.accent
                    } else if hovered.get() {
                      p.border_strong
                    } else {
                      TRANSPARENT_RGBA
                    }
                  }),
              )
          })
          // Bottom-right corner resize handle (free width & height drag)
          .child({
            let cb_c = commit_corner.clone();
            view()
              .test_id("board-resize-corner")
              .absolute()
              .right(px(0.0))
              .bottom(px(0.0))
              .w(px(16.0))
              .h(px(16.0))
              .z_index(28)
              .items_center()
              .justify_center()
              .cursor(Cursor::NwseResize)
              .claim_touches(TouchClaim::on_start())
              .keep_touches(true)
              .on_responder_grant(move |e: &ResponderEvent| {
                let w = effective_width.get_untracked();
                let h = effective_height.get_untracked();
                drag_start.set((f32::from(e.page.x), f32::from(e.page.y), w, h));
                resizing_size.set(Some((w, h)));
              })
              .on_responder_move(move |e: &ResponderEvent| {
                let (sx, sy, sw, sh) = drag_start.get_untracked();
                let max_w = container_w.get_untracked().max(240.0);
                let nw = (sw + (f32::from(e.page.x) - sx)).clamp(220.0, max_w);
                let nh = (sh + (f32::from(e.page.y) - sy)).clamp(180.0, 960.0);
                resizing_size.set(Some((nw, nh)));
              })
              .on_responder_release(move |e: &ResponderEvent| {
                let (sx, sy, _, _) = drag_start.get_untracked();
                let moved = (f32::from(e.page.x) - sx).hypot(f32::from(e.page.y) - sy);
                let fin = resizing_size.get_untracked();
                resizing_size.set(None);
                if moved < 4.0 {
                  let now = e.timestamp;
                  let prev = last_handle_click_ms.get_untracked();
                  if prev > 0.0 && now >= prev && (now - prev) <= 350.0 {
                    last_handle_click_ms.set(0.0);
                    cb_c(None, None);
                  } else {
                    last_handle_click_ms.set(now);
                  }
                  return;
                }
                if let Some((nw, nh)) = fin {
                  let max_w = container_w.get_untracked().max(240.0);
                  let persisted_w = if (max_w - nw).abs() <= 4.0 { None } else { Some(nw) };
                  cb_c(persisted_w, Some(nh));
                }
              })
              .on_responder_terminate(move |_: &ResponderEvent| resizing_size.set(None))
              .child(
                view()
                  .w(px(8.0))
                  .h(px(8.0))
                  .rounded(px(4.0))
                  .border_1()
                  .border_color(move || {
                    let p = palette(dark_mode.get());
                    if hovered.get() || resizing_size.get().is_some() { p.accent } else { TRANSPARENT_RGBA }
                  })
                  .bg(move || {
                    let p = palette(dark_mode.get());
                    if resizing_size.get().is_some() {
                      p.accent
                    } else if hovered.get() {
                      p.surface_base
                    } else {
                      TRANSPARENT_RGBA
                    }
                  }),
              )
          })
          // Live `W × H` pill badge shown while actively dragging resize handles
          .child(when(
            move || resizing_size.get().is_some(),
            move || {
              view()
                .test_id("board-resize-readout")
                .absolute()
                .right(px(16.0))
                .bottom(px(16.0))
                .z_index(30)
                .px_2()
                .py_1()
                .rounded(px(5.0))
                .bg(move || palette(dark_mode.get()).text_primary)
                .pointer_events(PointerEvents::None)
                .child(
                  text(move || {
                    let (w, h) = resizing_size.get().unwrap_or((0.0, 0.0));
                    format!("{} × {}", w.round() as i32, h.round() as i32)
                  })
                  .text_size(px(11.5))
                  .font_weight(FontWeight::MEDIUM)
                  .text_color(move || palette(dark_mode.get()).surface_base),
                )
            },
          )),
      )
      .into_element()
  }

  fn update(&self, node: &Node, _decorations: &[Decoration]) -> bool { node.ty().name() == "board" }
  fn handle_event(&self, _event: &NodeViewEvent) -> bool { true }
}

// ---------------------------------------------------------------------------
// 7. DividerNodeView, 8. TodoNodeView, 9. FlatListNodeView
// ---------------------------------------------------------------------------

struct DividerNodeView {
  dark_mode: Signal<bool>,
}

impl NodeView for DividerNodeView {
  fn create(&self, cx: &NodeViewCx, _node: &Node) -> AnyElement {
    let dark_mode = self.dark_mode;
    let selected_sig = cx.selected();
    view()
      .test_id("node-view-divider")
      .w_full().py_4().justify_center()
      .child(view().w_full().h(px(1.5)).bg(move || {
        let p = palette(dark_mode.get());
        if selected_sig.get() { p.accent } else { p.quote_bar }
      }))
      .into_element()
  }

  fn update(&self, node: &Node, _decorations: &[Decoration]) -> bool {
    matches!(node.ty().name(), "divider" | "horizontalRule")
  }
}

struct TodoNodeView {
  dark_mode: Signal<bool>,
}

impl NodeView for TodoNodeView {
  fn create(&self, cx: &NodeViewCx, node: &Node) -> AnyElement {
    let dark_mode = self.dark_mode;
    let node_sig = cx.node();
    let content = cx.take_content();

    if node.ty().spec().extra.get("render").and_then(Value::as_str) == Some("task") {
      return view().w_full().children(content).into_element();
    }

    let checked = computed(move || read_bool_attr(&node_sig.get(), "checked", false));
    let session_cb = cx.session.clone();
    let get_pos_cb = cx.get_pos();

    view()
      .test_id("node-view-todo")
      .w_full().py_1().flex_row().items_start().gap_2()
      .child(
        button()
          .test_id("todo-checkbox-btn").accessibility_label("切换待办完成状态")
          .w(px(22.0)).h(px(24.0)).items_center().justify_center().rounded(px(4.0))
          .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
          .on_press(move || dispatch_node_attr(&session_cb, &get_pos_cb, "checked", Value::Bool(!checked.get_untracked())))
          .child(
            text(move || if checked.get() { "☑" } else { "☐" })
              .text_size(px(16.0))
              .text_color(move || { let p = palette(dark_mode.get()); if checked.get() { p.accent } else { p.text_muted } }),
          ),
      )
      .child(
        view().flex_1().min_w(px(0.0))
          .opacity(move || if checked.get() { 0.6_f32 } else { 1.0_f32 })
          .children(content),
      )
      .into_element()
  }

  fn update(&self, node: &Node, _decorations: &[Decoration]) -> bool {
    matches!(node.ty().name(), "todo" | "taskItem")
  }
  fn editable_content_slot(&self) -> bool { true }
}

struct FlatListNodeView {
  dark_mode: Signal<bool>,
  ordered: bool,
}

fn read_indent(node: &Node) -> u64 {
  node.attrs().get("indent").and_then(Value::as_u64).unwrap_or(0).min(6)
}

fn compute_ordered_index(session: &EditorSession, pos: Option<usize>, indent: u64) -> usize {
  let Some(target_pos) = pos else { return 1 };
  let state = session.state();
  let doc = state.doc();
  let mut cursor = 0usize;
  let mut run = 0usize;
  for i in 0..doc.child_count() {
    let child = doc.child(i);
    let node_pos = cursor;
    cursor += child.node_size();
    let child_indent = read_indent(child);
    if child.ty().name() == "numbered_list" {
      if child_indent == indent { run += 1; } else if child_indent < indent { run = 0; }
    } else if child_indent <= indent {
      run = 0;
    }
    if node_pos == target_pos {
      return run.max(1);
    }
  }
  1
}

impl NodeView for FlatListNodeView {
  fn create(&self, cx: &NodeViewCx, _node: &Node) -> AnyElement {
    let dark_mode = self.dark_mode;
    let ordered = self.ordered;
    let node_sig = cx.node();
    let content = cx.take_content();
    let session = cx.session.clone();
    let get_pos = cx.get_pos();

    let indent_px = move || px((read_indent(&node_sig.get()) as f32) * 24.0);
    let marker = move || {
      let indent = read_indent(&node_sig.get());
      if ordered {
        format!("{}.", compute_ordered_index(&session, get_pos(), indent))
      } else {
        match indent % 3 { 0 => "•", 1 => "◦", _ => "▪" }.to_string()
      }
    };

    view()
      .w_full().pl(indent_px).flex_row().items_start().gap_2()
      .child(
        view().w(px(20.0)).pt(px(2.0)).items_end()
          .child(text(marker).text_size(px(14.0)).text_color(move || palette(dark_mode.get()).text_muted)),
      )
      .child(view().flex_1().min_w(px(0.0)).children(content))
      .into_element()
  }

  fn update(&self, node: &Node, _decorations: &[Decoration]) -> bool {
    node.ty().name() == if self.ordered { "numbered_list" } else { "bulleted_list" }
  }
  fn editable_content_slot(&self) -> bool { true }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn language_picker_pins_auto_and_sorts_rest() {
    let all = filter_code_languages("");
    assert_eq!(all.first().map(String::as_str), Some("auto"));
    let rest = &all[1..];
    let mut sorted = rest.to_vec();
    sorted.sort();
    assert_eq!(rest, sorted.as_slice());
    assert!(rest.contains(&"rust".to_string()));
    assert!(rest.contains(&"mermaid".to_string()));

    let rust_matches = filter_code_languages("ru");
    assert!(rust_matches.contains(&"rust".to_string()));
    assert!(rust_matches.contains(&"ruby".to_string()));
    assert!(filter_code_languages("zzzz-nonexistent").is_empty());
  }

  #[test]
  fn language_display_and_picker_resolution() {
    assert_eq!(picker_current_language(""), "auto");
    assert_eq!(picker_current_language("rs"), "rust");
    assert_eq!(picker_current_language("py"), "python");

    assert_eq!(display_code_language("rs", ""), "rust");
    assert_eq!(display_code_language("mica-board", "{}"), "mica-board");
    assert_eq!(display_code_language("", ""), "auto");
    assert_eq!(
      display_code_language("auto", "use std::io;\nfn main() { println!(\"hi\"); }"),
      "rust"
    );
  }

  #[test]
  fn callout_kind_parsing_and_labels() {
    assert_eq!(CalloutKind::parse("note"), Some(CalloutKind::Note));
    assert_eq!(CalloutKind::parse("TIP"), Some(CalloutKind::Tip));
    assert_eq!(CalloutKind::parse("Important"), Some(CalloutKind::Important));
    assert_eq!(CalloutKind::parse("warning"), Some(CalloutKind::Warning));
    assert_eq!(CalloutKind::parse("caution"), Some(CalloutKind::Caution));
    assert_eq!(CalloutKind::parse(""), None);
    assert_eq!(CalloutKind::parse("unknown"), None);

    assert_eq!(CalloutKind::Note.label(), "Note");
    assert_eq!(CalloutKind::Tip.label(), "Tip");
    assert_eq!(CalloutKind::Important.label(), "Important");
    assert_eq!(CalloutKind::Warning.label(), "Warning");
    assert_eq!(CalloutKind::Caution.label(), "Caution");
  }
}
