//! Mica L3 pure-Rust cross-platform UI shell (Route 2-B).
//!
//! Directly links Mica's L1 + L2 pure-Rust engines (`mica-core`, `md-core`,
//! `mica-board`, `mica-sync`, `mica-editor`, `mica-client`) with zero FFI,
//! zero Dart/JS bridge, and zero duplicate document representation.

pub mod board_view;
pub mod editor_view;
pub mod sidebar;

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use chrono::Utc;
use mica_api_types::View;
use mica_board::BoardSession;
use mica_client::PageTreeModel;
use mica_editor::EditorSession;
use nsh_base::{button, theme};
use nsh_core::prelude::*;
use uuid::Uuid;

use crate::editor_view::EditorViewState;

fn make_view(ws: Uuid, doc: Uuid, name: &str, object_type: &str, parent: Option<Uuid>, pos: &str) -> View {
  let now = Utc::now();
  View {
    id: Uuid::new_v4(),
    workspace_id: ws,
    parent_view_id: parent,
    object_id: doc,
    object_type: object_type.to_string(),
    name: name.to_string(),
    icon: None,
    position: pos.to_string(),
    is_deleted: false,
    created_by: Uuid::new_v4(),
    created_at: now,
    updated_at: now,
    state_bytes: None,
  }
}

/// Root component for the pure-Rust Mica client.
#[component]
pub fn MicaApp() -> impl IntoElement {
  let ws_id = Uuid::new_v4();
  let welcome_doc_id = Uuid::new_v4();
  let folder = make_view(ws_id, Uuid::new_v4(), "Architecture", "folder", None, "a");
  let welcome_view = make_view(
    ws_id,
    welcome_doc_id,
    "Fullstack Rust Overview",
    "page",
    Some(folder.id),
    "a",
  );

  let mut tree_model = PageTreeModel::new(vec![folder.clone(), welcome_view.clone()]);
  tree_model.set_expanded(folder.id, true);

  let docs_store: Rc<RefCell<HashMap<Uuid, String>>> = Rc::new(RefCell::new(HashMap::from([(
    welcome_doc_id,
    "# Mica Fullstack Rust\n\nWelcome to Mica L3 pure-Rust shell (Route 2-B).\n\n- Single-source MicaDoc CRDT\n- Virtualized flat blocks\n- Native OS CJK IME\n".to_string(),
  )])));

  let tree = Rc::new(RefCell::new(tree_model));
  let tree_rev = signal(0u64);
  let active_doc = signal(Some(welcome_doc_id));
  let active_title = signal(String::from("Fullstack Rust Overview"));
  let active_mode = signal(0u8); // 0 = Editor, 1 = Board Canvas

  let initial_md = docs_store
    .borrow()
    .get(&welcome_doc_id)
    .cloned()
    .unwrap_or_default();
  let editor_state = EditorViewState::new(EditorSession::from_markdown(&initial_md));

  let board_session = Rc::new(RefCell::new(BoardSession::from_json(
    &mica_board::starter_board_json("mindmap"),
  )));
  let board_rev = signal(0u64);

  let on_select_doc: Rc<dyn Fn(Uuid, String)> = {
    let docs = docs_store.clone();
    let ed = editor_state.clone();
    Rc::new(move |next_id: Uuid, title: String| {
      if let Some(prev_id) = active_doc.get_untracked() {
        docs.borrow_mut().insert(prev_id, ed.export_markdown());
      }
      let md = docs.borrow().get(&next_id).cloned().unwrap_or_default();
      ed.load_session(EditorSession::from_markdown(&md));
      active_doc.set(Some(next_id));
      active_title.set(title);
      active_mode.set(0);
    })
  };

  let on_new_page: Rc<dyn Fn()> = {
    let tree_ref = tree.clone();
    let docs = docs_store.clone();
    let ed = editor_state.clone();
    let folder_id = folder.id;
    Rc::new(move || {
      if let Some(prev_id) = active_doc.get_untracked() {
        docs.borrow_mut().insert(prev_id, ed.export_markdown());
      }
      let new_doc_id = Uuid::new_v4();
      let idx = tree_ref.borrow().count_live_pages() + 1;
      let name = format!("Untitled Page {idx}");
      let v = make_view(ws_id, new_doc_id, &name, "page", Some(folder_id), &format!("z{idx:04}"));
      tree_ref.borrow_mut().upsert_view(v);
      tree_ref.borrow_mut().set_expanded(folder_id, true);
      docs.borrow_mut().insert(new_doc_id, String::new());
      ed.load_session(EditorSession::from_markdown(""));
      active_doc.set(Some(new_doc_id));
      active_title.set(name);
      active_mode.set(0);
      tree_rev.set(tree_rev.get_untracked() + 1);
    })
  };

  view()
    .flex_1()
    .flex_row()
    .bg(move || theme().background)
    .child(sidebar::sidebar_view(
      tree,
      tree_rev,
      active_doc,
      on_select_doc,
      on_new_page,
    ))
    .child(
      view()
        .flex_1()
        .child(
          view()
            .flex_row()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_2()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(move || theme().border)
            .child(
              text(move || active_title.get())
                .text_base()
                .text_color(move || theme().foreground),
            )
            .child(
              view()
                .flex_row()
                .gap_2()
                .child(mode_btn("Document", "mode-doc", 0, active_mode))
                .child(mode_btn("Whiteboard", "mode-board", 1, active_mode)),
            ),
        )
        .child(when(move || active_mode.get() == 0, {
          let ed = editor_state.clone();
          move || editor_view::editor_view(ed.clone())
        }))
        .child(when(move || active_mode.get() == 1, {
          let bs = board_session.clone();
          move || board_view::board_view(bs.clone(), board_rev)
        })),
    )
}

fn mode_btn(
  label: &'static str,
  test_id: &'static str,
  idx: u8,
  active: Signal<u8>,
) -> impl IntoElement {
  button()
    .test_id(test_id)
    .px_3()
    .py_1()
    .rounded(move || theme().radius)
    .bg(move || {
      if active.get() == idx {
        theme().primary
      } else {
        theme().muted
      }
    })
    .on_press(move || active.set(idx))
    .child(
      text(label)
        .text_xs()
        .text_color(move || {
          if active.get() == idx {
            theme().primary_foreground
          } else {
            theme().foreground
          }
        }),
    )
}
