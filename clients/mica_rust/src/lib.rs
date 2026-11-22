//! Mica L3 pure-Rust cross-platform client (`clients/mica_rust`).
//!
//! Fullstack Rust architecture (Route 2-B: owned L1 + L2 Rust cores + NSH native shell):
//! - Direct Rust calls to `mica-core`, `mica-md`, `mica-board`, `mica-sync`,
//!   `mica-editor`, `mica-client` (zero Dart, zero Flutter, zero FFI/JSON bridge).
//! - Persistent SQLite local vault (`VaultStore`) on desktop + memory vault on Web.
//! - Cloud HTTP/WS sync adapter (`CloudConnector` + `mica_sync::SyncEngine`).
//! - Route 2-B virtualized editor with `/` slash menu, tables, RaTeX math,
//!   Mermaid diagrams, embedded whiteboards, and OS-native CJK IME.
//! - Interactive infinite whiteboard & mindmap (`BoardSession` + `nsh::Canvas`).

pub mod atomic_blocks;
pub mod board_view;
pub mod editor_slash;
pub mod editor_table;
pub mod editor_view;
pub mod net;
pub mod search_view;
pub mod settings_view;
pub mod sidebar;
pub mod storage;
#[cfg(target_arch = "wasm32")]
pub mod web;

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use chrono::Utc;
use mica_api_types::View;
use mica_board::BoardSession;
use mica_client::PageTreeModel;
use mica_editor::EditorSession;
use nsh_base::{button, theme};
use nsh_core::prelude::*;
use uuid::Uuid;

use crate::{
  editor_view::EditorViewState,
  net::{CloudConfig, CloudConnector},
  settings_view::SettingsCallbacks,
  sidebar::SidebarCallbacks,
  storage::{DEFAULT_ORIGIN, VaultStore},
};

/// Main workspace view mode.
/// 0 = Document, 1 = Whiteboard, 2 = Search, 3 = Settings
const MODE_DOC: u8 = 0;
const MODE_BOARD: u8 = 1;
const MODE_SEARCH: u8 = 2;
const MODE_SETTINGS: u8 = 3;

const DEFAULT_BOARD_ID: &str = "workspace-default-board";

/// Root pure-Rust Mica application component (shared by desktop & Web targets).
pub struct MicaApp;

impl Component for MicaApp {
  type Props = ();
  type Event = NoEvent;

  fn view(_props: Self::Props, _scope: Scope<Self::Event>) -> impl IntoElement {
    render_mica_app(VaultStore::open_default())
  }
}

/// Testable entry point backed by an explicit `VaultStore` (e.g. in-memory or temp SQLite).
pub struct MicaAppInMemory;

impl Component for MicaAppInMemory {
  type Props = ();
  type Event = NoEvent;

  fn view(_props: Self::Props, _scope: Scope<Self::Event>) -> impl IntoElement {
    render_mica_app(VaultStore::in_memory())
  }
}

