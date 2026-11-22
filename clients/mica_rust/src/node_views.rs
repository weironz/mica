//! Custom `nsh_base::rich_editor::view::NodeView` implementations for Mica's
//! pure-Rust L3 shell:
//! - `code_block` / `codeBlock`: real editable code block via `nsh_base::editor()`
//!   with language switcher, copy button, and live toggleable SVG preview when
//!   `language` is `"mermaid"` or `"mica-board"`.
//! - `math`: RaTeX SVG preview via `atomic_blocks` + click-to-edit LaTeX input.
//! - `mermaid`: Merman SVG preview + click-to-edit Mermaid source editor.
//! - `board`: embedded `mica-board` SVG preview + open-full-whiteboard button.
//! - `divider` / `horizontalRule`: crisp warm-gray 1px hairline rule.
//! - `todo` / `taskItem`: interactive checkbox (`☐` / `☑`) + editable content slot.

use std::{rc::Rc, sync::Arc};

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
  element::{Style, set_clipboard},
  geometry::px,
  prelude::*,
};
use serde_json::Value;

use crate::{
  atomic_blocks::{AtomicBlockInput, render_atomic_block},
  mica_icons,
  mica_theme::palette,
};

const CODE_LANGUAGES: &[&str] = &[
  "rust",
  "typescript",
  "python",
  "sql",
  "bash",
  "json",
  "mermaid",
  "text",
];

/// Reactive context handle allowing embedded `board` node views to open the
/// workspace's full whiteboard view while keeping every `NodeView: Send + Sync`.
#[derive(Clone)]
pub struct OpenBoardCallback(pub Rc<dyn Fn()>);

/// Builds the full `NodeViews` registry for Mica's rich document editor.
pub fn build_mica_node_views(dark_mode: Signal<bool>) -> NodeViews {
  let code_view: Arc<dyn NodeView> = Arc::new(CodeBlockNodeView { dark_mode });
  let math_view: Arc<dyn NodeView> = Arc::new(MathNodeView { dark_mode });
  let mermaid_view: Arc<dyn NodeView> = Arc::new(MermaidNodeView { dark_mode });
  let board_view: Arc<dyn NodeView> = Arc::new(BoardNodeView { dark_mode });
  let divider_view: Arc<dyn NodeView> = Arc::new(DividerNodeView { dark_mode });
  let todo_view: Arc<dyn NodeView> = Arc::new(TodoNodeView { dark_mode });
  let bullet_view: Arc<dyn NodeView> = Arc::new(FlatListNodeView {
    dark_mode,
    ordered: false,
  });
  let ordered_view: Arc<dyn NodeView> = Arc::new(FlatListNodeView {
    dark_mode,
    ordered: true,
  });

  let mut views = NodeViews::new();
  views
    .insert("code_block", code_view.clone())
    .insert("codeBlock", code_view)
    .insert("math", math_view)
    .insert("mermaid", mermaid_view)
    .insert("board", board_view)
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
  node
    .attrs()
    .get("language")
    .and_then(Value::as_str)
    .map(str::trim)
    .filter(|s| !s.is_empty())
    .unwrap_or("text")
    .to_string()
}

fn editor_language_for(lang: &str) -> &str {
  if lang.eq_ignore_ascii_case("text") || lang.eq_ignore_ascii_case("plain") {
    ""
  } else if lang.eq_ignore_ascii_case("mica-board") {
    "json"
  } else if lang.eq_ignore_ascii_case("mermaid") {
    ""
  } else {
    lang
  }
}

