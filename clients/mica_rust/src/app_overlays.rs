//! Root overlay / modal mounting and desktop updater wiring extracted from `lib.rs`.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use mica_client::PageTreeModel;
use nsh_core::prelude::*;
use uuid::Uuid;

#[cfg(not(target_arch = "wasm32"))]
use crate::net;
use crate::{
  APP_VERSION, WorkspaceEntry,
  ai_dialog::{self, GlobalAiState, InlineAiState},
  app_chrome,
  app_menu::{self, AccountMenuState, AppMenuCallbacks},
  app_state::ModalSignals,
  app_transfer_io::{self, CrossWorkspaceTransferCtx},
  cloud_sync::CloudSessionHandles,
  editor_bridge::MicaDocumentBridge,
  emoji_picker, graph_dialog,
  net::CloudConnector,
  responsive_shell,
  search_view::{self, SearchRecentItem, SearchViewProps, SearchWorkspaceChip},
  settings_view::{self, SettingsViewProps},
  share_dialog::{self, ShareDialogState},
  sidebar::{self, WorkspaceSwitcherCallbacks, WorkspaceSwitcherItem},
  storage::VaultStore,
  toast::{self, ToastHost},
  tooltip::{self, TooltipHost},
  transfer_dialog, updater,
  version_history::{self, VersionHistoryState},
  workspace_views::{self, WorkspaceSettingsState},
  world_switcher::{self, WorldSwitcherState},
  zoom_controller::{self, ZoomController},
};

#[derive(Clone)]
pub(crate) struct UpdaterBundle {
  pub available_update: Signal<Option<updater::UpdateInfo>>,
  pub update_busy: Signal<bool>,
  pub run_check_update: Rc<dyn Fn(bool)>,
  pub on_install_update: Rc<dyn Fn()>,
}

/// Wire desktop update check/install callbacks and 5s gentle startup update check.
pub(crate) fn init_desktop_updater(
  app_owner: Owner,
  allow_local: bool,
  modals: ModalSignals,
  bridge: Rc<MicaDocumentBridge>,
) -> UpdaterBundle {
  #[cfg(target_arch = "wasm32")]
  let _ = &app_owner;
  let available_update: Signal<Option<updater::UpdateInfo>> = signal(None);
  let update_busy = signal(false);
  #[cfg(not(target_arch = "wasm32"))]
  let update_check_gen = signal(0u64);

  let about_modal_open = modals.about_modal_open;
  let update_status = modals.update_status;

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
    let br_upd = bridge;
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

  if allow_local {
    provide_context(sidebar::SidebarUpdateState {
      available_update,
      update_busy,
      update_status,
      on_click_update: Rc::new(move || {
        about_modal_open.set(true);
      }),
    });
  }

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

  UpdaterBundle {
    available_update,
    update_busy,
    run_check_update,
    on_install_update,
  }
}

pub(crate) struct AppOverlaysProps {
  pub app_owner: Owner,
  pub allow_local: bool,
  pub dark_mode: Signal<bool>,
  pub vault: Rc<VaultStore>,
  pub tree: Rc<RefCell<PageTreeModel>>,
  pub docs_store: Rc<RefCell<HashMap<Uuid, String>>>,
  pub bridge: Rc<MicaDocumentBridge>,
  pub cloud_connector: Rc<RefCell<CloudConnector>>,
  pub cloud_session: CloudSessionHandles,
  pub modals: ModalSignals,
  pub is_cloud_signed_in: Signal<bool>,
  pub server_url: Signal<String>,
  pub account_email: Signal<String>,
  pub active_origin: Signal<String>,
  pub active_workspace_id: Signal<Uuid>,
  pub workspace_name: Signal<String>,
  pub workspaces: Signal<Vec<WorkspaceEntry>>,
  pub active_doc: Signal<Option<Uuid>>,
  pub tree_rev: Signal<u64>,
  pub ws_switcher_items: Signal<Vec<WorkspaceSwitcherItem>>,
  pub ws_switcher_cb: Rc<WorkspaceSwitcherCallbacks>,
  pub app_menu_cb: AppMenuCallbacks,
  pub account_menu_state: AccountMenuState,
  pub world_state: WorldSwitcherState,
  pub ws_settings_state: WorkspaceSettingsState,
  pub version_history_state: VersionHistoryState,
  pub share_state: ShareDialogState,
  pub global_ai_state: GlobalAiState,
  pub inline_ai_state: InlineAiState,
  pub settings_props: Rc<SettingsViewProps>,
  pub updater: UpdaterBundle,
  pub tooltip_host: TooltipHost,
  pub toast_host: ToastHost,
  pub zoom_ctrl: ZoomController,
  pub reload_tree_from_vault: Rc<dyn Fn()>,
  pub on_new_page: Rc<dyn Fn()>,
  pub on_open_search_with: Rc<dyn Fn(Option<String>, bool)>,
  pub on_select_doc: Rc<dyn Fn(Uuid, String)>,
  pub on_open_in_new_tab: Rc<dyn Fn(Uuid, String)>,
  pub on_switch_workspace: Rc<dyn Fn(WorkspaceEntry)>,
  pub on_rename_view: Rc<dyn Fn(Uuid, String)>,
  pub on_set_icon: Rc<dyn Fn(Uuid, Option<String>)>,
}

