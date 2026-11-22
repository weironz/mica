//! Comment panel & Add Comment modal — 1:1 restoration of Flutter's `CommentPanel`
//! and `_promptCommentBody` dialog.
//!
//! Design invariants (from `docs/features/comments-plan.md`):
//! - Unresolved threads sort first (orphaned open threads remain with open ones
//!   since deleting quoted text must not silently erase the discussion).
//! - At most one inline reply input is open at a time so the rail never turns
//!   into a wall of text fields.
//! - Anchors are resolved externally; clicking a non-orphaned quote card invokes
//!   `on_focus(thread_id)`.

use std::rc::Rc;

use nsh_base::{button, input};
use nsh_core::{
  SubmitBehavior,
  color::Rgba,
  element::{ImageSrc, Svg, svg},
  geometry::{px, relative},
  prelude::*,
};

use crate::{mica_icons, mica_theme::palette};

/// Display model for one comment thread in [`comment_panel_view`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentThreadView {
  pub id: String,
  pub quote: String,
  pub orphaned: bool,
  pub resolved: bool,
  pub created_at: String,
  pub author_name: String,
  /// `(author_name, body)` pairs in chronological order.
  pub comments: Vec<(String, String)>,
}

/// Sort comment threads: unresolved first, then ascending by `created_at`.
/// Orphaned unresolved threads remain with open threads (1:1 with Flutter).
pub fn sort_comment_threads(threads: &[CommentThreadView]) -> Vec<CommentThreadView> {
  let mut sorted = threads.to_vec();
  sorted.sort_by(|a, b| {
    a.resolved
      .cmp(&b.resolved)
      .then_with(|| a.created_at.cmp(&b.created_at))
  });
  sorted
}

fn check_circle_icon(color: &str) -> Svg {
  let doc = format!(
    r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="none" stroke="{color}" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><circle cx="10" cy="10" r="6.8"/><path d="M7 10.2L9.2 12.4L13.3 8"/></svg>"##
  );
  svg(ImageSrc::svg_markup(&doc)).w(px(13.0)).h(px(13.0))
}

/// Scrollable list of document comment threads with unresolved-first ordering,
/// quote cards, orphan/resolved badges, single-open inline reply, resolve toggle,
/// and delete action.
pub fn comment_panel_view(
  threads: Signal<Vec<CommentThreadView>>,
  focused_id: Signal<Option<String>>,
  dark_mode: Signal<bool>,
  on_focus: Rc<dyn Fn(String)>,
  on_reply: Rc<dyn Fn(String, String)>,
  on_toggle_resolve: Rc<dyn Fn(String, bool)>,
  on_delete: Rc<dyn Fn(String)>,
) -> impl IntoElement {
  let replying_to: Signal<Option<String>> = signal(None);
  let reply_draft = signal(String::new());

  view()
    .test_id("comment-panel")
    .w_full()
    .flex_1()
    .min_h(px(0.0))
    .child(dynamic(move || {
      let p = palette(dark_mode.get());
      let sorted = sort_comment_threads(&threads.get());
      let focused = focused_id.get();
      let active_reply_id = replying_to.get();

      if sorted.is_empty() {
        return view()
          .test_id("comment-panel-empty")
          .w_full()
          .py(px(24.0))
          .px(px(4.0))
          .child(
            text("还没有评论。选中文字后选择「添加评论」。")
              .text_size(px(12.5))
              .text_color(p.text_muted),
          )
          .into_element();
      }

      let mut list = view().w_full().py(px(8.0)).gap(px(20.0));
      for thread in sorted {
        let is_focused = focused.as_deref() == Some(thread.id.as_str());
        let is_replying = active_reply_id.as_deref() == Some(thread.id.as_str());
        list = list.child(thread_tile_view(
          thread,
          is_focused,
          is_replying,
          replying_to,
          reply_draft,
          dark_mode,
          on_focus.clone(),
          on_reply.clone(),
          on_toggle_resolve.clone(),
          on_delete.clone(),
        ));
      }

      scroll().flex_1().min_h(px(0.0)).child(list).into_element()
    }))
}

