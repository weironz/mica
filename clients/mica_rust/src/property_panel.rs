//! Structured page-properties panel — 1:1 restoration of Flutter's `PropertyPanel`.
//!
//! Renders a lazy, collapsible view over the document's YAML front matter (the
//! sole authority). Every edit recomputes the raw front-matter string via L1
//! `mica_md::properties::{upsert_property, remove_property, infer_value}` so
//! untouched keys, comments, and quoting remain byte-exact.

use std::rc::Rc;

use mica_md::properties::{
  Property, PropertyValue, infer_value, parse_properties, remove_property, upsert_property,
};
use nsh_base::{button, input};
use nsh_core::{
  SubmitBehavior,
  element::{ImageSrc, Svg, svg},
  geometry::px,
  prelude::*,
};

use crate::{mica_icons, mica_theme::palette};

/// Format a numeric property without trailing `.0` for whole numbers,
/// matching Flutter `_numText` and `mica_md::properties::format_number`.
pub fn format_property_number(n: f64) -> String {
  if n.fract() == 0.0 && n.abs() < 1e15 {
    format!("{}", n as i64)
  } else {
    format!("{n}")
  }
}

/// Build the single-line collapsed summary (`key: val  ·  key2: val2`).
pub fn format_property_summary(props: &[Property]) -> String {
  let mut bits = Vec::with_capacity(props.len());
  for p in props {
    let val = match &p.value {
      PropertyValue::Text(s) => s.clone(),
      PropertyValue::Number(n) => format_property_number(*n),
      PropertyValue::Checkbox(b) => {
        if *b {
          "✓".to_string()
        } else {
          "✗".to_string()
        }
      }
      PropertyValue::Date(d) => d.clone(),
      PropertyValue::List(items) => items.join(", "),
    };
    if val.is_empty() {
      bits.push(p.key.clone());
    } else {
      bits.push(format!("{}: {val}", p.key));
    }
  }
  bits.join("  ·  ")
}

fn scalar_display_text(value: &PropertyValue) -> String {
  match value {
    PropertyValue::Text(s) => s.clone(),
    PropertyValue::Number(n) => format_property_number(*n),
    PropertyValue::Date(d) => d.clone(),
    PropertyValue::Checkbox(b) => b.to_string(),
    PropertyValue::List(items) => items.join(", "),
  }
}

fn mini_icon(color: &str, inner: &str) -> Svg {
  let doc = format!(
    r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="none" stroke="{color}" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">{inner}</svg>"##
  );
  svg(ImageSrc::svg_markup(&doc)).w(px(14.0)).h(px(14.0))
}

