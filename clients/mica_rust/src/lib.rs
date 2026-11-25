//! Mica L3 pure-Rust cross-platform client (`clients/mica_rust`).
//!
//! 1:1 parity pure-Rust architecture aligned against `main:clients/mica_flutter`:
//! - Unified 34px frameless hidden system title bar (`TitleBarStyle::Hidden`) with clickable
//!   top-left M Application Menu (`文件`/`视图`/`历史`/`帮助`), About modal, and browser-style
//!   document tabs with per-tab history & keyboard shortcuts.
//! - Resizable left sidebar (`220..=480px`, default `280px`) with 320px Workspace Switcher,
//!   Knowledge Graph (`900×640`), Workspace Settings (`440px`), Favorites, full right-click
//!   context menus, 30/40/30 drag-and-drop reordering & reparenting, multi-select batch drag/menu,
//!   Emoji Picker (`360×420`), Recycle Bin, and stacked-card Account Menu + World Switcher.
//! - Isomorphic flat-block `nsh-base::RichEditor` projected directly against `mica-core::MicaDoc`
//!   CRDT with original borderless hover-only CodeBlock + 240×312 LanguagePicker, draggable
//!   `PageWidthFrame` (`680..=1880px`), `BlockMarquee` selection, caret-anchored 22-item Slash
//!   Menu, `Ctrl+F` in-page find, `PropertyPanel`, `BacklinksPanel`, `980×640` Version History
//!   Diff modal, Share dialog, Comments rail, Global & Inline AI dialogs.
//! - Full-size `1120×840` 8-tab Settings dialog, `600×440` Search dialog, real `mica-interchange`
//!   ZIP/Markdown/Notion/Folder import & export via native OS file dialogs, Cross-Workspace
//!   Transfer dialog, and Feishu-style 1:1 Infinite Whiteboard with 3-way embedded block merge.

pub mod ai_dialog;
mod app_chrome;
pub mod app_menu;
mod app_transfer_io;
pub mod atomic_blocks;
pub mod block_marquee;
pub mod block_views;
pub mod board_view;
pub mod clipboard_platform;
pub mod cloud_sync;
pub mod comment_panel;
pub mod doc_sync_session;
pub mod editor_bridge;
pub mod editor_positioning;
pub mod editor_slash;
pub mod editor_view;
pub mod emoji_picker;
pub mod graph_dialog;
pub mod mica_icons;
pub mod mica_theme;
pub mod page_width;
pub mod preview_render;
pub mod property_panel;
pub mod responsive_shell;
pub mod right_sidebar;
pub mod search_view;
pub mod settings_view;
pub mod share_dialog;
pub mod sidebar;
pub mod single_instance;
pub mod table_view;
pub mod text_geometry;
pub mod tooltip;
pub mod touch_selection;
pub mod transfer;
pub mod transfer_dialog;
pub mod tree_dnd;
pub mod updater;
pub mod usage_stats_view;
pub mod version_history;
pub mod workspace_views;
pub mod world_switcher;
// Re-export pure L2 client modules (zero `nsh-*` dependencies, owned by `crates/mica-client`).
pub use mica_client::{net, storage, web_idb, web_runtime, ws_transport};
#[cfg(target_arch = "wasm32")]
pub mod web;

use std::{
  cell::RefCell,
  collections::HashMap,
  rc::Rc,
};

use chrono::{DateTime, Local, Utc};
use mica_api_types::View;
use mica_board::BoardSession;
use mica_client::PageTreeModel;
#[cfg(not(target_arch = "wasm32"))]
use nsh_core::desktop::{TitleBarOptions, TitleBarStyle, WindowAttributes};
#[cfg(not(target_arch = "wasm32"))]
use nsh_core::element::desktop::current_window;
use nsh_core::{
  element::{KeyOptions, on_key_with},
  geometry::{Bounds, Points, px},
  keymap::When,
  prelude::*,
};
use uuid::Uuid;

use crate::{
  ai_dialog::{AiTarget, GlobalAiState, InlineAiPhase, InlineAiState},
  app_chrome::{EmbeddedBoardEdit, TopBarProps},
  app_menu::{AccountMenuState, AppMenuCallbacks},
  app_transfer_io::{AppTransferContext, AppTransferHandlers, CrossWorkspaceTransferCtx},
  block_views::{CodeAiAction, CodeAiCallback, OpenBoardCallback, OpenBoardWithBlockCallback},
  editor_bridge::MicaDocumentBridge,
  editor_view::EditorHeaderProps,
  graph_dialog::PageGraphData,
  mica_theme::{install_mica_theme, palette},
  net::{CloudConfig, CloudConnector},
  right_sidebar::{BacklinkEntry, RightSidebarExtras, RightSidebarProps, SnapshotEntry, ToolsTab},
  search_view::{SearchRecentItem, SearchViewProps, SearchWorkspaceChip},
  settings_view::{SettingsCallbacks, SettingsViewProps},
  share_dialog::ShareDialogState,
  sidebar::{SidebarCallbacks, WorkspaceSwitcherCallbacks, WorkspaceSwitcherItem},
  storage::{DEFAULT_ORIGIN, VaultStore},
  transfer::{ArchiveExportFormat, ImportedPageSpec},
  version_history::{VersionHistoryItem, VersionHistoryState, format_version_stamp_now},
  workspace_views::WorkspaceSettingsState,
  world_switcher::{LOCAL_ORIGIN, WorldSwitcherState},
};

pub(crate) const MODE_DOC: u8 = 0;
const MODE_BOARD: u8 = 1;
const DEFAULT_BOARD_ID: &str = "workspace-default-board";
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct OpenTab {
  pub(crate) doc_id: Uuid,
  pub(crate) title: String,
  history: Vec<(Uuid, String)>,
  cursor: usize,
}

impl OpenTab {
  pub(crate) fn single(doc_id: Uuid, title: String) -> Self {
    Self {
      doc_id,
      title: title.clone(),
      history: vec![(doc_id, title)],
      cursor: 0,
    }
  }

  fn navigate(&mut self, doc_id: Uuid, title: String) {
    self.doc_id = doc_id;
    self.title = title.clone();
    if self.history.get(self.cursor).map(|(id, _)| *id) != Some(doc_id) {
      self.history.truncate(self.cursor + 1);
      self.history.push((doc_id, title));
      if self.history.len() > 50 {
        let drop_n = self.history.len() - 50;
        self.history.drain(0..drop_n);
      }
      self.cursor = self.history.len().saturating_sub(1);
    }
  }
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceEntry {
  pub(crate) id: Uuid,
  pub(crate) name: String,
  pub(crate) origin: String,
  pub(crate) page_count: Option<usize>,
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
    tb.height = Some(px(34.0));
    let mut attrs = WindowAttributes::default();
    attrs.title_bar = Some(tb);
    window.set(attrs);
  }
}

fn format_version_entries(
  vault: &VaultStore,
  origin: &str,
  doc_id: Uuid,
) -> (Vec<SnapshotEntry>, Vec<String>, Vec<VersionHistoryItem>) {
  let raw = vault.list_versions(origin, doc_id);
  let mut entries = Vec::with_capacity(raw.len());
  let mut ids = Vec::with_capacity(raw.len());
  let mut history_items = Vec::with_capacity(raw.len());
  for (idx, (ver_id, label, ts_ms)) in raw.into_iter().enumerate() {
    let dt_iso = DateTime::from_timestamp_millis(ts_ms)
      .map(|d| d.to_rfc3339())
      .unwrap_or_default();
    let (group, stamp) = if ts_ms > 0 {
      format_version_stamp_now(&dt_iso)
    } else {
      (version_history::VersionGroup::Today, "刚刚".to_string())
    };
    let created_short = if ts_ms > 0 {
      DateTime::from_timestamp_millis(ts_ms)
        .map(|dt| dt.with_timezone(&Local).format("%m/%d %H:%M").to_string())
        .unwrap_or_else(|| "刚刚".to_string())
    } else {
      "刚刚".to_string()
    };
    let is_named = !label.is_empty() && label != "自动快照";
    entries.push(SnapshotEntry {
      id: idx as i64,
      label: label.clone(),
      created_at: created_short,
    });
    history_items.push(VersionHistoryItem {
      id: ver_id.clone(),
      label: if is_named { label } else { stamp.clone() },
      is_named,
      created_at: stamp,
      author: None,
      is_today: matches!(group, version_history::VersionGroup::Today),
    });
    ids.push(ver_id);
  }
  (entries, ids, history_items)
}

pub(crate) fn collect_all_views(tree: &PageTreeModel) -> (Vec<View>, Vec<View>) {
  let mut live = Vec::new();
  let mut trashed = Vec::new();
  for row in tree.all_live_rows() {
    if let Some(v) = tree.get(row.view_id).cloned() {
      live.push(v);
    }
  }
  for v in tree.trashed_views() {
    trashed.push(v.clone());
  }
  (live, trashed)
}

fn compute_backlinks_for(
  tree: &PageTreeModel,
  docs: &HashMap<Uuid, String>,
  target_doc: Option<Uuid>,
  target_title: &str,
) -> Vec<BacklinkEntry> {
  let Some(target_id) = target_doc else {
    return Vec::new();
  };
  let id_needle = format!("mica://page/{target_id}");
  let wiki_needle = if target_title.trim().is_empty() {
    None
  } else {
    Some(format!("[[{}]]", target_title.trim()))
  };
  let mut out = Vec::new();
  for row in tree.all_live_rows() {
    if row.is_folder || row.object_id == target_id {
      continue;
    }
    if let Some(body) = docs.get(&row.object_id) {
      let matches_id = body.contains(&id_needle);
      let matches_wiki = wiki_needle
        .as_ref()
        .is_some_and(|w| body.contains(w.as_str()));
      if matches_id || matches_wiki {
        out.push(BacklinkEntry {
          doc_id: row.object_id,
          title: row.name,
        });
      }
    }
  }
  out
}

