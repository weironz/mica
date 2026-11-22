//! Mica L3 pure-Rust cross-platform client (`clients/mica_rust`).
//!
//! Full M1~M4 pure-Rust architecture:
//! - Frameless hidden system title bar (`TitleBarStyle::Hidden`) fused seamlessly into
//!   the 36px Mica top tab strip with native Windows 11 Snap Layout caption buttons and
//!   `window_drag` support (macOS native traffic lights).
//! - Isomorphic flat-block `nsh-base::RichEditor` projected directly against `mica-core::MicaDoc`
//!   (single yrs CRDT storage & UndoManager truth, zero duplicate editor engine), with
//!   `nsh-base::Editor` real editable code blocks and RaTeX/Mermaid/Board `NodeView`s.
//! - Resizable collapsible left sidebar (`200..=380px`) with Workspace Switcher, Favorites,
//!   inline page/folder rename/reorder/delete, Recycle Bin, and bottom Account Menu.
//! - Collapsible 248px right sidebar with live Page Outline jump, Document Properties,
//!   real Version History list with one-click Restore, and Backlinks.
//! - Floating keyboard-navigable Search modal (`Ctrl+Shift+F`), two-column Settings modal
//!   (`Ctrl+,`) with Cloud Login/Sync & real disk Markdown/HTML/Bundle Import/Export,
//!   and full-featured Dot-Grid Infinite Whiteboard & Mindmap with floating Inspector.

pub mod atomic_blocks;
pub mod board_view;
pub mod editor_slash;
pub mod editor_view;
pub mod mica_icons;
pub mod mica_rich_bridge;
pub mod mica_theme;
pub mod net;
pub mod node_views;
pub mod right_sidebar;
pub mod search_view;
pub mod settings_view;
pub mod sidebar;
pub mod storage;
pub mod transfer;
#[cfg(target_arch = "wasm32")]
pub mod web;

use std::{
  cell::RefCell,
  collections::{HashMap, HashSet},
  rc::Rc,
};

use chrono::{DateTime, Local, Utc};
use mica_api_types::View;
use mica_board::BoardSession;
use mica_client::PageTreeModel;
use nsh_base::button;
use nsh_core::{
  desktop::{TitleBarOptions, TitleBarStyle, WindowAttributes},
  element::{
    KeyOptions,
    desktop::{current_window, title_bar},
    on_key_with,
  },
  geometry::px,
  keymap::When,
  prelude::*,
};
use uuid::Uuid;

