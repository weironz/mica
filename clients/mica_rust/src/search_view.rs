//! Floating modal workspace search (`Ctrl+Shift+F`) matching Mica's original
//! `search_dialog.dart` — opens over the active document without replacing it,
//! supports auto-focus, live title/body/board search, and Esc/backdrop dismiss.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use mica_client::{PageTreeModel, SearchHit, search_cached_documents};
use mica_md::{ParseMode, parse_body};
use nsh_base::{button, input};
use nsh_core::{geometry::px, prelude::*};
use uuid::Uuid;

use crate::{mica_icons, mica_theme::palette};

pub fn search_modal_view(
  tree: Rc<RefCell<PageTreeModel>>,
  docs: Rc<RefCell<HashMap<Uuid, String>>>,
  dark_mode: Signal<bool>,
  on_open_doc: Rc<dyn Fn(Uuid, String)>,
  on_close: Rc<dyn Fn()>,
) -> impl IntoElement {
  let query = signal(String::new());
  let hits: Signal<Vec<SearchHit>> = signal(Vec::new());

  let tree_q = tree.clone();
  let docs_q = docs.clone();
  let close_bg = on_close.clone();
  let close_btn = on_close.clone();

  view()
    .test_id("search-modal-backdrop")
    .absolute()
    .inset_0()
    .z_index(40)
    .items_center()
    .pt(px(88.0))
    .bg(move || palette(dark_mode.get()).backdrop)
    .child(
      // Clickable backdrop layer behind dialog card
      button()
        .test_id("search-backdrop-dismiss")
        .accessibility_label("关闭搜索弹窗")
        .absolute()
        .inset_0()
        .on_press(move || close_bg()),
    )
    .child(
      // Centered floating search command palette card (620px wide)
      view()
        .test_id("search-panel")
        .w(px(620.0))
        .max_h(px(520.0))
        .rounded(px(12.0))
        .border_1()
        .border_color(move || palette(dark_mode.get()).border_normal)
        .bg(move || palette(dark_mode.get()).surface_overlay)
        .overflow_hidden()
        .z_index(41)
        .child(
          // Top search bar row
          view()
            .h(px(48.0))
            .px_4()
            .flex_row()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(move || palette(dark_mode.get()).border_subtle)
            .child(dynamic(move || {
              mica_icons::search(palette(dark_mode.get()).text_faint_hex)
            }))
            .child(
              input()
                .test_id("search-query-input")
                .auto_focus(true)
                .placeholder("搜索工作区中的页面标题、正文与内嵌白板...")
                .flex_1()
                .text_size(px(14.0))
                .text_color(move || palette(dark_mode.get()).text_primary)
                .bg(move || palette(dark_mode.get()).surface_overlay)
                .value(move || query.get())
                .on_change(move |q: &str| {
                  query.set(q.to_owned());
                  let results = run_workspace_search(&tree_q.borrow(), &docs_q.borrow(), q);
                  hits.set(results);
                }),
            )
            .child(
              button()
                .test_id("search-close-btn")
                .accessibility_label("关闭搜索")
                .w(px(26.0))
                .h(px(26.0))
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                .on_press(move || close_btn())
                .child(dynamic(move || {
                  mica_icons::close_sm(palette(dark_mode.get()).text_faint_hex)
                })),
            ),
        )
        .child(
          // Status summary bar
          view()
            .px_4()
            .py_2()
            .bg(move || palette(dark_mode.get()).surface_raised)
            .border_b_1()
            .border_color(move || palette(dark_mode.get()).border_subtle)
            .child(
              text(move || {
                let q = query.get();
                let count = hits.get().len();
                if q.trim().is_empty() {
                  "输入关键词实时检索当前工作区全部文档".to_string()
                } else {
                  format!("找到 {count} 条与 \"{q}\" 相关的结果")
                }
              })
              .text_size(px(12.0))
              .text_color(move || palette(dark_mode.get()).text_faint),
            ),
        )
        .child(render_results_list(hits, dark_mode, on_open_doc, on_close))
        .child(
          // Footer keyboard hints (matches original _KeyCap chips)
          view()
            .h(px(34.0))
            .px_4()
            .flex_row()
            .items_center()
            .justify_between()
            .border_t_1()
            .border_color(move || palette(dark_mode.get()).border_subtle)
            .bg(move || palette(dark_mode.get()).surface_raised)
            .child(
              view()
                .flex_row()
                .items_center()
                .gap_3()
                .child(key_hint("点击打开", dark_mode))
                .child(key_hint("Esc 关闭", dark_mode)),
            )
            .child(
              text("全文索引 · 本地即时检索")
                .text_size(px(11.0))
                .text_color(move || palette(dark_mode.get()).text_ghost),
            ),
        ),
    )
}