fn render_mica_app(vault: VaultStore) -> impl IntoElement {
  let vault = Rc::new(vault);
  let origin = DEFAULT_ORIGIN.to_string();

  // Load persisted views from SQLite vault or seed starter workspace on first boot.
  let mut views = vault.list_views(&origin);
  let ws_id: Uuid;
  let default_folder_id: Uuid;
  let initial_doc_id: Uuid;
  let initial_title: String;
  let mut docs_map: HashMap<Uuid, String> = HashMap::new();

  if views.is_empty() {
    ws_id = Uuid::new_v4();
    let folder_id = Uuid::new_v4();
    let doc_id = Uuid::new_v4();
    let folder = make_view(ws_id, folder_id, "Architecture", "folder", None, "a0");
    let page = make_view(
      ws_id,
      doc_id,
      "Fullstack Rust Overview",
      "page",
      Some(folder.id),
      "a0",
    );
    let starter_md = concat!(
      "# Mica Fullstack Rust\n\n",
      "三层纯 Rust 架构（L1 数据引擎 + L2 无头内核 + L3 路线二-B 原生壳）。\n\n",
      "- 单一文档权威：EditorSession → MicaDoc CRDT\n",
      "- 原生中文 IME：支持预编辑实时回显\n",
      "- 无限画板与导图：BoardSession + Canvas\n",
    )
    .to_string();
    let _ = vault.save_view(&origin, &folder);
    let _ = vault.save_view(&origin, &page);
    let _ = vault.save_markdown(&origin, doc_id, &starter_md);
    docs_map.insert(doc_id, starter_md);
    default_folder_id = folder.id;
    initial_doc_id = doc_id;
    initial_title = page.name.clone();
    views = vec![folder, page];
  } else {
    ws_id = views[0].workspace_id;
    default_folder_id = views
      .iter()
      .find(|v| !v.is_deleted && v.object_type == "folder")
      .map(|v| v.id)
      .unwrap_or_else(|| {
        let fid = Uuid::new_v4();
        let f = make_view(ws_id, fid, "Workspace", "folder", None, "a0");
        let _ = vault.save_view(&origin, &f);
        views.push(f.clone());
        f.id
      });
    for v in &views {
      if v.object_type != "folder" {
        let md = vault.load_markdown(&origin, v.object_id).unwrap_or_default();
        docs_map.insert(v.object_id, md);
      }
    }
    if let Some(first_page) = views.iter().find(|v| !v.is_deleted && v.object_type != "folder") {
      initial_doc_id = first_page.object_id;
      initial_title = first_page.name.clone();
    } else {
      let doc_id = Uuid::new_v4();
      let page = make_view(
        ws_id,
        doc_id,
        "Untitled Page",
        "page",
        Some(default_folder_id),
        "a0",
      );
      let _ = vault.save_view(&origin, &page);
      docs_map.insert(doc_id, String::new());
      initial_doc_id = doc_id;
      initial_title = page.name.clone();
      views.push(page);
    }
  }

  let mut tree_model = PageTreeModel::new(views);
  tree_model.set_expanded(default_folder_id, true);

  let tree = Rc::new(RefCell::new(tree_model));
  let docs_store: Rc<RefCell<HashMap<Uuid, String>>> = Rc::new(RefCell::new(docs_map));
  let connector = Rc::new(RefCell::new(CloudConnector::new(CloudConfig {
    server_url: "http://127.0.0.1:8080".to_string(),
    access_token: None,
    email: None,
  })));

  let tree_rev = signal(0u64);
  let active_doc = signal(Some(initial_doc_id));
  let active_title = signal(initial_title);
  let active_mode = signal(MODE_DOC);
  let dark_mode = signal(false);
  let locale_zh = signal(true);
  let server_url = signal("http://127.0.0.1:8080".to_string());
  let cloud_status = signal("Mode: Local Offline Vault (SQLite)".to_string());
  let export_preview = signal(String::new());

  let initial_md = docs_store
    .borrow()
    .get(&initial_doc_id)
    .cloned()
    .unwrap_or_default();
  let editor_state = EditorViewState::new(EditorSession::from_markdown(&initial_md));

  // Auto-persist editor edits into SQLite vault + in-memory cache.
  {
    let vault_p = vault.clone();
    let docs_p = docs_store.clone();
    let orig_p = origin.clone();
    editor_state.set_on_persist(Rc::new(move |md: String, _crdt: Option<Vec<u8>>| {
      if let Some(doc_id) = active_doc.get_untracked() {
        docs_p.borrow_mut().insert(doc_id, md.clone());
        let _ = vault_p.save_markdown(&orig_p, doc_id, &md);
      }
    }));
  }

  let initial_board_json = vault
    .load_board_json(&origin, DEFAULT_BOARD_ID)
    .unwrap_or_else(|| mica_board::starter_board_json("mindmap"));
  let board_session = Rc::new(RefCell::new(BoardSession::from_json(&initial_board_json)));
  let board_rev = signal(0u64);

  let on_persist_board: Rc<dyn Fn(String)> = {
    let v = vault.clone();
    let orig = origin.clone();
    Rc::new(move |json: String| {
      let _ = v.save_board_json(&orig, DEFAULT_BOARD_ID, &json);
    })
  };

  // Global search shortcut
  on_key("ctrl+k", move |_| active_mode.set(MODE_SEARCH));
  on_key("cmd+k", move |_| active_mode.set(MODE_SEARCH));

  let on_select_doc: Rc<dyn Fn(Uuid, String)> = {
    let docs = docs_store.clone();
    let ed = editor_state.clone();
    let v = vault.clone();
    let orig = origin.clone();
    Rc::new(move |next_id: Uuid, title: String| {
      if let Some(prev_id) = active_doc.get_untracked() {
        let md = ed.export_markdown();
        docs.borrow_mut().insert(prev_id, md.clone());
        let _ = v.save_markdown(&orig, prev_id, &md);
      }
      let md = docs.borrow().get(&next_id).cloned().unwrap_or_default();
      ed.load_session(EditorSession::from_markdown(&md));
      active_doc.set(Some(next_id));
      active_title.set(title);
      active_mode.set(MODE_DOC);
    })
  };

  let on_new_page: Rc<dyn Fn()> = {
    let tree_ref = tree.clone();
    let docs = docs_store.clone();
    let ed = editor_state.clone();
    let v = vault.clone();
    let orig = origin.clone();
    Rc::new(move || {
      if let Some(prev_id) = active_doc.get_untracked() {
        let prev_md = ed.export_markdown();
        docs.borrow_mut().insert(prev_id, prev_md.clone());
        let _ = v.save_markdown(&orig, prev_id, &prev_md);
      }
      let new_doc_id = Uuid::new_v4();
      let idx = tree_ref.borrow().count_live_pages() + 1;
      let name = format!("Untitled Page {idx}");
      let view_row = make_view(
        ws_id,
        new_doc_id,
        &name,
        "page",
        Some(default_folder_id),
        &format!("z{idx:04}"),
      );
      let _ = v.save_view(&orig, &view_row);
      let _ = v.save_markdown(&orig, new_doc_id, "");
      {
        let mut t = tree_ref.borrow_mut();
        t.upsert_view(view_row);
        t.set_expanded(default_folder_id, true);
      }
      docs.borrow_mut().insert(new_doc_id, String::new());
      ed.load_session(EditorSession::from_markdown(""));
      active_doc.set(Some(new_doc_id));
      active_title.set(name);
      active_mode.set(MODE_DOC);
      tree_rev.set(tree_rev.get_untracked() + 1);
    })
  };

  let on_new_folder: Rc<dyn Fn()> = {
    let tree_ref = tree.clone();
    let v = vault.clone();
    let orig = origin.clone();
    Rc::new(move || {
      let fid = Uuid::new_v4();
      let count = tree_ref.borrow().visible_rows().len() + 1;
      let name = format!("Folder {count}");
      let folder = make_view(ws_id, fid, &name, "folder", None, &format!("f{count:04}"));
      let _ = v.save_view(&orig, &folder);
      {
        let mut t = tree_ref.borrow_mut();
        t.upsert_view(folder.clone());
        t.set_expanded(folder.id, true);
      }
      tree_rev.set(tree_rev.get_untracked() + 1);
    })
  };

  let on_rename_active: Rc<dyn Fn(String)> = {
    let tree_ref = tree.clone();
    let v = vault.clone();
    let orig = origin.clone();
    Rc::new(move |next_name: String| {
      let Some(doc_id) = active_doc.get_untracked() else {
        return;
      };
      let updated = {
        let mut t = tree_ref.borrow_mut();
        let target_id = t
          .visible_rows()
          .into_iter()
          .find(|r| !r.is_folder && r.object_id == doc_id)
          .map(|r| r.view_id);
        target_id.and_then(|vid| {
          let mut row = t.get(vid)?.clone();
          row.name = next_name.clone();
          row.updated_at = Utc::now();
          t.upsert_view(row.clone());
          Some(row)
        })
      };
      if let Some(row) = updated {
        let _ = v.save_view(&orig, &row);
        active_title.set(next_name);
        tree_rev.set(tree_rev.get_untracked() + 1);
      }
    })
  };

  let on_trash_active: Rc<dyn Fn()> = {
    let tree_ref = tree.clone();
    let v = vault.clone();
    let orig = origin.clone();
    Rc::new(move || {
      let Some(doc_id) = active_doc.get_untracked() else {
        return;
      };
      let updated = {
        let mut t = tree_ref.borrow_mut();
        let target_id = t
          .visible_rows()
          .into_iter()
          .find(|r| !r.is_folder && r.object_id == doc_id)
          .map(|r| r.view_id);
        target_id.and_then(|vid| {
          let mut row = t.get(vid)?.clone();
          row.is_deleted = true;
          row.updated_at = Utc::now();
          t.upsert_view(row.clone());
          Some(row)
        })
      };
      if let Some(row) = updated {
        let _ = v.save_view(&orig, &row);
        tree_rev.set(tree_rev.get_untracked() + 1);
      }
    })
  };

  let on_restore_view: Rc<dyn Fn(Uuid)> = {
    let tree_ref = tree.clone();
    let v = vault.clone();
    let orig = origin.clone();
    Rc::new(move |vid: Uuid| {
      let updated = {
        let mut t = tree_ref.borrow_mut();
        t.get(vid).cloned().map(|mut row| {
          row.is_deleted = false;
          row.updated_at = Utc::now();
          t.upsert_view(row.clone());
          row
        })
      };
      if let Some(row) = updated {
        let _ = v.save_view(&orig, &row);
        tree_rev.set(tree_rev.get_untracked() + 1);
      }
    })
  };

  let sidebar_cb = Rc::new(SidebarCallbacks {
    on_select_doc: on_select_doc.clone(),
    on_new_page,
    on_new_folder,
    on_rename_active,
    on_trash_active,
    on_restore_view,
    on_open_search: Rc::new(move || active_mode.set(MODE_SEARCH)),
    on_open_settings: Rc::new(move || active_mode.set(MODE_SETTINGS)),
  });

  let settings_cb = Rc::new(SettingsCallbacks {
    dark_mode,
    locale_zh,
    server_url,
    cloud_status,
    export_preview,
    on_toggle_theme: Rc::new(move || {
      let next = !dark_mode.get_untracked();
      dark_mode.set(next);
      set_color_scheme(if next { Appearance::Dark } else { Appearance::Light });
    }),
    on_toggle_locale: Rc::new(move || locale_zh.set(!locale_zh.get_untracked())),
    on_check_server: {
      let conn = connector.clone();
      Rc::new(move |url: String| {
        let res = {
          let mut c = conn.borrow_mut();
          c.set_server_url(url);
          c.check_health()
        };
        match res {
          Ok(body) => cloud_status.set(format!("Connected: {body}")),
          Err(err) => cloud_status.set(format!("Offline / Unreachable: {err}")),
        }
      })
    },
    on_export_markdown: {
      let ed = editor_state.clone();
      Rc::new(move || {
        let md = ed.export_markdown();
        export_preview.set(if md.is_empty() {
          "Exported Markdown (empty)".to_string()
        } else {
          format!("Exported Markdown OK ({} bytes): {}", md.len(), preview_head(&md))
        });
      })
    },
    on_export_html: {
      let ed = editor_state.clone();
      Rc::new(move || {
        let html = ed.export_html();
        export_preview.set(format!("Exported HTML OK ({} bytes)", html.len()));
      })
    },
    on_save_snapshot: {
      let v = vault.clone();
      let orig = origin.clone();
      Rc::new(move || {
        if let Some(doc_id) = active_doc.get_untracked() {
          match v.create_version_snapshot(&orig, doc_id, Some("Manual snapshot")) {
            Ok(()) => {
              let count = v.list_version_count(&orig, doc_id);
              export_preview.set(format!("Version snapshot saved (total: {count})"));
            }
            Err(e) => export_preview.set(format!("Snapshot failed: {e}")),
          }
        }
      })
    },
    on_close: Rc::new(move || active_mode.set(MODE_DOC)),
  });

  let tree_search = tree.clone();
  let docs_search = docs_store.clone();
  let open_search_doc = on_select_doc;

  view()
    .flex_1()
    .flex_row()
    .bg(move || theme().background)
    .child(sidebar::sidebar_view(
      tree,
      tree_rev,
      active_doc,
      active_title,
      sidebar_cb,
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
                .flex_wrap()
                .gap_1()
                .child(mode_btn("Document", "mode-doc", MODE_DOC, active_mode))
                .child(mode_btn("Whiteboard", "mode-board", MODE_BOARD, active_mode)),
            ),
        )
        .child(when(move || active_mode.get() == MODE_DOC, {
          let ed = editor_state.clone();
          move || editor_view::editor_view(ed.clone(), dark_mode)
        }))
        .child(when(move || active_mode.get() == MODE_BOARD, {
          let b = board_session.clone();
          let pb = on_persist_board.clone();
          move || board_view::board_view(b.clone(), board_rev, pb.clone())
        }))
        .child(when(move || active_mode.get() == MODE_SEARCH, {
          let ts = tree_search.clone();
          let ds = docs_search.clone();
          let od = open_search_doc.clone();
          move || {
            search_view::search_panel_view(
              ts.clone(),
              ds.clone(),
              od.clone(),
              Rc::new(move || active_mode.set(MODE_DOC)),
            )
          }
        }))
        .child(when(move || active_mode.get() == MODE_SETTINGS, {
          let scb = settings_cb.clone();
          move || settings_view::settings_panel_view(scb.clone())
        })),
    )
}