#[allow(clippy::too_many_arguments)]
fn thread_tile_view(
  thread: CommentThreadView,
  is_focused: bool,
  is_replying: bool,
  replying_to: Signal<Option<String>>,
  reply_draft: Signal<String>,
  dark_mode: Signal<bool>,
  on_focus: Rc<dyn Fn(String)>,
  on_reply: Rc<dyn Fn(String, String)>,
  on_toggle_resolve: Rc<dyn Fn(String, bool)>,
  on_delete: Rc<dyn Fn(String)>,
) -> impl IntoElement {
  let p = palette(dark_mode.get_untracked());
  let quote_text = if thread.quote.is_empty() {
    "—".to_string()
  } else {
    thread.quote.clone()
  };

  let quote_bg = if thread.orphaned {
    p.surface_sunken
  } else if is_focused {
    p.comment_highlight_active
  } else {
    p.comment_highlight
  };

  let mut quote_label = text(quote_text)
    .text_size(px(12.0))
    .text_color(p.text_muted)
    .number_of_lines(3);
  if thread.orphaned {
    quote_label = quote_label.line_through();
  }

  let focus_id = thread.id.clone();
  let quote_card = if !thread.orphaned {
    button()
      .test_id(&format!("comment-quote-{}", thread.id))
      .accessibility_label("定位评论引用位置")
      .w_full()
      .px_2()
      .py(px(6.0))
      .rounded(px(6.0))
      .bg(quote_bg)
      .border_1()
      .border_color(if is_focused {
        p.warning
      } else {
        Rgba {
          r: 0.0,
          g: 0.0,
          b: 0.0,
          a: 0.0,
        }
      })
      .cursor_pointer()
      .on_press(move || on_focus(focus_id.clone()))
      .child(quote_label)
      .into_element()
  } else {
    view()
      .test_id(&format!("comment-quote-{}", thread.id))
      .w_full()
      .px_2()
      .py(px(6.0))
      .rounded(px(6.0))
      .bg(quote_bg)
      .child(quote_label)
      .into_element()
  };

  let mut tile = view()
    .test_id(&format!("comment-thread-{}", thread.id))
    .w_full()
    .child(quote_card);

  if thread.orphaned {
    tile = tile.child(
      view()
        .test_id(&format!("comment-orphaned-badge-{}", thread.id))
        .pt(px(4.0))
        .child(
          text("被评论的文字已被删除")
            .text_size(px(11.0))
            .text_color(p.text_faint),
        ),
    );
  }

  if thread.resolved {
    tile = tile.child(
      view()
        .test_id(&format!("comment-resolved-badge-{}", thread.id))
        .pt(px(4.0))
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .child(check_circle_icon(p.text_faint_hex))
        .child(text("已解决").text_size(px(11.0)).text_color(p.text_faint)),
    );
  }

  tile = tile.child(view().h(px(6.0)));

  for (idx, (author, body)) in thread.comments.iter().enumerate() {
    let display_author = if !author.trim().is_empty() {
      Some(author.trim().to_string())
    } else if idx == 0 && !thread.author_name.trim().is_empty() {
      Some(thread.author_name.trim().to_string())
    } else {
      None
    };

    let mut msg = view().pb(px(6.0)).gap(px(2.0));
    if let Some(a) = display_author {
      msg = msg.child(
        text(a)
          .text_size(px(11.5))
          .font_weight(FontWeight::MEDIUM)
          .text_color(p.text_faint),
      );
    }
    msg = msg.child(
      text(body.clone())
        .text_size(px(12.5))
        .text_color(p.text_primary),
    );
    tile = tile.child(msg);
  }

  if is_replying {
    let reply_id_submit = thread.id.clone();
    let on_reply_submit = on_reply.clone();
    tile = tile.child(
      view().pt(px(2.0)).pb(px(6.0)).child(
        input()
          .test_id(&format!("comment-reply-input-{}", thread.id))
          .auto_focus(true)
          .submit_behavior(SubmitBehavior::Submit)
          .placeholder("回复…")
          .w_full()
          .px_2()
          .py(px(6.0))
          .rounded(px(6.0))
          .border_1()
          .border_color(p.border_normal)
          .bg(p.surface_base)
          .text_size(px(12.5))
          .text_color(p.text_primary)
          .value(move || reply_draft.get())
          .on_change(move |v: &str| reply_draft.set(v.to_owned()))
          .on_key_press(move |k: &str| {
            if k == "Escape" {
              reply_draft.set(String::new());
              replying_to.set(None);
            }
          })
          .on_submit(move |raw: &str| {
            let trimmed = raw.trim().to_string();
            if trimmed.is_empty() {
              return;
            }
            reply_draft.set(String::new());
            replying_to.set(None);
            on_reply_submit(reply_id_submit.clone(), trimmed);
          }),
      ),
    );
  }

  let reply_id_btn = thread.id.clone();
  let reply_id_open = thread.id.clone();
  let resolve_id = thread.id.clone();
  let next_resolved = !thread.resolved;
  let delete_id = thread.id.clone();

  let mut actions = view()
    .w_full()
    .flex_row()
    .items_center()
    .gap(px(4.0));

  if is_replying {
    actions = actions
      .child(
        button()
          .test_id(&format!("comment-post-reply-{}", thread.id))
          .px_2()
          .py(px(4.0))
          .rounded(px(5.0))
          .bg(p.accent)
          .hover(move |s| s.bg(palette(dark_mode.get_untracked()).accent_hover))
          .on_press(move || {
            let trimmed = reply_draft.get_untracked().trim().to_string();
            if trimmed.is_empty() {
              return;
            }
            reply_draft.set(String::new());
            replying_to.set(None);
            on_reply(reply_id_btn.clone(), trimmed);
          })
          .child(
            text("评论")
              .text_size(px(12.0))
              .font_weight(FontWeight::MEDIUM)
              .text_color(p.on_primary),
          ),
      )
      .child(
        button()
          .test_id(&format!("comment-cancel-reply-{}", thread.id))
          .px_2()
          .py(px(4.0))
          .rounded(px(5.0))
          .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
          .on_press(move || {
            reply_draft.set(String::new());
            replying_to.set(None);
          })
          .child(text("取消").text_size(px(12.0)).text_color(p.text_muted)),
      );
  } else {
    actions = actions.child(
      button()
        .test_id(&format!("comment-reply-btn-{}", thread.id))
        .px_2()
        .py(px(4.0))
        .rounded(px(5.0))
        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
        .on_press(move || {
          reply_draft.set(String::new());
          replying_to.set(Some(reply_id_open.clone()));
        })
        .child(text("回复").text_size(px(12.0)).text_color(p.accent)),
    );
  }

  actions = actions
    .child(
      button()
        .test_id(&format!("comment-resolve-btn-{}", thread.id))
        .px_2()
        .py(px(4.0))
        .rounded(px(5.0))
        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
        .on_press(move || on_toggle_resolve(resolve_id.clone(), next_resolved))
        .child(
          text(if thread.resolved {
            "重新打开"
          } else {
            "标记已解决"
          })
          .text_size(px(12.0))
          .text_color(p.text_muted),
        ),
    )
    .child(view().flex_1())
    .child(
      button()
        .test_id(&format!("comment-delete-btn-{}", thread.id))
        .accessibility_label("删除该评论")
        .px(px(6.0))
        .py(px(4.0))
        .rounded(px(5.0))
        .flex_row()
        .items_center()
        .gap(px(3.0))
        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).danger_wash))
        .on_press(move || on_delete(delete_id.clone()))
        .child(mica_icons::trash_icon(p.text_faint_hex))
        .child(text("删除").text_size(px(11.5)).text_color(p.text_faint)),
    );

  tile.child(actions)
}