fn dispatch_node_source_update(
  session: &EditorSession,
  get_pos: &Rc<dyn Fn() -> Option<usize>>,
  new_text: &str,
) {
  let Some(pos) = get_pos() else {
    return;
  };
  let state = session.state();
  let Some(node) = state.doc().node_at(pos) else {
    return;
  };
  if read_node_source(&node) == new_text {
    return;
  }
  let mut tr = state.tr();
  if node.is_leaf() || !node.inline_content() {
    let attr_key = if node.ty().spec().attrs.iter().any(|(k, _)| k == "source") {
      "source"
    } else if node.ty().spec().attrs.iter().any(|(k, _)| k == "text") {
      "text"
    } else if node.ty().spec().attrs.iter().any(|(k, _)| k == "code") {
      "code"
    } else {
      "source"
    };
    if tr
      .set_node_attribute(pos, attr_key, Value::from(new_text))
      .is_ok()
    {
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
  let Some(pos) = get_pos() else {
    return;
  };
  let state = session.state();
  let mut tr = state.tr();
  if tr.set_node_attribute(pos, attr, value).is_ok() {
    let _ = session.dispatch(tr);
  }
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
    num_prop(StyleKey::PaddingLeft, 8.0),
    num_prop(StyleKey::PaddingRight, 8.0),
  ])));
  styles.line_number = Some(PartStyle(decl_style([color_prop(
    StyleKey::Color,
    p.text_ghost,
  )])));
  styles.active_line_number = Some(PartStyle(decl_style([color_prop(
    StyleKey::Color,
    p.text_muted,
  )])));
  styles.content = Some(PartStyle(decl_style([
    color_prop(StyleKey::BackgroundColor, p.code_bg),
    color_prop(StyleKey::Color, p.text_primary),
    num_prop(StyleKey::PaddingTop, 8.0),
    num_prop(StyleKey::PaddingBottom, 8.0),
    num_prop(StyleKey::PaddingRight, 12.0),
  ])));
  styles.active_line = Some(PartStyle(decl_style([color_prop(
    StyleKey::BackgroundColor,
    p.surface_hover,
  )])));
  styles.selection = Some(PartStyle(decl_style([color_prop(
    StyleKey::BackgroundColor,
    p.selection_bg,
  )])));
  styles.caret = Some(PartStyle(decl_style([color_prop(
    StyleKey::BackgroundColor,
    p.text_primary,
  )])));
  styles.placeholder = Some(PartStyle(decl_style([color_prop(
    StyleKey::Color,
    p.text_ghost,
  )])));
  styles
}

// ---------------------------------------------------------------------------
// 1. CodeBlockNodeView (real `nsh_base::editor()` + language switcher + preview)
// ---------------------------------------------------------------------------

struct CodeBlockNodeView {
  dark_mode: Signal<bool>,
}

