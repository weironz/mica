//! Full-text document & embedded-board search panel backed by `mica_client::search`.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use mica_client::{PageTreeModel, SearchHit, search_cached_documents};
use mica_md::{ParseMode, parse_body};
use nsh_base::{button, input, theme};
use nsh_core::prelude::*;
use uuid::Uuid;

pub fn search_panel_view(
  tree: Rc<RefCell<PageTreeModel>>,
  docs: Rc<RefCell<HashMap<Uuid, String>>>,
  on_open_doc: Rc<dyn Fn(Uuid, String)>,
  on_close: Rc<dyn Fn()>,
) -> impl IntoElement {
  let query = signal(String::new());
  let hits: Signal<Vec<SearchHit>> = signal(Vec::new());

  let tree_q = tree.clone();
  let docs_q = docs.clone();

  view()
    .test_id("search-panel")
    .flex_1()
    .p_4()
    .gap_3()
    .bg(move || theme().background)
    .child(
      view()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_2()
        .child(
          text("Search Workspace")
            .text_base()
            .text_color(move || theme().foreground),
        )
        .child(
          button()
            .test_id("search-close-btn")
            .px_2()
            .py_1()
            .border_1()
            .border_color(move || theme().border)
            .rounded(move || theme().radius)
            .on_press(move || on_close())
            .child(
              text("Close")
                .text_xs()
                .text_color(move || theme().foreground),
            ),
        ),
    )
    .child(
      input()
        .test_id("search-query-input")
        .placeholder("输入关键词搜索标题、正文与内嵌白板...")
        .w_full()
        .px_3()
        .py_2()
        .border_1()
        .border_color(move || theme().primary)
        .rounded(move || theme().radius)
        .text_color(move || theme().foreground)
        .value(move || query.get())
        .on_change(move |q: &str| {
          query.set(q.to_owned());
          let results = run_workspace_search(&tree_q.borrow(), &docs_q.borrow(), q);
          hits.set(results);
        }),
    )
    .child(
      text(move || {
        let q = query.get();
        let count = hits.get().len();
        if q.trim().is_empty() {
          "Enter keywords to search all pages in this workspace.".to_string()
        } else {
          format!("Found {count} result(s) for \"{q}\"")
        }
      })
      .text_xs()
      .text_color(move || theme().muted_foreground),
    )
    .child(render_results_list(hits, on_open_doc))
}

fn render_results_list(
  hits: Signal<Vec<SearchHit>>,
  on_open_doc: Rc<dyn Fn(Uuid, String)>,
) -> impl IntoElement {
  view()
    .flex_1()
    .gap_2()
    .child(dynamic(move || {
      let current_hits = hits.get();
      let mut list = view().w_full().gap_2();
      for (i, hit) in current_hits.into_iter().enumerate() {
        let open = on_open_doc.clone();
        let doc_id = hit.document_id;
        let title = hit.title.clone();
        let snippet = hit.snippet.clone();
        list = list.child(
          button()
            .test_id(&format!("search-hit-{i}"))
            .w_full()
            .p_3()
            .border_1()
            .border_color(move || theme().border)
            .rounded(move || theme().radius)
            .bg(move || theme().card)
            .on_press(move || open(doc_id, title.clone()))
            .child(
              view()
                .w_full()
                .gap_1()
                .child(
                  text(hit.title)
                    .text_sm()
                    .text_color(move || theme().foreground),
                )
                .child(
                  text(snippet)
                    .text_xs()
                    .text_color(move || theme().muted_foreground),
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
  let rows = tree.visible_rows();
  let mut corpus = Vec::new();
  for row in rows {
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
