//! Mica L3 pure-Rust cross-platform client (`clients/mica_rust`).
//!
//! 1:1 visual & structural parity with Mica's warm-gray Notion-style shell
//! (`docs/plans/ui-visual-refresh.md`):
//! - 36px sunken top bar with Mica teal logo, sidebar toggle, `< >` history,
//!   seamless browser-style multi-document tabs (`DocTabStrip`) + `+`.
//! - Collapsible 244px warm-gray sidebar (`#F7F7F5`) with workspace switcher,
//!   recessed search pill, compact 30px page/folder tree, and bottom account bar.
//! - Centered 740px paper editor with breadcrumb + borderless icon formatting bar,
//!   large borderless editable title, true typographic scale, in-place native IME,
//!   `/` slash menu, tables, RaTeX/Mermaid/embedded-board atomic blocks, and word count pill.
//! - Direct L1 + L2 Rust core calls (zero Dart, zero Flutter, zero FFI/JSON bridge).

pub mod atomic_blocks;
pub mod board_view;
pub mod editor_slash;
pub mod editor_table;
pub mod editor_view;
pub mod mica_icons;
pub mod mica_theme;
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
use mica_editor::{EditorBlock, EditorSession};
use nsh_base::button;
use nsh_core::{geometry::px, prelude::*};
use uuid::Uuid;

use crate::{
  editor_view::{EditorHeaderProps, EditorViewState},
  mica_theme::{install_mica_theme, palette},
  net::{CloudConfig, CloudConnector},
  settings_view::SettingsCallbacks,
  sidebar::SidebarCallbacks,
  storage::{DEFAULT_ORIGIN, VaultStore},
};

const MODE_DOC: u8 = 0;
const MODE_BOARD: u8 = 1;
const MODE_SEARCH: u8 = 2;
const MODE_SETTINGS: u8 = 3;

const DEFAULT_BOARD_ID: &str = "workspace-default-board";

#[derive(Clone, PartialEq, Eq)]
struct OpenTab {
  doc_id: Uuid,
  title: String,
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
  let origin = DEFAULT_ORIGIN.to_string();

  let mut views = vault.list_views(&origin);
  let ws_id: Uuid;
  let default_folder_id: Uuid;
  let initial_doc_id: Uuid;
  let initial_title: String;
  let initial_folder_name: String;
  let mut docs_map: HashMap<Uuid, String> = HashMap::new();