fn key_hint(label: &'static str, dark_mode: Signal<bool>) -> impl IntoElement {
  view()
    .px_2()
    .py_1()
    .rounded(px(4.0))
    .bg(move || palette(dark_mode.get()).surface_sunken)
    .child(
      text(label)
        .text_size(px(11.0))
        .text_color(move || palette(dark_mode.get()).text_faint),
    )
}

fn render_results_list(
  hits: Signal<Vec<SearchHit>>,
  dark_mode: Signal<bool>,
  on_open_doc: Rc<dyn Fn(Uuid, String)>,
  on_close: Rc<dyn Fn()>,
) -> impl IntoElement {
  view()
    .flex_1()
    .min_h(px(160.0))
    .max_h(px(340.0))
    .p_2()
    .child(dynamic(move || {
      let current_hits = hits.get();
      let p = palette(dark_mode.get());
      let mut list = view().w_full().gap_1();
      if current_hits.is_empty() {
        return list.child(
          view().py_8().items_center().justify_center().child(
            text("输入标题或正文关键词开始搜索")
              .text_size(px(13.0))
              .text_color(p.text_ghost),
          ),
        );
      }
      for (i, hit) in current_hits.into_iter().enumerate() {
        let open = on_open_doc.clone();
        let close = on_close.clone();
        let doc_id = hit.document_id;
        let title = hit.title.clone();
        let snippet = hit.snippet.clone();
        list = list.child(
          button()
            .test_id(&format!("search-hit-{i}"))
            .accessibility_label(format!("打开搜索结果 {}", title))
            .w_full()
            .px_3()
            .py_2()
            .rounded(px(8.0))
            .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
            .on_press(move || {
              open(doc_id, title.clone());
              close();
            })
            .child(
              view()
                .w_full()
                .flex_row()
                .items_center()
                .gap_2()
                .child(mica_icons::file_doc(p.text_faint_hex))
                .child(
                  view()
                    .flex_1()
                    .gap_1()
                    .child(
                      text(hit.title)
                        .text_size(px(13.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.text_primary)
                        .truncate(),
                    )
                    .child(
                      text(snippet)
                        .text_size(px(12.0))
                        .text_color(p.text_muted)
                        .truncate(),
                    ),
                ),
            ),
        );
      }
      list
    }))
}

pub fn run_workspace_search(
  tree: &PageTreeModel,
  docs: &HashMap<Uuid, String>,
  query: &str,
) -> Vec<SearchHit> {
  if query.trim().is_empty() {
    return Vec::new();
  }
  let mut corpus = Vec::new();
  for row in tree.visible_rows() {
    if row.is_folder {
      continue;
    }
    let md = docs.get(&row.object_id).map(String::as_str).unwrap_or("");
    let md_blocks = parse_body(md, ParseMode::Document);
    let core_blocks: Vec<mica_core::Block> = md_blocks
      .into_iter()
      .map(|b| mica_core::Block {
        id: b.id,
        kind: b.kind,
        text: b.text,
        data: b.data,
        children: Vec::new(),
      })
      .collect();
    corpus.push((row.object_id, row.name, core_blocks));
  }
  search_cached_documents(&corpus, query, 20)
}