use crate::{
  editor_view::EditorHeaderProps,
  mica_rich_bridge::MicaDocumentBridge,
  mica_theme::{install_mica_theme, palette},
  net::{CloudConfig, CloudConnector},
  right_sidebar::{RightSidebarProps, SnapshotEntry},
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
    configure_hidden_title_bar();
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

fn configure_hidden_title_bar() {
  #[cfg(not(target_arch = "wasm32"))]
  {
    let window = current_window();
    let mut tb = TitleBarOptions::default();
    tb.style = TitleBarStyle::Hidden;
    tb.height = Some(px(36.0));
    let mut attrs = WindowAttributes::default();
    attrs.title_bar = Some(tb);
    window.set(attrs);
  }
}

fn format_version_entries(vault: &VaultStore, origin: &str, doc_id: Uuid) -> (Vec<SnapshotEntry>, Vec<String>) {
  let raw = vault.list_versions(origin, doc_id);
  let mut entries = Vec::with_capacity(raw.len());
  let mut ids = Vec::with_capacity(raw.len());
  for (idx, (ver_id, label, ts_ms)) in raw.into_iter().enumerate() {
    let created_at = if ts_ms > 0 {
      DateTime::from_timestamp_millis(ts_ms)
        .map(|dt| dt.with_timezone(&Local).format("%m/%d %H:%M").to_string())
        .unwrap_or_else(|| "刚刚".to_string())
    } else {
      "刚刚".to_string()
    };
    entries.push(SnapshotEntry {
      id: idx as i64,
      label,
      created_at,
    });
    ids.push(ver_id);
  }
  (entries, ids)
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
  let account_email = signal(String::new());
  let is_cloud_signed_in = signal(false);
  let active_mode = signal(MODE_DOC);
  let sidebar_open = signal(true);
  let sidebar_width = signal(244.0f32);
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

  let nav_history: Rc<RefCell<Vec<(Uuid, String)>>> =
    Rc::new(RefCell::new(vec![(initial_doc_id, active_title.get_untracked())]));
  let nav_cursor = signal(0usize);
  let nav_len = signal(1usize);

  let favorites: Rc<RefCell<HashSet<Uuid>>> = Rc::new(RefCell::new(HashSet::new()));
  let favorite_pages: Signal<Vec<(Uuid, String)>> = signal(Vec::new());
  let active_is_favorite = signal(false);

  let (init_snaps, init_snap_ids) = format_version_entries(&vault, &initial_origin, initial_doc_id);
  let snapshot_count = signal(init_snaps.len());
  let snapshots_sig = signal(init_snaps);
  let snapshot_id_map: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(init_snap_ids));

  let dark_mode = signal(false);
  let locale_zh = signal(true);
  let server_url = signal("http://127.0.0.1:8080".to_string());
  let cloud_status = signal("模式：本地离线 SQLite Vault".to_string());
  let export_preview = signal(String::new());
  let cloud_connector = Rc::new(RefCell::new(CloudConnector::new(CloudConfig {
    server_url: server_url.get_untracked(),
    access_token: None,
    email: None,
  })));

  let initial_md = docs_store
    .borrow()
    .get(&initial_doc_id)
    .cloned()
    .unwrap_or_default();
  let bridge = MicaDocumentBridge::from_markdown(&initial_md);
  node_views::register_mica_node_views(&bridge.session(), dark_mode);

  {
    let vault_p = vault.clone();
    let docs_p = docs_store.clone();
    bridge.set_on_persist(Rc::new(move |md: String, _crdt: Option<Vec<u8>>| {
      if let Some(doc_id) = active_doc.get_untracked() {
        let orig = active_origin.get_untracked();
        docs_p.borrow_mut().insert(doc_id, md.clone());
        let _ = vault_p.save_markdown(&orig, doc_id, &md);
      }
    }));
  }

  let refresh_snapshots: Rc<dyn Fn(Uuid)> = {
    let v = vault.clone();
    let id_map = snapshot_id_map.clone();
    Rc::new(move |doc_id: Uuid| {
      let orig = active_origin.get_untracked();
      let (entries, ids) = format_version_entries(&v, &orig, doc_id);
      snapshot_count.set(entries.len());
      snapshots_sig.set(entries);
      *id_map.borrow_mut() = ids;
    })
  };

  let refresh_favorite_pages: Rc<dyn Fn()> = {
    let favs = favorites.clone();
    let tree_ref = tree.clone();
    Rc::new(move || {
      let set = favs.borrow();
      let list: Vec<(Uuid, String)> = tree_ref
        .borrow()
        .visible_rows()
        .into_iter()
        .filter(|r| !r.is_folder && set.contains(&r.object_id))
        .map(|r| (r.object_id, r.name))
        .collect();
      favorite_pages.set(list);
    })
  };

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

  // Global window keyboard shortcuts
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

  let open_doc_internal: Rc<dyn Fn(Uuid, String, bool)> = {
    let docs = docs_store.clone();
    let br = bridge.clone();
    let tree_ref = tree.clone();
    let v = vault.clone();
    let favs = favorites.clone();
    let hist = nav_history.clone();
    let ref_snaps = refresh_snapshots.clone();
    Rc::new(move |doc_id: Uuid, title: String, record_history: bool| {
      let orig = active_origin.get_untracked();
      let md = docs
        .borrow()
        .get(&doc_id)
        .cloned()
        .or_else(|| v.load_markdown(&orig, doc_id))
        .unwrap_or_default();
      docs.borrow_mut().insert(doc_id, md.clone());
      br.load_markdown(&md);
      active_doc.set(Some(doc_id));
      active_doc_id_str.set(doc_id.to_string());
      active_title.set(title.clone());
      active_is_favorite.set(favs.borrow().contains(&doc_id));
      ref_snaps(doc_id);

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

  let create_page_with_content: Rc<dyn Fn(String, String)> = {
    let tree_ref = tree.clone();
    let docs = docs_store.clone();
    let v = vault.clone();
    let select = on_select_doc.clone();
    Rc::new(move |name: String, markdown: String| {
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
      let view_row = make_view(
        cur_ws,
        new_doc_id,
        &name,
        "page",
        Some(cur_folder),
        &format!("z{idx:04}"),
      );
      let _ = v.save_view(&orig, &view_row);
      let _ = v.save_markdown(&orig, new_doc_id, &markdown);
      {
        let mut t = tree_ref.borrow_mut();
        t.upsert_view(view_row);
        t.set_expanded(cur_folder, true);
      }
      docs.borrow_mut().insert(new_doc_id, markdown);
      tree_rev.set(tree_rev.get_untracked() + 1);
      select(new_doc_id, name);
    })
  };

  let on_new_page: Rc<dyn Fn()> = {
    let tree_ref = tree.clone();
    let create = create_page_with_content.clone();
    Rc::new(move || {
      let idx = tree_ref
        .borrow()
        .visible_rows()
        .iter()
        .filter(|r| !r.is_folder)
        .count()
        + 1;
      create(format!("未命名页面 {idx}"), String::new());
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

  let on_rename_view: Rc<dyn Fn(Uuid, String)> = {
    let tree_ref = tree.clone();
    let v = vault.clone();
    let ref_favs = refresh_favorite_pages.clone();
    Rc::new(move |vid: Uuid, next_name: String| {
      let orig = active_origin.get_untracked();
      let updated = {
        let mut t = tree_ref.borrow_mut();
        t.get(vid).cloned().map(|mut row| {
          row.name = next_name.clone();
          row.updated_at = Utc::now();
          t.upsert_view(row.clone());
          row
        })
      };
      if let Some(row) = updated {
        let _ = v.save_view(&orig, &row);
        if active_doc.get_untracked() == Some(row.object_id) {
          active_title.set(next_name.clone());
        }
        let mut tabs = open_tabs.get_untracked();
        for tab in &mut tabs {
          if tab.doc_id == row.object_id {
            tab.title = next_name.clone();
          }
        }
        open_tabs.set(tabs);
        ref_favs();
        tree_rev.set(tree_rev.get_untracked() + 1);
      }
    })
  };

  let on_trash_view: Rc<dyn Fn(Uuid)> = {
    let tree_ref = tree.clone();
    let v = vault.clone();
    let ref_favs = refresh_favorite_pages.clone();
    Rc::new(move |vid: Uuid| {
      let orig = active_origin.get_untracked();
      let updated = {
        let mut t = tree_ref.borrow_mut();
        t.get(vid).cloned().map(|mut row| {
          row.is_deleted = true;
          t.upsert_view(row.clone());
          row
        })
      };
      if let Some(row) = updated {
        let _ = v.save_view(&orig, &row);
        ref_favs();
        tree_rev.set(tree_rev.get_untracked() + 1);
      }
    })
  };

  let on_move_view: Rc<dyn Fn(Uuid, i32)> = {
    let tree_ref = tree.clone();
    let v = vault.clone();
    Rc::new(move |vid: Uuid, delta: i32| {
      let orig = active_origin.get_untracked();
      let swapped = {
        let mut t = tree_ref.borrow_mut();
        let rows = t.visible_rows();
        let Some(pos_idx) = rows.iter().position(|r| r.view_id == vid) else {
          return;
        };
        let target_parent = rows[pos_idx].parent_view_id;
        let siblings: Vec<Uuid> = rows
          .iter()
          .filter(|r| r.parent_view_id == target_parent)
          .map(|r| r.view_id)
          .collect();
        let Some(sib_idx) = siblings.iter().position(|&id| id == vid) else {
          return;
        };
        let next_idx = (sib_idx as isize + delta as isize)
          .clamp(0, siblings.len().saturating_sub(1) as isize) as usize;
        if next_idx == sib_idx {
          return;
        }
        let other_id = siblings[next_idx];
        match (t.get(vid).cloned(), t.get(other_id).cloned()) {
          (Some(mut a), Some(mut b)) => {
            std::mem::swap(&mut a.position, &mut b.position);
            if a.position == b.position {
              a.position = format!("m{next_idx:04}");
              b.position = format!("m{sib_idx:04}");
            }
            t.upsert_view(a.clone());
            t.upsert_view(b.clone());
            Some((a, b))
          }
          _ => None,
        }
      };
      if let Some((a, b)) = swapped {
        let _ = v.save_view(&orig, &a);
        let _ = v.save_view(&orig, &b);
        tree_rev.set(tree_rev.get_untracked() + 1);
      }
    })
  };

  let on_switch_workspace: Rc<dyn Fn(WorkspaceEntry)> = {
    let tree_ref = tree.clone();
    let docs = docs_store.clone();
    let v = vault.clone();
    let br = bridge.clone();
    let hist = nav_history.clone();
    let ref_snaps = refresh_snapshots.clone();
    let ref_favs = refresh_favorite_pages.clone();
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
      ref_snaps(doc_id);
      ref_favs();
      br.load_markdown(&active_md);
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
    let rename_v = on_rename_view.clone();
    Rc::new(move |next_name: String| {
      active_title.set(next_name.clone());
      let Some(doc_id) = active_doc.get_untracked() else {
        return;
      };
      let target_id = tree_ref
        .borrow()
        .visible_rows()
        .into_iter()
        .find(|r| !r.is_folder && r.object_id == doc_id)
        .map(|r| r.view_id);
      if let Some(vid) = target_id {
        rename_v(vid, next_name);
      }
    })
  };

  let on_trash_active: Rc<dyn Fn()> = {
    let tree_ref = tree.clone();
    let trash_v = on_trash_view.clone();
    Rc::new(move || {
      let Some(doc_id) = active_doc.get_untracked() else {
        return;
      };
      let target_id = tree_ref
        .borrow()
        .visible_rows()
        .into_iter()
        .find(|r| !r.is_folder && r.object_id == doc_id)
        .map(|r| r.view_id);
      if let Some(vid) = target_id {
        trash_v(vid);
      }
    })
  };

  let on_restore_view: Rc<dyn Fn(Uuid)> = {
    let tree_ref = tree.clone();
    let v = vault.clone();
    let ref_favs = refresh_favorite_pages.clone();
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
        ref_favs();
        tree_rev.set(tree_rev.get_untracked() + 1);
      }
    })
  };

  let on_toggle_favorite: Rc<dyn Fn()> = {
    let favs = favorites.clone();
    let br = bridge.clone();
    let ref_favs = refresh_favorite_pages.clone();
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
        drop(set);
        active_is_favorite.set(now_fav);
        ref_favs();
        br.set_status_flash(if now_fav { "已加入收藏" } else { "已取消收藏" });
      }
    })
  };

  let on_export_markdown: Rc<dyn Fn()> = {
    let br = bridge.clone();
    Rc::new(move || {
      let md = br.export_markdown();
      let title = active_title.get_untracked();
      match transfer::export_markdown_file(&title, &md) {
        Ok(path) => {
          export_preview.set(format!("已导出 Markdown：{}（{} 字节）", path, md.len()));
          br.set_status_flash("已导出 Markdown");
        }
        Err(err) => export_preview.set(format!("导出 Markdown 失败：{err}")),
      }
    })
  };

  let on_export_html: Rc<dyn Fn()> = {
    let br = bridge.clone();
    Rc::new(move || {
      let html = br.export_html();
      let title = active_title.get_untracked();
      match transfer::export_html_file(&title, &html) {
        Ok(path) => {
          export_preview.set(format!("已导出 HTML：{}（{} 字节）", path, html.len()));
          br.set_status_flash("已导出 HTML");
        }
        Err(err) => export_preview.set(format!("导出 HTML 失败：{err}")),
      }
    })
  };

  let on_export_bundle: Rc<dyn Fn()> = {
    let tree_ref = tree.clone();
    let docs = docs_store.clone();
    let br = bridge.clone();
    Rc::new(move || {
      let ws_name = workspace_name.get_untracked();
      let pages: Vec<(String, String)> = tree_ref
        .borrow()
        .visible_rows()
        .into_iter()
        .filter(|r| !r.is_folder)
        .map(|r| {
          let md = docs.borrow().get(&r.object_id).cloned().unwrap_or_default();
          (r.name, md)
        })
        .collect();
      match transfer::export_workspace_bundle(&ws_name, &pages) {
        Ok(path) => {
          export_preview.set(format!(
            "已导出工作区备份：{}（共 {} 页）",
            path,
            pages.len()
          ));
          br.set_status_flash("已导出工作区备份");
        }
        Err(err) => export_preview.set(format!("导出备份失败：{err}")),
      }
    })
  };

  let on_import_markdown: Rc<dyn Fn(String)> = {
    let create_page = create_page_with_content.clone();
    Rc::new(move |path: String| match transfer::import_markdown_from_path(&path) {
      Ok((title, md)) => {
        export_preview.set(format!("已导入页面「{}」（{} 字节）", title, md.len()));
        create_page(title, md);
      }
      Err(err) => export_preview.set(format!("导入失败：{err}")),
    })
  };

  let on_save_snapshot: Rc<dyn Fn()> = {
    let v = vault.clone();
    let br = bridge.clone();
    let ref_snaps = refresh_snapshots.clone();
    Rc::new(move || {
      if let Some(doc_id) = active_doc.get_untracked() {
        let orig = active_origin.get_untracked();
        match v.create_version_snapshot(&orig, doc_id, Some("手动快照")) {
          Ok(()) => {
            ref_snaps(doc_id);
            let count = snapshot_count.get_untracked();
            export_preview.set(format!("版本快照已保存（共 {count} 个）"));
            br.set_status_flash(format!("已保存快照 #{count}"));
          }
          Err(e) => export_preview.set(format!("保存快照失败：{e}")),
        }
      }
    })
  };

  let on_restore_snapshot: Rc<dyn Fn(i64)> = {
    let v = vault.clone();
    let br = bridge.clone();
    let docs = docs_store.clone();
    let id_map = snapshot_id_map.clone();
    let ref_snaps = refresh_snapshots.clone();
    Rc::new(move |idx: i64| {
      let Some(doc_id) = active_doc.get_untracked() else {
        return;
      };
      let Some(ver_id) = id_map.borrow().get(idx as usize).cloned() else {
        return;
      };
      let orig = active_origin.get_untracked();
      match v.restore_version_markdown(&orig, doc_id, &ver_id) {
        Ok(md) => {
          docs.borrow_mut().insert(doc_id, md.clone());
          br.load_markdown(&md);
          ref_snaps(doc_id);
          br.set_status_flash("已恢复历史版本");
        }
        Err(e) => br.set_status_flash(format!("恢复失败：{e}")),
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
    on_rename_view,
    on_trash_view,
    on_move_view,
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
    outline_items: bridge.outline_signal(),
    word_count_text: bridge.word_count_signal(),
    block_count: bridge.block_count_signal(),
    snapshot_count,
    snapshots: snapshots_sig,
    tree: tree.clone(),
    docs: docs_store.clone(),
    tree_rev,
    on_jump_pos: {
      let br = bridge.clone();
      Rc::new(move |pos: usize| {
        active_mode.set(MODE_DOC);
        br.focus_pos(pos);
      })
    },
    on_open_doc: on_select_doc.clone(),
    on_save_snapshot: on_save_snapshot.clone(),
    on_restore_snapshot,
    on_close: Rc::new(move || right_panel_open.set(false)),
  });

  let settings_cb = Rc::new(SettingsCallbacks {
    dark_mode,
    locale_zh,
    server_url,
    cloud_status,
    export_preview,
    account_email,
    is_cloud_signed_in,
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
    on_cloud_login: {
      let cc = cloud_connector.clone();
      Rc::new(move |url: String, email: String, password: String| {
        let mut conn = CloudConnector::new(CloudConfig {
          server_url: url.clone(),
          access_token: None,
          email: Some(email.clone()),
        });
        match conn.login(&email, &password) {
          Ok(msg) => {
            *cc.borrow_mut() = conn;
            server_url.set(url);
            account_email.set(email.clone());
            account_name.set(email);
            is_cloud_signed_in.set(true);
            cloud_status.set(format!("已登录云端：{msg}"));
          }
          Err(err) => cloud_status.set(format!("登录失败：{err}")),
        }
      })
    },
    on_cloud_logout: {
      let cc = cloud_connector.clone();
      Rc::new(move || {
        *cc.borrow_mut() = CloudConnector::new(CloudConfig {
          server_url: server_url.get_untracked(),
          access_token: None,
          email: None,
        });
        is_cloud_signed_in.set(false);
        account_email.set(String::new());
        account_name.set("local".to_string());
        cloud_status.set("已切换回本地离线 SQLite Vault".to_string());
      })
    },
    on_sync_now: {
      let cc = cloud_connector.clone();
      Rc::new(move || {
        let phase = cc.borrow().sync_phase_label();
        match cc.borrow().check_health() {
          Ok(ver) => cloud_status.set(format!("同步就绪（{phase} · 服务器 {ver}）")),
          Err(err) => cloud_status.set(format!("同步检查失败：{err}")),
        }
      })
    },
    on_export_markdown,
    on_export_html,
    on_export_bundle,
    on_import_markdown,
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
      // 1. Frameless 36px fused top bar (`TitleBarStyle::Hidden` + `window_drag`)
      view()
        .h(px(36.0))
        .w_full()
        .flex_row()
        .items_end()
        .window_drag(true)
        .bg(move || palette(dark_mode.get()).surface_sunken)
        .child(
          view()
            .w(move || {
              if sidebar_open.get() {
                px(sidebar_width.get())
              } else {
                px(72.0)
              }
            })
            .h_full()
            .pl(move || {
              // Reserve space on macOS for traffic lights when hidden titlebar is active.
              let inset = title_bar().left.0;
              px(if inset > 0.0 { inset + 8.0 } else { 12.0 })
            })
            .pr_3()
            .flex_row()
            .items_center()
            .gap_2()
            .flex_shrink_0()
            .window_drag(true)
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
              view().window_drag(false).child(
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
            ),
        )
        .child(
          // DocTabStrip: `<` `>` real history + open tabs + `+` + right caption button safe inset
          view()
            .flex_1()
            .h_full()
            .pl_2()
            .pr(move || {
              let inset = title_bar().right.0;
              px(if inset > 0.0 { inset + 8.0 } else { 8.0 })
            })
            .flex_row()
            .items_end()
            .gap_1()
            .window_drag(true)
            .child(
              view()
                .h_full()
                .flex_row()
                .items_center()
                .gap_1()
                .pr_1()
                .window_drag(false)
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
              let mut strip = view().flex_row().items_end().gap_1().window_drag(false);
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
              view()
                .h_full()
                .items_center()
                .justify_center()
                .pl_1()
                .window_drag(false)
                .child(
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
      // 2. Main workspace body: Resizable Left Sidebar + RichEditor/Board + Collapsible Right Sidebar
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
              favorite_pages,
              sidebar_width,
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
              let br = bridge.clone();
              let hdr = editor_header.clone();
              move || editor_view::editor_view(br.clone(), hdr.clone(), dark_mode)
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
                  .child(board_view::board_view(bs.clone(), board_rev, dark_mode, pb.clone()))
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
                    text(move || {
                      account_name
                        .get()
                        .chars()
                        .next()
                        .map(|c| c.to_uppercase().to_string())
                        .unwrap_or_else(|| "L".to_string())
                    })
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
                    text(move || {
                      if is_cloud_signed_in.get() {
                        "已连接 Mica 云端同步".to_string()
                      } else {
                        "本地离线 Vault · SQLite".to_string()
                      }
                    })
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

这是一个 100% 全栈纯 Rust 知识库与无限白板工作区，支持本地离线 SQLite 存储与云端 CRDT 同步。

## 核心能力

- 原生富文本块编辑：跨块鼠标拖选、回车分块、行首退格合并、`Ctrl+A` 全选与原位中文 IME
- 真实多行代码编辑器、表格、RaTeX 公式、Mermaid 流程图与内嵌无限白板
- 无边框融合标题栏、可拖拽调宽侧栏、实时页面大纲与版本快照一键恢复

```rust
fn main() {
    println!(\"Hello from Mica Pure Rust!\");
}
```
";