/// Attach all root popovers, modals, and floating feedback layers directly to
/// the root `View` so positioning and `z_index` stacking remain unchanged.
pub(crate) fn mount_app_overlays(root: View, p: AppOverlaysProps) -> View {
  let modals = p.modals;
  let dark_mode = p.dark_mode;
  let app_menu_open = modals.app_menu_open;
  let about_modal_open = modals.about_modal_open;
  let new_tab_menu_open = modals.new_tab_menu_open;
  let workspace_menu_open = modals.workspace_menu_open;
  let account_menu_open = modals.account_menu_open;
  let graph_modal_open = modals.graph_modal_open;
  let transfer_modal_open = modals.transfer_modal_open;
  let sidebar_rename_open = modals.sidebar_rename_open;
  let sidebar_rename_view = modals.sidebar_rename_view;
  let sidebar_rename_initial = modals.sidebar_rename_initial;
  let sidebar_emoji_open = modals.sidebar_emoji_open;
  let sidebar_emoji_view = modals.sidebar_emoji_view;
  let sidebar_emoji_current = modals.sidebar_emoji_current;
  let search_modal_open = modals.search_modal_open;
  let search_open_in_new_tab = modals.search_open_in_new_tab;
  let search_initial_query = modals.search_initial_query;
  let settings_modal_open = modals.settings_modal_open;
  let embedded_board_edit = modals.embedded_board_edit;
  let tree_rev = p.tree_rev;
  let workspaces = p.workspaces;
  let active_workspace_id = p.active_workspace_id;
  let workspace_name = p.workspace_name;

  root
    // 3. New-Tab `+` 2-item popup menu
    .child(when(move || new_tab_menu_open.get(), {
      let np = p.on_new_page.clone();
      let os = p.on_open_search_with.clone();
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
    .child(app_menu::app_menu_popover(app_menu_open, dark_mode, p.app_menu_cb))
    .child(app_menu::about_modal_view(app_menu::AboutModalProps {
      open: about_modal_open,
      dark_mode,
      app_version: APP_VERSION,
      show_check_update: p.allow_local,
      update_status: modals.update_status,
      available_update: p.updater.available_update,
      update_busy: p.updater.update_busy,
      on_check_update: {
        let chk = p.updater.run_check_update;
        Rc::new(move || chk(false))
      },
      on_install_update: p.updater.on_install_update,
    }))
    // 5. 320px Workspace Switcher Popover & Stacked-Card Account Menu Popover
    .child(sidebar::workspace_switcher_popover(
      workspace_menu_open,
      p.ws_switcher_items,
      active_workspace_id,
      dark_mode,
      p.ws_switcher_cb,
    ))
    .child(app_menu::account_menu_popover(account_menu_open, dark_mode, p.account_menu_state))
    // 6. Full-Window Split World Switcher (`切换世界…`)
    .child(world_switcher::world_switcher_overlay(p.world_state))
    // 7. Workspace Settings, Knowledge Graph (900×640), Version History Diff (980×640), Share (440px)
    .child(workspace_views::workspace_settings_modal(p.ws_settings_state, dark_mode))
    .child(graph_dialog::graph_dialog_modal(
      graph_modal_open,
      modals.graph_data,
      p.active_doc,
      dark_mode,
      {
        let tree_graph = p.tree.clone();
        let sel = p.on_select_doc.clone();
        Rc::new(move |doc_id: Uuid| {
          let title = tree_graph
            .borrow()
            .all_live_rows()
            .into_iter()
            .find(|r| !r.is_folder && r.object_id == doc_id)
            .map(|r| r.name)
            .unwrap_or_else(|| "新页面".to_string());
          graph_modal_open.set(false);
          sel(doc_id, title);
        })
      },
      Rc::new(move || graph_modal_open.set(false)),
    ))
    .child(version_history::version_history_dialog(p.version_history_state, dark_mode))
    .child(share_dialog::share_dialog_modal(p.share_state, dark_mode))
    // 8. Global & Inline AI Dialogs
    .child(ai_dialog::global_ai_dialog(p.global_ai_state, dark_mode))
    .child(ai_dialog::inline_ai_dialog(p.inline_ai_state, dark_mode))
    // 9. Cross-Workspace Transfer Dialog (440px)
    .child(when(move || transfer_modal_open.get(), {
      let vt = p.vault;
      let br = p.bridge.clone();
      let docs = p.docs_store.clone();
      let cc = p.cloud_connector;
      let cs = p.cloud_session;
      let rel = p.reload_tree_from_vault;
      let app_owner = p.app_owner;
      let allow_local = p.allow_local;
      let is_cloud_signed_in = p.is_cloud_signed_in;
      let server_url = p.server_url;
      let account_email = p.account_email;
      let active_origin = p.active_origin;
      let transfer_source_ids = modals.transfer_source_ids;
      let transfer_copy_mode = modals.transfer_copy_mode;
      move || {
        let state = app_transfer_io::build_transfer_dialog_state(CrossWorkspaceTransferCtx {
          app_owner,
          vault: vt.clone(),
          bridge: br.clone(),
          docs_store: docs.clone(),
          cloud_connector: cc.clone(),
          cloud_session: cs.clone(),
          allow_local,
          is_cloud_signed_in,
          server_url,
          account_email,
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
    // 9b. Root-mounted Sidebar Rename & Emoji Picker modals (never clipped by 280px sidebar)
    .child(sidebar::rename_dialog_modal(
      sidebar_rename_open,
      sidebar_rename_initial,
      "重命名",
      dark_mode,
      {
        let on_ren = p.on_rename_view;
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
        let on_ic = p.on_set_icon;
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
      let ts = p.tree;
      let ds = p.docs_store;
      let od = p.on_select_doc;
      let od_new = p.on_open_in_new_tab;
      let sw = p.on_switch_workspace;
      move || {
        let cur_ws = active_workspace_id.get_untracked();
        let ws_chips: Vec<SearchWorkspaceChip> = workspaces
          .get_untracked()
          .into_iter()
          .map(|w| SearchWorkspaceChip { id: w.id, name: w.name })
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
      let sp = p.settings_props;
      move || settings_view::settings_modal_view_with_props(sp.clone())
    }))
    // 11. Embedded Document Board Fullscreen Modal (mounted below 34px titlebar)
    .child(when(move || embedded_board_edit.get().is_some(), {
      let br_m = p.bridge;
      move || app_chrome::render_embedded_board_modal(embedded_board_edit, br_m.clone(), dark_mode)
    }))
    // 12. Global dark-pill hover tooltip, unified toast, and zoom badge overlays
    .child(tooltip::tooltip_overlay(p.tooltip_host, dark_mode))
    .child(toast::toast_overlay(p.toast_host, dark_mode))
    .child(zoom_controller::zoom_badge_overlay(&p.zoom_ctrl, dark_mode))
}

/// Bind `narrow_tab_count` & `narrow_tab_entries` reactively to open tabs state.
pub(crate) fn bind_narrow_tab_signals(
  open_tabs: Signal<Vec<crate::OpenTab>>,
  active_tab_idx: Signal<usize>,
) -> (Signal<usize>, Signal<Vec<responsive_shell::NarrowTabEntry>>) {
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
  (narrow_tab_count, narrow_tab_entries)
}
