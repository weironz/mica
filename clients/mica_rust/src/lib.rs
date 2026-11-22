//! Mica L3 pure-Rust cross-platform client (`clients/mica_rust`).
//!
//! 1:1 visual & interaction parity with Mica's warm-gray Notion-style shell:
//! - Official blue-purple Mica brand mark (`assets/mica-logo.svg`).
//! - 36px top bar: continuous 244px warm-gray sidebar top + `< >` real history navigation +
//!   seamless browser-style multi-document tabs (`DocTabStrip`) + `+`.
//! - Collapsible 244px left sidebar with interactive Workspace Switcher popover, recessed
//!   Search pill (opening floating modal), collapsible page tree, Recycle Bin, interactive
//!   bottom Account Menu popover, and Settings gear (opening two-column Settings modal).
//! - Centered 740px paper editor with strict single-row breadcrumb + icon toolbar, native
//!   cross-block mouse drag selection, rich inline spans, Enter/Backspace/↑↓/Ctrl+A/Z/Y/B/I/E,
//!   and collapsible 248px right sidebar (Page Outline, Properties, Version Snapshots, Backlinks).
//! - Direct L1 + L2 Rust core calls (zero Dart, zero Flutter, zero FFI/JSON bridge).

pub mod atomic_blocks;
pub mod board_view;
pub mod editor_slash;
pub mod editor_table;
pub mod editor_view;
pub mod mica_icons;
pub mod mica_theme;
pub mod net;
pub mod right_sidebar;
pub mod search_view;
pub mod settings_view;
pub mod sidebar;
pub mod storage;
#[cfg(target_arch = "wasm32")]
pub mod web;

use std::{
  cell::RefCell,
  collections::{HashMap, HashSet},
  rc::Rc,
};

use chrono::Utc;
use mica_api_types::View;
use mica_board::BoardSession;
use mica_client::PageTreeModel;
use mica_editor::{EditorBlock, EditorSession};
use nsh_base::button;
use nsh_core::{
  element::{KeyOptions, on_key_with},
  geometry::px,
  keymap::When,
  prelude::*,
};
use uuid::Uuid;

use crate::{
  editor_view::{EditorHeaderProps, EditorViewState},
  mica_theme::{install_mica_theme, palette},
  net::{CloudConfig, CloudConnector},
  right_sidebar::RightSidebarProps,
  settings_view::SettingsCallbacks,
  sidebar::SidebarCallbacks,
  storage::{DEFAULT_ORIGIN, VaultStore},
};

const MODE_DOC: u8 = 0;
const MODE_BOARD: u8 = 1;

const DEFAULT_BOARD_ID: &str = "workspace-default-board";

#[derive(Clone, PartialEq, Eq)]
struct OpenTab {
  doc_id: Uuid,
  title: String,
}

#[derive(Clone, PartialEq, Eq)]
struct WorkspaceEntry {
  id: Uuid,
  name: String,
  origin: String,
}

/// Root pure-Rust Mica application component (shared by desktop & Web targets).
pub struct MicaApp;

impl Component for MicaApp {
  type Props = ();
  type Event = NoEvent;

  fn view(_props: Self::Props, _scope: Scope<Self::Event>) -> impl IntoElement {
    install_mica_theme();
    render_mica_app(VaultStore::open_default())
  }
}

/// Testable entry point backed by an explicit in-memory `VaultStore`.
pub struct MicaAppInMemory;

impl Component for MicaAppInMemory {
  type Props = ();
  type Event = NoEvent;

  fn view(_props: Self::Props, _scope: Scope<Self::Event>) -> impl IntoElement {
    install_mica_theme();
    render_mica_app(VaultStore::in_memory())
  }
}