fn render_mica_app(vault: VaultStore) -> impl IntoElement {
  let vault = Rc::new(vault);
  let allow_local = !web_runtime::is_web();
  let initial_origin = if allow_local {
    DEFAULT_ORIGIN.to_string()
  } else {
    web_runtime::web_origin().unwrap_or_else(|| "http://127.0.0.1:8080".to_string())
  };
  let active_origin = signal(initial_origin.clone());

  let mut views = if allow_local { vault.list_views(&initial_origin) } else { Vec::new() };
  let ws_id: Uuid;
  let default_folder_id: Uuid;
  let initial_doc_id: Option<Uuid>;
  let initial_title: String;
  let initial_folder_name: String;

  if !allow_local {
    ws_id = Uuid::nil();
    default_folder_id = Uuid::nil();
    initial_doc_id = None;
    initial_title = String::new();
    initial_folder_name = String::new();
  } else if views.is_empty() {
    ws_id = Uuid::new_v4();
    let folder_id = Uuid::new_v4();
    let doc_id = Uuid::new_v4();
    let folder_view = make_view(ws_id, folder_id, "知识库", "folder", None, "0000000010");
    let doc_view = make_view(ws_id, doc_id, "欢迎使用 Mica", "page", Some(folder_id), "0000000010");
    let _ = vault.save_view(&initial_origin, &folder_view);
    let _ = vault.save_view(&initial_origin, &doc_view);
    let _ = vault.save_markdown(&initial_origin, doc_id, STARTER_MARKDOWN);
    let _ = vault.create_version_snapshot(&initial_origin, doc_id, Some("初始版本"));
    views = vec![folder_view, doc_view];
    default_folder_id = folder_id;
    initial_doc_id = Some(doc_id);
    initial_title = "欢迎使用 Mica".to_string();
    initial_folder_name = "知识库".to_string();
  } else {
    ws_id = views[0].workspace_id;
    let folder = views.iter().find(|v| v.object_type == "folder" && !v.is_deleted);
    default_folder_id = folder.map(|v| v.id).unwrap_or_else(Uuid::new_v4);
    initial_folder_name = folder
      .map(|v| v.name.clone())
      .unwrap_or_else(|| "工作区".to_string());
    if let Some(page) = views.iter().find(|v| cloud_sync::is_page_view(v) && !v.is_deleted) {
      initial_doc_id = Some(page.object_id);
      initial_title = page.name.clone();
    } else {
      let doc_id = Uuid::new_v4();
      let doc_view = make_view(ws_id, doc_id, "未命名页面", "page", Some(default_folder_id), "0000000010");
      let _ = vault.save_view(&initial_origin, &doc_view);
      let _ = vault.save_markdown(&initial_origin, doc_id, "");
      views.push(doc_view);
      initial_doc_id = Some(doc_id);
      initial_title = "未命名页面".to_string();
    }
  }

  let mut docs_map: HashMap<Uuid, String> = HashMap::new();
  for v in &views {
    if cloud_sync::is_page_view(v) {
      let md = vault.load_markdown(&initial_origin, v.object_id).unwrap_or_default();
      docs_map.insert(v.object_id, md);
    }
  }
  let docs_store = Rc::new(RefCell::new(docs_map));

  // Start every workspace with all sidebar folders collapsed by default.
  let tree_model = PageTreeModel::new(views);
  let (init_live, init_trashed) = collect_all_views(&tree_model);
  let tree = Rc::new(RefCell::new(tree_model));

  let (initial_ws_name, initial_ws_entries) = if allow_local {
    let saved_local_ws = vault.list_workspaces(&initial_origin);
    let ws_name = saved_local_ws
      .iter()
      .find(|(id, _)| *id == ws_id)
      .map(|(_, n)| n.clone())
      .unwrap_or_else(|| "Mica 本地工作区".to_string());
    let mut entries: Vec<WorkspaceEntry> = saved_local_ws
      .into_iter()
      .map(|(id, name)| WorkspaceEntry {
        id,
        name,
        origin: initial_origin.clone(),
        page_count: None,
      })
      .collect();
    if !entries.iter().any(|w| w.id == ws_id) {
      entries.insert(
        0,
        WorkspaceEntry {
          id: ws_id,
          name: ws_name.clone(),
          origin: initial_origin.clone(),
          page_count: None,
        },
      );
    }
    (ws_name, entries)
  } else {
    (String::new(), Vec::new())
  };
  let workspaces = signal(initial_ws_entries);
  let active_workspace_id = signal(ws_id);
  let active_folder_id = signal(default_folder_id);

  // Persisted UI preferences
  let init_page_width = vault.page_width().clamp(page_width::PAGE_WIDTH_MIN, page_width::PAGE_WIDTH_MAX);
  let init_sidebar_width = vault
    .sidebar_width()
    .clamp(sidebar::SIDEBAR_MIN_WIDTH, sidebar::SIDEBAR_MAX_WIDTH);
  let init_dark = vault.bool_pref("darkMode", false);
  let init_locale_zh = vault.bool_pref("localeZh", true);
  let init_show_format_bar = vault.bool_pref("showFormatBar", true);
  let init_show_page_title = vault.bool_pref("showPageTitle", true);

  let tree_rev = signal(0u64);
  let active_doc: Signal<Option<Uuid>> = signal(initial_doc_id);
  let active_doc_id_str = signal(initial_doc_id.map(|id| id.to_string()).unwrap_or_default());
  let active_title = signal(initial_title.clone());
  let active_folder_name = signal(initial_folder_name);
  let workspace_name = signal(initial_ws_name);
  let account_name = signal(if allow_local { "本地工作区" } else { "未登录" }.to_string());
  let account_email = signal(if allow_local { "这台设备" } else { "" }.to_string());
  let is_cloud_signed_in = signal(false);
  let active_mode = signal(MODE_DOC);
  let sidebar_open = signal(true);
  let sidebar_width = signal(init_sidebar_width);
  let right_panel_open = signal(false);
  let right_panel_width = signal(right_sidebar::RIGHT_SIDEBAR_DEFAULT_WIDTH);
  let right_panel_tab = signal(ToolsTab::Outline);
  let viewport_width = signal(1280.0_f32);
  let tooltip_host = tooltip::install_tooltip_host(viewport_width);
  let is_narrow = computed(move || viewport_width.get() < responsive_shell::NARROW_BREAKPOINT);
  let narrow_tabs_open = signal(false);
  {
    let prev_narrow = signal(false);
    effect(move || {
      let now_narrow = is_narrow.get();
      if now_narrow && !prev_narrow.get_untracked() {
        sidebar_open.set(false);
        right_panel_open.set(false);
      }
      prev_narrow.set(now_narrow);
    });
  }

  // Overlays & modals
  let app_menu_open = signal(false);
  let about_modal_open = signal(false);
  let world_switcher_open = signal(!allow_local);
  let world_can_close = signal(allow_local);
  let search_modal_open = signal(false);
  let search_initial_query: Signal<Option<String>> = signal(None);
  let search_open_in_new_tab = signal(false);
  let settings_modal_open = signal(false);
  let workspace_menu_open = signal(false);
  let workspace_settings_open = signal(false);
  let account_menu_open = signal(false);
  let new_tab_menu_open = signal(false);
  let graph_modal_open = signal(false);
  let graph_data = signal(PageGraphData::default());
  let version_modal_open = signal(false);
  let share_modal_open = signal(false);
  let transfer_modal_open = signal(false);
  let transfer_copy_mode = signal(false);
  let transfer_source_ids: Signal<Vec<Uuid>> = signal(Vec::new());
  let embedded_board_edit: Signal<Option<EmbeddedBoardEdit>> = signal(None);
  let update_status = signal(String::new());

  // Editor display & preferences signals
  let page_width_sig = signal(init_page_width);
  let page_width_px_sig = signal(init_page_width.round() as u32);
  let show_format_bar = signal(init_show_format_bar);
  let show_page_title = signal(init_show_page_title);
  let show_properties = signal(false);
  let ai_enabled = signal(true);
  let dark_mode = signal(init_dark);
  let locale_zh = signal(init_locale_zh);
  let init_ui_scale = match vault.get_pref("uiFontScale").as_deref() {
    Some("small") => settings_view::UiFontScale::Small,
    Some("large") => settings_view::UiFontScale::Large,
    _ => settings_view::UiFontScale::Standard,
  };
  let init_editor_font_px = vault
    .get_pref("editorFontPx")
    .and_then(|s| s.parse::<u32>().ok())
    .unwrap_or(16)
    .clamp(13, 22);
  let init_editor_font_family = match vault.get_pref("editorFontFamily").as_deref() {
    Some("serif") => settings_view::EditorFontFamily::Serif,
    Some("mono") => settings_view::EditorFontFamily::Mono,
    _ => settings_view::EditorFontFamily::System,
  };
  let ui_font_scale_sig = signal(init_ui_scale);
  let ui_scale_f32_sig = signal(match init_ui_scale {
    settings_view::UiFontScale::Small => 0.90_f32,
    settings_view::UiFontScale::Standard => 1.0_f32,
    settings_view::UiFontScale::Large => 1.15_f32,
  });
  let editor_font_px_sig = signal(init_editor_font_px);
  let editor_font_family_sig = signal(init_editor_font_family);
  let editor_font_css_sig: Signal<&'static str> = signal(match init_editor_font_family {
    settings_view::EditorFontFamily::System => "",
    settings_view::EditorFontFamily::Serif => "Georgia, serif",
    settings_view::EditorFontFamily::Mono => "monospace",
  });
  provide_context(mica_theme::UiAppearance {
    ui_scale: ui_scale_f32_sig,
    editor_font_px: editor_font_px_sig,
    editor_font_family: editor_font_css_sig,
    locale_zh,
  });
  {
    let v_ui = vault.clone();
    effect(move || {
      let sc = ui_font_scale_sig.get();
      let (f, key) = match sc {
        settings_view::UiFontScale::Small => (0.90_f32, "small"),
        settings_view::UiFontScale::Standard => (1.0_f32, "standard"),
        settings_view::UiFontScale::Large => (1.15_f32, "large"),
      };
      ui_scale_f32_sig.set(f);
      let _ = v_ui.set_pref("uiFontScale", key);
    });
  }
  {
    let v_fp = vault.clone();
    effect(move || {
      let px_v = editor_font_px_sig.get().clamp(13, 22);
      let _ = v_fp.set_pref("editorFontPx", &px_v.to_string());
    });
  }
  {
    let v_ff = vault.clone();
    effect(move || {
      let fam = editor_font_family_sig.get();
      let (css, key) = match fam {
        settings_view::EditorFontFamily::System => ("", "system"),
        settings_view::EditorFontFamily::Serif => ("Georgia, serif", "serif"),
        settings_view::EditorFontFamily::Mono => ("monospace", "mono"),
      };
      editor_font_css_sig.set(css);
      let _ = v_ff.set_pref("editorFontFamily", key);
    });
  }
  let live_views_sig = signal(init_live);
  let trashed_views_sig = signal(init_trashed);

  // Sync page_width_px_sig (Settings) -> page_width_sig (Editor) + persist
  {
    let v_pw = vault.clone();
    effect(move || {
      let px_val = page_width_px_sig.get() as f32;
      if (page_width_sig.get_untracked() - px_val).abs() > 0.5 {
        page_width_sig.set(px_val);
        let _ = v_pw.set_page_width(px_val);
      }
    });
  }
  {
    let v_sw = vault.clone();
    effect(move || {
      let w = sidebar_width.get();
      let _ = v_sw.set_sidebar_width(w);
    });
  }

  // Root owner for long-lived async tasks (native OS file dialogs, exports, imports,
  // backups, cloud sync) so closing a context menu or popover NEVER cancels them.
  let app_owner = Owner::current();
  preview_render::init_preview_owner(app_owner);

  // Root-mounted sidebar rename & emoji picker modal state (avoids 280px sidebar clipping)
  let sidebar_rename_open = signal(false);
  let sidebar_rename_view: Signal<Option<Uuid>> = signal(None);
  let sidebar_rename_initial = signal(String::new());
  let sidebar_emoji_open = signal(false);
  let sidebar_emoji_view: Signal<Option<Uuid>> = signal(None);
  let sidebar_emoji_current: Signal<Option<String>> = signal(None);

  // Tabs (with per-tab history)
  let open_tabs = signal(
    initial_doc_id
      .map(|id| vec![OpenTab::single(id, initial_title.clone())])
      .unwrap_or_default(),
  );
  let active_tab_idx = signal(0usize);
  let can_go_back = computed(move || {
    let tabs = open_tabs.get();
    let idx = active_tab_idx.get().min(tabs.len().saturating_sub(1));
    tabs.get(idx).is_some_and(|t| t.cursor > 0)
  });
  let can_go_forward = computed(move || {
    let tabs = open_tabs.get();
    let idx = active_tab_idx.get().min(tabs.len().saturating_sub(1));
    tabs.get(idx).is_some_and(|t| t.cursor + 1 < t.history.len())
  });
  let can_close_tab = signal(false);
  effect(move || {
    can_close_tab.set(open_tabs.get().len() > 1);
  });

  // Favorites (ordered Vec<Uuid>, persisted per workspace)
  let init_favs = if ws_id.is_nil() { Vec::new() } else { vault.favorites_for(ws_id) };
  let favorites: Rc<RefCell<Vec<Uuid>>> = Rc::new(RefCell::new(init_favs));
  let favorite_pages: Signal<Vec<(Uuid, String)>> = signal(Vec::new());
  let active_is_favorite = signal(initial_doc_id.is_some_and(|id| favorites.borrow().contains(&id)));

  // Snapshots & Version History dialog state
  let (init_snaps, init_snap_ids, init_ver_items) = initial_doc_id
    .map(|id| format_version_entries(&vault, &initial_origin, id))
    .unwrap_or_default();
  let snapshot_count = signal(init_snaps.len());
  let snapshots_sig = signal(init_snaps);
  let snapshot_id_map: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(init_snap_ids));
  let version_items_sig = signal(init_ver_items);
  let version_selected_id: Signal<Option<String>> = signal(None);
  let version_current_md = signal(String::new());
  let version_prev_md: Signal<Option<String>> = signal(None);

  // Cloud & World switcher state
  let default_servers = if !allow_local {
    vec![initial_origin.clone()]
  } else {
    let saved_servers = vault.known_servers();
    if saved_servers.is_empty() {
      vec!["http://127.0.0.1:8080".to_string()]
    } else {
      saved_servers
    }
  };
  let server_url = signal(default_servers[0].clone());
  let known_servers_sig = signal(default_servers);
  let saved_creds: Vec<(String, String)> = vault
    .saved_server_credentials()
    .into_iter()
    .filter(|(orig, _, tok)| {
      !tok.trim().is_empty()
        || vault
          .saved_server_credential_full(orig)
          .is_some_and(|(_, _, rt)| !rt.trim().is_empty())
    })
    .map(|(orig, email, _)| (orig, email))
    .collect();
  let signed_in_origins_sig = signal(saved_creds);
  let selected_cloud_origin = signal(server_url.get_untracked());
  let world_health_status = signal("checking".to_string());
  let world_busy = signal(false);
  let world_error = signal(String::new());
  let world_info = signal(String::new());
  let world_active_origin = signal(if allow_local { LOCAL_ORIGIN.to_string() } else { initial_origin.clone() });
  let cloud_status = signal(
    if allow_local {
      "模式：本地离线 SQLite Vault"
    } else {
      "请登录云端账户"
    }
    .to_string(),
  );
  let export_preview = signal(String::new());
  let cloud_connector = Rc::new(RefCell::new(CloudConnector::new(CloudConfig {
    server_url: server_url.get_untracked(),
    access_token: None,
    email: None,
  })));
  let auth_gen = signal(0u64);
  let probe_gen = signal(0u64);
  let check_gen = signal(0u64);
  let cloud_load_gen = signal(0u64);
  let doc_fetch_gen = signal(0u64);
  let doc_save_gen = signal(0u64);
  let doc_dirty_gen = signal(0u64);

  // Share dialog state
  let share_loading = signal(false);
  let share_busy = signal(false);
  let share_enabled = signal(false);
  let share_token: Signal<Option<String>> = signal(None);
  let share_error: Signal<Option<String>> = signal(None);

  // AI dialogs state
  let global_ai_open = signal(false);
  let global_ai_prompt = signal(String::new());
  let global_ai_target = signal(AiTarget::NewPage);
  let global_ai_output = signal(String::new());
  let global_ai_streaming = signal(false);
  let global_ai_done = signal(false);
  let global_ai_error: Signal<Option<String>> = signal(None);

  let inline_ai_open = signal(false);
  let inline_ai_prompt = signal(String::new());
  let inline_ai_context = signal(String::new());
  let inline_ai_allow_replace = signal(true);
  let inline_ai_output = signal(String::new());
  let inline_ai_phase = signal(InlineAiPhase::Idle);
  let inline_ai_error: Signal<Option<String>> = signal(None);

  // Virtualized `mica-editor` <-> `MicaDoc` CRDT Bridge
  let initial_md = initial_doc_id
    .and_then(|id| docs_store.borrow().get(&id).cloned())
    .unwrap_or_default();
  let bridge = MicaDocumentBridge::from_markdown(&initial_md);

  // Backlinks signal below editor
  let backlinks_sig: Signal<Vec<BacklinkEntry>> = signal(compute_backlinks_for(
    &tree.borrow(),
    &docs_store.borrow(),
    initial_doc_id,
    &initial_title,
  ));

  // Provide NodeView ambient callbacks (embedded whiteboard 3-way merge + code block AI)
  provide_context(OpenBoardCallback(Rc::new(move || {
    active_mode.set(MODE_BOARD);
  })));
  provide_context(OpenBoardWithBlockCallback(Rc::new(move |pos: usize, json: String| {
    let initial = if json.trim().is_empty() {
      mica_board::starter_board_json("whiteboard")
    } else {
      json
    };
    let sess = Rc::new(RefCell::new(BoardSession::from_json(&initial)));
    embedded_board_edit.set(Some(EmbeddedBoardEdit {
      block_pos: pos,
      base_json: initial,
      session: sess,
      rev: signal(0u64),
    }));
  })));
  provide_context(CodeAiCallback(Rc::new(move |action: CodeAiAction, _pos: usize, code: String| {
    let prompt = match action {
      CodeAiAction::Explain => "请用中文简明解释下面这段代码的作用与关键逻辑。".to_string(),
      CodeAiAction::Improve => "请优化下面这段代码的可读性与性能，并说明改动点。".to_string(),
      CodeAiAction::Fix => "请排查下面这段代码中可能存在的 Bug 或边界问题。".to_string(),
      CodeAiAction::Custom => String::new(),
    };
    inline_ai_context.set(format!("```\n{code}\n```"));
    inline_ai_prompt.set(prompt);
    inline_ai_allow_replace.set(false);
    inline_ai_output.set(String::new());
    inline_ai_phase.set(InlineAiPhase::Idle);
    inline_ai_error.set(None);
    inline_ai_open.set(true);
  })));

  let doc_sync_phase = signal(mica_sync::SyncPhase::Offline);
  let doc_presence_users: Signal<Vec<mica_sync::PresenceUser>> = signal(Vec::new());
  let doc_sync_badge = signal(String::new());
  let doc_loading = signal(false);
  let doc_sync = doc_sync_session::DocSyncController::new(
    vault.clone(),
    bridge.clone(),
    docs_store.clone(),
    cloud_connector.clone(),
    cloud_status,
    doc_sync_phase,
    doc_presence_users,
    doc_sync_badge,
    doc_loading,
  );
  let views_watcher = doc_sync_session::ViewsWsWatcher::new(cloud_connector.clone());

  {
    let ds_cb = doc_sync.clone();
    bridge.set_on_crdt_diff(Rc::new(move |diff: Vec<u8>| {
      ds_cb.push_local_diff(diff);
    }));
  }

  // Persist editor changes -> docs_store + SQLite Vault (+ debounced cloud PATCH).
  // When real-time `/ws/` CRDT sync owns the active cloud page, edits flow
  // incrementally through `doc_sync` instead of HTTP `PATCH /markdown` (`replace_all`),
  // eliminating concurrent-edit overwrites.
  {
    let vault_p = vault.clone();
    let docs_p = docs_store.clone();
    let tree_p = tree.clone();
    let cc_p = cloud_connector.clone();
    let ds_p = doc_sync.clone();
    let disk_save_seqs: std::sync::Arc<std::sync::Mutex<HashMap<(String, Uuid), u64>>> =
      std::sync::Arc::new(std::sync::Mutex::new(HashMap::new()));

    let schedule_cloud_patch = {
      let cc_c = cc_p.clone();
      let vault_c = vault_p.clone();
      let ds_c = ds_p.clone();
      Rc::new(move |orig: String, ws_id: Uuid, doc_id: Uuid, md: String| {
        if ds_c.is_active_for(&orig, ws_id, doc_id) {
          return;
        }
        if cloud_sync::is_cloud_origin(&orig)
          && is_cloud_signed_in.get_untracked()
          && !md.trim().is_empty()
        {
          let epoch = doc_save_gen.get_untracked().wrapping_add(1);
          doc_save_gen.set(epoch);
          let (cc_a, v_a, srv_req, email_req) = (
            cc_c.clone(),
            vault_c.clone(),
            server_url.get_untracked(),
            account_email.get_untracked(),
          );
          spawn(async move {
            timer(std::time::Duration::from_millis(600)).await;
            if doc_save_gen.get_untracked() != epoch
              || active_origin.get_untracked() != orig
              || active_workspace_id.get_untracked() != ws_id
            {
              return;
            }
            let mut conn = cc_a.borrow().clone();
            let res = cloud_sync::push_cloud_document(&mut conn, ws_id, doc_id, &md)
              .await
              .map(|r| (conn, r));
            if active_origin.get_untracked() != orig {
              return;
            }
            match res {
              Ok((conn, rot)) if rot.is_some() => {
                *cc_a.borrow_mut() = conn;
                if allow_local {
                  cloud_sync::persist_refreshed_tokens(&v_a, &srv_req, &email_req, rot);
                }
              }
              Ok(_) => {}
              Err(err) => cloud_status.set(format!("云端保存失败：{err}")),
            }
          });
        }
      })
    };

    // Synchronous flush path (invoked by `flush_pending_persist` and `set_front_matter`)
    {
      let vault_s = vault_p.clone();
      let docs_s = docs_p.clone();
      let tree_s = tree_p.clone();
      let seqs_s = disk_save_seqs.clone();
      let cloud_s = schedule_cloud_patch.clone();
      bridge.set_on_persist(Rc::new(move |md: String, _crdt: Option<Vec<u8>>| {
        doc_dirty_gen.set(doc_dirty_gen.get_untracked().wrapping_add(1));
        if let Some(doc_id) = active_doc.get_untracked() {
          let orig = active_origin.get_untracked();
          let ws_id = active_workspace_id.get_untracked();
          docs_s.borrow_mut().insert(doc_id, md.clone());
          if let Ok(mut map) = seqs_s.lock() {
            let entry = map.entry((orig.clone(), doc_id)).or_insert(0);
            *entry = entry.wrapping_add(1);
          }
          if !cloud_sync::is_cloud_origin(&orig) {
            let _ = vault_s.save_markdown_tracked(
              &orig,
              ws_id,
              doc_id,
              &md,
              mica_core::ActivitySource::Manual,
            );
          }
          let next_bl = compute_backlinks_for(
            &tree_s.borrow(),
            &docs_s.borrow(),
            Some(doc_id),
            &active_title.get_untracked(),
          );
          if backlinks_sig.get_untracked() != next_bl {
            backlinks_sig.set(next_bl);
          }
          cloud_s(orig, ws_id, doc_id, md);
        }
      }));
    }

    // Debounced background persist path for rapid interactive edits
    {
      let br_d = bridge.clone();
      let vault_d = vault_p.clone();
      let docs_d = docs_p.clone();
      let tree_d = tree_p.clone();
      let seqs_d = disk_save_seqs.clone();
      let cloud_d = schedule_cloud_patch;
      bridge.set_on_dirty(Rc::new(move || {
        let epoch = doc_dirty_gen.get_untracked().wrapping_add(1);
        doc_dirty_gen.set(epoch);
        let Some(doc_id) = active_doc.get_untracked() else {
          return;
        };
        let orig = active_origin.get_untracked();
        let ws_id = active_workspace_id.get_untracked();
        let br_a = br_d.clone();
        let vault_a = vault_d.clone();
        let docs_a = docs_d.clone();
        let tree_a = tree_d.clone();
        let seqs_a = seqs_d.clone();
        let cloud_a = cloud_d.clone();
        spawn(async move {
          timer(std::time::Duration::from_millis(300)).await;
          if doc_dirty_gen.get_untracked() != epoch
            || active_origin.get_untracked() != orig
            || active_doc.get_untracked() != Some(doc_id)
          {
            return;
          }
          let Some(md) = br_a.take_pending_markdown() else {
            return;
          };
          docs_a.borrow_mut().insert(doc_id, md.clone());
          let next_bl = compute_backlinks_for(
            &tree_a.borrow(),
            &docs_a.borrow(),
            Some(doc_id),
            &active_title.get_untracked(),
          );
          if backlinks_sig.get_untracked() != next_bl {
            backlinks_sig.set(next_bl);
          }
          let write_seq = seqs_a
            .lock()
            .ok()
            .map(|mut map| {
              let entry = map.entry((orig.clone(), doc_id)).or_insert(0);
              *entry = entry.wrapping_add(1);
              *entry
            })
            .unwrap_or(0);
          let v_bg = (*vault_a).clone();
          let orig_bg = orig.clone();
          let md_bg = md.clone();
          let seqs_bg = seqs_a.clone();
          spawn(async move {
            let _ = net::run_blocking(move || {
              let still_latest = seqs_bg
                .lock()
                .ok()
                .and_then(|map| map.get(&(orig_bg.clone(), doc_id)).copied())
                .is_none_or(|cur| cur == write_seq);
              if still_latest && !cloud_sync::is_cloud_origin(&orig_bg) {
                let _ = v_bg.save_markdown_tracked(
                  &orig_bg,
                  ws_id,
                  doc_id,
                  &md_bg,
                  mica_core::ActivitySource::Manual,
                );
              }
            })
            .await;
          });
          cloud_a(orig, ws_id, doc_id, md);
        });
      }));
    }
  }

  let sync_tree_snapshots: Rc<dyn Fn()> = {
    let tree_ref = tree.clone();
    let docs_ref = docs_store.clone();
    Rc::new(move || {
      let (live, trashed) = collect_all_views(&tree_ref.borrow());
      live_views_sig.set(live);
      trashed_views_sig.set(trashed);
      backlinks_sig.set(compute_backlinks_for(
        &tree_ref.borrow(),
        &docs_ref.borrow(),
        active_doc.get_untracked(),
        &active_title.get_untracked(),
      ));
      tree_rev.set(tree_rev.get_untracked() + 1);
    })
  };

  let refresh_snapshots: Rc<dyn Fn(Uuid)> = {
    let v = vault.clone();
    let id_map = snapshot_id_map.clone();
    Rc::new(move |doc_id: Uuid| {
      let orig = active_origin.get_untracked();
      let (entries, ids, hist_items) = format_version_entries(&v, &orig, doc_id);
      snapshot_count.set(entries.len());
      snapshots_sig.set(entries);
      version_items_sig.set(hist_items);
      *id_map.borrow_mut() = ids;
    })
  };

  let refresh_favorite_pages: Rc<dyn Fn()> = {
    let favs = favorites.clone();
    let tree_ref = tree.clone();
    Rc::new(move || {
      let order = favs.borrow();
      let t = tree_ref.borrow();
      let rows = t.all_live_rows();
      let mut list = Vec::new();
      for fav_id in order.iter() {
        if let Some(r) = rows.iter().find(|r| !r.is_folder && r.object_id == *fav_id) {
          list.push((r.object_id, r.name.clone()));
        }
      }
      favorite_pages.set(list);
    })
  };
  refresh_favorite_pages();

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

  let close_all_popovers: Rc<dyn Fn()> = Rc::new(move || {
    app_menu_open.set(false);
    workspace_menu_open.set(false);
    account_menu_open.set(false);
    new_tab_menu_open.set(false);
  });

  // Load doc into active tab (or new tab)
  let open_doc_internal: Rc<dyn Fn(Uuid, String, bool, bool)> = {
    let docs = docs_store.clone();
    let br = bridge.clone();
    let tree_ref = tree.clone();
    let v = vault.clone();
    let favs = favorites.clone();
    let ref_snaps = refresh_snapshots.clone();
    let cc = cloud_connector.clone();
    let ds = doc_sync.clone();
    Rc::new(move |doc_id: Uuid, title: String, record_history: bool, force_new_tab: bool| {
      br.flush_pending_persist();
      let orig = active_origin.get_untracked();
      let ws_id = active_workspace_id.get_untracked();
      let cached_md = docs
        .borrow()
        .get(&doc_id)
        .cloned()
        .or_else(|| v.load_markdown(&orig, doc_id));
      let is_cloud_signed =
        cloud_sync::is_cloud_origin(&orig) && is_cloud_signed_in.get_untracked() && !ws_id.is_nil();
      let needs_remote_load = is_cloud_signed && cached_md.is_none();
      let md = cached_md.unwrap_or_default();
      if !needs_remote_load {
        docs.borrow_mut().insert(doc_id, md.clone());
      }
      doc_loading.set(needs_remote_load);
      br.load_markdown(&md);
      active_doc.set(Some(doc_id));
      active_doc_id_str.set(doc_id.to_string());
      active_title.set(title.clone());
      active_is_favorite.set(favs.borrow().contains(&doc_id));
      ref_snaps(doc_id);
      backlinks_sig.set(compute_backlinks_for(
        &tree_ref.borrow(),
        &docs.borrow(),
        Some(doc_id),
        &title,
      ));

      if let Some(parent_name) = resolve_parent_folder_name(&tree_ref.borrow(), doc_id) {
        active_folder_name.set(parent_name);
      }

      let mut tabs = open_tabs.get_untracked();
      if force_new_tab {
        // Always open a fresh tab so right-clicking any page (including the active one)
        // and choosing "在新标签页打开" visibly creates and switches to a new tab.
        tabs.push(OpenTab::single(doc_id, title.clone()));
        active_tab_idx.set(tabs.len() - 1);
      } else {
        let cur = active_tab_idx.get_untracked().min(tabs.len().saturating_sub(1));
        if tabs.get(cur).is_some_and(|t| t.doc_id == doc_id) {
          tabs[cur].title = title.clone();
        } else if let Some(pos) = tabs.iter().position(|t| t.doc_id == doc_id) {
          tabs[pos].title = title.clone();
          active_tab_idx.set(pos);
        } else if record_history {
          if tabs.is_empty() {
            tabs.push(OpenTab::single(doc_id, title.clone()));
            active_tab_idx.set(0);
          } else {
            tabs[cur].navigate(doc_id, title.clone());
          }
        } else if !tabs.is_empty() {
          tabs[cur].doc_id = doc_id;
          tabs[cur].title = title.clone();
        }
      }
      open_tabs.set(tabs);
      active_mode.set(MODE_DOC);
      if is_narrow.get_untracked() {
        sidebar_open.set(false);
      }

      if cloud_sync::is_cloud_origin(&orig) && is_cloud_signed_in.get_untracked() && !ws_id.is_nil() {
        let srv_req = server_url.get_untracked();
        ds.open_cloud_doc(srv_req.clone(), orig.clone(), ws_id, doc_id, account_name.get_untracked());
        let fetch_epoch = doc_fetch_gen.get_untracked().wrapping_add(1);
        doc_fetch_gen.set(fetch_epoch);
        let dirty_snap = doc_dirty_gen.get_untracked();
        let (cc_a, docs_a, v_a, br_a, tree_a, ds_a) =
          (cc.clone(), docs.clone(), v.clone(), br.clone(), tree_ref.clone(), ds.clone());
        let email_req = account_email.get_untracked();
        spawn(async move {
          let mut conn = cc_a.borrow().clone();
          let res = cloud_sync::fetch_cloud_document(&mut conn, ws_id, doc_id)
            .await
            .map(|(m, r)| (conn, m, r));
          if doc_fetch_gen.get_untracked() != fetch_epoch
            || active_origin.get_untracked() != orig
            || active_workspace_id.get_untracked() != ws_id
            || active_doc.get_untracked() != Some(doc_id)
          {
            return;
          }
          doc_loading.set(false);
          if let Ok((conn, remote_md, rot)) = res {
            if rot.is_some() {
              *cc_a.borrow_mut() = conn;
              if allow_local {
                cloud_sync::persist_refreshed_tokens(&v_a, &srv_req, &email_req, rot);
              }
            }
            docs_a.borrow_mut().insert(doc_id, remote_md.clone());
            if !ds_a.is_ready_for(&orig, ws_id, doc_id)
              && doc_dirty_gen.get_untracked() == dirty_snap
              && (needs_remote_load || remote_md != md)
            {
              br_a.load_markdown(&remote_md);
            }
            backlinks_sig.set(compute_backlinks_for(
              &tree_a.borrow(),
              &docs_a.borrow(),
              Some(doc_id),
              &title,
            ));
          }
        });
      } else {
        doc_loading.set(false);
        ds.close();
      }
    })
  };

  let on_select_doc: Rc<dyn Fn(Uuid, String)> = {
    let inner = open_doc_internal.clone();
    Rc::new(move |doc_id: Uuid, title: String| inner(doc_id, title, true, false))
  };
  let on_open_in_new_tab: Rc<dyn Fn(Uuid, String)> = {
    let inner = open_doc_internal.clone();
    Rc::new(move |doc_id: Uuid, title: String| inner(doc_id, title, true, true))
  };

  let on_nav_back: Rc<dyn Fn()> = {
    let inner = open_doc_internal.clone();
    Rc::new(move || {
      let mut tabs = open_tabs.get_untracked();
      let idx = active_tab_idx.get_untracked().min(tabs.len().saturating_sub(1));
      if let Some(tab) = tabs.get_mut(idx)
        && tab.cursor > 0
      {
        tab.cursor -= 1;
        let (doc_id, title) = tab.history[tab.cursor].clone();
        open_tabs.set(tabs);
        inner(doc_id, title, false, false);
      }
    })
  };

  let on_nav_forward: Rc<dyn Fn()> = {
    let inner = open_doc_internal.clone();
    Rc::new(move || {
      let mut tabs = open_tabs.get_untracked();
      let idx = active_tab_idx.get_untracked().min(tabs.len().saturating_sub(1));
      if let Some(tab) = tabs.get_mut(idx)
        && tab.cursor + 1 < tab.history.len()
      {
        tab.cursor += 1;
        let (doc_id, title) = tab.history[tab.cursor].clone();
        open_tabs.set(tabs);
        inner(doc_id, title, false, false);
      }
    })
  };

  let on_close_tab_at: Rc<dyn Fn(usize)> = {
    let inner = open_doc_internal.clone();
    Rc::new(move |closing_idx: usize| {
      let mut cur_tabs = open_tabs.get_untracked();
      if cur_tabs.len() <= 1 || closing_idx >= cur_tabs.len() {
        return;
      }
      let cur_active = active_tab_idx.get_untracked().min(cur_tabs.len() - 1);
      cur_tabs.remove(closing_idx);
      let next_active = if closing_idx == cur_active {
        closing_idx.min(cur_tabs.len() - 1)
      } else if closing_idx < cur_active {
        cur_active - 1
      } else {
        cur_active
      };
      let next_doc = cur_tabs[next_active].clone();
      active_tab_idx.set(next_active);
      open_tabs.set(cur_tabs);
      if closing_idx == cur_active {
        inner(next_doc.doc_id, next_doc.title, false, false);
      }
    })
  };

  let account_id = signal(String::new());
  let cloud_session = cloud_sync::CloudSessionHandles {
    vault: vault.clone(),
    tree: tree.clone(),
    docs_store: docs_store.clone(),
    favorites: favorites.clone(),
    bridge: bridge.clone(),
    cloud_connector: cloud_connector.clone(),
    active_origin,
    active_workspace_id,
    active_folder_id,
    active_folder_name,
    workspace_name,
    workspaces,
    active_doc,
    active_doc_id_str,
    active_title,
    active_is_favorite,
    open_tabs,
    active_tab_idx,
    active_mode,
    server_url,
    known_servers_sig,
    signed_in_origins_sig,
    selected_cloud_origin,
    world_active_origin,
    world_switcher_open,
    world_can_close,
    allow_local,
    world_busy,
    world_error,
    world_info,
    account_id,
    account_name,
    account_email,
    is_cloud_signed_in,
    cloud_status,
    auth_gen,
    cloud_load_gen,
    doc_fetch_gen,
    doc_save_gen,
    doc_loading,
    doc_sync: doc_sync.clone(),
    views_watcher: views_watcher.clone(),
    refresh_snapshots: refresh_snapshots.clone(),
    refresh_favorite_pages: refresh_favorite_pages.clone(),
    sync_tree_snapshots: sync_tree_snapshots.clone(),
    open_doc_internal: open_doc_internal.clone(),
  };

  {
    let cs_views = cloud_session.clone();
    views_watcher.set_on_views_changed(Rc::new(move |srv: String, cache_orig: String, ws_id: Uuid| {
      cs_views.refresh_views_only(srv, cache_orig, ws_id);
    }));
  }

  let create_page_in_parent: Rc<dyn Fn(Option<Uuid>, String, String, bool)> = {
    let tree_ref = tree.clone();
    let docs = docs_store.clone();
    let v = vault.clone();
    let inner = open_doc_internal.clone();
    let sync_t = sync_tree_snapshots.clone();
    let cs = cloud_session.clone();
    Rc::new(move |parent: Option<Uuid>, name: String, markdown: String, in_new_tab: bool| {
      let orig = active_origin.get_untracked();
      let cur_ws = active_workspace_id.get_untracked();
      let valid_parent = parent.filter(|&pid| {
        tree_ref
          .borrow()
          .get(pid)
          .is_some_and(|row| row.object_type == "folder" && !row.is_deleted)
      });
      if cloud_sync::is_cloud_origin(&orig) && is_cloud_signed_in.get_untracked() && !cur_ws.is_nil() {
        cs.create_cloud_page(orig, cur_ws, valid_parent, name, markdown, in_new_tab);
        return;
      }
      let new_doc_id = Uuid::new_v4();
      let count = tree_ref.borrow().all_live_rows().len() + 1;
      let pos = format!("{:010}", count * 10);
      let view_row = make_view(cur_ws, new_doc_id, &name, "page", valid_parent, &pos);
      let _ = v.save_view(&orig, &view_row);
      let _ = v.save_markdown(&orig, new_doc_id, &markdown);
      v.record_doc_created(&orig, cur_ws, new_doc_id, mica_core::ActivitySource::Manual, &markdown);
      {
        let mut t = tree_ref.borrow_mut();
        t.upsert_view(view_row);
        if let Some(pid) = valid_parent {
          t.set_expanded(pid, true);
        }
      }
      docs.borrow_mut().insert(new_doc_id, markdown);
      sync_t();
      inner(new_doc_id, name, true, in_new_tab);
    })
  };

  let create_page_with_content: Rc<dyn Fn(String, String)> = {
    let create = create_page_in_parent.clone();
    Rc::new(move |name: String, markdown: String| {
      let fid = active_folder_id.get_untracked();
      create((!fid.is_nil()).then_some(fid), name, markdown, false);
    })
  };

  let on_new_page: Rc<dyn Fn()> = {
    let tree_ref = tree.clone();
    let create = create_page_with_content.clone();
    Rc::new(move || {
      let idx = tree_ref.borrow().count_live_pages() + 1;
      create(format!("未命名页面 {idx}"), String::new());
    })
  };

  let create_folder_in_parent: Rc<dyn Fn(Option<Uuid>, Option<String>) -> Uuid> = {
    let tree_ref = tree.clone();
    let v = vault.clone();
    let sync_t = sync_tree_snapshots.clone();
    let cs = cloud_session.clone();
    Rc::new(move |parent: Option<Uuid>, custom_name: Option<String>| {
      let orig = active_origin.get_untracked();
      let cur_ws = active_workspace_id.get_untracked();
      let count = tree_ref.borrow().all_live_rows().len() + 1;
      let name = custom_name.unwrap_or_else(|| format!("文件夹 {count}"));
      let valid_parent = parent.filter(|&pid| {
        tree_ref
          .borrow()
          .get(pid)
          .is_some_and(|row| row.object_type == "folder" && !row.is_deleted)
      });
      if cloud_sync::is_cloud_origin(&orig) && is_cloud_signed_in.get_untracked() && !cur_ws.is_nil() {
        cs.create_cloud_folder(orig, cur_ws, valid_parent, name);
        return Uuid::nil();
      }
      let fid = Uuid::new_v4();
      let folder = make_view(cur_ws, fid, &name, "folder", valid_parent, &format!("{:010}", count * 10));
      let _ = v.save_view(&orig, &folder);
      {
        let mut t = tree_ref.borrow_mut();
        t.upsert_view(folder.clone());
        t.set_expanded(folder.id, true);
        if let Some(pid) = valid_parent {
          t.set_expanded(pid, true);
        }
      }
      sync_t();
      fid
    })
  };

  let on_new_folder: Rc<dyn Fn()> = {
    let create_f = create_folder_in_parent.clone();
    Rc::new(move || {
      let _ = create_f(None, None);
    })
  };

  // Import batch of ImportedPageSpec into an optional parent folder
  let import_page_specs: Rc<dyn Fn(Option<Uuid>, Vec<ImportedPageSpec>) -> usize> = {
    let tree_ref = tree.clone();
    let docs = docs_store.clone();
    let v = vault.clone();
    let inner = open_doc_internal.clone();
    let sync_t = sync_tree_snapshots.clone();
    let cs = cloud_session.clone();
    Rc::new(move |base_parent: Option<Uuid>, specs: Vec<ImportedPageSpec>| {
      if specs.is_empty() {
        return 0;
      }
      let orig = active_origin.get_untracked();
      let cur_ws = active_workspace_id.get_untracked();
      if cloud_sync::is_cloud_origin(&orig) && is_cloud_signed_in.get_untracked() && !cur_ws.is_nil() {
        let count = specs.len();
        cs.import_cloud_pages(orig, cur_ws, base_parent, specs);
        return count;
      }
      let mut folder_cache: HashMap<(Option<Uuid>, String), Uuid> = HashMap::new();
      let mut last_opened: Option<(Uuid, String)> = None;
      let mut imported = 0usize;
      for spec in specs {
        let mut parent = base_parent;
        for seg in spec.folder_path {
          let seg_trim = seg.trim().to_string();
          if seg_trim.is_empty() {
            continue;
          }
          let key = (parent, seg_trim.clone());
          let fid = if let Some(&existing) = folder_cache.get(&key) {
            existing
          } else {
            let fid = Uuid::new_v4();
            let pos = format!("{:010}", (tree_ref.borrow().all_live_rows().len() + 1) * 10);
            let f = make_view(cur_ws, fid, &seg_trim, "folder", parent, &pos);
            let _ = v.save_view(&orig, &f);
            {
              let mut t = tree_ref.borrow_mut();
              t.upsert_view(f);
              t.set_expanded(fid, true);
            }
            folder_cache.insert(key, fid);
            fid
          };
          parent = Some(fid);
        }
        let doc_id = Uuid::new_v4();
        let pos = format!("{:010}", (tree_ref.borrow().all_live_rows().len() + 1) * 10);
        let page = make_view(cur_ws, doc_id, &spec.title, "page", parent, &pos);
        let _ = v.save_view(&orig, &page);
        let _ = v.save_markdown(&orig, doc_id, &spec.markdown);
        v.record_doc_created(&orig, cur_ws, doc_id, mica_core::ActivitySource::Import, &spec.markdown);
        {
          let mut t = tree_ref.borrow_mut();
          t.upsert_view(page);
          if let Some(pid) = parent {
            t.set_expanded(pid, true);
          }
        }
        docs.borrow_mut().insert(doc_id, spec.markdown);
        last_opened = Some((doc_id, spec.title));
        imported += 1;
      }
      sync_t();
      if let Some((did, title)) = last_opened {
        inner(did, title, true, false);
      }
      imported
    })
  };

  let reload_tree_from_vault: Rc<dyn Fn()> = {
    let tree_ref = tree.clone();
    let docs = docs_store.clone();
    let v = vault.clone();
    let ref_favs = refresh_favorite_pages.clone();
    let sync_t = sync_tree_snapshots.clone();
    Rc::new(move || {
      let orig = active_origin.get_untracked();
      let cur_ws = active_workspace_id.get_untracked();
      let views = v.list_workspace_views(&orig, cur_ws);
      let mut next_docs = docs.borrow().clone();
      for row in &views {
        if cloud_sync::is_page_view(row)
          && !next_docs.contains_key(&row.object_id)
          && let Some(md) = v.load_markdown(&orig, row.object_id)
        {
          next_docs.insert(row.object_id, md);
        }
      }
      *docs.borrow_mut() = next_docs;
      let expanded: Vec<Uuid> = tree_ref
        .borrow()
        .visible_rows()
        .into_iter()
        .filter(|r| r.is_folder && r.expanded)
        .map(|r| r.view_id)
        .collect();
      let mut next_tree = PageTreeModel::new(views);
      for id in expanded {
        next_tree.set_expanded(id, true);
      }
      *tree_ref.borrow_mut() = next_tree;
      ref_favs();
      sync_t();
    })
  };

  let on_rename_view: Rc<dyn Fn(Uuid, String)> = {
    let tree_ref = tree.clone();
    let v = vault.clone();
    let ref_favs = refresh_favorite_pages.clone();
    let sync_t = sync_tree_snapshots.clone();
    let cs = cloud_session.clone();
    Rc::new(move |vid: Uuid, next_name: String| {
      let orig = active_origin.get_untracked();
      let cur_ws = active_workspace_id.get_untracked();
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
        let rename_req = next_name.clone();
        cs.spawn_cloud_op("云端重命名", move |c| {
          let rename_req = rename_req.clone();
          async move { c.update_view(cur_ws, vid, &rename_req, None).await.map(|_| ()) }
        });
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
        sync_t();
      }
    })
  };

  let on_trash_view: Rc<dyn Fn(Uuid)> = {
    let v = vault.clone();
    let reload = reload_tree_from_vault.clone();
    let cs = cloud_session.clone();
    Rc::new(move |vid: Uuid| {
      let orig = active_origin.get_untracked();
      let cur_ws = active_workspace_id.get_untracked();
      let _ = v.trash_view_subtree(&orig, vid);
      reload();
      cs.spawn_cloud_op("云端移入回收站", move |c| async move { c.delete_view(cur_ws, vid).await });
    })
  };

  let on_restore_view: Rc<dyn Fn(Uuid)> = {
    let v = vault.clone();
    let reload = reload_tree_from_vault.clone();
    let cs = cloud_session.clone();
    Rc::new(move |vid: Uuid| {
      let orig = active_origin.get_untracked();
      let cur_ws = active_workspace_id.get_untracked();
      let _ = v.restore_view_subtree(&orig, vid);
      reload();
      cs.spawn_cloud_op("云端恢复页面", move |c| async move { c.restore_view(cur_ws, vid).await });
    })
  };

  let on_purge_view: Rc<dyn Fn(Uuid)> = {
    let v = vault.clone();
    let reload = reload_tree_from_vault.clone();
    let cs = cloud_session.clone();
    Rc::new(move |vid: Uuid| {
      let orig = active_origin.get_untracked();
      let cur_ws = active_workspace_id.get_untracked();
      let _ = v.purge_view_subtree(&orig, vid);
      reload();
      cs.spawn_cloud_op("云端永久删除", move |c| async move { c.purge_view(cur_ws, vid).await });
    })
  };

  let on_purge_all_trash: Rc<dyn Fn() -> usize> = {
    let v = vault.clone();
    let reload = reload_tree_from_vault.clone();
    Rc::new(move || {
      let orig = active_origin.get_untracked();
      let ws = active_workspace_id.get_untracked();
      let n = v.purge_all_trash(&orig, ws).unwrap_or(0);
      reload();
      n
    })
  };

  let on_reorder_views: Rc<dyn Fn(Option<Uuid>, Vec<Uuid>)> = {
    let v = vault.clone();
    let reload = reload_tree_from_vault.clone();
    let cs = cloud_session.clone();
    Rc::new(move |parent_id: Option<Uuid>, ordered_ids: Vec<Uuid>| {
      let orig = active_origin.get_untracked();
      let cur_ws = active_workspace_id.get_untracked();
      if v.reorder_views(&orig, parent_id, &ordered_ids).is_ok() {
        reload();
        cs.spawn_cloud_op("云端排序同步", move |c| {
          let ordered_ids = ordered_ids.clone();
          async move { c.reorder_views(cur_ws, parent_id, &ordered_ids).await }
        });
      }
    })
  };

  let on_move_view: Rc<dyn Fn(Uuid, i32)> = {
    let tree_ref = tree.clone();
    let reorder = on_reorder_views.clone();
    Rc::new(move |vid: Uuid, delta: i32| {
      let rows = tree_ref.borrow().visible_rows();
      let Some(pos_idx) = rows.iter().position(|r| r.view_id == vid) else {
        return;
      };
      let target_parent = rows[pos_idx].parent_view_id;
      let mut siblings: Vec<Uuid> = rows
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
      siblings.swap(sib_idx, next_idx);
      reorder(target_parent, siblings);
    })
  };

  let on_set_icon: Rc<dyn Fn(Uuid, Option<String>)> = {
    let v = vault.clone();
    let tree_ref = tree.clone();
    let sync_t = sync_tree_snapshots.clone();
    Rc::new(move |vid: Uuid, icon: Option<String>| {
      let _ = v.set_view_icon(vid, icon.as_deref());
      {
        let mut t = tree_ref.borrow_mut();
        if let Some(mut row) = t.get(vid).cloned() {
          row.icon = icon;
          t.upsert_view(row);
        }
      }
      sync_t();
    })
  };

  let on_duplicate_view: Rc<dyn Fn(Uuid)> = {
    let v = vault.clone();
    let tree_ref = tree.clone();
    let reload = reload_tree_from_vault.clone();
    Rc::new(move |vid: Uuid| {
      let orig = active_origin.get_untracked();
      let base_name = tree_ref
        .borrow()
        .get(vid)
        .map(|r| format!("{} (副本)", r.name))
        .unwrap_or_else(|| "未命名副本".to_string());
      if v.clone_view(&orig, vid, &base_name).is_ok() {
        reload();
      }
    })
  };

  let on_switch_workspace: Rc<dyn Fn(WorkspaceEntry)> = {
    let v = vault.clone();
    let cs = cloud_session.clone();
    Rc::new(move |entry: WorkspaceEntry| {
      workspace_menu_open.set(false);
      let mut ws_views = v.list_workspace_views(&entry.origin, entry.id);
      if cloud_sync::is_cloud_origin(&entry.origin) {
        cs.apply_workspace_views(
          entry.origin.clone(),
          entry.id,
          entry.name.clone(),
          ws_views,
          None,
          false,
        );
        cs.refresh_workspace(entry.origin, entry.id, entry.name, None);
        return;
      }

      if ws_views.is_empty() {
        let fid = Uuid::new_v4();
        let did = Uuid::new_v4();
        let f = make_view(entry.id, fid, "知识库", "folder", None, "0000000010");
        let p = make_view(entry.id, did, "工作区首页", "page", Some(fid), "0000000010");
        let _ = v.save_view(&entry.origin, &f);
        let _ = v.save_view(&entry.origin, &p);
        let _ = v.save_markdown(&entry.origin, did, "# 工作区首页\n\n在此开始记录。");
        ws_views = vec![f, p];
      } else if !ws_views.iter().any(|r| cloud_sync::is_page_view(r) && !r.is_deleted) {
        let fid = ws_views
          .iter()
          .find(|r| r.object_type == "folder" && !r.is_deleted)
          .map(|r| r.id);
        let did = Uuid::new_v4();
        let p = make_view(entry.id, did, "未命名页面", "page", fid, "0000000010");
        let _ = v.save_view(&entry.origin, &p);
        let _ = v.save_markdown(&entry.origin, did, "");
        ws_views.push(p);
      }
      cs.apply_workspace_views(entry.origin, entry.id, entry.name, ws_views, None, false);
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
        .all_live_rows()
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
        .all_live_rows()
        .into_iter()
        .find(|r| !r.is_folder && r.object_id == doc_id)
        .map(|r| r.view_id);
      if let Some(vid) = target_id {
        trash_v(vid);
      }
    })
  };

  let on_toggle_favorite: Rc<dyn Fn()> = {
    let favs = favorites.clone();
    let v = vault.clone();
    let br = bridge.clone();
    let ref_favs = refresh_favorite_pages.clone();
    Rc::new(move || {
      if let Some(doc_id) = active_doc.get_untracked() {
        let now_fav = {
          let mut vec = favs.borrow_mut();
          if let Some(pos) = vec.iter().position(|&id| id == doc_id) {
            vec.remove(pos);
            false
          } else {
            vec.push(doc_id);
            true
          }
        };
        let _ = v.set_favorites_for(active_workspace_id.get_untracked(), &favs.borrow());
        active_is_favorite.set(now_fav);
        ref_favs();
        br.set_status_flash(if now_fav { "已加入收藏" } else { "已取消收藏" });
      }
    })
  };

  let on_remove_favorite: Rc<dyn Fn(Uuid)> = {
    let favs = favorites.clone();
    let v = vault.clone();
    let ref_favs = refresh_favorite_pages.clone();
    Rc::new(move |doc_id: Uuid| {
      favs.borrow_mut().retain(|&id| id != doc_id);
      let _ = v.set_favorites_for(active_workspace_id.get_untracked(), &favs.borrow());
      if active_doc.get_untracked() == Some(doc_id) {
        active_is_favorite.set(false);
      }
      ref_favs();
    })
  };

  // Real Export & Import callbacks (`app_transfer_io` + `transfer.rs` + `mica-interchange`).
  let AppTransferHandlers {
    on_export_page_markdown,
    on_export_page_html,
    on_export_markdown,
    on_export_html,
    on_export_bundle,
    export_workspace_zip,
    export_folder_zip,
    on_import_markdown,
    on_import_zip_picker,
    on_import_files_picker,
    on_import_folder_picker,
  } = app_transfer_io::build_transfer_handlers(AppTransferContext {
    app_owner,
    vault: vault.clone(),
    bridge: bridge.clone(),
    tree: tree.clone(),
    docs_store: docs_store.clone(),
    active_doc,
    active_origin,
    active_title,
    active_workspace_id,
    workspace_name,
    workspaces,
    export_preview,
    create_page_with_content: create_page_with_content.clone(),
    import_page_specs: import_page_specs.clone(),
  });

  // Snapshots & 980×640 Version History Diff callbacks
  let load_version_diff_preview: Rc<dyn Fn(String)> = {
    let v = vault.clone();
    let id_map = snapshot_id_map.clone();
    Rc::new(move |ver_id: String| {
      version_selected_id.set(Some(ver_id.clone()));
      let Some(doc_id) = active_doc.get_untracked() else {
        return;
      };
      let orig = active_origin.get_untracked();
      if let Ok(curr_md) = v.load_version_markdown(&orig, doc_id, &ver_id) {
        version_current_md.set(curr_md);
      }
      let ids = id_map.borrow();
      let prev = ids
        .iter()
        .position(|x| x == &ver_id)
        .and_then(|pos| ids.get(pos + 1))
        .and_then(|prev_id| v.load_version_markdown(&orig, doc_id, prev_id).ok());
      version_prev_md.set(prev);
    })
  };

  let on_open_version_history: Rc<dyn Fn()> = {
    let ref_snaps = refresh_snapshots.clone();
    let load_prev = load_version_diff_preview.clone();
    let id_map = snapshot_id_map.clone();
    Rc::new(move || {
      if let Some(doc_id) = active_doc.get_untracked() {
        ref_snaps(doc_id);
        if let Some(first_id) = id_map.borrow().first().cloned() {
          load_prev(first_id);
        }
      }
      version_modal_open.set(true);
    })
  };

  let on_save_snapshot: Rc<dyn Fn()> = {
    let v = vault.clone();
    let br = bridge.clone();
    let ref_snaps = refresh_snapshots.clone();
    Rc::new(move || {
      br.flush_pending_persist();
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

  let on_restore_version_by_id: Rc<dyn Fn(String)> = {
    let v = vault.clone();
    let br = bridge.clone();
    let docs = docs_store.clone();
    let ref_snaps = refresh_snapshots.clone();
    Rc::new(move |ver_id: String| {
      let Some(doc_id) = active_doc.get_untracked() else {
        return;
      };
      let orig = active_origin.get_untracked();
      match v.restore_version_markdown(&orig, doc_id, &ver_id) {
        Ok(md) => {
          docs.borrow_mut().insert(doc_id, md.clone());
          br.load_markdown(&md);
          ref_snaps(doc_id);
          version_modal_open.set(false);
          br.set_status_flash("已恢复历史版本");
        }
        Err(e) => br.set_status_flash(format!("恢复失败：{e}")),
      }
    })
  };

  let on_restore_snapshot: Rc<dyn Fn(i64)> = {
    let id_map = snapshot_id_map.clone();
    let restore_id = on_restore_version_by_id.clone();
    Rc::new(move |idx: i64| {
      if let Some(ver_id) = id_map.borrow().get(idx as usize).cloned() {
        restore_id(ver_id);
      }
    })
  };

  let on_open_graph: Rc<dyn Fn()> = {
    let tree_ref = tree.clone();
    let docs = docs_store.clone();
    Rc::new(move || {
      let (live, _) = collect_all_views(&tree_ref.borrow());
      graph_data.set(graph_dialog::build_local_page_graph(&live, &docs.borrow()));
      graph_modal_open.set(true);
    })
  };

  let on_toggle_theme: Rc<dyn Fn()> = {
    let v = vault.clone();
    Rc::new(move || {
      let next = !dark_mode.get_untracked();
      dark_mode.set(next);
      let _ = v.set_bool_pref("darkMode", next);
      set_color_scheme(if next { Appearance::Dark } else { Appearance::Light });
    })
  };

  let on_open_search_with: Rc<dyn Fn(Option<String>, bool)> = {
    let close_p = close_all_popovers.clone();
    Rc::new(move |query: Option<String>, in_new_tab: bool| {
      close_p();
      settings_modal_open.set(false);
      search_initial_query.set(query);
      search_open_in_new_tab.set(in_new_tab);
      search_modal_open.set(true);
    })
  };

  let on_open_search: Rc<dyn Fn()> = {
    let open_s = on_open_search_with.clone();
    Rc::new(move || open_s(None, false))
  };

  let on_open_settings: Rc<dyn Fn()> = {
    let close_p = close_all_popovers.clone();
    Rc::new(move || {
      close_p();
      search_modal_open.set(false);
      version_modal_open.set(false);
      graph_modal_open.set(false);
      workspace_settings_open.set(false);
      share_modal_open.set(false);
      about_modal_open.set(false);
      settings_modal_open.set(true);
    })
  };

  // Global App-Level Keyboard Shortcuts (1:1 Flutter `_appShortcuts` + `tab_shortcuts.dart`)
  {
    let open_s = on_open_search.clone();
    on_key_with("shift+mod+f", KeyOptions { when: When::Always, ..KeyOptions::default() }, move |_| {
      if search_modal_open.get_untracked() {
        search_modal_open.set(false);
      } else {
        open_s();
      }
    });
  }
  {
    let open_set = on_open_settings.clone();
    on_key_with("mod+,", KeyOptions { when: When::Always, ..KeyOptions::default() }, move |_| {
      if settings_modal_open.get_untracked() {
        settings_modal_open.set(false);
      } else {
        open_set();
      }
    });
  }
  {
    let new_p = on_new_page.clone();
    on_key_with("mod+n", KeyOptions { when: When::Always, ..KeyOptions::default() }, move |_| {
      new_p();
    });
  }
  {
    let open_s = on_open_search_with.clone();
    on_key_with("mod+t", KeyOptions { when: When::Always, ..KeyOptions::default() }, move |_| {
      open_s(None, true);
    });
  }
  {
    let close_tab = on_close_tab_at.clone();
    on_key_with("mod+w", KeyOptions { when: When::Always, ..KeyOptions::default() }, move |_| {
      close_tab(active_tab_idx.get_untracked());
    });
  }
  {
    let inner = open_doc_internal.clone();
    on_key_with("ctrl+tab", KeyOptions { when: When::Always, ..KeyOptions::default() }, move |_| {
      let tabs = open_tabs.get_untracked();
      if tabs.len() > 1 {
        let next = (active_tab_idx.get_untracked() + 1) % tabs.len();
        active_tab_idx.set(next);
        inner(tabs[next].doc_id, tabs[next].title.clone(), false, false);
      }
    });
  }
  {
    let inner = open_doc_internal.clone();
    on_key_with("shift+ctrl+tab", KeyOptions { when: When::Always, ..KeyOptions::default() }, move |_| {
      let tabs = open_tabs.get_untracked();
      if tabs.len() > 1 {
        let cur = active_tab_idx.get_untracked();
        let next = (cur + tabs.len() - 1) % tabs.len();
        active_tab_idx.set(next);
        inner(tabs[next].doc_id, tabs[next].title.clone(), false, false);
      }
    });
  }
  {
    let close_p = close_all_popovers.clone();
    on_key_with(
      "escape",
      KeyOptions { when: When::Always, pass_through: true, ..KeyOptions::default() },
      move |_| {
        close_p();
        search_modal_open.set(false);
        settings_modal_open.set(false);
        about_modal_open.set(false);
        workspace_settings_open.set(false);
        graph_modal_open.set(false);
        version_modal_open.set(false);
        share_modal_open.set(false);
        transfer_modal_open.set(false);
        global_ai_open.set(false);
        inline_ai_open.set(false);
        world_switcher_open.set(false);
      },
    );
  }

  // Sidebar callbacks (full 1:1 right-click + DnD + multi-select + Emoji + folder import/export)
  let sidebar_cb = Rc::new(SidebarCallbacks {
    on_select_doc: on_select_doc.clone(),
    on_new_page: on_new_page.clone(),
    on_new_folder,
    on_rename_view: on_rename_view.clone(),
    on_trash_view: on_trash_view.clone(),
    on_move_view,
    on_open_board: Rc::new(move || {
      active_mode.set(if active_mode.get_untracked() == MODE_BOARD { MODE_DOC } else { MODE_BOARD });
    }),
    on_restore_view: on_restore_view.clone(),
    on_open_search: on_open_search.clone(),
    on_open_settings: on_open_settings.clone(),
    on_toggle_workspace_menu: Rc::new(move || {
      account_menu_open.set(false);
      app_menu_open.set(false);
      workspace_menu_open.set(!workspace_menu_open.get_untracked());
    }),
    on_toggle_account_menu: Rc::new(move || {
      workspace_menu_open.set(false);
      app_menu_open.set(false);
      account_menu_open.set(!account_menu_open.get_untracked());
    }),
    on_open_graph: on_open_graph.clone(),
    on_open_workspace_settings: Rc::new(move || workspace_settings_open.set(true)),
    on_remove_favorite,
    on_open_in_new_tab: on_open_in_new_tab.clone(),
    on_new_child_page: {
      let create = create_page_in_parent.clone();
      let tree_ref = tree.clone();
      Rc::new(move |folder_id: Uuid| {
        let idx = tree_ref.borrow().count_live_pages() + 1;
        create(Some(folder_id), format!("未命名页面 {idx}"), String::new(), false);
      })
    },
    on_new_child_folder: {
      let create_f = create_folder_in_parent.clone();
      Rc::new(move |folder_id: Uuid| {
        let _ = create_f(Some(folder_id), None);
      })
    },
    on_set_icon: on_set_icon.clone(),
    on_duplicate_view,
    on_export_folder_backup: {
      let exp = export_folder_zip.clone();
      Rc::new(move |fid: Uuid| exp(fid, ArchiveExportFormat::BackupMicaZip))
    },
    on_export_folder_markdown: {
      let exp = export_folder_zip.clone();
      Rc::new(move |fid: Uuid| exp(fid, ArchiveExportFormat::MarkdownZip))
    },
    on_export_page_markdown: on_export_page_markdown.clone(),
    on_export_page_html: on_export_page_html.clone(),
    on_import_files_into_folder: {
      let imp = on_import_files_picker.clone();
      Rc::new(move |fid: Uuid| imp(Some(fid)))
    },
    on_import_folder_into_folder: {
      let imp = on_import_folder_picker.clone();
      Rc::new(move |fid: Uuid| imp(Some(fid)))
    },
    on_open_rename_modal: Some(Rc::new(move |vid: Uuid, cur_name: String| {
      sidebar_rename_view.set(Some(vid));
      sidebar_rename_initial.set(cur_name);
      sidebar_rename_open.set(true);
    })),
    on_open_emoji_modal: Some(Rc::new(move |vid: Uuid, cur_icon: Option<String>| {
      sidebar_emoji_view.set(Some(vid));
      sidebar_emoji_current.set(cur_icon);
      sidebar_emoji_open.set(true);
    })),
    on_transfer_move: Rc::new(move |ids: Vec<Uuid>| {
      transfer_source_ids.set(ids);
      transfer_copy_mode.set(false);
      transfer_modal_open.set(true);
    }),
    on_transfer_copy: Rc::new(move |ids: Vec<Uuid>| {
      transfer_source_ids.set(ids);
      transfer_copy_mode.set(true);
      transfer_modal_open.set(true);
    }),
    on_batch_trash: {
      let trash_v = on_trash_view.clone();
      Rc::new(move |ids: Vec<Uuid>| {
        for id in ids {
          trash_v(id);
        }
      })
    },
    on_reorder_views,
  });

  // Editor header props (1:1 with PropertyPanel + BacklinksPanel + PageWidthFrame + VersionHistory + Share + Inline AI)
  let prop_panel_slot: Rc<dyn Fn() -> AnyElement> = {
    let br_fm = bridge.clone();
    let fm_sig = bridge.front_matter_signal();
    let open_s = on_open_search_with.clone();
    Rc::new(move || {
      let br_c = br_fm.clone();
      let open_c = open_s.clone();
      property_panel::property_panel_view(
        show_properties,
        fm_sig,
        dark_mode,
        Rc::new(move |yaml: String| br_c.set_front_matter(&yaml)),
        Rc::new(move |tag: String| open_c(Some(format!("#{tag}")), false)),
      )
      .into_element()
    })
  };

  let backlinks_slot: Rc<dyn Fn() -> AnyElement> = {
    let sel = on_select_doc.clone();
    let tree_ref = tree.clone();
    Rc::new(move || {
      let sel_c = sel.clone();
      let t_c = tree_ref.clone();
      right_sidebar::backlinks_below_editor_view(
        backlinks_sig,
        dark_mode,
        Rc::new(move |doc_id: Uuid| {
          let title = t_c
            .borrow()
            .all_live_rows()
            .into_iter()
            .find(|r| !r.is_folder && r.object_id == doc_id)
            .map(|r| r.name)
            .unwrap_or_else(|| "未命名页面".to_string());
          sel_c(doc_id, title);
        }),
      )
      .into_element()
    })
  };

  let editor_header = Rc::new(EditorHeaderProps {
    workspace_name,
    folder_name: active_folder_name,
    page_title: active_title,
    page_id_str: active_doc_id_str,
    is_favorite: active_is_favorite,
    right_panel_open,
    is_cloud_signed_in,
    show_format_bar,
    show_page_title,
    show_properties,
    comments_visible: right_panel_open,
    page_width: page_width_sig,
    ai_enabled,
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
    on_share: {
      let cc = cloud_connector.clone();
      let share_fetch_gen = signal(0u64);
      Rc::new(move || {
        share_error.set(None);
        share_modal_open.set(true);
        if let Some(doc_id) = active_doc.get_untracked()
          && is_cloud_signed_in.get_untracked()
        {
          let ws_id = active_workspace_id.get_untracked();
          let conn = cc.borrow().clone();
          let epoch = share_fetch_gen.get_untracked().wrapping_add(1);
          share_fetch_gen.set(epoch);
          share_loading.set(true);
          spawn(async move {
            let res = conn.get_share(ws_id, doc_id).await;
            if share_fetch_gen.get_untracked() != epoch
              || active_doc.get_untracked() != Some(doc_id)
            {
              return;
            }
            share_loading.set(false);
            if let Ok(st) = res {
              share_enabled.set(st.shared);
              share_token.set(st.token);
            }
          });
        }
      })
    },
    on_toggle_comments: Rc::new(move || {
      right_panel_tab.set(ToolsTab::Comments);
      right_panel_open.set(!right_panel_open.get_untracked());
    }),
    on_toggle_properties: Rc::new(move || {
      show_properties.set(!show_properties.get_untracked());
    }),
    on_export_zip: {
      let exp = export_folder_zip.clone();
      Rc::new(move || {
        if let Some(doc_id) = active_doc.get_untracked() {
          exp(doc_id, ArchiveExportFormat::MarkdownZip);
        }
      })
    },
    on_export_pdf: {
      let exp_html = on_export_html.clone();
      let br = bridge.clone();
      Rc::new(move || {
        exp_html();
        br.set_status_flash("已导出可打印 HTML/PDF 版本");
      })
    },
    on_import_markdown_picker: {
      let imp = on_import_files_picker.clone();
      Rc::new(move || imp(Some(active_folder_id.get_untracked())))
    },
    on_open_version_history: on_open_version_history.clone(),
    on_inline_ai: Rc::new(move |selection_text: String| {
      inline_ai_context.set(selection_text.clone());
      inline_ai_prompt.set(String::new());
      inline_ai_allow_replace.set(!selection_text.is_empty());
      inline_ai_output.set(String::new());
      inline_ai_phase.set(InlineAiPhase::Idle);
      inline_ai_error.set(None);
      inline_ai_open.set(true);
    }),
    on_insert_image: {
      let br = bridge.clone();
      Rc::new(move || {
        let br_c = br.clone();
        app_owner.run(move || {
          spawn(async move {
            if let Ok(paths) = transfer::pick_import_files("Images", &["png", "jpg", "jpeg", "gif", "webp", "svg"], false).await
              && let Some(p) = paths.first()
            {
              let uri = format!("file:///{}", p.to_string_lossy().replace('\\', "/"));
              let alt = p.file_stem().and_then(|s| s.to_str()).unwrap_or("图片");
              br_c.insert_custom_block("image", alt, Some(&uri));
            }
          });
        });
      })
    },
    property_panel_slot: Some(prop_panel_slot),
    backlinks_panel_slot: Some(backlinks_slot),
  });

  // Right sidebar props + extras
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
  let right_extras = Rc::new(RightSidebarExtras {
    width: right_panel_width,
    active_tab: right_panel_tab,
    on_open_version_history: Some(on_open_version_history.clone()),
    ..RightSidebarExtras::default()
  });

  // Cloud auth handlers shared between Settings and World Switcher
  let do_cloud_login: Rc<dyn Fn(String, String, String)> = {
    let cs = cloud_session.clone();
    Rc::new(move |url: String, email: String, password: String| {
      let cs_run = cs.clone();
      app_owner.run(move || cs_run.login(url, email, password));
    })
  };

  let do_cloud_logout: Rc<dyn Fn()> = {
    let cs = cloud_session.clone();
    Rc::new(move || {
      settings_modal_open.set(false);
      account_menu_open.set(false);
      let cs_run = cs.clone();
      app_owner.run(move || cs_run.logout_to_local());
    })
  };

  // Settings 1120×840 Props
  let settings_legacy = Rc::new(SettingsCallbacks {
    dark_mode,
    locale_zh,
    server_url,
    cloud_status,
    export_preview,
    account_email,
    is_cloud_signed_in,
    on_toggle_theme: on_toggle_theme.clone(),
    on_toggle_locale: {
      let v = vault.clone();
      Rc::new(move || {
        let next = !locale_zh.get_untracked();
        locale_zh.set(next);
        let _ = v.set_bool_pref("localeZh", next);
      })
    },
    on_check_server: Rc::new(move |url: String| {
      let epoch = check_gen.get_untracked().wrapping_add(1);
      check_gen.set(epoch);
      cloud_status.set(format!("正在检测服务器：{url}…"));
      let url_req = url.clone();
      spawn(async move {
        let connector = CloudConnector::new(CloudConfig {
          server_url: url_req,
          access_token: None,
          email: None,
        });
        let res = connector.check_health().await;
        if check_gen.get_untracked() != epoch {
          return;
        }
        match res {
          Ok(ver) => cloud_status.set(format!("连接正常：{url}（{ver}）")),
          Err(err) => cloud_status.set(format!("离线 / 无法连接：{err}")),
        }
      });
    }),
    on_cloud_login: do_cloud_login.clone(),
    on_cloud_logout: do_cloud_logout.clone(),
    on_sync_now: {
      let cc = cloud_connector.clone();
      let cs = cloud_session.clone();
      Rc::new(move || {
        let orig = active_origin.get_untracked();
        if cloud_sync::is_cloud_origin(&orig) && is_cloud_signed_in.get_untracked() {
          let srv = server_url.get_untracked();
          let cs_run = cs.clone();
          app_owner.run(move || cs_run.enter_origin(srv, false));
          return;
        }
        let epoch = check_gen.get_untracked().wrapping_add(1);
        check_gen.set(epoch);
        let conn = cc.borrow().clone();
        let phase = conn.sync_phase_label();
        cloud_status.set("正在检查同步连接…".to_string());
        spawn(async move {
          let res = conn.check_health().await;
          if check_gen.get_untracked() != epoch {
            return;
          }
          match res {
            Ok(ver) => cloud_status.set(format!("同步就绪（{phase} · 服务器 {ver}）")),
            Err(err) => cloud_status.set(format!("同步检查失败：{err}")),
          }
        });
      })
    },
    on_export_markdown,
    on_export_html,
    on_export_bundle,
    on_import_markdown,
    on_save_snapshot,
    on_close: Rc::new(move || settings_modal_open.set(false)),
  });
  let mut settings_props_val = SettingsViewProps::from_legacy(settings_legacy);
  settings_props_val.page_width_px = page_width_px_sig;
  settings_props_val.ui_font_scale = ui_font_scale_sig;
  settings_props_val.editor_font_px = editor_font_px_sig;
  settings_props_val.editor_font_family = editor_font_family_sig;
  settings_props_val.show_format_bar = show_format_bar;
  settings_props_val.show_page_title = show_page_title;
  settings_props_val.account_name = account_name;
  settings_props_val.ai_enabled = ai_enabled;
  settings_props_val.trashed_views = trashed_views_sig;
  settings_props_val.live_views = live_views_sig;
  settings_props_val.on_export_mica_zip = Some({
    let exp = export_workspace_zip.clone();
    Rc::new(move || exp(None, ArchiveExportFormat::BackupMicaZip))
  });
  settings_props_val.on_export_markdown_zip = Some({
    let exp = export_workspace_zip.clone();
    Rc::new(move || exp(None, ArchiveExportFormat::MarkdownZip))
  });
  settings_props_val.on_import_zip = Some({
    let imp = on_import_zip_picker.clone();
    Rc::new(move |notion: bool| imp(None, notion))
  });
  settings_props_val.on_import_folder = Some({
    let imp = on_import_folder_picker.clone();
    Rc::new(move || imp(None))
  });
  settings_props_val.on_restore_view = Some(on_restore_view.clone());
  settings_props_val.on_purge_view = Some(on_purge_view);
  settings_props_val.on_purge_all_trash = Some(on_purge_all_trash);
  settings_props_val.on_refresh_trash = Some(reload_tree_from_vault.clone());
  let settings_active_tab = signal(settings_view::TAB_APPEARANCE);
  let stats_loading = signal(false);
  let stats_error: Signal<Option<String>> = signal(None);
  let stats_overview: Signal<Option<mica_core::UsageStatsOverview>> = signal(None);
  let stats_activity: Signal<Option<mica_core::UsageStatsActivity>> = signal(None);
  let stats_ws_filter: Signal<Option<String>> = signal(None);
  let stats_range = signal("30d".to_string());
  let stats_include_imports = signal(false);
  let stats_fetch_gen = signal(0u64);
  let stats_is_cloud = signal(false);

  let refresh_usage_stats: Rc<dyn Fn()> = {
    let v = vault.clone();
    let cc = cloud_connector.clone();
    Rc::new(move || {
      let orig = active_origin.get_untracked();
      let range = stats_range.get_untracked();
      let ws_filter = stats_ws_filter.get_untracked();
      let inc_imp = stats_include_imports.get_untracked();
      let tz_offset = Local::now().offset().local_minus_utc() / 60;
      let epoch = stats_fetch_gen.get_untracked().wrapping_add(1);
      stats_fetch_gen.set(epoch);
      let is_cloud = cloud_sync::is_cloud_origin(&orig) && is_cloud_signed_in.get_untracked();
      stats_is_cloud.set(is_cloud);
      stats_error.set(None);

      if !is_cloud {
        stats_loading.set(false);
        match (
          v.usage_overview(&orig),
          v.usage_activity(&orig, &range, tz_offset, ws_filter.as_deref(), inc_imp),
        ) {
          (Ok(ov), Ok(act)) => {
            stats_overview.set(Some(ov));
            stats_activity.set(Some(act));
          }
          (Err(e), _) | (_, Err(e)) => stats_error.set(Some(e)),
        }
        return;
      }

      stats_loading.set(true);
      let conn = cc.borrow().clone();
      let orig_expect = orig.clone();
      app_owner.run(move || {
        spawn(async move {
          let res = match conn.get_usage_overview().await {
            Ok(ov) => conn
              .get_usage_activity(&range, tz_offset, ws_filter.as_deref(), inc_imp)
              .await
              .map(|act| (ov, act)),
            Err(e) => Err(e),
          };
          if stats_fetch_gen.get_untracked() != epoch
            || active_origin.get_untracked() != orig_expect
          {
            return;
          }
          stats_loading.set(false);
          match res {
            Ok((ov, act)) => {
              stats_overview.set(Some(ov));
              stats_activity.set(Some(act));
            }
            Err(e) => stats_error.set(Some(e)),
          }
        });
      });
    })
  };

  let usage_stats_props = usage_stats_view::UsageStatsProps {
    dark_mode,
    is_cloud: stats_is_cloud,
    loading: stats_loading,
    error: stats_error,
    overview: stats_overview,
    activity: stats_activity,
    selected_workspace_id: stats_ws_filter,
    selected_range: stats_range,
    include_imports: stats_include_imports,
    on_refresh: refresh_usage_stats.clone(),
    on_select_workspace: {
      let r = refresh_usage_stats.clone();
      Rc::new(move |ws_id| {
        stats_ws_filter.set(ws_id);
        r();
      })
    },
    on_select_range: {
      let r = refresh_usage_stats.clone();
      Rc::new(move |range| {
        stats_range.set(range);
        r();
      })
    },
    on_toggle_imports: {
      let r = refresh_usage_stats.clone();
      Rc::new(move |inc| {
        stats_include_imports.set(inc);
        r();
      })
    },
  };

  let on_open_usage_stats: Rc<dyn Fn()> = {
    let r = refresh_usage_stats.clone();
    Rc::new(move || {
      account_menu_open.set(false);
      settings_active_tab.set(settings_view::TAB_USAGE_STATS);
      settings_modal_open.set(true);
      r();
    })
  };

  settings_props_val.on_open_about = Some(Rc::new(move || {
    settings_modal_open.set(false);
    about_modal_open.set(true);
  }));
  settings_props_val.active_tab = Some(settings_active_tab);
  settings_props_val.usage_stats = Some(usage_stats_props);
  let settings_props = Rc::new(settings_props_val);

  // Desktop auto-updater state + callbacks (Web reports server-managed updates)
  let available_update: Signal<Option<updater::UpdateInfo>> = signal(None);
  let update_busy = signal(false);
  #[cfg(not(target_arch = "wasm32"))]
  let update_check_gen = signal(0u64);

  let run_check_update: Rc<dyn Fn(bool)> = Rc::new(move |open_dialog: bool| {
    if open_dialog {
      about_modal_open.set(true);
    }
    #[cfg(target_arch = "wasm32")]
    {
      update_status.set(format!("Web 版随服务器自动更新（v{APP_VERSION}）"));
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
      if update_busy.get_untracked() {
        return;
      }
      let epoch = update_check_gen.get_untracked().wrapping_add(1);
      update_check_gen.set(epoch);
      update_busy.set(true);
      update_status.set("正在检查更新…".to_string());
      app_owner.run(move || {
        spawn(async move {
          let res = net::run_blocking(|| updater::check_for_update_blocking(APP_VERSION)).await;
          if update_check_gen.get_untracked() != epoch {
            return;
          }
          update_busy.set(false);
          match res {
            Ok(Some(info)) => {
              let ver = info.version.clone();
              available_update.set(Some(info));
              update_status.set(format!("发现新版本 v{ver}（当前 v{APP_VERSION}）"));
            }
            Ok(None) => {
              available_update.set(None);
              update_status.set(format!("已是最新版本（v{APP_VERSION}）"));
            }
            Err(err) => update_status.set(format!("检查更新失败：{err}")),
          }
        });
      });
    }
  });

  let on_install_update: Rc<dyn Fn()> = {
    let br_upd = bridge.clone();
    Rc::new(move || {
      #[cfg(target_arch = "wasm32")]
      {
        let _ = &br_upd;
        update_status.set(format!("Web 版随服务器自动更新（v{APP_VERSION}）"));
      }
      #[cfg(not(target_arch = "wasm32"))]
      {
        if update_busy.get_untracked() {
          return;
        }
        let Some(info) = available_update.get_untracked() else {
          return;
        };
        if !updater::update_supported() {
          update_status.set(updater::UpdateError::UnsupportedPlatform.to_string());
          return;
        }
        br_upd.flush_pending_persist();
        update_busy.set(true);
        let ver = info.version.clone();
        update_status.set(format!("正在下载并校验 v{ver} 安装包…"));
        let pid = std::process::id();
        app_owner.run(move || {
          spawn(async move {
            let res = net::run_blocking(move || {
              updater::download_and_launch_installer_blocking(&info, pid, |_| {})
            })
            .await;
            match res {
              Ok(_) => {
                update_status.set(format!("正在启动 v{ver} 安装程序并重启…"));
                std::process::exit(0);
              }
              Err(err) => {
                update_busy.set(false);
                update_status.set(err.to_string());
              }
            }
          });
        });
      }
    })
  };

  // Gentle non-intrusive startup update check on desktop (5s delay).
  #[cfg(not(target_arch = "wasm32"))]
  {
    app_owner.run(move || {
      spawn(async move {
        timer(std::time::Duration::from_secs(5)).await;
        if update_busy.get_untracked() || available_update.get_untracked().is_some() {
          return;
        }
        if let Ok(Some(info)) =
          net::run_blocking(|| updater::check_for_update_blocking(APP_VERSION)).await
        {
          let ver = info.version.clone();
          available_update.set(Some(info));
          update_status.set(format!("发现新版本 v{ver}"));
        }
      });
    });
  }

  // App Menu, Account Menu, World Switcher, Version History, Share, AI & Workspace Settings states
  let app_menu_cb = AppMenuCallbacks {
    on_new_page: on_new_page.clone(),
    on_new_tab: {
      let open_s = on_open_search_with.clone();
      Rc::new(move || open_s(None, true))
    },
    on_close_tab: {
      let close_t = on_close_tab_at.clone();
      Rc::new(move || close_t(active_tab_idx.get_untracked()))
    },
    can_close_tab,
    on_open_settings: on_open_settings.clone(),
    sidebar_open,
    on_toggle_sidebar: Rc::new(move || sidebar_open.set(!sidebar_open.get_untracked())),
    right_panel_open,
    on_toggle_right_panel: Rc::new(move || right_panel_open.set(!right_panel_open.get_untracked())),
    can_go_back: signal(false),
    can_go_forward: signal(false),
    on_go_back: on_nav_back.clone(),
    on_go_forward: on_nav_forward.clone(),
    on_open_about: Rc::new(move || about_modal_open.set(true)),
    on_check_update: {
      let chk = run_check_update.clone();
      Rc::new(move || chk(true))
    },
  };
  {
    let back_sig = app_menu_cb.can_go_back;
    let fwd_sig = app_menu_cb.can_go_forward;
    effect(move || {
      back_sig.set(can_go_back.get());
      fwd_sig.set(can_go_forward.get());
    });
  }

  let account_menu_state = AccountMenuState {
    allow_local,
    is_local: signal(allow_local),
    is_signed_in: is_cloud_signed_in,
    show_cloud_signin: signal(true),
    display_name: account_name,
    email: account_email,
    app_version: APP_VERSION,
    update_status,
    on_open_settings: on_open_settings.clone(),
    on_open_usage_stats: Some(on_open_usage_stats.clone()),
    on_switch_world: Rc::new(move || {
      account_menu_open.set(false);
      world_switcher_open.set(true);
    }),
    on_check_update: {
      let chk = run_check_update.clone();
      Rc::new(move || chk(true))
    },
    on_sign_out: do_cloud_logout.clone(),
    on_sign_in: Rc::new(move || {
      account_menu_open.set(false);
      world_switcher_open.set(true);
    }),
  };
  {
    let is_loc = account_menu_state.is_local;
    effect(move || {
      is_loc.set(allow_local && !is_cloud_signed_in.get());
    });
  }

  let on_probe_server: Rc<dyn Fn(String)> = Rc::new(move |origin: String| {
    let epoch = probe_gen.get_untracked().wrapping_add(1);
    probe_gen.set(epoch);
    world_health_status.set("checking".to_string());
    let origin_req = origin.clone();
    spawn(async move {
      let conn = CloudConnector::new(CloudConfig {
        server_url: origin_req,
        access_token: None,
        email: None,
      });
      let ok = conn.check_health().await.is_ok();
      if probe_gen.get_untracked() != epoch {
        return;
      }
      world_health_status.set(if ok { "reachable" } else { "unreachable" }.to_string());
    });
  });

  let world_state = WorldSwitcherState {
    open: world_switcher_open,
    can_close: world_can_close,
    allow_local,
    dark_mode,
    app_version: APP_VERSION,
    active_origin: world_active_origin,
    servers: known_servers_sig,
    signed_in_origins: signed_in_origins_sig,
    selected_cloud_origin,
    health_status: world_health_status,
    busy: world_busy,
    error_msg: world_error,
    info_msg: world_info,
    on_close: Rc::new(move || world_switcher_open.set(false)),
    on_enter_local: {
      let logout = do_cloud_logout.clone();
      Rc::new(move || {
        logout();
        world_switcher_open.set(false);
      })
    },
    on_select_server: Rc::new(move |origin: String| {
      selected_cloud_origin.set(origin.clone());
      server_url.set(origin);
    }),
    on_probe_server: on_probe_server.clone(),
    on_add_server: {
      let v = vault.clone();
      Rc::new(move |url: String| {
        let mut list = known_servers_sig.get_untracked();
        if list.iter().any(|s| s == &url) {
          return Err("这台服务器已经在列表里了".to_string());
        }
        list.push(url);
        let _ = v.set_known_servers(&list);
        known_servers_sig.set(list);
        Ok(())
      })
    },
    on_remove_server: {
      let v = vault.clone();
      Rc::new(move |url: String| {
        let mut list = known_servers_sig.get_untracked();
        list.retain(|s| s != &url);
        let _ = v.set_known_servers(&list);
        let _ = v.clear_server_credential(&url);
        known_servers_sig.set(list);
        let mut creds = signed_in_origins_sig.get_untracked();
        creds.retain(|(o, _)| o != &url);
        signed_in_origins_sig.set(creds);
      })
    },
    on_enter_signed_in: {
      let cs = cloud_session.clone();
      Rc::new(move |origin: String| {
        let cs_run = cs.clone();
        app_owner.run(move || cs_run.enter_origin(origin, true));
      })
    },
    on_login: do_cloud_login,
    on_register: {
      let cs = cloud_session.clone();
      Rc::new(move |url: String, email: String, display_name: String, password: String| {
        let cs_run = cs.clone();
        app_owner.run(move || cs_run.register(url, email, display_name, password));
      })
    },
    on_forgot_password: Rc::new(move |url: String, email: String| {
      let epoch = auth_gen.get_untracked().wrapping_add(1);
      auth_gen.set(epoch);
      world_busy.set(true);
      world_error.set(String::new());
      world_info.set(String::new());
      app_owner.run(move || {
        spawn(async move {
          let conn = CloudConnector::new(CloudConfig {
            server_url: url,
            access_token: None,
            email: None,
          });
          let outcome = conn.forgot_password(&email).await;
          if auth_gen.get_untracked() != epoch {
            return;
          }
          match outcome {
            Ok(()) => world_info.set("重置密码邮件已发送，请检查收件箱。".to_string()),
            Err(e) => world_error.set(e),
          }
          world_busy.set(false);
        });
      });
    }),
  };

  let version_history_state = VersionHistoryState {
    open: version_modal_open,
    items: version_items_sig,
    selected_id: version_selected_id,
    current_md: version_current_md,
    prev_md: version_prev_md,
    loading: signal(false),
    preview_loading: signal(false),
    error: signal(None),
    on_select: load_version_diff_preview,
    on_create_checkpoint: {
      let v = vault.clone();
      let ref_snaps = refresh_snapshots.clone();
      let br = bridge.clone();
      Rc::new(move |name: String| {
        br.flush_pending_persist();
        if let Some(doc_id) = active_doc.get_untracked() {
          let orig = active_origin.get_untracked();
          let label = if name.trim().is_empty() { "检查点" } else { name.trim() };
          if v.create_version_snapshot(&orig, doc_id, Some(label)).is_ok() {
            ref_snaps(doc_id);
            br.set_status_flash("已保存检查点");
          }
        }
      })
    },
    on_restore: on_restore_version_by_id,
    on_close: Rc::new(move || version_modal_open.set(false)),
  };

  let share_state = ShareDialogState {
    open: share_modal_open,
    loading: share_loading,
    busy: share_busy,
    shared: share_enabled,
    token: share_token,
    origin: server_url,
    error: share_error,
    on_toggle_public: {
      let cc = cloud_connector.clone();
      let toggle_gen = signal(0u64);
      Rc::new(move |enable: bool| {
        let Some(doc_id) = active_doc.get_untracked() else {
          return;
        };
        let ws = active_workspace_id.get_untracked();
        let epoch = toggle_gen.get_untracked().wrapping_add(1);
        toggle_gen.set(epoch);
        share_busy.set(true);
        let conn = cc.borrow().clone();
        spawn(async move {
          let res = if enable {
            conn.create_share(ws, doc_id).await
          } else {
            conn.delete_share(ws, doc_id).await.map(|_| net::ShareState {
              shared: false,
              token: None,
              url: None,
            })
          };
          if toggle_gen.get_untracked() != epoch || active_doc.get_untracked() != Some(doc_id) {
            return;
          }
          match res {
            Ok(st) => {
              share_enabled.set(st.shared);
              share_token.set(st.token);
              share_error.set(None);
            }
            Err(e) => share_error.set(Some(e)),
          }
          share_busy.set(false);
        });
      })
    },
    on_close: Rc::new(move || share_modal_open.set(false)),
  };

  let global_ai_state = GlobalAiState {
    open: global_ai_open,
    can_edit: signal(true),
    has_workspace: signal(true),
    has_current_page: signal(true),
    edit_mode: signal(false),
    prompt: global_ai_prompt,
    target: global_ai_target,
    output: global_ai_output,
    edit_diffs: signal(Vec::new()),
    streaming: global_ai_streaming,
    done: global_ai_done,
    applying: signal(false),
    error: global_ai_error,
    on_generate: {
      let cc = cloud_connector.clone();
      let ai_gen = signal(0u64);
      Rc::new(move |prompt: String, _target: AiTarget| {
        let epoch = ai_gen.get_untracked().wrapping_add(1);
        ai_gen.set(epoch);
        global_ai_streaming.set(true);
        global_ai_error.set(None);
        let conn = cc.borrow().clone();
        spawn(async move {
          let res = conn
            .stream_ai_prompt("你是 Mica 知识库写作助手，请直接输出干净的 Markdown。", &prompt)
            .await;
          if ai_gen.get_untracked() != epoch {
            return;
          }
          match res {
            Ok(out) => {
              global_ai_output.set(out);
              global_ai_done.set(true);
            }
            Err(e) => global_ai_error.set(Some(e)),
          }
          global_ai_streaming.set(false);
        });
      })
    },
    on_apply: {
      let create_p = create_page_with_content.clone();
      let br = bridge.clone();
      Rc::new(move |md: String, target: AiTarget| {
        match target {
          AiTarget::NewPage | AiTarget::NewWorkspace => create_p("AI 生成页面".to_string(), md),
          AiTarget::CurrentPage => {
            let cur = br.export_markdown();
            br.load_markdown(&format!("{cur}\n\n{md}"));
          }
        }
        global_ai_open.set(false);
      })
    },
    on_close: Rc::new(move || global_ai_open.set(false)),
  };

  let inline_ai_state = InlineAiState {
    open: inline_ai_open,
    prompt: inline_ai_prompt,
    context_text: inline_ai_context,
    allow_replace: inline_ai_allow_replace,
    output: inline_ai_output,
    phase: inline_ai_phase,
    error: inline_ai_error,
    on_generate: {
      let cc = cloud_connector.clone();
      let ai_gen = signal(0u64);
      Rc::new(move |prompt: String, ctx: String| {
        let epoch = ai_gen.get_untracked().wrapping_add(1);
        ai_gen.set(epoch);
        inline_ai_phase.set(InlineAiPhase::Running);
        inline_ai_error.set(None);
        let full = if ctx.is_empty() { prompt } else { format!("{prompt}\n\n原文：\n{ctx}") };
        let conn = cc.borrow().clone();
        spawn(async move {
          let res = conn.stream_ai_prompt(ai_dialog::K_AI_REWRITE_SYSTEM, &full).await;
          if ai_gen.get_untracked() != epoch {
            return;
          }
          match res {
            Ok(out) => {
              inline_ai_output.set(out);
              inline_ai_phase.set(InlineAiPhase::Succeeded);
            }
            Err(e) => {
              inline_ai_error.set(Some(e));
              inline_ai_phase.set(InlineAiPhase::Failed);
            }
          }
        });
      })
    },
    on_insert_below: {
      let br = bridge.clone();
      Rc::new(move |md: String| {
        br.insert_custom_block("paragraph", md.trim(), None);
        inline_ai_open.set(false);
      })
    },
    on_replace_selection: {
      let br = bridge.clone();
      Rc::new(move |md: String| {
        br.insert_custom_block("paragraph", md.trim(), None);
        inline_ai_open.set(false);
      })
    },
    on_close: Rc::new(move || inline_ai_open.set(false)),
  };

  let ws_settings_id_sig = signal(ws_id.to_string());
  let ws_settings_gen = signal(0u64);
  let ws_storage_usage = signal(None);
  let ws_members = signal(Vec::new());
  let ws_pending_invitations = signal(Vec::new());
  let ws_invite_status: Signal<Option<(String, bool)>> = signal(None);
  {
    let cs = cloud_session.clone();
    effect(move || {
      let cur = active_workspace_id.get();
      ws_settings_id_sig.set(cur.to_string());
      if workspace_settings_open.get() {
        ws_invite_status.set(None);
        cs.load_workspace_settings(
          ws_settings_gen,
          ws_storage_usage,
          ws_members,
          ws_pending_invitations,
        );
      }
    });
  }
  let ws_settings_state = WorkspaceSettingsState {
    open: workspace_settings_open,
    workspace_id: ws_settings_id_sig,
    workspace_name,
    role: signal("owner".to_string()),
    is_cloud: is_cloud_signed_in,
    can_manage: signal(true),
    storage_usage: ws_storage_usage,
    members: ws_members,
    pending_invitations: ws_pending_invitations,
    invite_status: ws_invite_status,
    on_rename: {
      let v = vault.clone();
      let cs = cloud_session.clone();
      Rc::new(move |next_name: String| {
        let cur_ws = active_workspace_id.get_untracked();
        let orig = active_origin.get_untracked();
        let _ = v.save_workspace_name(&orig, cur_ws, &next_name);
        let name_req = next_name.clone();
        cs.spawn_cloud_op("云端重命名工作区", move |c| {
          let name_req = name_req.clone();
          async move { c.update_workspace(cur_ws, &name_req).await }
        });
        workspace_name.set(next_name.clone());
        let mut all = workspaces.get_untracked();
        for w in &mut all {
          if w.id == cur_ws {
            w.name = next_name.clone();
          }
        }
        workspaces.set(all);
      })
    },
    on_invite: {
      let cs = cloud_session.clone();
      Rc::new(move |email: String, role: String| {
        let cur_ws = active_workspace_id.get_untracked();
        let orig = active_origin.get_untracked();
        if !cloud_sync::is_cloud_origin(&orig) || !is_cloud_signed_in.get_untracked() {
          return;
        }
        let mut conn = cs.cloud_connector.borrow().clone();
        let cs_done = cs.clone();
        spawn(async move {
          let res = cloud_sync::with_auto_refresh(&mut conn, move |c| {
            let (email, role) = (email.clone(), role.clone());
            async move { c.invite_member(cur_ws, &email, &role).await }
          })
          .await;
          if active_origin.get_untracked() != orig || active_workspace_id.get_untracked() != cur_ws {
            return;
          }
          match res {
            Ok(_) => {
              ws_invite_status.set(Some(("邀请已发送".to_string(), false)));
              cs_done.load_workspace_settings(
                ws_settings_gen,
                ws_storage_usage,
                ws_members,
                ws_pending_invitations,
              );
            }
            Err(e) => ws_invite_status.set(Some((format!("邀请失败：{e}"), true))),
          }
        });
      })
    },
    on_revoke_invitation: {
      let cs = cloud_session.clone();
      Rc::new(move |inv_id: String| {
        let cur_ws = active_workspace_id.get_untracked();
        let inv_req = inv_id.clone();
        cs.spawn_cloud_op("撤回邀请", move |c| {
          let inv_req = inv_req.clone();
          async move { c.revoke_invitation(cur_ws, &inv_req).await }
        });
        ws_pending_invitations.update(|list| list.retain(|i| i.id != inv_id));
      })
    },
    on_change_member_role: {
      let cs = cloud_session.clone();
      Rc::new(move |uid: String, role: String| {
        let cur_ws = active_workspace_id.get_untracked();
        let (uid_req, role_req) = (uid.clone(), role.clone());
        cs.spawn_cloud_op("修改成员角色", move |c| {
          let (uid_req, role_req) = (uid_req.clone(), role_req.clone());
          async move { c.update_member_role(cur_ws, &uid_req, &role_req).await }
        });
        ws_members.update(|list| {
          for m in list.iter_mut() {
            if m.user_id == uid {
              m.role = role.clone();
            }
          }
        });
      })
    },
    on_remove_member: {
      let cs = cloud_session.clone();
      Rc::new(move |uid: String| {
        let cur_ws = active_workspace_id.get_untracked();
        let uid_req = uid.clone();
        cs.spawn_cloud_op("移除成员", move |c| {
          let uid_req = uid_req.clone();
          async move { c.remove_member(cur_ws, &uid_req).await }
        });
        ws_members.update(|list| list.retain(|m| m.user_id != uid));
      })
    },
    on_delete_workspace: {
      let v = vault.clone();
      let sw = on_switch_workspace.clone();
      let cs = cloud_session.clone();
      Rc::new(move || {
        let cur = active_workspace_id.get_untracked();
        let mut all = workspaces.get_untracked();
        if all.len() <= 1 {
          return;
        }
        let orig = active_origin.get_untracked();
        let _ = v.delete_workspace(&orig, cur);
        cs.spawn_cloud_op("云端删除工作区", move |c| async move { c.delete_workspace(cur).await });
        all.retain(|w| w.id != cur);
        let next = all[0].clone();
        workspaces.set(all);
        workspace_settings_open.set(false);
        sw(next);
      })
    },
    on_close: Rc::new(move || workspace_settings_open.set(false)),
  };

  // Workspace switcher 320px popover items & callbacks
  let ws_switcher_items: Signal<Vec<WorkspaceSwitcherItem>> = signal(Vec::new());
  {
    let tree_ref = tree.clone();
    effect(move || {
      let _rev = tree_rev.get();
      let list = workspaces.get();
      let cur_id = active_workspace_id.get();
      let cur_pages = tree_ref.borrow().count_live_pages();
      ws_switcher_items.set(
        list
          .into_iter()
          .map(|w| WorkspaceSwitcherItem {
            id: w.id,
            name: w.name,
            is_local: !cloud_sync::is_cloud_origin(&w.origin),
            page_count: if w.id == cur_id { cur_pages } else { w.page_count.unwrap_or(0) },
          })
          .collect(),
      );
    });
  }
  let ws_switcher_cb = Rc::new(WorkspaceSwitcherCallbacks {
    on_select: {
      let sw = on_switch_workspace.clone();
      Rc::new(move |id: Uuid| {
        if let Some(entry) = workspaces.get_untracked().into_iter().find(|w| w.id == id) {
          sw(entry);
        }
      })
    },
    on_create: {
      let sw = on_switch_workspace.clone();
      let v = vault.clone();
      let cs = cloud_session.clone();
      Rc::new(move |name: String| {
        let cur_orig = active_origin.get_untracked();
        if cloud_sync::is_cloud_origin(&cur_orig) && is_cloud_signed_in.get_untracked() {
          workspace_menu_open.set(false);
          cs.create_cloud_workspace(cur_orig, name);
          return;
        }
        let id = Uuid::new_v4();
        let origin = DEFAULT_ORIGIN.to_string();
        let _ = v.save_workspace_name(&origin, id, &name);
        let entry = WorkspaceEntry { id, name, origin, page_count: None };
        let mut all = workspaces.get_untracked();
        all.push(entry.clone());
        workspaces.set(all);
        sw(entry);
      })
    },
    on_rename: {
      let v = vault.clone();
      let cs = cloud_session.clone();
      Rc::new(move |id: Uuid, next_name: String| {
        let mut all = workspaces.get_untracked();
        for w in &mut all {
          if w.id == id {
            let _ = v.save_workspace_name(&w.origin, id, &next_name);
            w.name = next_name.clone();
          }
        }
        workspaces.set(all);
        let name_req = next_name.clone();
        cs.spawn_cloud_op("云端重命名工作区", move |c| {
          let name_req = name_req.clone();
          async move { c.update_workspace(id, &name_req).await }
        });
        if active_workspace_id.get_untracked() == id {
          workspace_name.set(next_name);
        }
      })
    },
    on_export_backup: {
      let exp = export_workspace_zip.clone();
      Rc::new(move |ws_id: Uuid| exp(Some(ws_id), ArchiveExportFormat::BackupMicaZip))
    },
    on_export_markdown: {
      let exp = export_workspace_zip.clone();
      Rc::new(move |ws_id: Uuid| exp(Some(ws_id), ArchiveExportFormat::MarkdownZip))
    },
    on_import_files: {
      let imp = on_import_files_picker.clone();
      Rc::new(move |_| imp(None))
    },
    on_import_folder: {
      let imp = on_import_folder_picker.clone();
      Rc::new(move |_| imp(None))
    },
    on_delete: {
      let sw = on_switch_workspace.clone();
      let v = vault.clone();
      let cs = cloud_session.clone();
      Rc::new(move |id: Uuid| {
        let mut all = workspaces.get_untracked();
        if all.len() <= 1 {
          return;
        }
        if let Some(target) = all.iter().find(|w| w.id == id) {
          let _ = v.delete_workspace(&target.origin, id);
        }
        cs.spawn_cloud_op("云端删除工作区", move |c| async move { c.delete_workspace(id).await });
        all.retain(|w| w.id != id);
        let next = all[0].clone();
        workspaces.set(all);
        if active_workspace_id.get_untracked() == id {
          sw(next);
        }
      })
    },
    on_import_workspace_zip: {
      let imp = on_import_zip_picker.clone();
      Rc::new(move || imp(None, false))
    },
    on_import_workspace_notion: {
      let imp = on_import_zip_picker.clone();
      Rc::new(move || imp(None, true))
    },
    on_close: Rc::new(move || workspace_menu_open.set(false)),
  });

  // Clones for UI tree
  let sel_for_tabs = on_select_doc.clone();
  let close_tab_ui = on_close_tab_at.clone();
  let new_page_tab = on_new_page.clone();
  let open_search_tab = on_open_search_with.clone();
  let tree_search = tree.clone();
  let docs_search = docs_store.clone();
  let tree_graph = tree.clone();
  let br_embed = bridge.clone();
  let v_transfer = vault.clone();
  let reload_transfer = reload_tree_from_vault.clone();
  let narrow_tab_count = signal(open_tabs.get_untracked().len().max(1));
  let narrow_tab_entries: Signal<Vec<responsive_shell::NarrowTabEntry>> = signal(Vec::new());
  effect(move || {
    let tabs = open_tabs.get();
    let active_idx = active_tab_idx.get();
    narrow_tab_count.set(tabs.len().max(1));
    narrow_tab_entries.set(
      tabs
        .into_iter()
        .enumerate()
        .map(|(idx, t)| responsive_shell::NarrowTabEntry {
          idx,
          doc_id: t.doc_id,
          title: t.title,
          active: idx == active_idx,
        })
        .collect(),
    );
  });

  cloud_session.restore_startup_session();
  if !allow_local {
    on_probe_server(selected_cloud_origin.get_untracked());
  }

  view()
    .flex_1()
    .relative()
    .bg(move || palette(dark_mode.get()).surface_base)
    .on_layout(move |b: &Bounds<Points>| {
      let w = b.size.width.0;
      if w > 0.0 && (viewport_width.get_untracked() - w).abs() > 2.0 {
        spawn(async move {
          if (viewport_width.get_untracked() - w).abs() > 2.0 {
            viewport_width.set(w);
          }
        });
      }
    })
    .child(app_chrome::render_top_bar(TopBarProps {
      dark_mode,
      is_narrow,
      sidebar_open,
      sidebar_width,
      app_menu_open,
      workspace_menu_open,
      account_menu_open,
      new_tab_menu_open,
      active_title,
      narrow_tab_count,
      narrow_tabs_open,
      open_tabs,
      active_tab_idx,
      active_mode,
      can_go_back,
      can_go_forward,
      on_nav_back,
      on_nav_forward,
      on_select_tab: sel_for_tabs,
      on_close_tab: close_tab_ui,
    }))
    .child(
      // 2. Main workspace body: Resizable Left Sidebar + Editor/Board + Resizable Right Sidebar
      //    On narrow viewports (< 768px), sidebars float as overlay drawers so the editor keeps full width.
      view()
        .flex_1()
        .w_full()
        .min_h(px(0.0))
        .flex_row()
        .child(when(move || !is_narrow.get() && sidebar_open.get(), {
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
            .relative()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .h_full()
            .overflow_hidden()
            .bg(move || palette(dark_mode.get()).surface_base)
            .child(when(move || active_mode.get() == MODE_DOC, {
              let br = bridge.clone();
              let hdr = editor_header.clone();
              move || editor_view::editor_view(br.clone(), hdr.clone(), dark_mode)
            }))
            .child(when(move || active_mode.get() == MODE_DOC && doc_loading.get(), move || {
              app_chrome::render_doc_loading_skeleton(active_title, dark_mode)
            }))
            .child(when(move || active_mode.get() == MODE_BOARD, {
              let bs = board_session.clone();
              let pb = on_persist_board.clone();
              move || {
                board_view::board_view_with_rev_and_close(
                  bs.clone(),
                  board_rev,
                  dark_mode,
                  pb.clone(),
                  Some(Rc::new(move || active_mode.set(MODE_DOC))),
                )
              }
            })),
        )
        .child(when(move || !is_narrow.get() && right_panel_open.get(), {
          let rp = right_props.clone();
          let re = right_extras.clone();
          move || right_sidebar::right_sidebar_view_with_extras(rp.clone(), re.clone())
        })),
    )
    // 2b. Narrow-viewport (< 768px) overlay drawers & compact tab dropdown
    .child(when(move || is_narrow.get() && sidebar_open.get(), {
      let t = tree.clone();
      let cb = sidebar_cb.clone();
      move || {
        responsive_shell::mobile_drawer_overlay(
          "mobile-left-drawer",
          false,
          sidebar_open,
          dark_mode,
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
          ),
        )
      }
    }))
    .child(when(move || is_narrow.get() && right_panel_open.get(), {
      let rp = right_props.clone();
      let re = right_extras.clone();
      move || {
        responsive_shell::mobile_drawer_overlay(
          "mobile-right-drawer",
          true,
          right_panel_open,
          dark_mode,
          right_sidebar::right_sidebar_view_with_extras(rp.clone(), re.clone()),
        )
      }
    }))
    .child(responsive_shell::narrow_tab_popover_view(
      narrow_tabs_open,
      narrow_tab_entries,
      dark_mode,
      {
        let sel = on_select_doc.clone();
        Rc::new(move |idx: usize, doc_id: Uuid, title: String| {
          active_tab_idx.set(idx);
          sel(doc_id, title);
        })
      },
      on_close_tab_at.clone(),
      on_new_page.clone(),
    ))
    // 3. New-Tab `+` 2-item popup menu
    .child(when(move || new_tab_menu_open.get(), {
      let np = new_page_tab.clone();
      let os = open_search_tab.clone();
      move || {
        app_chrome::render_new_tab_menu_popover(
          new_tab_menu_open,
          dark_mode,
          np.clone(),
          os.clone(),
        )
      }
    }))
    // 4. Top-left Application Menu & About modal
    .child(app_menu::app_menu_popover(app_menu_open, dark_mode, app_menu_cb))
    .child(app_menu::about_modal_view(app_menu::AboutModalProps {
      open: about_modal_open,
      dark_mode,
      app_version: APP_VERSION,
      update_status,
      available_update,
      update_busy,
      on_check_update: {
        let chk = run_check_update.clone();
        Rc::new(move || chk(false))
      },
      on_install_update,
    }))
    // 5. 320px Workspace Switcher Popover & Stacked-Card Account Menu Popover
    .child(sidebar::workspace_switcher_popover(
      workspace_menu_open,
      ws_switcher_items,
      active_workspace_id,
      dark_mode,
      ws_switcher_cb,
    ))
    .child(app_menu::account_menu_popover(account_menu_open, dark_mode, account_menu_state))
    // 6. Full-Window Split World Switcher (`切换世界…`)
    .child(world_switcher::world_switcher_overlay(world_state))
    // 7. Workspace Settings, Knowledge Graph (900×640), Version History Diff (980×640), Share (440px)
    .child(workspace_views::workspace_settings_modal(ws_settings_state, dark_mode))
    .child(graph_dialog::graph_dialog_modal(
      graph_modal_open,
      graph_data,
      active_doc,
      dark_mode,
      {
        let sel = on_select_doc.clone();
        Rc::new(move |doc_id: Uuid| {
          let title = tree_graph
            .borrow()
            .all_live_rows()
            .into_iter()
            .find(|r| !r.is_folder && r.object_id == doc_id)
            .map(|r| r.name)
            .unwrap_or_else(|| "未命名页面".to_string());
          graph_modal_open.set(false);
          sel(doc_id, title);
        })
      },
      Rc::new(move || graph_modal_open.set(false)),
    ))
    .child(version_history::version_history_dialog(version_history_state, dark_mode))
    .child(share_dialog::share_dialog_modal(share_state, dark_mode))
    // 8. Global & Inline AI Dialogs
    .child(ai_dialog::global_ai_dialog(global_ai_state, dark_mode))
    .child(ai_dialog::inline_ai_dialog(inline_ai_state, dark_mode))
    // 9. Cross-Workspace Transfer Dialog (440px)
    .child(when(move || transfer_modal_open.get(), {
      let vt = v_transfer.clone();
      let rel = reload_transfer.clone();
      move || {
        let state = app_transfer_io::build_transfer_dialog_state(CrossWorkspaceTransferCtx {
          vault: vt.clone(),
          reload_tree: rel.clone(),
          active_workspace_id,
          active_origin,
          workspaces,
          transfer_source_ids,
          transfer_copy_mode,
          transfer_modal_open,
        });
        transfer_dialog::transfer_dialog_modal(state, dark_mode)
      }
    }))
    // 9b. Root-mounted Sidebar Rename & Emoji Picker modals (centered over full window, never clipped by 280px sidebar)
    .child(sidebar::rename_dialog_modal(
      sidebar_rename_open,
      sidebar_rename_initial,
      "重命名",
      dark_mode,
      {
        let on_ren = on_rename_view.clone();
        Rc::new(move |new_name: String| {
          if let Some(vid) = sidebar_rename_view.get_untracked() {
            on_ren(vid, new_name);
          }
          sidebar_rename_open.set(false);
          sidebar_rename_view.set(None);
        })
      },
      Rc::new(move || {
        sidebar_rename_open.set(false);
        sidebar_rename_view.set(None);
      }),
    ))
    .child(emoji_picker::emoji_picker_modal(
      sidebar_emoji_open,
      sidebar_emoji_current,
      dark_mode,
      {
        let on_ic = on_set_icon;
        Rc::new(move |picked: Option<String>| {
          if let Some(vid) = sidebar_emoji_view.get_untracked() {
            on_ic(vid, picked);
          }
          sidebar_emoji_open.set(false);
          sidebar_emoji_view.set(None);
        })
      },
      Rc::new(move || {
        sidebar_emoji_open.set(false);
        sidebar_emoji_view.set(None);
      }),
    ))
    // 10. Search Modal (600×440) & Settings Modal (1120×840)
    .child(when(move || search_modal_open.get(), {
      let ts = tree_search.clone();
      let ds = docs_search.clone();
      let od = on_select_doc.clone();
      let od_new = on_open_in_new_tab.clone();
      let sw = on_switch_workspace.clone();
      move || {
        let cur_ws = active_workspace_id.get_untracked();
        let ws_chips: Vec<SearchWorkspaceChip> = workspaces
          .get_untracked()
          .into_iter()
          .map(|w| SearchWorkspaceChip {
            id: w.id,
            name: w.name,
          })
          .collect();
        let recents: Vec<SearchRecentItem> = ts
          .borrow()
          .all_live_rows()
          .into_iter()
          .filter(|r| !r.is_folder)
          .take(6)
          .map(|r| SearchRecentItem {
            view_id: r.view_id,
            document_id: r.object_id,
            title: r.name,
            icon: r.icon,
            relative_time: "最近编辑".to_string(),
          })
          .collect();
        let open_in_new = search_open_in_new_tab.get_untracked();
        let on_open_primary = if open_in_new { od_new.clone() } else { od.clone() };
        let sw_c = sw.clone();
        search_view::search_modal_view_with_props(Rc::new(SearchViewProps {
          tree: ts.clone(),
          docs: ds.clone(),
          dark_mode,
          current_workspace_id: Some(cur_ws),
          current_workspace_name: Some(workspace_name.get_untracked()),
          workspaces: ws_chips,
          recents,
          initial_query: search_initial_query.get_untracked(),
          on_open_doc: on_open_primary,
          on_open_doc_new_tab: Some(od_new.clone()),
          on_reveal_folder: Some({
            let t_rev = ts.clone();
            Rc::new(move |fid: Uuid| {
              t_rev.borrow_mut().set_expanded(fid, true);
              tree_rev.set(tree_rev.get_untracked() + 1);
            })
          }),
          on_switch_workspace: Some(Rc::new(move |ws_id: Uuid| {
            if let Some(entry) = workspaces.get_untracked().into_iter().find(|w| w.id == ws_id) {
              sw_c(entry);
            }
          })),
          on_close: Rc::new(move || search_modal_open.set(false)),
        }))
      }
    }))
    .child(when(move || settings_modal_open.get(), {
      let sp = settings_props.clone();
      move || settings_view::settings_modal_view_with_props(sp.clone())
    }))
    // 11. Embedded Document Board Fullscreen Modal (mounted below 34px titlebar so
    // window min/max/close buttons never overlap; continuously auto-saves every edit
    // back into the document block via 3-way semantic merge).
    .child(when(move || embedded_board_edit.get().is_some(), {
      let br_m = br_embed.clone();
      move || app_chrome::render_embedded_board_modal(embedded_board_edit, br_m.clone(), dark_mode)
    }))
    // 12. Global dark-pill hover tooltip overlay (never clipped by overflow_hidden)
    .child(tooltip::tooltip_overlay(tooltip_host, dark_mode))
}

fn resolve_parent_folder_name(tree: &PageTreeModel, doc_id: Uuid) -> Option<String> {
  let rows = tree.all_live_rows();
  let page = rows.iter().find(|r| !r.is_folder && r.object_id == doc_id)?;
  let mut ancestors = Vec::new();
  let mut cursor = page.parent_view_id;
  let mut seen = std::collections::HashSet::new();
  while let Some(pid) = cursor {
    if !seen.insert(pid) {
      break;
    }
    let Some(folder) = tree.get(pid) else {
      break;
    };
    if !folder.name.trim().is_empty() {
      ancestors.push(folder.name.trim().to_string());
    }
    cursor = folder.parent_view_id;
  }
  if ancestors.is_empty() {
    None
  } else {
    ancestors.reverse();
    Some(ancestors.join(" > "))
  }
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

- 原生富文本块编辑：跨块鼠标拖选、多块框选、回车分块、行首退格合并、`Ctrl+A` 全选与原位中文 IME
- 真实多行代码编辑器、表格、RaTeX 公式、Mermaid 流程图与内嵌无限白板
- 无边框融合标题栏、应用菜单、可拖拽调宽正文与侧栏、右键/多选拖拽目录树、关系图谱与版本历史 Diff

```rust
fn main() {
    println!(\"Hello from Mica Pure Rust!\");
}
```
";