fn property_type_icon(value: &PropertyValue, color: &str) -> Svg {
  match value {
    PropertyValue::Text(_) => {
      mini_icon(color, r##"<path d="M3.5 5.5H16.5M3.5 10H16.5M3.5 14.5H11.5"/>"##)
    }
    PropertyValue::Number(_) => {
      mini_icon(color, r##"<path d="M7.5 3.5L6 16.5M14 3.5L12.5 16.5M4 7.5H16M3.5 12.5H15.5"/>"##)
    }
    PropertyValue::Checkbox(_) => mini_icon(
      color,
      r##"<rect x="3.5" y="3.5" width="13" height="13" rx="2.5"/><path d="M7 10.2L9.2 12.4L13.5 7.8"/>"##,
    ),
    PropertyValue::Date(_) => mini_icon(
      color,
      r##"<rect x="3.5" y="4.5" width="13" height="12" rx="2"/><path d="M3.5 8.5H16.5M7 3V6M13 3V6"/>"##,
    ),
    PropertyValue::List(_) => mini_icon(
      color,
      r##"<path d="M3.5 10.5V4.5A1 1 0 0 1 4.5 3.5H10.5L16.5 9.5L10.5 15.5L3.5 10.5Z"/><circle cx="7.2" cy="7.2" r="1"/>"##,
    ),
  }
}

/// Page properties panel mounted above the document body.
///
/// Renders nothing when `!visible.get()`.
pub fn property_panel_view(
  visible: Signal<bool>,
  front_matter: Signal<String>,
  dark_mode: Signal<bool>,
  on_change_front_matter: Rc<dyn Fn(String)>,
  on_open_tag: Rc<dyn Fn(String)>,
) -> impl IntoElement {
  let collapsed = signal(false);
  let adding_key = signal(false);
  let new_key_draft = signal(String::new());

  when(
    move || visible.get(),
    move || {
      let on_change = on_change_front_matter.clone();
      let on_tag = on_open_tag.clone();

      view()
        .test_id("property-panel")
        .w_full()
        .pt(px(2.0))
        .pb(px(6.0))
        .child(dynamic(move || {
          let p = palette(dark_mode.get());
          let raw_fm = front_matter.get();
          let props = parse_properties(&raw_fm);
          let is_collapsed = collapsed.get();
          let is_adding = adding_key.get();

          let mut container = view().w_full().gap(px(2.0));

          if !props.is_empty() {
            let summary = format_property_summary(&props);
            let count_text = props.len().to_string();
            container = container.child(
              button()
                .test_id("property-panel-collapse-toggle")
                .accessibility_label(if is_collapsed {
                  "展开页面属性"
                } else {
                  "折叠页面属性"
                })
                .w_full()
                .py(px(2.0))
                .px(px(2.0))
                .rounded(px(4.0))
                .flex_row()
                .items_center()
                .gap(px(4.0))
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                .on_press(move || collapsed.set(!collapsed.get_untracked()))
                .child(if is_collapsed {
                  mica_icons::chevron_right(p.text_faint_hex)
                } else {
                  mica_icons::chevron_down(p.text_faint_hex)
                })
                .child(if is_collapsed {
                  text(summary)
                    .flex_1()
                    .text_size(px(12.0))
                    .text_color(p.text_faint)
                    .truncate()
                } else {
                  text(count_text)
                    .text_size(px(12.0))
                    .text_color(p.text_faint)
                }),
            );
          }

          if props.is_empty() || !is_collapsed {
            for prop in props {
              container = container.child(property_row_view(
                prop,
                front_matter,
                dark_mode,
                on_change.clone(),
                on_tag.clone(),
              ));
            }

            if is_adding {
              let on_change_add = on_change.clone();
              let on_change_blur = on_change.clone();
              container = container.child(
                view()
                  .test_id("property-add-key-row")
                  .pt(px(2.0))
                  .pl(px(20.0))
                  .child(
                    input()
                      .test_id("property-key-input")
                      .auto_focus(true)
                      .submit_behavior(SubmitBehavior::Submit)
                      .placeholder("属性名")
                      .w(px(160.0))
                      .py(px(4.0))
                      .text_size(px(12.5))
                      .text_color(p.text_primary)
                      .bg(p.surface_base)
                      .value(move || new_key_draft.get())
                      .on_change(move |v: &str| new_key_draft.set(v.to_owned()))
                      .on_key_press(move |k: &str| {
                        if k == "Escape" {
                          new_key_draft.set(String::new());
                          adding_key.set(false);
                        }
                      })
                      .on_submit(move |raw: &str| {
                        let k = raw.trim();
                        new_key_draft.set(String::new());
                        adding_key.set(false);
                        if !k.is_empty() {
                          let next = upsert_property(
                            &front_matter.get_untracked(),
                            k,
                            &PropertyValue::Text(String::new()),
                          );
                          front_matter.set(next.clone());
                          on_change_add(next);
                        }
                      })
                      .on_blur(move || {
                        let k = new_key_draft.get_untracked();
                        let trimmed = k.trim().to_string();
                        new_key_draft.set(String::new());
                        adding_key.set(false);
                        if !trimmed.is_empty() {
                          let next = upsert_property(
                            &front_matter.get_untracked(),
                            &trimmed,
                            &PropertyValue::Text(String::new()),
                          );
                          front_matter.set(next.clone());
                          on_change_blur(next);
                        }
                      }),
                  ),
              );
            } else {
              container = container.child(
                view().pt(px(1.0)).flex_row().child(
                  button()
                    .test_id("property-add-btn")
                    .accessibility_label("添加属性")
                    .py(px(3.0))
                    .px(px(4.0))
                    .rounded(px(4.0))
                    .flex_row()
                    .items_center()
                    .gap(px(4.0))
                    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                    .on_press(move || {
                      new_key_draft.set(String::new());
                      adding_key.set(true);
                    })
                    .child(mica_icons::plus(p.text_faint_hex))
                    .child(
                      text("添加属性")
                        .text_size(px(12.5))
                        .text_color(p.text_faint),
                    ),
                ),
              );
            }
          }

          container
        }))
    },
  )
}

fn property_row_view(
  prop: Property,
  front_matter: Signal<String>,
  dark_mode: Signal<bool>,
  on_change_front_matter: Rc<dyn Fn(String)>,
  on_open_tag: Rc<dyn Fn(String)>,
) -> impl IntoElement {
  let p = palette(dark_mode.get_untracked());
  let key_name = prop.key.clone();
  let key_for_remove = prop.key.clone();
  let on_change_remove = on_change_front_matter.clone();

  view()
    .test_id(&format!("property-row-{}", prop.key))
    .w_full()
    .py(px(1.0))
    .flex_row()
    .items_start()
    .group()
    .child(
      view()
        .pt(px(5.0))
        .flex_shrink_0()
        .child(property_type_icon(&prop.value, p.text_faint_hex)),
    )
    .child(view().w(px(6.0)).flex_shrink_0())
    .child(
      view().pt(px(3.0)).w(px(84.0)).flex_shrink_0().child(
        text(key_name)
          .text_size(px(12.5))
          .text_color(p.text_muted)
          .truncate(),
      ),
    )
    .child(view().w(px(8.0)).flex_shrink_0())
    .child(
      view()
        .flex_1()
        .min_w(px(0.0))
        .child(property_value_editor(
          prop.key.clone(),
          prop.value,
          front_matter,
          dark_mode,
          on_change_front_matter,
          on_open_tag,
        )),
    )
    .child(
      // Reserved 22px slot so hover-revealed delete never causes layout shift.
      view().w(px(22.0)).h(px(22.0)).flex_shrink_0().child(
        button()
          .test_id(&format!("property-delete-{}", prop.key))
          .accessibility_label(format!("删除属性 {}", prop.key))
          .w(px(22.0))
          .h(px(22.0))
          .items_center()
          .justify_center()
          .rounded(px(4.0))
          .opacity(0.0_f32)
          .group_hover(|s| s.opacity(1.0_f32))
          .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
          .on_press(move || {
            let next = remove_property(&front_matter.get_untracked(), &key_for_remove);
            front_matter.set(next.clone());
            on_change_remove(next);
          })
          .child(mica_icons::close_sm(p.text_faint_hex)),
      ),
    )
}

fn property_value_editor(
  key: String,
  value: PropertyValue,
  front_matter: Signal<String>,
  dark_mode: Signal<bool>,
  on_change_front_matter: Rc<dyn Fn(String)>,
  on_open_tag: Rc<dyn Fn(String)>,
) -> AnyElement {
  let p = palette(dark_mode.get_untracked());

  match value {
    PropertyValue::Checkbox(checked) => {
      let key_toggle = key.clone();
      let on_change = on_change_front_matter;
      view()
        .flex_row()
        .items_center()
        .h(px(24.0))
        .child(
          button()
            .test_id(&format!("property-checkbox-{key}"))
            .accessibility_label(format!("切换属性 {key}"))
            .w(px(24.0))
            .h(px(24.0))
            .items_center()
            .justify_center()
            .rounded(px(4.0))
            .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
            .on_press(move || {
              let next = upsert_property(
                &front_matter.get_untracked(),
                &key_toggle,
                &PropertyValue::Checkbox(!checked),
              );
              front_matter.set(next.clone());
              on_change(next);
            })
            .child(
              text(if checked { "☑" } else { "☐" })
                .text_size(px(15.0))
                .text_color(if checked { p.accent } else { p.text_faint }),
            ),
        )
        .into_element()
    }
    PropertyValue::List(items) => {
      let tag_draft = signal(String::new());
      let mut wrap = view()
        .test_id(&format!("property-list-{key}"))
        .pt(px(2.0))
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(6.0));

      for (idx, tag) in items.iter().enumerate() {
        let tag_open = tag.clone();
        let on_tag = on_open_tag.clone();
        let key_del = key.clone();
        let items_del = items.clone();
        let on_change_del = on_change_front_matter.clone();

        wrap = wrap.child(
          view()
            .test_id(&format!("property-tag-chip-{key}-{idx}"))
            .pl(px(8.0))
            .pr(px(3.0))
            .py(px(2.0))
            .rounded(px(6.0))
            .bg(p.code_bg)
            .flex_row()
            .items_center()
            .gap(px(3.0))
            .child(
              button()
                .test_id(&format!("property-tag-open-{key}-{idx}"))
                .accessibility_label(format!("搜索标签 {tag}"))
                .cursor_pointer()
                .on_press(move || on_tag(tag_open.clone()))
                .child(
                  text(tag.clone())
                    .text_size(px(12.0))
                    .text_color(p.text_primary),
                ),
            )
            .child(
              button()
                .test_id(&format!("property-tag-remove-{key}-{idx}"))
                .accessibility_label(format!("移除标签 {tag}"))
                .w(px(16.0))
                .h(px(16.0))
                .items_center()
                .justify_center()
                .rounded(px(3.0))
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                .on_press(move || {
                  let mut next_items = items_del.clone();
                  if idx < next_items.len() {
                    next_items.remove(idx);
                  }
                  let next = upsert_property(
                    &front_matter.get_untracked(),
                    &key_del,
                    &PropertyValue::List(next_items),
                  );
                  front_matter.set(next.clone());
                  on_change_del(next);
                })
                .child(mica_icons::close_sm(p.text_faint_hex)),
            ),
        );
      }

      let key_add = key.clone();
      let items_add = items;
      let on_change_add = on_change_front_matter;
      wrap = wrap.child(
        input()
          .test_id(&format!("property-tag-add-{key}"))
          .submit_behavior(SubmitBehavior::Submit)
          .placeholder("添加标签")
          .w(px(88.0))
          .py(px(3.0))
          .text_size(px(12.0))
          .text_color(p.text_primary)
          .bg(p.surface_base)
          .value(move || tag_draft.get())
          .on_change(move |v: &str| tag_draft.set(v.to_owned()))
          .on_submit(move |raw: &str| {
            let trimmed = raw.trim();
            tag_draft.set(String::new());
            if trimmed.is_empty() {
              return;
            }
            let mut next_items = items_add.clone();
            next_items.push(trimmed.to_string());
            let next = upsert_property(
              &front_matter.get_untracked(),
              &key_add,
              &PropertyValue::List(next_items),
            );
            front_matter.set(next.clone());
            on_change_add(next);
          }),
      );

      wrap.into_element()
    }
    PropertyValue::Text(_) | PropertyValue::Number(_) | PropertyValue::Date(_) => {
      let initial = scalar_display_text(&value);
      let draft = signal(initial.clone());
      let last_committed = signal(initial);
      let key_submit = key.clone();
      let key_blur = key.clone();
      let on_change_submit = on_change_front_matter.clone();
      let on_change_blur = on_change_front_matter;

      input()
        .test_id(&format!("property-scalar-{key}"))
        .submit_behavior(SubmitBehavior::Submit)
        .placeholder("—")
        .w_full()
        .py(px(3.0))
        .text_size(px(12.5))
        .text_color(p.text_primary)
        .bg(p.surface_base)
        .value(move || draft.get())
        .on_change(move |v: &str| draft.set(v.to_owned()))
        .on_submit(move |raw: &str| {
          if raw == last_committed.get_untracked() {
            return;
          }
          last_committed.set(raw.to_owned());
          let inferred = infer_value(raw);
          let next = upsert_property(&front_matter.get_untracked(), &key_submit, &inferred);
          front_matter.set(next.clone());
          on_change_submit(next);
        })
        .on_blur(move || {
          let cur = draft.get_untracked();
          if cur == last_committed.get_untracked() {
            return;
          }
          last_committed.set(cur.clone());
          let inferred = infer_value(&cur);
          let next = upsert_property(&front_matter.get_untracked(), &key_blur, &inferred);
          front_matter.set(next.clone());
          on_change_blur(next);
        })
        .into_element()
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn format_property_number_drops_integer_decimal() {
    assert_eq!(format_property_number(42.0), "42");
    assert_eq!(format_property_number(-3.0), "-3");
    assert_eq!(format_property_number(1.5), "1.5");
  }

  #[test]
  fn format_property_summary_covers_all_property_types() {
    let fm = "status: draft\ncount: 3\ndone: true\ndue: 2026-07-22\ntags: [rust, crdt]\nempty:";
    let props = parse_properties(fm);
    assert_eq!(
      format_property_summary(&props),
      "status: draft  ·  count: 3  ·  done: ✓  ·  due: 2026-07-22  ·  tags: rust, crdt  ·  empty"
    );
  }

  #[test]
  fn surgical_upsert_and_remove_preserves_untouched_lines() {
    let fm = "# keep comment\ntitle: Original\ntags: [a, b]";
    let updated = upsert_property(fm, "tags", &PropertyValue::List(vec!["a".into(), "c".into()]));
    assert_eq!(updated, "# keep comment\ntitle: Original\ntags: [a, c]");

    let removed = remove_property(&updated, "title");
    assert_eq!(removed, "# keep comment\ntags: [a, c]");
  }
}