fn render_mica_app(vault: VaultStore) -> impl IntoElement {
  let vault = Rc::new(vault);
  let active_origin = signal(DEFAULT_ORIGIN.to_string());
  let initial_origin = DEFAULT_ORIGIN.to_string();

  let mut views = vault.list_views(&initial_origin);
  let ws_id: Uuid;
  let default_folder_id: Uuid;
  let initial_doc_id: Uuid;
  let initial_title: String;
  let initial_folder_name: String;

  if views.is_empty() {
    ws_id = Uuid::new_v4();
    let folder_id = Uuid::new_v4();
    let doc_id = Uuid::new_v4();
    let folder_view = make_view(ws_id, folder_id, "知识库", "folder", None, "a0");
    let doc_view = make_view(ws_id, doc_id, "欢迎使用 Mica", "page", Some(folder_id), "a1");
    let _ = vault.save_view(&initial_origin, &folder_view);
    let _ = vault.save_view(&initial_origin, &doc_view);
    let _ = vault.save_markdown(&initial_origin, doc_id, STARTER_MARKDOWN);
    let _ = vault.create_version_snapshot(&initial_origin, doc_id, Some("初始版本"));
    views = vec![folder_view, doc_view];
    default_folder_id = folder_id;
    initial_doc_id = doc_id;
    initial_title = "欢迎使用 Mica".to_string();
    initial_folder_name = "知识库".to_string();
  } else {
    ws_id = views[0].workspace_id;
    let folder = views.iter().find(|v| v.object_type == "folder" && !v.is_deleted);
    default_folder_id = folder.map(|v| v.id).unwrap_or_else(Uuid::new_v4);
    initial_folder_name = folder
      .map(|v| v.name.clone())
      .unwrap_or_else(|| "工作区".to_string());
    if let Some(page) = views.iter().find(|v| v.object_type == "page" && !v.is_deleted) {
      initial_doc_id = page.object_id;
      initial_title = page.name.clone();
    } else {
      let doc_id = Uuid::new_v4();
      let doc_view = make_view(ws_id, doc_id, "未命名页面", "page", Some(default_folder_id), "a1");
      let _ = vault.save_view(&initial_origin, &doc_view);
      let _ = vault.save_markdown(&initial_origin, doc_id, "");
      views.push(doc_view);
      initial_doc_id = doc_id;
      initial_title = "未命名页面".to_string();
    }
  }

  let mut docs_map: HashMap<Uuid, String> = HashMap::new();
  for v in &views {
    if v.object_type == "page" {
      let md = vault
        .load_markdown(&initial_origin, v.object_id)
        .unwrap_or_default();
      docs_map.insert(v.object_id, md);
    }
  }
  let docs_store = Rc::new(RefCell::new(docs_map));

  let mut tree_model = PageTreeModel::new(views);
  tree_model.set_expanded(default_folder_id, true);
  let tree = Rc::new(RefCell::new(tree_model));

  let workspaces = signal(vec![WorkspaceEntry {
    id: ws_id,
    name: "Mica 本地工作区".to_string(),
    origin: initial_origin.clone(),
  }]);
  let active_workspace_id = signal(ws_id);
  let active_folder_id = signal(default_folder_id);

  let tree_rev = signal(0u64);
  let active_doc: Signal<Option<Uuid>> = signal(Some(initial_doc_id));
  let active_doc_id_str = signal(initial_doc_id.to_string());
  let active_title = signal(initial_title.clone());
  let active_folder_name = signal(initial_folder_name);
  let workspace_name = signal("Mica 本地工作区".to_string());
  let account_name = signal("local".to_string());
  let active_mode = signal(MODE_DOC);
  let sidebar_open = signal(true);
  let right_panel_open = signal(false);
  let search_modal_open = signal(false);
  let settings_modal_open = signal(false);
  let workspace_menu_open = signal(false);
  let account_menu_open = signal(false);
  let new_ws_draft = signal(String::new());

  let open_tabs = signal(vec![OpenTab {
    doc_id: initial_doc_id,
    title: initial_title,
  }]);

  // Real back/forward navigation history stack
  let nav_history: Rc<RefCell<Vec<(Uuid, String)>>> =
    Rc::new(RefCell::new(vec![(initial_doc_id, active_title.get_untracked())]));
  let nav_cursor = signal(0usize);
  let nav_len = signal(1usize);

  // Favorites set
  let favorites: Rc<RefCell<HashSet<Uuid>>> = Rc::new(RefCell::new(HashSet::new()));
  let active_is_favorite = signal(false);

  // Snapshot count for active doc
  let snapshot_count = signal(vault.list_version_count(&initial_origin, initial_doc_id));

  let dark_mode = signal(false);
  let locale_zh = signal(true);
  let server_url = signal("http://127.0.0.1:8080".to_string());
  let cloud_status = signal("模式：本地离线 SQLite Vault".to_string());
  let export_preview = signal(String::new());

  let initial_md = docs_store
    .borrow()
    .get(&initial_doc_id)
    .cloned()
    .unwrap_or_default();
  let editor_state = EditorViewState::new(session_for_markdown(&initial_md));

  {
    let vault_p = vault.clone();
    let docs_p = docs_store.clone();
    editor_state.set_on_persist(Rc::new(move |md: String, _crdt: Option<Vec<u8>>| {
      if let Some(doc_id) = active_doc.get_untracked() {
        let orig = active_origin.get_untracked();
        docs_p.borrow_mut().insert(doc_id, md.clone());
        let _ = vault_p.save_markdown(&orig, doc_id, &md);
      }
    }));
  }

  let initial_board_json = vault
    .load_board_json(&initial_origin, DEFAULT_BOARD_ID)
    .unwrap_or_else(|| mica_board::starter_board_json("mindmap"));
  let board_session = Rc::new(RefCell::new(BoardSession::from_json(&initial_board_json)));
  let board_rev = signal(0u64);

  let on_persist_board: Rc<dyn Fn(String)> = {
    let v = vault.clone();
    Rc::new(move |json: String| {
      let orig = active_origin.get_untracked();
      let _ = v.save_board_json(&orig, DEFAULT_BOARD_ID, &json);
    })
  };

  // Global window keyboard shortcuts (Ctrl+Shift+F Search, Ctrl+, Settings, Esc dismiss modals)
  on_key_with(
    "shift+mod+f",
    KeyOptions {
      when: When::Always,
      ..KeyOptions::default()
    },
    move |_| {
      workspace_menu_open.set(false);
      account_menu_open.set(false);
      settings_modal_open.set(false);
      search_modal_open.set(!search_modal_open.get_untracked());
    },
  );
  on_key_with(
    "mod+,",
    KeyOptions {
      when: When::Always,
      ..KeyOptions::default()
    },
    move |_| {
      workspace_menu_open.set(false);
      account_menu_open.set(false);
      search_modal_open.set(false);
      settings_modal_open.set(!settings_modal_open.get_untracked());
    },
  );
  on_key_with(
    "escape",
    KeyOptions {
      when: When::Always,
      pass_through: true,
      ..KeyOptions::default()
    },
    move |_| {
      search_modal_open.set(false);
      settings_modal_open.set(false);
      workspace_menu_open.set(false);
      account_menu_open.set(false);
    },
  );

  // Core document selection helper (with optional history push)
  let open_doc_internal: Rc<dyn Fn(Uuid, String, bool)> = {
    let docs = docs_store.clone();
    let ed = editor_state.clone();
    let tree_ref = tree.clone();
    let v = vault.clone();
    let favs = favorites.clone();
    let hist = nav_history.clone();
    Rc::new(move |doc_id: Uuid, title: String, record_history: bool| {
      let orig = active_origin.get_untracked();
      let md = docs
        .borrow()
        .get(&doc_id)
        .cloned()
        .or_else(|| v.load_markdown(&orig, doc_id))
        .unwrap_or_default();
      docs.borrow_mut().insert(doc_id, md.clone());
      ed.load_session(session_for_markdown(&md));
      active_doc.set(Some(doc_id));
      active_doc_id_str.set(doc_id.to_string());
      active_title.set(title.clone());
      active_is_favorite.set(favs.borrow().contains(&doc_id));
      snapshot_count.set(v.list_version_count(&orig, doc_id));

      if let Some(parent_name) = resolve_parent_folder_name(&tree_ref.borrow(), doc_id) {
        active_folder_name.set(parent_name);
      }

      let mut tabs = open_tabs.get_untracked();
      if let Some(existing) = tabs.iter_mut().find(|t| t.doc_id == doc_id) {
        existing.title = title.clone();
      } else {
        tabs.push(OpenTab {
          doc_id,
          title: title.clone(),
        });
      }
      open_tabs.set(tabs);

      if record_history {
        let mut h = hist.borrow_mut();
        let cur = nav_cursor.get_untracked();
        if h.get(cur).map(|(id, _)| *id) != Some(doc_id) {
          h.truncate(cur + 1);
          h.push((doc_id, title));
          let next_idx = h.len() - 1;
          nav_cursor.set(next_idx);
          nav_len.set(h.len());
        }
      }

      active_mode.set(MODE_DOC);
    })
  };

  let on_select_doc: Rc<dyn Fn(Uuid, String)> = {
    let inner = open_doc_internal.clone();
    Rc::new(move |doc_id: Uuid, title: String| inner(doc_id, title, true))
  };

  let on_nav_back: Rc<dyn Fn()> = {
    let hist = nav_history.clone();
    let inner = open_doc_internal.clone();
    Rc::new(move || {
      let cur = nav_cursor.get_untracked();
      if cur > 0 {
        let next = cur - 1;
        if let Some((doc_id, title)) = hist.borrow().get(next).cloned() {
          nav_cursor.set(next);
          inner(doc_id, title, false);
        }
      }
    })
  };

  let on_nav_forward: Rc<dyn Fn()> = {
    let hist = nav_history.clone();
    let inner = open_doc_internal.clone();
    Rc::new(move || {
      let cur = nav_cursor.get_untracked();
      let next = cur + 1;
      if let Some((doc_id, title)) = hist.borrow().get(next).cloned() {
        nav_cursor.set(next);
        inner(doc_id, title, false);
      }
    })
  };

  let on_new_page: Rc<dyn Fn()> = {
    let tree_ref = tree.clone();
    let docs = docs_store.clone();
    let v = vault.clone();
    let select = on_select_doc.clone();
    Rc::new(move || {
      let orig = active_origin.get_untracked();
      let cur_ws = active_workspace_id.get_untracked();
      let cur_folder = active_folder_id.get_untracked();
      let new_doc_id = Uuid::new_v4();
      let idx = tree_ref
        .borrow()
        .visible_rows()
        .iter()
        .filter(|r| !r.is_folder)
        .count()
        + 1;
      let name = format!("未命名页面 {idx}");
      let view_row = make_view(
        cur_ws,
        new_doc_id,
        &name,
        "page",
        Some(cur_folder),
        &format!("z{idx:04}"),
      );
      let _ = v.save_view(&orig, &view_row);
      let _ = v.save_markdown(&orig, new_doc_id, "");
      {
        let mut t = tree_ref.borrow_mut();
        t.upsert_view(view_row);
        t.set_expanded(cur_folder, true);
      }
      docs.borrow_mut().insert(new_doc_id, String::new());
      tree_rev.set(tree_rev.get_untracked() + 1);
      select(new_doc_id, name);
    })
  };

  let on_new_folder: Rc<dyn Fn()> = {
    let tree_ref = tree.clone();
    let v = vault.clone();
    Rc::new(move || {
      let orig = active_origin.get_untracked();
      let cur_ws = active_workspace_id.get_untracked();
      let fid = Uuid::new_v4();
      let count = tree_ref.borrow().visible_rows().len() + 1;
      let name = format!("文件夹 {count}");
      let folder = make_view(cur_ws, fid, &name, "folder", None, &format!("f{count:04}"));
      let _ = v.save_view(&orig, &folder);
      {
        let mut t = tree_ref.borrow_mut();
        t.upsert_view(folder.clone());
        t.set_expanded(folder.id, true);
      }
      tree_rev.set(tree_rev.get_untracked() + 1);
    })
  };

  let on_switch_workspace: Rc<dyn Fn(WorkspaceEntry)> = {
    let tree_ref = tree.clone();
    let docs = docs_store.clone();
    let v = vault.clone();
    let ed = editor_state.clone();
    let hist = nav_history.clone();
    Rc::new(move |entry: WorkspaceEntry| {
      workspace_menu_open.set(false);
      active_workspace_id.set(entry.id);
      active_origin.set(entry.origin.clone());
      workspace_name.set(entry.name.clone());

      let mut ws_views = v.list_views(&entry.origin);
      let (fid, fname, doc_id, title) = if ws_views.is_empty() {
        let fid = Uuid::new_v4();
        let did = Uuid::new_v4();
        let f = make_view(entry.id, fid, "知识库", "folder", None, "a0");
        let p = make_view(entry.id, did, "工作区首页", "page", Some(fid), "a1");
        let _ = v.save_view(&entry.origin, &f);
        let _ = v.save_view(&entry.origin, &p);
        let _ = v.save_markdown(&entry.origin, did, "# 工作区首页\n\n在此开始记录。");
        ws_views = vec![f, p];
        (fid, "知识库".to_string(), did, "工作区首页".to_string())
      } else {
        let f = ws_views
          .iter()
          .find(|r| r.object_type == "folder" && !r.is_deleted);
        let fid = f.map(|r| r.id).unwrap_or_else(Uuid::new_v4);
        let fname = f.map(|r| r.name.clone()).unwrap_or_else(|| "工作区".into());
        if let Some(p) = ws_views
          .iter()
          .find(|r| r.object_type == "page" && !r.is_deleted)
        {
          (fid, fname, p.object_id, p.name.clone())
        } else {
          let did = Uuid::new_v4();
          let p = make_view(entry.id, did, "未命名页面", "page", Some(fid), "a1");
          let _ = v.save_view(&entry.origin, &p);
          let _ = v.save_markdown(&entry.origin, did, "");
          ws_views.push(p);
          (fid, fname, did, "未命名页面".to_string())
        }
      };

      let mut next_docs = HashMap::new();
      for row in &ws_views {
        if row.object_type == "page" {
          next_docs.insert(
            row.object_id,
            v.load_markdown(&entry.origin, row.object_id)
              .unwrap_or_default(),
          );
        }
      }
      let active_md = next_docs.get(&doc_id).cloned().unwrap_or_default();
      *docs.borrow_mut() = next_docs;
      {
        let mut next_tree = PageTreeModel::new(ws_views);
        next_tree.set_expanded(fid, true);
        *tree_ref.borrow_mut() = next_tree;
      }
      active_folder_id.set(fid);
      active_folder_name.set(fname);
      active_doc.set(Some(doc_id));
      active_doc_id_str.set(doc_id.to_string());
      active_title.set(title.clone());
      snapshot_count.set(v.list_version_count(&entry.origin, doc_id));
      ed.load_session(session_for_markdown(&active_md));
      open_tabs.set(vec![OpenTab {
        doc_id,
        title: title.clone(),
      }]);
      *hist.borrow_mut() = vec![(doc_id, title)];
      nav_cursor.set(0);
      nav_len.set(1);
      active_mode.set(MODE_DOC);
      tree_rev.set(tree_rev.get_untracked() + 1);
    })
  };

  let on_rename_active: Rc<dyn Fn(String)> = {
    let tree_ref = tree.clone();
    let v = vault.clone();
    Rc::new(move |next_name: String| {
      active_title.set(next_name.clone());
      let Some(doc_id) = active_doc.get_untracked() else {
        return;
      };
      let orig = active_origin.get_untracked();
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
        let mut tabs = open_tabs.get_untracked();
        for tab in &mut tabs {
          if tab.doc_id == doc_id {
            tab.title = next_name.clone();
          }
        }
        open_tabs.set(tabs);
        tree_rev.set(tree_rev.get_untracked() + 1);
      }
    })
  };

  let on_trash_active: Rc<dyn Fn()> = {
    let tree_ref = tree.clone();
    let v = vault.clone();
    Rc::new(move || {
      let Some(doc_id) = active_doc.get_untracked() else {
        return;
      };
      let orig = active_origin.get_untracked();
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
    Rc::new(move |vid: Uuid| {
      let orig = active_origin.get_untracked();
      let updated = {
        let mut t = tree_ref.borrow_mut();
        t.get(vid).cloned().map(|mut row| {
          row.is_deleted = false;
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

  let on_toggle_favorite: Rc<dyn Fn()> = {
    let favs = favorites.clone();
    let ed = editor_state.clone();
    Rc::new(move || {
      if let Some(doc_id) = active_doc.get_untracked() {
        let mut set = favs.borrow_mut();
        let now_fav = if set.contains(&doc_id) {
          set.remove(&doc_id);
          false
        } else {
          set.insert(doc_id);
          true
        };
        active_is_favorite.set(now_fav);
        ed.set_status_flash(if now_fav { "已加入收藏" } else { "已取消收藏" });
      }
    })
  };

  let on_export_markdown: Rc<dyn Fn()> = {
    let ed = editor_state.clone();
    Rc::new(move || {
      let md = ed.export_markdown();
      let msg = if md.is_empty() {
        "已导出 Markdown（空文档）".to_string()
      } else {
        format!("已导出 Markdown（{} 字节）：{}", md.len(), preview_head(&md))
      };
      export_preview.set(msg);
      ed.set_status_flash("已导出 Markdown");
    })
  };

  let on_export_html: Rc<dyn Fn()> = {
    let ed = editor_state.clone();
    Rc::new(move || {
      let html = ed.export_html();
      export_preview.set(format!("已导出 HTML（{} 字节）", html.len()));
      ed.set_status_flash("已导出 HTML");
    })
  };

  let on_save_snapshot: Rc<dyn Fn()> = {
    let v = vault.clone();
    let ed = editor_state.clone();
    Rc::new(move || {
      if let Some(doc_id) = active_doc.get_untracked() {
        let orig = active_origin.get_untracked();
        match v.create_version_snapshot(&orig, doc_id, Some("手动快照")) {
          Ok(()) => {
            let count = v.list_version_count(&orig, doc_id);
            snapshot_count.set(count);
            export_preview.set(format!("版本快照已保存（共 {count} 个）"));
            ed.set_status_flash(format!("已保存快照 #{count}"));
          }
          Err(e) => export_preview.set(format!("保存快照失败：{e}")),
        }
      }
    })
  };

  let on_toggle_theme: Rc<dyn Fn()> = Rc::new(move || {
    let next = !dark_mode.get_untracked();
    dark_mode.set(next);
    set_color_scheme(if next {
      Appearance::Dark
    } else {
      Appearance::Light
    });
  });

  let on_open_search: Rc<dyn Fn()> = Rc::new(move || {
    workspace_menu_open.set(false);
    account_menu_open.set(false);
    settings_modal_open.set(false);
    search_modal_open.set(true);
  });

  let on_open_settings: Rc<dyn Fn()> = Rc::new(move || {
    workspace_menu_open.set(false);
    account_menu_open.set(false);
    search_modal_open.set(false);
    settings_modal_open.set(true);
  });

  let sidebar_cb = Rc::new(SidebarCallbacks {
    on_select_doc: on_select_doc.clone(),
    on_new_page: on_new_page.clone(),
    on_new_folder,
    on_open_board: Rc::new(move || {
      let next = if active_mode.get_untracked() == MODE_BOARD {
        MODE_DOC
      } else {
        MODE_BOARD
      };
      active_mode.set(next);
    }),
    on_restore_view,
    on_open_search: on_open_search.clone(),
    on_open_settings: on_open_settings.clone(),
    on_toggle_workspace_menu: Rc::new(move || {
      account_menu_open.set(false);
      workspace_menu_open.set(!workspace_menu_open.get_untracked());
    }),
    on_toggle_account_menu: Rc::new(move || {
      workspace_menu_open.set(false);
      account_menu_open.set(!account_menu_open.get_untracked());
    }),
  });

  let editor_header = Rc::new(EditorHeaderProps {
    workspace_name,
    folder_name: active_folder_name,
    page_title: active_title,
    page_id_str: active_doc_id_str,
    is_favorite: active_is_favorite,
    right_panel_open,
    on_rename_title: on_rename_active,
    on_open_board: Rc::new(move || active_mode.set(MODE_BOARD)),
    on_toggle_favorite,
    on_toggle_right_panel: Rc::new(move || {
      right_panel_open.set(!right_panel_open.get_untracked());
    }),
    on_trash_page: on_trash_active,
    on_export_markdown: on_export_markdown.clone(),
    on_export_html: on_export_html.clone(),
    on_save_snapshot: on_save_snapshot.clone(),
  });

  let right_props = Rc::new(RightSidebarProps {
    dark_mode,
    page_title: active_title,
    active_doc,
    outline_items: editor_state.outline_signal(),
    word_count_text: editor_state.word_count_signal(),
    block_count: editor_state.block_count_signal(),
    snapshot_count,
    tree: tree.clone(),
    docs: docs_store.clone(),
    tree_rev,
    on_jump_block: {
      let ed = editor_state.clone();
      Rc::new(move |idx: usize| {
        active_mode.set(MODE_DOC);
        ed.select_block(idx);
      })
    },
    on_open_doc: on_select_doc.clone(),
    on_save_snapshot: on_save_snapshot.clone(),
    on_close: Rc::new(move || right_panel_open.set(false)),
  });

  let settings_cb = Rc::new(SettingsCallbacks {
    dark_mode,
    locale_zh,
    server_url,
    cloud_status,
    export_preview,
    on_toggle_theme: on_toggle_theme.clone(),
    on_toggle_locale: Rc::new(move || locale_zh.set(!locale_zh.get_untracked())),
    on_check_server: Rc::new(move |url: String| {
      let connector = CloudConnector::new(CloudConfig {
        server_url: url.clone(),
        access_token: None,
        email: None,
      });
      match connector.check_health() {
        Ok(ver) => cloud_status.set(format!("连接正常：{url}（{ver}）")),
        Err(err) => cloud_status.set(format!("离线 / 无法连接：{err}")),
      }
    }),
    on_export_markdown,
    on_export_html,
    on_save_snapshot,
    on_close: Rc::new(move || settings_modal_open.set(false)),
  });

  let tree_search = tree.clone();
  let docs_search = docs_store.clone();
  let sel_for_tabs = on_select_doc.clone();
  let new_for_tabs = on_new_page.clone();
  let ws_switch_cb = on_switch_workspace.clone();
  let ws_create_switch = on_switch_workspace;

  view()
    .flex_1()
    .relative()
    .bg(move || palette(dark_mode.get()).surface_base)
    .child(
      // 1. Original 36px top bar: Official Mica Logo + Sidebar Toggle | `< >` + Tabs + `+`
      view()
        .h(px(36.0))
        .w_full()
        .flex_row()
        .items_end()
        .bg(move || palette(dark_mode.get()).surface_sunken)
        .child(
          view()
            .w(move || {
              if sidebar_open.get() {
                px(244.0)
              } else {
                px(72.0)
              }
            })
            .h_full()
            .px_3()
            .flex_row()
            .items_center()
            .gap_2()
            .flex_shrink_0()
            .bg(move || {
              let p = palette(dark_mode.get());
              if sidebar_open.get() {
                p.surface_raised
              } else {
                p.surface_sunken
              }
            })
            .border_r(move || {
              if sidebar_open.get() {
                px(1.0)
              } else {
                px(0.0)
              }
            })
            .border_r_color(move || palette(dark_mode.get()).border_subtle)
            .child(mica_icons::mica_logo())
            .child(
              button()
                .test_id("toggle-sidebar-btn")
                .accessibility_label("折叠或展开左侧边栏")
                .w(px(26.0))
                .h(px(26.0))
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                .on_press(move || sidebar_open.set(!sidebar_open.get_untracked()))
                .child(dynamic(move || {
                  mica_icons::sidebar_toggle(palette(dark_mode.get()).text_muted_hex)
                })),
            ),
        )
        .child(
          // DocTabStrip: `<` `>` real history + open document tabs + `+`
          view()
            .flex_1()
            .h_full()
            .px_2()
            .flex_row()
            .items_end()
            .gap_1()
            .child(
              view()
                .h_full()
                .flex_row()
                .items_center()
                .gap_1()
                .pr_1()
                .child(
                  button()
                    .test_id("nav-back-btn")
                    .accessibility_label("后退到上一页")
                    .w(px(22.0))
                    .h(px(22.0))
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                    .on_press(move || on_nav_back())
                    .child(dynamic(move || {
                      let p = palette(dark_mode.get());
                      mica_icons::chevron_left(if nav_cursor.get() > 0 {
                        p.text_muted_hex
                      } else {
                        p.text_ghost_hex
                      })
                    })),
                )
                .child(
                  button()
                    .test_id("nav-forward-btn")
                    .accessibility_label("前进到下一页")
                    .w(px(22.0))
                    .h(px(22.0))
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                    .on_press(move || on_nav_forward())
                    .child(dynamic(move || {
                      let p = palette(dark_mode.get());
                      let can_fwd = nav_cursor.get() + 1 < nav_len.get();
                      mica_icons::chevron_right(if can_fwd {
                        p.text_muted_hex
                      } else {
                        p.text_ghost_hex
                      })
                    })),
                ),
            )
            .child(dynamic(move || {
              let tabs = open_tabs.get();
              let curr = active_doc.get();
              let mode = active_mode.get();
              let p = palette(dark_mode.get());
              let mut strip = view().flex_row().items_end().gap_1();
              for (idx, tab) in tabs.iter().enumerate() {
                let is_active = mode == MODE_DOC && curr == Some(tab.doc_id);
                let sel_cb = sel_for_tabs.clone();
                let doc_id = tab.doc_id;
                let title_for_sel = tab.title.clone();
                let title_display = if tab.title.trim().is_empty() {
                  "无标题".to_string()
                } else {
                  tab.title.clone()
                };
                let can_close = tabs.len() > 1;
                let close_id = tab.doc_id;
                let sel_after_close = sel_for_tabs.clone();

                let mut tab_el = view()
                  .h(px(30.0))
                  .px_3()
                  .flex_row()
                  .items_center()
                  .gap_2()
                  .rounded_t(px(8.0))
                  .bg(if is_active {
                    p.surface_base
                  } else {
                    p.surface_sunken
                  });

                tab_el = tab_el.child(
                  button()
                    .test_id(&format!("doc-tab-{idx}"))
                    .accessibility_label(format!("切换到标签 {}", title_display))
                    .on_press(move || sel_cb(doc_id, title_for_sel.clone()))
                    .child(
                      text(title_display)
                        .text_size(px(13.0))
                        .font_weight(if is_active {
                          FontWeight::MEDIUM
                        } else {
                          FontWeight::NORMAL
                        })
                        .text_color(if is_active {
                          p.text_primary
                        } else {
                          p.text_muted
                        }),
                    ),
                );

                if can_close {
                  tab_el = tab_el.child(
                    button()
                      .test_id(&format!("doc-tab-close-{idx}"))
                      .accessibility_label("关闭标签页")
                      .w(px(18.0))
                      .h(px(18.0))
                      .items_center()
                      .justify_center()
                      .rounded(px(4.0))
                      .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                      .on_press(move || {
                        let mut cur_tabs = open_tabs.get_untracked();
                        if cur_tabs.len() <= 1 {
                          return;
                        }
                        cur_tabs.retain(|t| t.doc_id != close_id);
                        let next_tab = cur_tabs.last().cloned();
                        open_tabs.set(cur_tabs);
                        if active_doc.get_untracked() == Some(close_id)
                          && let Some(nt) = next_tab
                        {
                          sel_after_close(nt.doc_id, nt.title);
                        }
                      })
                      .child(mica_icons::close_sm(p.text_faint_hex)),
                  );
                }

                strip = strip.child(tab_el);
              }
              strip
            }))
            .child(
              view().h_full().items_center().justify_center().pl_1().child(
                button()
                  .test_id("tab-new-page-btn")
                  .accessibility_label("新建页面")
                  .w(px(24.0))
                  .h(px(24.0))
                  .items_center()
                  .justify_center()
                  .rounded(px(6.0))
                  .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                  .on_press(move || new_for_tabs())
                  .child(dynamic(move || {
                    mica_icons::plus(palette(dark_mode.get()).text_muted_hex)
                  })),
              ),
            ),
        ),
    )
    .child(
      // 2. Main workspace body: Collapsible Left Sidebar + Editor/Board + Collapsible Right Sidebar
      view()
        .flex_1()
        .w_full()
        .min_h(px(0.0))
        .flex_row()
        .child(when(move || sidebar_open.get(), {
          let t = tree.clone();
          let cb = sidebar_cb.clone();
          move || {
            sidebar::sidebar_view(
              t.clone(),
              tree_rev,
              active_doc,
              workspace_name,
              account_name,
              dark_mode,
              cb.clone(),
            )
          }
        }))
        .child(
          view()
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .bg(move || palette(dark_mode.get()).surface_base)
            .child(when(move || active_mode.get() == MODE_DOC, {
              let ed = editor_state.clone();
              let hdr = editor_header.clone();
              move || editor_view::editor_view(ed.clone(), hdr.clone(), dark_mode)
            }))
            .child(when(move || active_mode.get() == MODE_BOARD, {
              let bs = board_session.clone();
              let pb = on_persist_board.clone();
              move || {
                view()
                  .flex_1()
                  .child(
                    view()
                      .h(px(40.0))
                      .px_4()
                      .flex_row()
                      .items_center()
                      .justify_between()
                      .border_b_1()
                      .border_color(move || palette(dark_mode.get()).border_subtle)
                      .child(
                        text("无限画板与思维导图")
                          .text_size(px(13.0))
                          .font_weight(FontWeight::MEDIUM)
                          .text_color(move || palette(dark_mode.get()).text_primary),
                      )
                      .child(
                        button()
                          .test_id("board-back-to-doc")
                          .accessibility_label("返回文档编辑")
                          .px_2()
                          .py_1()
                          .rounded(px(6.0))
                          .bg(move || palette(dark_mode.get()).surface_sunken)
                          .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                          .on_press(move || active_mode.set(MODE_DOC))
                          .child(
                            text("返回文档")
                              .text_size(px(12.0))
                              .text_color(move || palette(dark_mode.get()).text_primary),
                          ),
                      ),
                  )
                  .child(board_view::board_view(bs.clone(), board_rev, pb.clone()))
              }
            })),
        )
        .child(when(move || right_panel_open.get(), {
          let rp = right_props.clone();
          move || right_sidebar::right_sidebar_view(rp.clone())
        })),
    )
    .child(when(move || workspace_menu_open.get(), {
      let sw_list = ws_switch_cb.clone();
      let sw_create = ws_create_switch.clone();
      move || {
        let sw_l = sw_list.clone();
        let sw_c = sw_create.clone();
        view()
          .test_id("workspace-switcher-popover")
          .absolute()
          .top(px(78.0))
          .left(px(10.0))
          .w(px(256.0))
          .p_2()
          .gap_2()
          .rounded(px(10.0))
          .border_1()
          .border_color(move || palette(dark_mode.get()).border_normal)
          .bg(move || palette(dark_mode.get()).surface_overlay)
          .z_index(35)
          .child(
            view()
              .px_2()
              .flex_row()
              .items_center()
              .justify_between()
              .child(
                text("切换工作区")
                  .text_size(px(12.0))
                  .font_weight(FontWeight::MEDIUM)
                  .text_color(move || palette(dark_mode.get()).text_faint),
              )
              .child(
                button()
                  .test_id("workspace-menu-close")
                  .accessibility_label("关闭工作区菜单")
                  .w(px(20.0))
                  .h(px(20.0))
                  .items_center()
                  .justify_center()
                  .rounded(px(4.0))
                  .on_press(move || workspace_menu_open.set(false))
                  .child(dynamic(move || {
                    mica_icons::close_sm(palette(dark_mode.get()).text_faint_hex)
                  })),
              ),
          )
          .child(dynamic(move || {
            let list = workspaces.get();
            let cur_id = active_workspace_id.get();
            let p = palette(dark_mode.get());
            let mut col = view().w_full().gap_1();
            for (i, entry) in list.into_iter().enumerate() {
              let is_cur = entry.id == cur_id;
              let sw = sw_l.clone();
              let name = entry.name.clone();
              let entry_c = entry.clone();
              col = col.child(
                button()
                  .test_id(&format!("workspace-item-{i}"))
                  .accessibility_label(format!("切换到工作区 {name}"))
                  .w_full()
                  .px_2()
                  .py_1()
                  .flex_row()
                  .items_center()
                  .justify_between()
                  .rounded(px(6.0))
                  .bg(if is_cur {
                    p.surface_hover
                  } else {
                    p.surface_overlay
                  })
                  .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                  .on_press(move || sw(entry_c.clone()))
                  .child(
                    view()
                      .flex_row()
                      .items_center()
                      .gap_2()
                      .child(mica_icons::cloud(p.text_muted_hex))
                      .child(
                        text(name)
                          .text_size(px(13.0))
                          .font_weight(if is_cur {
                            FontWeight::MEDIUM
                          } else {
                            FontWeight::NORMAL
                          })
                          .text_color(p.text_primary)
                          .truncate(),
                      ),
                  )
                  .child(if is_cur {
                    view().child(mica_icons::check_icon(p.accent_hex))
                  } else {
                    view()
                  }),
              );
            }
            col
          }))
          .child(
            view()
              .w_full()
              .h(px(1.0))
              .bg(move || palette(dark_mode.get()).border_subtle),
          )
          .child(
            view()
              .flex_row()
              .items_center()
              .gap_1()
              .child(
                nsh_base::input()
                  .test_id("new-workspace-input")
                  .placeholder("新建本地工作区名称...")
                  .flex_1()
                  .px_2()
                  .py_1()
                  .border_1()
                  .border_color(move || palette(dark_mode.get()).border_normal)
                  .rounded(px(6.0))
                  .text_size(px(12.5))
                  .text_color(move || palette(dark_mode.get()).text_primary)
                  .value(move || new_ws_draft.get())
                  .on_change(move |v: &str| new_ws_draft.set(v.to_owned())),
              )
              .child(
                button()
                  .test_id("create-workspace-btn")
                  .accessibility_label("创建新工作区")
                  .px_2()
                  .py_1()
                  .rounded(px(6.0))
                  .bg(move || palette(dark_mode.get()).surface_sunken)
                  .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                  .on_press(move || {
                    let raw = new_ws_draft.get_untracked();
                    let name = if raw.trim().is_empty() {
                      format!("本地工作区 {}", workspaces.get_untracked().len() + 1)
                    } else {
                      raw.trim().to_string()
                    };
                    let id = Uuid::new_v4();
                    let entry = WorkspaceEntry {
                      id,
                      name,
                      origin: format!("local://{id}"),
                    };
                    let mut all = workspaces.get_untracked();
                    all.push(entry.clone());
                    workspaces.set(all);
                    new_ws_draft.set(String::new());
                    sw_c(entry);
                  })
                  .child(
                    text("新建")
                      .text_size(px(12.0))
                      .text_color(move || palette(dark_mode.get()).text_primary),
                  ),
              ),
          )
      }
    }))
    .child(when(move || account_menu_open.get(), {
      let toggle_t = on_toggle_theme.clone();
      let open_set = on_open_settings.clone();
      let open_srch = on_open_search.clone();
      move || {
        let tt = toggle_t.clone();
        let os = open_set.clone();
        let osr = open_srch.clone();
        view()
          .test_id("account-menu-popover")
          .absolute()
          .bottom(px(50.0))
          .left(px(10.0))
          .w(px(256.0))
          .p_2()
          .gap_1()
          .rounded(px(10.0))
          .border_1()
          .border_color(move || palette(dark_mode.get()).border_normal)
          .bg(move || palette(dark_mode.get()).surface_overlay)
          .z_index(35)
          .child(
            view()
              .px_2()
              .py_2()
              .flex_row()
              .items_center()
              .gap_2()
              .child(
                view()
                  .w(px(30.0))
                  .h(px(30.0))
                  .rounded_full()
                  .items_center()
                  .justify_center()
                  .bg(move || palette(dark_mode.get()).surface_hover)
                  .child(
                    text("L")
                      .text_size(px(13.0))
                      .font_weight(FontWeight::SEMIBOLD)
                      .text_color(move || palette(dark_mode.get()).text_primary),
                  ),
              )
              .child(
                view()
                  .flex_1()
                  .child(
                    text(move || account_name.get())
                      .text_size(px(13.5))
                      .font_weight(FontWeight::MEDIUM)
                      .text_color(move || palette(dark_mode.get()).text_primary),
                  )
                  .child(
                    text("本地离线 Vault · SQLite")
                      .text_size(px(11.5))
                      .text_color(move || palette(dark_mode.get()).text_faint),
                  ),
              ),
          )
          .child(
            view()
              .w_full()
              .h(px(1.0))
              .bg(move || palette(dark_mode.get()).border_subtle),
          )
          .child(
            button()
              .test_id("account-toggle-theme")
              .accessibility_label("切换浅色或深色主题")
              .w_full()
              .px_2()
              .py_1()
              .flex_row()
              .items_center()
              .gap_2()
              .rounded(px(6.0))
              .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
              .on_press(move || {
                tt();
                account_menu_open.set(false);
              })
              .child(dynamic(move || {
                let hex = palette(dark_mode.get()).text_muted_hex;
                if dark_mode.get() {
                  mica_icons::sun_icon(hex)
                } else {
                  mica_icons::moon_icon(hex)
                }
              }))
              .child(
                text(move || {
                  if dark_mode.get() {
                    "切换到浅色模式"
                  } else {
                    "切换到深色模式"
                  }
                })
                .text_size(px(13.0))
                .text_color(move || palette(dark_mode.get()).text_primary),
              ),
          )
          .child(
            button()
              .test_id("account-open-search")
              .accessibility_label("打开工作区搜索")
              .w_full()
              .px_2()
              .py_1()
              .flex_row()
              .items_center()
              .justify_between()
              .rounded(px(6.0))
              .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
              .on_press(move || osr())
              .child(
                view()
                  .flex_row()
                  .items_center()
                  .gap_2()
                  .child(dynamic(move || {
                    mica_icons::search(palette(dark_mode.get()).text_muted_hex)
                  }))
                  .child(
                    text("搜索工作区")
                      .text_size(px(13.0))
                      .text_color(move || palette(dark_mode.get()).text_primary),
                  ),
              )
              .child(
                text("Ctrl+Shift+F")
                  .text_size(px(11.0))
                  .text_color(move || palette(dark_mode.get()).text_faint),
              ),
          )
          .child(
            button()
              .test_id("account-open-settings")
              .accessibility_label("打开设置窗口")
              .w_full()
              .px_2()
              .py_1()
              .flex_row()
              .items_center()
              .justify_between()
              .rounded(px(6.0))
              .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
              .on_press(move || os())
              .child(
                view()
                  .flex_row()
                  .items_center()
                  .gap_2()
                  .child(dynamic(move || {
                    mica_icons::settings_gear(palette(dark_mode.get()).text_muted_hex)
                  }))
                  .child(
                    text("设置与云端同步")
                      .text_size(px(13.0))
                      .text_color(move || palette(dark_mode.get()).text_primary),
                  ),
              )
              .child(
                text("Ctrl+,")
                  .text_size(px(11.0))
                  .text_color(move || palette(dark_mode.get()).text_faint),
              ),
          )
          .child(
            view()
              .w_full()
              .h(px(1.0))
              .bg(move || palette(dark_mode.get()).border_subtle),
          )
          .child(
            view().px_2().py_1().child(
              text(format!("Mica v{} · Pure Rust", env!("CARGO_PKG_VERSION")))
                .text_size(px(11.5))
                .text_color(move || palette(dark_mode.get()).text_ghost),
            ),
          )
      }
    }))
    .child(when(move || search_modal_open.get(), {
      let ts = tree_search.clone();
      let ds = docs_search.clone();
      let od = on_select_doc.clone();
      move || {
        search_view::search_modal_view(
          ts.clone(),
          ds.clone(),
          dark_mode,
          od.clone(),
          Rc::new(move || search_modal_open.set(false)),
        )
      }
    }))
    .child(when(move || settings_modal_open.get(), {
      let scb = settings_cb.clone();
      move || settings_view::settings_modal_view(scb.clone())
    }))
}

fn session_for_markdown(markdown: &str) -> EditorSession {
  let session = EditorSession::from_markdown(markdown);
  if session.blocks().is_empty() {
    EditorSession::from_blocks(
      "root",
      vec![EditorBlock::new(
        Uuid::new_v4().to_string(),
        "paragraph",
        "",
      )],
    )
  } else {
    session
  }
}

fn preview_head(s: &str) -> String {
  let single = s.replace('\n', " ");
  let mut out: String = single.chars().take(60).collect();
  if single.chars().count() > 60 {
    out.push('…');
  }
  out
}

fn resolve_parent_folder_name(tree: &PageTreeModel, doc_id: Uuid) -> Option<String> {
  let rows = tree.visible_rows();
  let page = rows.iter().find(|r| !r.is_folder && r.object_id == doc_id)?;
  let pid = page.parent_view_id?;
  tree.get(pid).map(|v| v.name.clone())
}

fn make_view(
  ws_id: Uuid,
  object_id: Uuid,
  name: &str,
  object_type: &str,
  parent_view_id: Option<Uuid>,
  position: &str,
) -> View {
  let now = Utc::now();
  View {
    id: if object_type == "folder" {
      object_id
    } else {
      Uuid::new_v4()
    },
    workspace_id: ws_id,
    object_id,
    name: name.to_string(),
    object_type: object_type.to_string(),
    icon: None,
    parent_view_id,
    position: position.to_string(),
    is_deleted: false,
    created_by: Uuid::nil(),
    created_at: now,
    updated_at: now,
    state_bytes: None,
  }
}

const STARTER_MARKDOWN: &str = "\
# 欢迎使用 Mica

这是一个纯 Rust 原生知识库与无限白板工作区，支持本地离线 SQLite 存储与云端 CRDT 同步。

## 核心特性

- 块级 Markdown 编辑：支持回车分块、行首退格合并、上下键跨块移动与原位中文 IME
- 鼠标拖动跨块选区与 Ctrl+A 两段式全选
- 无限白板、思维导图、表格、公式与 Mermaid 图表
";