/// Centered modal composer for adding a new comment on selected text.
///
/// Renders nothing when `!open.get()`.
pub fn add_comment_modal(
  open: Signal<bool>,
  quote: Signal<String>,
  dark_mode: Signal<bool>,
  on_submit: Rc<dyn Fn(String)>,
  on_close: Rc<dyn Fn()>,
) -> impl IntoElement {
  let body_draft = signal(String::new());

  when(
    move || open.get(),
    move || {
      let close_bg = on_close.clone();
      let close_x = on_close.clone();
      let close_cancel = on_close.clone();
      let close_key = on_close.clone();
      let close_submit = on_close.clone();
      let submit_btn = on_submit.clone();

      view()
        .test_id("add-comment-modal-backdrop")
        .absolute()
        .inset_0()
        .z_index(55)
        .items_center()
        .justify_center()
        .bg(move || palette(dark_mode.get()).backdrop)
        .child(
          button()
            .test_id("add-comment-backdrop-dismiss")
            .accessibility_label("关闭添加评论弹窗")
            .absolute()
            .inset_0()
            .on_press(move || {
              body_draft.set(String::new());
              close_bg();
            }),
        )
        .child(
          view()
            .test_id("add-comment-modal")
            .w(px(420.0))
            .max_w(relative(0.92))
            .rounded(px(12.0))
            .border_1()
            .border_color(move || palette(dark_mode.get()).border_normal)
            .bg(move || palette(dark_mode.get()).surface_overlay)
            .overflow_hidden()
            .z_index(56)
            .child(
              // Header
              view()
                .px_5()
                .py_3()
                .flex_row()
                .items_center()
                .justify_between()
                .border_b_1()
                .border_color(move || palette(dark_mode.get()).border_subtle)
                .child(
                  text("添加评论")
                    .text_size(px(15.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(move || palette(dark_mode.get()).text_primary),
                )
                .child(
                  button()
                    .test_id("add-comment-close-btn")
                    .accessibility_label("关闭")
                    .w(px(26.0))
                    .h(px(26.0))
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                    .on_press(move || {
                      body_draft.set(String::new());
                      close_x();
                    })
                    .child(dynamic(move || {
                      mica_icons::close_sm(palette(dark_mode.get()).text_faint_hex)
                    })),
                ),
            )
            .child(
              // Body: quote preview + multiline input
              view()
                .px_5()
                .py_4()
                .gap_3()
                .child(when(
                  move || !quote.get().trim().is_empty(),
                  move || {
                    view()
                      .test_id("add-comment-quote-preview")
                      .w_full()
                      .px_3()
                      .py_2()
                      .rounded(px(6.0))
                      .bg(move || palette(dark_mode.get()).comment_highlight)
                      .child(
                        text(move || quote.get())
                          .text_size(px(12.0))
                          .text_color(move || palette(dark_mode.get()).text_muted)
                          .number_of_lines(3),
                      )
                  },
                ))
                .child(
                  input()
                    .test_id("add-comment-body-input")
                    .auto_focus(true)
                    .multiline(true)
                    .placeholder("写下评论…")
                    .w_full()
                    .h(px(96.0))
                    .px_3()
                    .py_2()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(move || palette(dark_mode.get()).border_normal)
                    .bg(move || palette(dark_mode.get()).surface_base)
                    .text_size(px(13.0))
                    .text_color(move || palette(dark_mode.get()).text_primary)
                    .value(move || body_draft.get())
                    .on_change(move |v: &str| body_draft.set(v.to_owned()))
                    .on_key_press(move |k: &str| {
                      if k == "Escape" {
                        body_draft.set(String::new());
                        close_key();
                      }
                    }),
                ),
            )
            .child(
              // Footer actions
              view()
                .px_5()
                .py_3()
                .flex_row()
                .justify_end()
                .gap_2()
                .border_t_1()
                .border_color(move || palette(dark_mode.get()).border_subtle)
                .child(
                  button()
                    .test_id("add-comment-cancel-btn")
                    .px_3()
                    .py(px(6.0))
                    .rounded(px(6.0))
                    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                    .on_press(move || {
                      body_draft.set(String::new());
                      close_cancel();
                    })
                    .child(
                      text("取消")
                        .text_size(px(13.0))
                        .text_color(move || palette(dark_mode.get()).text_muted),
                    ),
                )
                .child(
                  button()
                    .test_id("add-comment-submit-btn")
                    .px_4()
                    .py(px(6.0))
                    .rounded(px(6.0))
                    .bg(move || palette(dark_mode.get()).accent)
                    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).accent_hover))
                    .on_press(move || {
                      let trimmed = body_draft.get_untracked().trim().to_string();
                      if trimmed.is_empty() {
                        return;
                      }
                      body_draft.set(String::new());
                      submit_btn(trimmed);
                      close_submit();
                    })
                    .child(
                      text("评论")
                        .text_size(px(13.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || palette(dark_mode.get()).on_primary),
                    ),
                ),
            ),
        )
    },
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  fn make_thread(id: &str, resolved: bool, orphaned: bool, created_at: &str) -> CommentThreadView {
    CommentThreadView {
      id: id.into(),
      quote: format!("quote {id}"),
      orphaned,
      resolved,
      created_at: created_at.into(),
      author_name: "Tester".into(),
      comments: vec![("Tester".into(), format!("body {id}"))],
    }
  }

  #[test]
  fn sort_comment_threads_places_unresolved_and_orphans_before_resolved() {
    let input = vec![
      make_thread("r1", true, false, "2026-07-22T08:00:00Z"),
      make_thread("o2", false, true, "2026-07-22T10:00:00Z"),
      make_thread("u1", false, false, "2026-07-22T09:00:00Z"),
      make_thread("r0", true, false, "2026-07-22T07:00:00Z"),
    ];
    let sorted = sort_comment_threads(&input);
    let ids: Vec<&str> = sorted.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, vec!["u1", "o2", "r0", "r1"]);
  }
}