fn preview_head(s: &str) -> String {
  let single = s.replace('\n', " ");
  let mut out: String = single.chars().take(60).collect();
  if single.chars().count() > 60 {
    out.push('…');
  }
  out
}

fn mode_btn(label: &'static str, tid: &'static str, mode: u8, active: Signal<u8>) -> impl IntoElement {
  button()
    .test_id(tid)
    .px_3()
    .py_1()
    .rounded(move || theme().radius)
    .border_1()
    .border_color(move || theme().border)
    .bg(move || {
      if active.get() == mode {
        theme().primary
      } else {
        theme().secondary
      }
    })
    .on_press(move || active.set(mode))
    .child(
      text(label)
        .text_xs()
        .text_color(move || {
          if active.get() == mode {
            theme().primary_foreground
          } else {
            theme().secondary_foreground
          }
        }),
    )
}

fn make_view(
  workspace_id: Uuid,
  object_id: Uuid,
  name: &str,
  object_type: &str,
  parent_view_id: Option<Uuid>,
  sort_order: &str,
) -> View {
  let now = Utc::now();
  View {
    id: Uuid::new_v4(),
    workspace_id,
    parent_view_id,
    object_type: object_type.to_owned(),
    object_id,
    name: name.to_owned(),
    icon: None,
    position: sort_order.to_owned(),
    is_deleted: false,
    created_by: Uuid::nil(),
    created_at: now,
    updated_at: now,
    state_bytes: None,
  }
}