impl NodeView for CodeBlockNodeView {
  fn create(&self, cx: &NodeViewCx, _node: &Node) -> AnyElement {
    let dark_mode = self.dark_mode;
    let node_sig = cx.node();
    let selected_sig = cx.selected();
    let show_preview = signal(true);
    let copied = signal(false);

    let session_ed = cx.session.clone();
    let get_pos_ed = cx.get_pos();

    let mut lang_bar = view().flex_row().flex_wrap().items_center().gap_1();
    for &lang in CODE_LANGUAGES {
      let session_l = cx.session.clone();
      let get_pos_l = cx.get_pos();
      let is_active = computed(move || read_code_language(&node_sig.get()).eq_ignore_ascii_case(lang));
      lang_bar = lang_bar.child(
        button()
          .test_id(&format!("code-lang-{lang}"))
          .accessibility_label(format!("切换代码语言为 {lang}"))
          .px_2()
          .py_1()
          .rounded(px(4.0))
          .bg(move || {
            let p = palette(dark_mode.get());
            if is_active.get() {
              p.surface_hover
            } else {
              p.surface_raised
            }
          })
          .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
          .on_press(move || {
            dispatch_node_attr(&session_l, &get_pos_l, "language", Value::from(lang));
          })
          .child(
            text(lang)
              .text_size(px(11.0))
              .font_weight(FontWeight::MEDIUM)
              .text_color(move || {
                let p = palette(dark_mode.get());
                if is_active.get() {
                  p.text_primary
                } else {
                  p.text_faint
                }
              }),
          ),
      );
    }

    let is_previewable = computed(move || {
      let lang = read_code_language(&node_sig.get());
      lang.eq_ignore_ascii_case("mermaid") || lang.eq_ignore_ascii_case("mica-board")
    });

    view()
      .test_id("node-view-code-block")
      .w_full()
      .my_2()
      .rounded(px(8.0))
      .border_1()
      .border_color(move || {
        let p = palette(dark_mode.get());
        if selected_sig.get() {
          p.accent
        } else {
          p.border_normal
        }
      })
      .bg(move || palette(dark_mode.get()).code_bg)
      .overflow_hidden()
      .child(
        // Quiet code header bar
        view()
          .w_full()
          .min_h(px(34.0))
          .px_3()
          .py_1()
          .flex_row()
          .items_center()
          .justify_between()
          .gap_2()
          .border_b_1()
          .border_color(move || palette(dark_mode.get()).border_subtle)
          .bg(move || palette(dark_mode.get()).surface_raised)
          .child(lang_bar)
          .child(
            view()
              .flex_shrink_0()
              .flex_row()
              .items_center()
              .gap_1()
              .child(when(
                move || is_previewable.get(),
                move || {
                  button()
                    .test_id("code-preview-toggle")
                    .accessibility_label("切换实时图表预览")
                    .px_2()
                    .py_1()
                    .rounded(px(4.0))
                    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                    .on_press(move || show_preview.set(!show_preview.get_untracked()))
                    .child(
                      text(move || {
                        if show_preview.get() {
                          "隐藏预览"
                        } else {
                          "显示预览"
                        }
                      })
                      .text_size(px(11.0))
                      .text_color(move || palette(dark_mode.get()).text_muted),
                    )
                },
              ))
              .child(
                button()
                  .test_id("code-copy-btn")
                  .accessibility_label("复制代码块内容")
                  .px_2()
                  .py_1()
                  .rounded(px(4.0))
                  .flex_row()
                  .items_center()
                  .gap_1()
                  .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                  .on_press(move || {
                    let src = read_node_source(&node_sig.get_untracked());
                    set_clipboard(src);
                    copied.set(true);
                  })
                  .child(dynamic(move || {
                    let p = palette(dark_mode.get());
                    if copied.get() {
                      mica_icons::check_icon(p.accent_hex)
                    } else {
                      mica_icons::copy_icon(p.text_muted_hex)
                    }
                  }))
                  .child(
                    text(move || if copied.get() { "已复制" } else { "复制" })
                      .text_size(px(11.0))
                      .text_color(move || palette(dark_mode.get()).text_muted),
                  ),
              ),
          ),
      )
      .child(
        // Real pure-Rust editable code editor (`nsh_base::editor()`)
        editor()
          .test_id("code-block-editor")
          .accessibility_label("代码块编辑器")
          .w_full()
          .value(move || Some(SharedString::from(read_node_source(&node_sig.get()))))
          .language(move || {
            let lang = read_code_language(&node_sig.get());
            Some(SharedString::from(editor_language_for(&lang).to_string()))
          })
          .placeholder("输入代码...")
          .line_numbers(true)
          .soft_wrap(true)
          .auto_indent(true)
          .auto_close(true)
          .folding(true)
          .font_size(px(13.0))
          .line_height(px(20.0))
          .min_rows(Some(3u32))
          .max_rows(Some(28u32))
          .scroll_beyond_last_line(EditorScrollBeyond::None)
          .styles(move || code_editor_styles(dark_mode.get()))
          .on_change(move |val: &SharedString| {
            copied.set(false);
            dispatch_node_source_update(&session_ed, &get_pos_ed, val.as_ref());
          }),
      )
      .child(when(
        move || is_previewable.get() && show_preview.get(),
        move || {
          view()
            .w_full()
            .p_3()
            .border_t_1()
            .border_color(move || palette(dark_mode.get()).border_subtle)
            .bg(move || palette(dark_mode.get()).surface_base)
            .child(dynamic(move || {
              let node = node_sig.get();
              let lang = read_code_language(&node);
              let src = read_node_source(&node);
              render_atomic_block(
                &AtomicBlockInput {
                  id: "code-preview".into(),
                  kind: "code_block".into(),
                  text: src,
                  data: serde_json::json!({ "language": lang }),
                },
                dark_mode.get(),
              )
            }))
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
// 2. MathNodeView (RaTeX SVG preview + click-to-toggle LaTeX source input)
// ---------------------------------------------------------------------------

struct MathNodeView {
  dark_mode: Signal<bool>,
}

impl NodeView for MathNodeView {
  fn create(&self, cx: &NodeViewCx, _node: &Node) -> AnyElement {
    let dark_mode = self.dark_mode;
    let node_sig = cx.node();
    let selected_sig = cx.selected();
    let editing = signal(false);

    let session_in = cx.session.clone();
    let get_pos_in = cx.get_pos();

    view()
      .test_id("node-view-math")
      .w_full()
      .my_2()
      .rounded(px(8.0))
      .border_1()
      .border_color(move || {
        let p = palette(dark_mode.get());
        if selected_sig.get() || editing.get() {
          p.accent
        } else {
          p.border_normal
        }
      })
      .bg(move || palette(dark_mode.get()).surface_raised)
      .overflow_hidden()
      .child(
        view()
          .w_full()
          .h(px(32.0))
          .px_3()
          .flex_row()
          .items_center()
          .justify_between()
          .border_b_1()
          .border_color(move || palette(dark_mode.get()).border_subtle)
          .child(
            text("LaTeX 公式")
              .text_size(px(11.5))
              .font_weight(FontWeight::MEDIUM)
              .text_color(move || palette(dark_mode.get()).text_muted),
          )
          .child(
            view()
              .flex_row()
              .items_center()
              .gap_1()
              .child(
                button()
                  .test_id("math-copy-btn")
                  .accessibility_label("复制 LaTeX 源码")
                  .px_2()
                  .py_1()
                  .rounded(px(4.0))
                  .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                  .on_press(move || {
                    set_clipboard(read_node_source(&node_sig.get_untracked()));
                  })
                  .child(
                    text("复制")
                      .text_size(px(11.0))
                      .text_color(move || palette(dark_mode.get()).text_muted),
                  ),
              )
              .child(
                button()
                  .test_id("math-toggle-edit")
                  .accessibility_label("切换公式编辑模式")
                  .px_2()
                  .py_1()
                  .rounded(px(4.0))
                  .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                  .on_press(move || editing.set(!editing.get_untracked()))
                  .child(
                    text(move || if editing.get() { "完成" } else { "编辑" })
                      .text_size(px(11.0))
                      .font_weight(FontWeight::MEDIUM)
                      .text_color(move || palette(dark_mode.get()).text_primary),
                  ),
              ),
          ),
      )
      .child(
        button()
          .test_id("math-preview-btn")
          .accessibility_label("点击编辑数学公式")
          .w_full()
          .p_3()
          .bg(move || palette(dark_mode.get()).surface_base)
          .on_press(move || editing.set(!editing.get_untracked()))
          .child(dynamic(move || {
            let src = read_node_source(&node_sig.get());
            render_atomic_block(
              &AtomicBlockInput {
                id: "math-node".into(),
                kind: "math".into(),
                text: src,
                data: serde_json::json!({}),
              },
              dark_mode.get(),
            )
          })),
      )
      .child(when(
        move || editing.get(),
        move || {
          let session_c = session_in.clone();
          let get_pos_c = get_pos_in.clone();
          view()
            .w_full()
            .px_3()
            .py_2()
            .border_t_1()
            .border_color(move || palette(dark_mode.get()).border_subtle)
            .bg(move || palette(dark_mode.get()).code_bg)
            .child(
              input()
                .test_id("math-source-input")
                .auto_focus(true)
                .submit_behavior(SubmitBehavior::Submit)
                .placeholder("输入 LaTeX 公式，例如 E = mc^2")
                .w_full()
                .px_2()
                .py_1()
                .rounded(px(4.0))
                .text_size(px(13.0))
                .text_color(move || palette(dark_mode.get()).text_primary)
                .bg(move || palette(dark_mode.get()).surface_base)
                .value(move || read_node_source(&node_sig.get()))
                .on_change(move |v: &str| {
                  dispatch_node_source_update(&session_c, &get_pos_c, v);
                })
                .on_submit(move |_: &str| editing.set(false)),
            )
        },
      ))
      .into_element()
  }

  fn update(&self, node: &Node, _decorations: &[Decoration]) -> bool {
    node.ty().name() == "math"
  }

  fn handle_event(&self, _event: &NodeViewEvent) -> bool {
    true
  }
}

// ---------------------------------------------------------------------------
// 3. MermaidNodeView (Merman SVG preview + click-to-toggle source editor)
// ---------------------------------------------------------------------------

struct MermaidNodeView {
  dark_mode: Signal<bool>,
}

impl NodeView for MermaidNodeView {
  fn create(&self, cx: &NodeViewCx, _node: &Node) -> AnyElement {
    let dark_mode = self.dark_mode;
    let node_sig = cx.node();
    let selected_sig = cx.selected();
    let editing = signal(false);

    let session_ed = cx.session.clone();
    let get_pos_ed = cx.get_pos();

    view()
      .test_id("node-view-mermaid")
      .w_full()
      .my_2()
      .rounded(px(8.0))
      .border_1()
      .border_color(move || {
        let p = palette(dark_mode.get());
        if selected_sig.get() || editing.get() {
          p.accent
        } else {
          p.border_normal
        }
      })
      .bg(move || palette(dark_mode.get()).surface_raised)
      .overflow_hidden()
      .child(
        view()
          .w_full()
          .h(px(32.0))
          .px_3()
          .flex_row()
          .items_center()
          .justify_between()
          .border_b_1()
          .border_color(move || palette(dark_mode.get()).border_subtle)
          .child(
            text("Mermaid 图表")
              .text_size(px(11.5))
              .font_weight(FontWeight::MEDIUM)
              .text_color(move || palette(dark_mode.get()).text_muted),
          )
          .child(
            view()
              .flex_row()
              .items_center()
              .gap_1()
              .child(
                button()
                  .test_id("mermaid-copy-btn")
                  .accessibility_label("复制 Mermaid 源码")
                  .px_2()
                  .py_1()
                  .rounded(px(4.0))
                  .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                  .on_press(move || {
                    set_clipboard(read_node_source(&node_sig.get_untracked()));
                  })
                  .child(
                    text("复制")
                      .text_size(px(11.0))
                      .text_color(move || palette(dark_mode.get()).text_muted),
                  ),
              )
              .child(
                button()
                  .test_id("mermaid-toggle-edit")
                  .accessibility_label("切换 Mermaid 源码编辑器")
                  .px_2()
                  .py_1()
                  .rounded(px(4.0))
                  .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                  .on_press(move || editing.set(!editing.get_untracked()))
                  .child(
                    text(move || if editing.get() { "收起源码" } else { "编辑源码" })
                      .text_size(px(11.0))
                      .font_weight(FontWeight::MEDIUM)
                      .text_color(move || palette(dark_mode.get()).text_primary),
                  ),
              ),
          ),
      )
      .child(
        button()
          .test_id("mermaid-preview-btn")
          .accessibility_label("点击编辑 Mermaid 图表")
          .w_full()
          .p_3()
          .bg(move || palette(dark_mode.get()).surface_base)
          .on_press(move || editing.set(!editing.get_untracked()))
          .child(dynamic(move || {
            let src = read_node_source(&node_sig.get());
            render_atomic_block(
              &AtomicBlockInput {
                id: "mermaid-node".into(),
                kind: "mermaid".into(),
                text: src,
                data: serde_json::json!({}),
              },
              dark_mode.get(),
            )
          })),
      )
      .child(when(
        move || editing.get(),
        move || {
          let session_c = session_ed.clone();
          let get_pos_c = get_pos_ed.clone();
          view()
            .w_full()
            .border_t_1()
            .border_color(move || palette(dark_mode.get()).border_subtle)
            .bg(move || palette(dark_mode.get()).code_bg)
            .child(
              editor()
                .test_id("mermaid-source-editor")
                .accessibility_label("Mermaid 源码编辑器")
                .w_full()
                .value(move || Some(SharedString::from(read_node_source(&node_sig.get()))))
                .placeholder("graph LR\n  A[Mica] --> B[Pure Rust]")
                .line_numbers(true)
                .soft_wrap(true)
                .auto_indent(true)
                .auto_close(true)
                .font_size(px(12.5))
                .line_height(px(19.0))
                .min_rows(Some(4u32))
                .max_rows(Some(18u32))
                .scroll_beyond_last_line(EditorScrollBeyond::None)
                .styles(move || code_editor_styles(dark_mode.get()))
                .on_change(move |v: &SharedString| {
                  dispatch_node_source_update(&session_c, &get_pos_c, v.as_ref());
                }),
            )
        },
      ))
      .into_element()
  }

  fn update(&self, node: &Node, _decorations: &[Decoration]) -> bool {
    node.ty().name() == "mermaid"
  }

  fn handle_event(&self, _event: &NodeViewEvent) -> bool {
    true
  }
}

// ---------------------------------------------------------------------------
// 4. BoardNodeView (embedded `mica-board` SVG preview + open whiteboard)
// ---------------------------------------------------------------------------

struct BoardNodeView {
  dark_mode: Signal<bool>,
}

impl NodeView for BoardNodeView {
  fn create(&self, cx: &NodeViewCx, _node: &Node) -> AnyElement {
    let dark_mode = self.dark_mode;
    let node_sig = cx.node();
    let selected_sig = cx.selected();
    let open_board = use_context::<OpenBoardCallback>();

    view()
      .test_id("node-view-board")
      .w_full()
      .my_2()
      .rounded(px(8.0))
      .border_1()
      .border_color(move || {
        let p = palette(dark_mode.get());
        if selected_sig.get() {
          p.accent
        } else {
          p.border_normal
        }
      })
      .bg(move || palette(dark_mode.get()).surface_raised)
      .overflow_hidden()
      .child(
        view()
          .w_full()
          .h(px(34.0))
          .px_3()
          .flex_row()
          .items_center()
          .justify_between()
          .border_b_1()
          .border_color(move || palette(dark_mode.get()).border_subtle)
          .child(
            view()
              .flex_row()
              .items_center()
              .gap_2()
              .child(dynamic(move || {
                mica_icons::whiteboard(palette(dark_mode.get()).text_muted_hex)
              }))
              .child(
                text("内嵌白板")
                  .text_size(px(12.0))
                  .font_weight(FontWeight::MEDIUM)
                  .text_color(move || palette(dark_mode.get()).text_muted),
              ),
          )
          .child(
            button()
              .test_id("board-open-full-btn")
              .accessibility_label("打开完整无限白板")
              .px_2()
              .py_1()
              .rounded(px(5.0))
              .bg(move || palette(dark_mode.get()).surface_base)
              .border_1()
              .border_color(move || palette(dark_mode.get()).border_normal)
              .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
              .on_press(move || {
                if let Some(cb) = &open_board {
                  (cb.0)();
                }
              })
              .child(
                text("打开完整白板")
                  .text_size(px(11.5))
                  .font_weight(FontWeight::MEDIUM)
                  .text_color(move || palette(dark_mode.get()).text_primary),
              ),
          ),
      )
      .child(
        view()
          .w_full()
          .p_3()
          .bg(move || palette(dark_mode.get()).surface_base)
          .child(dynamic(move || {
            let src = read_node_source(&node_sig.get());
            render_atomic_block(
              &AtomicBlockInput {
                id: "board-node".into(),
                kind: "board".into(),
                text: src,
                data: serde_json::json!({}),
              },
              dark_mode.get(),
            )
          })),
      )
      .into_element()
  }

  fn update(&self, node: &Node, _decorations: &[Decoration]) -> bool {
    node.ty().name() == "board"
  }

  fn handle_event(&self, _event: &NodeViewEvent) -> bool {
    true
  }
}

// ---------------------------------------------------------------------------
// 5. DividerNodeView (crisp warm-gray 1px hairline with generous padding)
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
      .w_full()
      .py_4()
      .justify_center()
      .child(
        view()
          .w_full()
          .h(px(1.0))
          .bg(move || {
            let p = palette(dark_mode.get());
            if selected_sig.get() {
              p.accent
            } else {
              p.border_normal
            }
          }),
      )
      .into_element()
  }

  fn update(&self, node: &Node, _decorations: &[Decoration]) -> bool {
    matches!(node.ty().name(), "divider" | "horizontalRule")
  }
}

// ---------------------------------------------------------------------------
// 6. TodoNodeView (interactive checkbox + editable content slot)
// ---------------------------------------------------------------------------

struct TodoNodeView {
  dark_mode: Signal<bool>,
}

impl NodeView for TodoNodeView {
  fn create(&self, cx: &NodeViewCx, node: &Node) -> AnyElement {
    let dark_mode = self.dark_mode;
    let node_sig = cx.node();
    let content = cx.take_content();

    // If the node spec declares `extra.render = "task"`, `RichEditor` already
    // renders its built-in checkbox beside the node view; avoid duplicating it.
    let host_renders_checkbox = node
      .ty()
      .spec()
      .extra
      .get("render")
      .and_then(Value::as_str)
      == Some("task");
    if host_renders_checkbox {
      return view().w_full().children(content).into_element();
    }

    let checked = computed(move || {
      node_sig
        .get()
        .attrs()
        .get("checked")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    });
    let session_cb = cx.session.clone();
    let get_pos_cb = cx.get_pos();

    view()
      .test_id("node-view-todo")
      .w_full()
      .py_1()
      .flex_row()
      .items_start()
      .gap_2()
      .child(
        button()
          .test_id("todo-checkbox-btn")
          .accessibility_label("切换待办完成状态")
          .w(px(22.0))
          .h(px(24.0))
          .items_center()
          .justify_center()
          .rounded(px(4.0))
          .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
          .on_press(move || {
            let next = !checked.get_untracked();
            dispatch_node_attr(&session_cb, &get_pos_cb, "checked", Value::Bool(next));
          })
          .child(
            text(move || if checked.get() { "☑" } else { "☐" })
              .text_size(px(16.0))
              .text_color(move || {
                let p = palette(dark_mode.get());
                if checked.get() {
                  p.accent
                } else {
                  p.text_muted
                }
              }),
          ),
      )
      .child(
        view()
          .flex_1()
          .min_w(px(0.0))
          .opacity(move || if checked.get() { 0.6_f32 } else { 1.0_f32 })
          .children(content),
      )
      .into_element()
  }

  fn update(&self, node: &Node, _decorations: &[Decoration]) -> bool {
    matches!(node.ty().name(), "todo" | "taskItem")
  }

  fn editable_content_slot(&self) -> bool {
    true
  }
}

// ---------------------------------------------------------------------------
// 7. FlatListNodeView (bullet `•` / ordinal `1.` markers + indent for flat blocks)
// ---------------------------------------------------------------------------

struct FlatListNodeView {
  dark_mode: Signal<bool>,
  ordered: bool,
}

fn read_indent(node: &Node) -> u64 {
  node
    .attrs()
    .get("indent")
    .and_then(Value::as_u64)
    .unwrap_or(0)
    .min(6)
}

fn compute_ordered_index(session: &EditorSession, pos: Option<usize>, indent: u64) -> usize {
  let Some(target_pos) = pos else {
    return 1;
  };
  let state = session.state();
  let doc = state.doc();
  let mut cursor = 0usize;
  let mut run = 0usize;
  for i in 0..doc.child_count() {
    let child = doc.child(i);
    let node_pos = cursor;
    cursor += child.node_size();
    let ty = child.ty().name();
    let child_indent = read_indent(child);
    if ty == "numbered_list" {
      if child_indent == indent {
        run += 1;
      } else if child_indent < indent {
        run = 0;
      }
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
      let node = node_sig.get();
      let indent = read_indent(&node);
      if ordered {
        let idx = compute_ordered_index(&session, get_pos(), indent);
        format!("{idx}.")
      } else {
        match indent % 3 {
          0 => "•".to_string(),
          1 => "◦".to_string(),
          _ => "▪".to_string(),
        }
      }
    };

    view()
      .w_full()
      .pl(indent_px)
      .flex_row()
      .items_start()
      .gap_2()
      .child(
        view()
          .w(px(20.0))
          .pt(px(2.0))
          .items_end()
          .child(
            text(marker)
              .text_size(px(14.0))
              .text_color(move || palette(dark_mode.get()).text_muted),
          ),
      )
      .child(view().flex_1().min_w(px(0.0)).children(content))
      .into_element()
  }

  fn update(&self, node: &Node, _decorations: &[Decoration]) -> bool {
    let expected = if self.ordered {
      "numbered_list"
    } else {
      "bulleted_list"
    };
    node.ty().name() == expected
  }

  fn editable_content_slot(&self) -> bool {
    true
  }
}