  if views.is_empty() {
    ws_id = Uuid::new_v4();
    let folder_id = Uuid::new_v4();
    let doc_id = Uuid::new_v4();
    let folder = make_view(ws_id, folder_id, "知识库", "folder", None, "a0");
    let page = make_view(
      ws_id,
      doc_id,
      "欢迎使用 Mica",
      "page",
      Some(folder.id),
      "a0",
    );
    let starter_md = concat!(
      "## 全栈纯 Rust 知识库\n\n",
      "你的笔记就是 Markdown 文件：本地 SQLite 离线存储，或实时云端协同。\n\n",
      "- 原生中文输入法：点击任意段落直接原地编辑\n",
      "- 斜杠命令：输入 / 插入标题、待办、表格、公式或 Mermaid 图表\n",
      "- 无限画板与导图：点击右上角或侧栏图标随时进入白板视图\n",
    )
    .to_string();
    let _ = vault.save_view(&origin, &folder);
    let _ = vault.save_view(&origin, &page);
    let _ = vault.save_markdown(&origin, doc_id, &starter_md);
    docs_map.insert(doc_id, starter_md);
    default_folder_id = folder.id;
    initial_doc_id = doc_id;
    initial_title = page.name.clone();
    initial_folder_name = folder.name.clone();
    views = vec![folder, page];
  } else {
    ws_id = views[0].workspace_id;
    let folder_view = views
      .iter()
      .find(|v| !v.is_deleted && v.object_type == "folder")
      .cloned()
      .unwrap_or_else(|| {
        let fid = Uuid::new_v4();
        let f = make_view(ws_id, fid, "知识库", "folder", None, "a0");
        let _ = vault.save_view(&origin, &f);
        views.push(f.clone());
        f
      });
    default_folder_id = folder_view.id;
    initial_folder_name = folder_view.name.clone();
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
      let page = make_view(ws_id, doc_id, "无标题", "page", Some(default_folder_id), "a0");
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
  let sidebar_open = signal(true);
  let workspace_name = signal(String::from("Mica 本地工作区"));
  let account_name = signal(String::from("local"));
  let folder_name = signal(initial_folder_name);
  let active_doc = signal(Some(initial_doc_id));
  let active_title = signal(initial_title.clone());
  let open_tabs: Signal<Vec<OpenTab>> = signal(vec![OpenTab {
    doc_id: initial_doc_id,
    title: initial_title,
  }]);
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
  let editor_state = EditorViewState::new(session_for_markdown(&initial_md));

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

  on_key("ctrl+k", move |_| active_mode.set(MODE_SEARCH));
  on_key("cmd+k", move |_| active_mode.set(MODE_SEARCH));
  on_key("ctrl+shift+f", move |_| active_mode.set(MODE_SEARCH));

  let on_select_doc: Rc<dyn Fn(Uuid, String)> = {
    let docs = docs_store.clone();
    let ed = editor_state.clone();
    let v = vault.clone();
    let orig = origin.clone();
    let tree_ref = tree.clone();
    Rc::new(move |next_id: Uuid, title: String| {
      if let Some(prev_id) = active_doc.get_untracked() {
        let md = ed.export_markdown();
        docs.borrow_mut().insert(prev_id, md.clone());
        let _ = v.save_markdown(&orig, prev_id, &md);
      }
      let md = docs.borrow().get(&next_id).cloned().unwrap_or_default();
      ed.load_session(session_for_markdown(&md));
      active_doc.set(Some(next_id));
      active_title.set(title.clone());
      // Update breadcrumb folder name
      let parent_folder = {
        let t = tree_ref.borrow();
        t.visible_rows()
          .into_iter()
          .find(|r| !r.is_folder && r.object_id == next_id)
          .and_then(|r| r.parent_view_id)
          .and_then(|pid| t.get(pid).map(|pv| pv.name.clone()))
      };
      if let Some(pf) = parent_folder {
        folder_name.set(pf);
      }
      // Ensure tab exists in multi-tab strip
      let mut tabs = open_tabs.get_untracked();
      if !tabs.iter().any(|t| t.doc_id == next_id) {
        tabs.push(OpenTab {
          doc_id: next_id,
          title,
        });
        open_tabs.set(tabs);
      }
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
      let name = format!("未命名页面 {idx}");
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
      ed.load_session(session_for_markdown(""));
      active_doc.set(Some(new_doc_id));
      active_title.set(name.clone());
      let mut tabs = open_tabs.get_untracked();
      tabs.push(OpenTab {
        doc_id: new_doc_id,
        title: name,
      });
      open_tabs.set(tabs);
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
      let name = format!("文件夹 {count}");
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
      active_title.set(next_name.clone());
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
    on_new_page: on_new_page.clone(),
    on_new_folder,
    on_open_board: Rc::new(move || active_mode.set(MODE_BOARD)),
    on_restore_view,
    on_open_search: Rc::new(move || active_mode.set(MODE_SEARCH)),
    on_open_settings: Rc::new(move || active_mode.set(MODE_SETTINGS)),
  });

  let editor_header = Rc::new(EditorHeaderProps {
    workspace_name,
    folder_name,
    page_title: active_title,
    on_rename_title: on_rename_active,
    on_open_board: Rc::new(move || active_mode.set(MODE_BOARD)),
    on_trash_page: on_trash_active,
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
      set_color_scheme(if next {
        Appearance::Dark
      } else {
        Appearance::Light
      });
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
  let sel_for_tabs = on_select_doc.clone();
  let new_for_tabs = on_new_page.clone();

  view()
    .flex_1()
    .bg(move || palette(dark_mode.get()).surface_base)
    .child(
      // 1. Original 36px sunken top bar: Teal Logo + Sidebar Toggle | `< >` + Browser Tabs + `+`
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
            .child(dynamic(move || {
              mica_icons::mica_logo(palette(dark_mode.get()).brand_hex)
            }))
            .child(
              button()
                .test_id("toggle-sidebar-btn")
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
          // DocTabStrip: `<` `>` + open document tabs + `+`
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
                    .w(px(22.0))
                    .h(px(22.0))
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                    .child(dynamic(move || {
                      mica_icons::chevron_left(palette(dark_mode.get()).text_ghost_hex)
                    })),
                )
                .child(
                  button()
                    .w(px(22.0))
                    .h(px(22.0))
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                    .child(dynamic(move || {
                      mica_icons::chevron_right(palette(dark_mode.get()).text_ghost_hex)
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
                      .w(px(18.0))
                      .h(px(18.0))
                      .items_center()
                      .justify_center()
                      .rounded(px(4.0))
                      .on_press(move || {
                        let mut list = open_tabs.get_untracked();
                        list.retain(|t| t.doc_id != close_id);
                        if let Some(next) = list.last().cloned() {
                          open_tabs.set(list);
                          if active_doc.get_untracked() == Some(close_id) {
                            sel_after_close(next.doc_id, next.title);
                          }
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
      // 2. Main workspace body: collapsible sidebar (left) + active view (right)
      view()
        .flex_1()
        .w_full()
        .flex_row()
        .child(when(move || sidebar_open.get(), {
          let t = tree.clone();
          let scb = sidebar_cb.clone();
          move || {
            sidebar::sidebar_view(
              t.clone(),
              tree_rev,
              active_doc,
              workspace_name,
              account_name,
              dark_mode,
              scb.clone(),
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
              move || editor_view::editor_view(ed.clone(), dark_mode, hdr.clone())
            }))
            .child(when(move || active_mode.get() == MODE_BOARD, {
              let b = board_session.clone();
              let pb = on_persist_board.clone();
              move || {
                view()
                  .flex_1()
                  .child(
                    view()
                      .h(px(38.0))
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
                          .test_id("mode-doc")
                          .px_3()
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
                  .child(board_view::board_view(b.clone(), board_rev, pb.clone()))
              }
            }))
            .child(when(move || active_mode.get() == MODE_SEARCH, {
              let ts = tree_search.clone();
              let ds = docs_search.clone();
              let od = on_select_doc.clone();
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
        ),
    )
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
