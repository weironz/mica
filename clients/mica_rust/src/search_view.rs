//! 1:1 Floating Workspace Search Dialog (`search_dialog.dart`, `Ctrl+Shift+F`):
//! - Centered `600×440` modal palette (`max_w(92%)`, `max_h(86%)`)
//! - Empty query state: up to 8 other-workspace quick-switch chips (`工作区`) + up to 6 `最近编辑` pages
//! - Active query state:
//!   - Synchronous workspace-name matches (up to 5) above page/folder hits with `切换` hint
//!   - Includes FOLDERS (`文件夹 —— 在侧栏中定位`, clicking reveals folder in sidebar)
//!   - Fixed `82px` 3-line result rows: title / full ancestor path / highlighted snippet (`comment_highlight`)
//! - Wrapping `↑` / `↓` navigation across workspace + page/folder hits
//! - `Enter` opens selected hit, `Ctrl/Cmd+Enter` opens selected page in a new tab

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use chrono::Utc;
use mica_client::{PageTreeModel, SearchHit, search_cached_documents};
use mica_md::{ParseMode, parse_body};
use nsh_base::{button, input};
use nsh_core::{SubmitBehavior, color::rgba, geometry::px, prelude::*};
use uuid::Uuid;

use crate::{mica_icons, mica_theme::palette, settings_view::format_relative_time};

const MAX_WORKSPACE_CHIPS: usize = 8;
const MAX_WORKSPACE_HITS: usize = 5;
const MAX_RECENT_ITEMS: usize = 6;
const SEARCH_ROW_HEIGHT: f32 = 82.0;

/// Workspace chip / match entry shown in the search dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchWorkspaceChip {
  pub id: Uuid,
  pub name: String,
}

/// Recently edited page entry shown in the empty-query state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchRecentItem {
  pub view_id: Uuid,
  pub document_id: Uuid,
  pub title: String,
  pub icon: Option<String>,
  pub relative_time: String,
}

/// Enriched search hit covering both pages and folders with ancestor path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RichSearchHit {
  pub view_id: Uuid,
  pub document_id: Uuid,
  pub title: String,
  pub is_folder: bool,
  pub title_match: bool,
  pub snippet: String,
  pub ancestor_path: Vec<String>,
}

/// A highlighted or plain slice of snippet text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightRun {
  pub text: String,
  pub hit: bool,
}

/// Full props for the 1:1 Search modal dialog.
pub struct SearchViewProps {
  pub tree: Rc<RefCell<PageTreeModel>>,
  pub docs: Rc<RefCell<HashMap<Uuid, String>>>,
  pub dark_mode: Signal<bool>,
  pub current_workspace_id: Option<Uuid>,
  pub current_workspace_name: Option<String>,
  pub workspaces: Vec<SearchWorkspaceChip>,
  pub recents: Vec<SearchRecentItem>,
  pub initial_query: Option<String>,
  pub on_open_doc: Rc<dyn Fn(Uuid, String)>,
  pub on_open_doc_new_tab: Option<Rc<dyn Fn(Uuid, String)>>,
  pub on_reveal_folder: Option<Rc<dyn Fn(Uuid)>>,
  pub on_switch_workspace: Option<Rc<dyn Fn(Uuid)>>,
  pub on_close: Rc<dyn Fn()>,
}

/// Backward-compatible entrypoint called by existing `lib.rs`.
pub fn search_modal_view(
  tree: Rc<RefCell<PageTreeModel>>,
  docs: Rc<RefCell<HashMap<Uuid, String>>>,
  dark_mode: Signal<bool>,
  on_open_doc: Rc<dyn Fn(Uuid, String)>,
  on_close: Rc<dyn Fn()>,
) -> impl IntoElement {
  search_modal_view_with_props(Rc::new(SearchViewProps {
    tree,
    docs,
    dark_mode,
    current_workspace_id: None,
    current_workspace_name: None,
    workspaces: Vec::new(),
    recents: Vec::new(),
    initial_query: None,
    on_open_doc,
    on_open_doc_new_tab: None,
    on_reveal_folder: None,
    on_switch_workspace: None,
    on_close,
  }))
}

/// Full 1:1 `600×440` Search dialog supporting workspaces, recents, folders, highlights,
/// wrapping keyboard navigation, and `Ctrl/Cmd+Enter` new-tab open.
pub fn search_modal_view_with_props(props: Rc<SearchViewProps>) -> impl IntoElement {
  let initial_q = props.initial_query.clone().unwrap_or_default();
  let initial_hits = if initial_q.trim().is_empty() {
    Vec::new()
  } else {
    run_rich_workspace_search(
      &props.tree.borrow(),
      &props.docs.borrow(),
      props.current_workspace_name.as_deref(),
      &initial_q,
    )
  };
  let initial_ws_hits = matching_workspaces(
    &props.workspaces,
    props.current_workspace_id,
    &initial_q,
    props.on_switch_workspace.is_some(),
  );

  let query = signal(initial_q);
  let hits: Signal<Vec<RichSearchHit>> = signal(initial_hits);
  let ws_hits: Signal<Vec<SearchWorkspaceChip>> = signal(initial_ws_hits);
  let selected_idx: Signal<usize> = signal(0);

  let dark_mode = props.dark_mode;
  let tree_q = props.tree.clone();
  let docs_q = props.docs.clone();
  let ws_name_q = props.current_workspace_name.clone();
  let workspaces_q = props.workspaces.clone();
  let current_ws_id = props.current_workspace_id;
  let has_ws_switch = props.on_switch_workspace.is_some();

  let recents = if props.recents.is_empty() {
    derive_recents_from_tree(&props.tree.borrow())
  } else {
    props.recents.iter().take(MAX_RECENT_ITEMS).cloned().collect()
  };

  let other_workspaces: Vec<SearchWorkspaceChip> = if props.on_switch_workspace.is_some() {
    props
      .workspaces
      .iter()
      .filter(|w| Some(w.id) != props.current_workspace_id)
      .take(MAX_WORKSPACE_CHIPS)
      .cloned()
      .collect()
  } else {
    Vec::new()
  };

  // Unified activation helper (works for Enter, Ctrl+Enter, and row click)
  let activate_selected: Rc<dyn Fn(bool)> = {
    let p = props.clone();
    Rc::new(move |in_new_tab: bool| {
      let cur_ws = ws_hits.get_untracked();
      let cur_hits = hits.get_untracked();
      let total = cur_ws.len() + cur_hits.len();
      if total == 0 {
        return;
      }
      let idx = selected_idx.get_untracked().min(total.saturating_sub(1));
      if idx < cur_ws.len() {
        if let Some(cb) = &p.on_switch_workspace {
          cb(cur_ws[idx].id);
        }
        (p.on_close)();
        return;
      }
      let hit = &cur_hits[idx - cur_ws.len()];
      activate_hit(&p, hit, in_new_tab);
    })
  };

  // Bind Ctrl/Cmd+Enter while modal is mounted to open selected page in a new tab
  {
    let act_new_tab = activate_selected.clone();
    on_key("mod+enter", move |_| {
      act_new_tab(true);
    });
  }

  let close_bg = props.on_close.clone();
  let close_btn = props.on_close.clone();
  let close_key = props.on_close.clone();
  let act_submit = activate_selected.clone();

  view()
    .test_id("search-modal-backdrop")
    .absolute()
    .inset_0()
    .z_index(40)
    .items_center()
    .pt(px(80.0))
    .bg(move || palette(dark_mode.get()).backdrop)
    .child(
      button()
        .test_id("search-backdrop-dismiss")
        .accessibility_label("关闭搜索弹窗")
        .absolute()
        .inset_0()
        .on_press(move || close_bg()),
    )
    .child(
      // Centered floating 600×440 search dialog matching Flutter `search_dialog.dart`
      view()
        .test_id("search-panel")
        .w(px(600.0))
        .max_w(relative(0.92))
        .h(px(440.0))
        .max_h(relative(0.86))
        .rounded(px(12.0))
        .border_1()
        .border_color(move || palette(dark_mode.get()).border_normal)
        .bg(move || palette(dark_mode.get()).surface_overlay)
        .overflow_hidden()
        .z_index(41)
        .child(
          // Top search input row
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
                .submit_behavior(SubmitBehavior::Submit)
                .placeholder("搜索标题和内容…")
                .flex_1()
                .text_size(px(14.0))
                .text_color(move || palette(dark_mode.get()).text_primary)
                .bg(move || palette(dark_mode.get()).surface_overlay)
                .value(move || query.get())
                .on_change(move |q: &str| {
                  query.set(q.to_owned());
                  let w_hits =
                    matching_workspaces(&workspaces_q, current_ws_id, q, has_ws_switch);
                  let r_hits = run_rich_workspace_search(
                    &tree_q.borrow(),
                    &docs_q.borrow(),
                    ws_name_q.as_deref(),
                    q,
                  );
                  selected_idx.set(0);
                  ws_hits.set(w_hits);
                  hits.set(r_hits);
                })
                .on_key_press(move |k: &str| match k {
                  "ArrowDown" => {
                    let count = ws_hits.get_untracked().len() + hits.get_untracked().len();
                    if let Some(next) =
                      move_selection_wrap(selected_idx.get_untracked(), count, 1)
                    {
                      selected_idx.set(next);
                    }
                  }
                  "ArrowUp" => {
                    let count = ws_hits.get_untracked().len() + hits.get_untracked().len();
                    if let Some(next) =
                      move_selection_wrap(selected_idx.get_untracked(), count, -1)
                    {
                      selected_idx.set(next);
                    }
                  }
                  "Escape" => close_key(),
                  _ => {}
                })
                .on_submit(move |_: &str| {
                  act_submit(false);
                }),
            )
            .child(key_cap("esc", dark_mode))
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
          // Status summary bar (preserves exact E2E status text contract)
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
        .child(render_search_body(
          props.clone(),
          query,
          ws_hits,
          hits,
          selected_idx,
          other_workspaces,
          recents,
        ))
        .child(
          // Footer keyboard hints matching Flutter `_KeyCap` bar
          view()
            .h(px(36.0))
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
                .child(
                  view()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .child(key_cap("↑", dark_mode))
                    .child(key_cap("↓", dark_mode))
                    .child(
                      text("选择")
                        .text_size(px(11.0))
                        .text_color(move || palette(dark_mode.get()).text_faint),
                    ),
                )
                .child(
                  view()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .child(key_cap("↵", dark_mode))
                    .child(
                      text("打开")
                        .text_size(px(11.0))
                        .text_color(move || palette(dark_mode.get()).text_faint),
                    ),
                )
                .child(
                  view()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .child(key_cap("Ctrl+↵", dark_mode))
                    .child(
                      text("新标签页")
                        .text_size(px(11.0))
                        .text_color(move || palette(dark_mode.get()).text_faint),
                    ),
                ),
            )
            .child(
              view()
                .flex_row()
                .items_center()
                .gap_1()
                .child(
                  text("页内查找")
                    .text_size(px(11.0))
                    .text_color(move || palette(dark_mode.get()).text_faint),
                )
                .child(key_cap("Ctrl+F", dark_mode)),
            ),
        ),
    )
}

fn activate_hit(props: &SearchViewProps, hit: &RichSearchHit, in_new_tab: bool) {
  if hit.is_folder {
    props.tree.borrow_mut().reveal(hit.view_id);
    if let Some(cb) = &props.on_reveal_folder {
      cb(hit.view_id);
    }
    (props.on_close)();
    return;
  }
  if in_new_tab
    && let Some(cb) = &props.on_open_doc_new_tab
  {
    cb(hit.document_id, hit.title.clone());
    (props.on_close)();
    return;
  }
  (props.on_open_doc)(hit.document_id, hit.title.clone());
  (props.on_close)();
}

fn key_cap(label: &'static str, dark_mode: Signal<bool>) -> impl IntoElement {
  view()
    .px_2()
    .py_1()
    .rounded(px(5.0))
    .bg(move || palette(dark_mode.get()).surface_sunken)
    .child(
      text(label)
        .text_size(px(10.5))
        .font_family("Consolas")
        .text_color(move || palette(dark_mode.get()).text_faint),
    )
}

#[allow(clippy::too_many_arguments)]
fn render_search_body(
  props: Rc<SearchViewProps>,
  query: Signal<String>,
  ws_hits: Signal<Vec<SearchWorkspaceChip>>,
  hits: Signal<Vec<RichSearchHit>>,
  selected_idx: Signal<usize>,
  other_workspaces: Vec<SearchWorkspaceChip>,
  recents: Vec<SearchRecentItem>,
) -> impl IntoElement {
  let dark_mode = props.dark_mode;

  scroll().flex_1().w_full().p_3().child(dynamic(move || {
    let q = query.get();
    let p = palette(dark_mode.get());

    if q.trim().is_empty() {
      let mut empty_view = view().w_full().gap_3();

      // Up to 8 other-workspace quick-switch chips
      if !other_workspaces.is_empty() {
        let mut chips = view().flex_row().flex_wrap().gap_2();
        for w in &other_workspaces {
          let wid = w.id;
          let on_ws = props.on_switch_workspace.clone();
          let on_close = props.on_close.clone();
          chips = chips.child(
            button()
              .test_id(&format!("search-ws-chip-{}", w.id))
              .accessibility_label(format!("切换到工作区 {}", w.name))
              .px_3()
              .py_1()
              .rounded(px(6.0))
              .border_1()
              .border_color(p.border_normal)
              .bg(p.surface_base)
              .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
              .on_press(move || {
                if let Some(cb) = &on_ws {
                  cb(wid);
                }
                on_close();
              })
              .child(
                text(w.name.clone())
                  .text_size(px(12.5))
                  .text_color(p.text_primary),
              ),
          );
        }
        empty_view = empty_view.child(
          view()
            .w_full()
            .gap_2()
            .child(text("工作区").text_size(px(11.5)).text_color(p.text_faint))
            .child(chips),
        );
      }

      // Up to 6 recently edited pages
      if !recents.is_empty() {
        let mut rec_col = view()
          .w_full()
          .gap_1()
          .child(text("最近编辑").text_size(px(11.5)).text_color(p.text_faint));
        for (i, item) in recents.iter().enumerate() {
          let doc_id = item.document_id;
          let title = item.title.clone();
          let open = props.on_open_doc.clone();
          let close = props.on_close.clone();
          rec_col = rec_col.child(
            button()
              .test_id(&format!("search-recent-{i}"))
              .accessibility_label(format!("打开最近编辑 {}", item.title))
              .w_full()
              .h(px(36.0))
              .px_2()
              .flex_row()
              .items_center()
              .justify_between()
              .gap_2()
              .rounded(px(6.0))
              .bg(p.surface_overlay)
              .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
              .on_press(move || {
                open(doc_id, title.clone());
                close();
              })
              .child(
                view()
                  .flex_1()
                  .flex_row()
                  .items_center()
                  .gap_2()
                  .child(mica_icons::file_doc(p.text_muted_hex))
                  .child(
                    text(item.title.clone())
                      .flex_1()
                      .text_size(px(13.0))
                      .text_color(p.text_primary)
                      .truncate(),
                  ),
              )
              .child(
                text(item.relative_time.clone())
                  .text_size(px(11.5))
                  .text_color(p.text_faint),
              ),
          );
        }
        empty_view = empty_view.child(rec_col);
      } else if other_workspaces.is_empty() {
        empty_view = empty_view.child(
          view().py_8().items_center().justify_center().gap_1().child(
            text("按标题或内容查找页面。")
              .text_size(px(13.0))
              .text_color(p.text_ghost),
          ),
        );
      }
      return empty_view;
    }

    // Active query state
    let current_ws_hits = ws_hits.get();
    let current_hits = hits.get();
    let total = current_ws_hits.len() + current_hits.len();
    let active_idx = selected_idx.get().min(total.saturating_sub(1));
    let transparent = rgba(0x0000_0000);
    let mut list = view().w_full().gap_1();

    // 1. Synchronous workspace matches above page/folder hits
    if !current_ws_hits.is_empty() {
      list = list.child(
        view()
          .px_1()
          .pb_1()
          .child(text("工作区").text_size(px(11.5)).text_color(p.text_faint)),
      );
      for (wi, ws) in current_ws_hits.iter().enumerate() {
        let is_selected = wi == active_idx;
        let wid = ws.id;
        let on_ws = props.on_switch_workspace.clone();
        let on_close = props.on_close.clone();
        list = list.child(
          button()
            .test_id(&format!("search-ws-hit-{wi}"))
            .accessibility_label(format!("切换到工作区 {}", ws.name))
            .w_full()
            .h(px(36.0))
            .px_2()
            .flex_row()
            .items_center()
            .justify_between()
            .gap_2()
            .rounded(px(6.0))
            .bg(if is_selected {
              p.surface_hover
            } else {
              p.surface_overlay
            })
            .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
            .on_press(move || {
              selected_idx.set(wi);
              if let Some(cb) = &on_ws {
                cb(wid);
              }
              on_close();
            })
            .child(
              view()
                .flex_1()
                .flex_row()
                .items_center()
                .gap_2()
                .child(mica_icons::folder(p.text_muted_hex))
                .child(
                  text(ws.name.clone())
                    .flex_1()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(p.text_primary)
                    .truncate(),
                ),
            )
            .child(text("切换").text_size(px(11.5)).text_color(p.text_faint)),
        );
      }
    }

    // 2. Page + Folder hits (fixed 82px 3-line rows)
    if current_hits.is_empty() && current_ws_hits.is_empty() {
      return list.child(
        view().py_8().items_center().justify_center().gap_1().child(
          text(format!("没有找到与“{}”匹配的内容。", q.trim()))
            .text_size(px(13.0))
            .text_color(p.text_ghost),
        ),
      );
    }

    let ws_offset = current_ws_hits.len();
    for (i, hit) in current_hits.into_iter().enumerate() {
      let combined_idx = ws_offset + i;
      let is_selected = combined_idx == active_idx;
      let props_click = props.clone();
      let hit_click = hit.clone();
      let row_bg = if is_selected {
        p.surface_hover
      } else {
        p.surface_overlay
      };
      let accent_bar_bg = if is_selected { p.accent } else { transparent };
      let icon_hex = if is_selected {
        p.accent_hex
      } else {
        p.text_faint_hex
      };
      let path_str = hit.ancestor_path.join(" / ");
      let line3 = render_match_line(&hit, &q, dark_mode);

      list = list.child(
        button()
          .test_id(&format!("search-hit-{i}"))
          .accessibility_label(format!("打开搜索结果 {}", hit.title))
          .w_full()
          .h(px(SEARCH_ROW_HEIGHT))
          .px_2()
          .py_2()
          .rounded(px(8.0))
          .bg(row_bg)
          .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
          .on_press(move || {
            selected_idx.set(combined_idx);
            activate_hit(&props_click, &hit_click, false);
          })
          .child(
            view()
              .w_full()
              .h_full()
              .flex_row()
              .items_center()
              .gap_2()
              .child(
                view()
                  .w(px(3.0))
                  .h(px(44.0))
                  .rounded(px(2.0))
                  .bg(accent_bar_bg),
              )
              .child(if hit.is_folder {
                mica_icons::folder(icon_hex)
              } else {
                mica_icons::file_doc(icon_hex)
              })
              .child(
                view()
                  .flex_1()
                  .justify_center()
                  .gap_1()
                  .child(
                    // Line 1: Title
                    text(hit.title)
                      .text_size(px(13.5))
                      .font_weight(FontWeight::MEDIUM)
                      .text_color(p.text_primary)
                      .truncate(),
                  )
                  .child(
                    // Line 2: Full ancestor path (Workspace / Folder A / Folder B)
                    text(if path_str.is_empty() {
                      "工作区根目录".to_string()
                    } else {
                      path_str
                    })
                    .text_size(px(11.0))
                    .text_color(p.text_faint)
                    .truncate(),
                  )
                  .child(
                    // Line 3: Highlighted snippet / folder match / title match
                    line3,
                  ),
              ),
          ),
      );
    }

    list
  }))
}

fn render_match_line(hit: &RichSearchHit, query: &str, dark_mode: Signal<bool>) -> View {
  let p = palette(dark_mode.get_untracked());
  if !hit.snippet.is_empty() {
    let runs = highlight_runs(&hit.snippet, query);
    let mut row = view().w_full().flex_row().items_center().overflow_hidden();
    for run in runs {
      if run.hit {
        row = row.child(
          view()
            .px(px(2.0))
            .rounded(px(3.0))
            .bg(p.comment_highlight)
            .child(
              text(run.text)
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(p.text_primary),
            ),
        );
      } else {
        row = row.child(
          text(run.text)
            .text_size(px(12.0))
            .text_color(p.text_muted),
        );
      }
    }
    return row;
  }

  let fallback = if hit.is_folder {
    "文件夹 —— 在侧栏中定位"
  } else if hit.title_match {
    "标题匹配"
  } else {
    ""
  };
  view().w_full().child(
    text(fallback)
      .text_size(px(12.0))
      .text_color(p.text_faint)
      .truncate(),
  )
}

// ── Pure Search & Navigation Algorithms (unit-tested) ────────────────────────

/// Split `snippet` into plain and case-insensitive matched runs for `query`.
pub fn highlight_runs(snippet: &str, query: &str) -> Vec<HighlightRun> {
  let q = query.trim();
  if q.is_empty() || snippet.is_empty() {
    return vec![HighlightRun {
      text: snippet.to_string(),
      hit: false,
    }];
  }

  let needle_chars: Vec<char> = q.to_lowercase().chars().collect();
  let hay_chars: Vec<char> = snippet.chars().collect();
  let hay_lower: Vec<char> = hay_chars
    .iter()
    .map(|c| c.to_lowercase().next().unwrap_or(*c))
    .collect();

  if needle_chars.is_empty() || hay_chars.len() < needle_chars.len() {
    return vec![HighlightRun {
      text: snippet.to_string(),
      hit: false,
    }];
  }

  let mut runs = Vec::new();
  let mut i = 0usize;
  let mut plain_start = 0usize;
  let nlen = needle_chars.len();

  while i + nlen <= hay_chars.len() {
    if hay_lower[i..i + nlen] == needle_chars[..] {
      if plain_start < i {
        runs.push(HighlightRun {
          text: hay_chars[plain_start..i].iter().collect(),
          hit: false,
        });
      }
      runs.push(HighlightRun {
        text: hay_chars[i..i + nlen].iter().collect(),
        hit: true,
      });
      i += nlen;
      plain_start = i;
    } else {
      i += 1;
    }
  }

  if plain_start < hay_chars.len() {
    runs.push(HighlightRun {
      text: hay_chars[plain_start..].iter().collect(),
      hit: false,
    });
  }

  if runs.is_empty() {
    runs.push(HighlightRun {
      text: snippet.to_string(),
      hit: false,
    });
  }
  runs
}

/// Wrapping keyboard selection step over `0..count`.
pub fn move_selection_wrap(current: usize, count: usize, delta: isize) -> Option<usize> {
  if count == 0 {
    return None;
  }
  let c = count as isize;
  let cur = (current.min(count - 1)) as isize;
  let next = (cur + delta).rem_euclid(c) as usize;
  Some(next)
}

/// Synchronous case-insensitive workspace name filter (capped at 5).
pub fn matching_workspaces(
  workspaces: &[SearchWorkspaceChip],
  current_workspace_id: Option<Uuid>,
  query: &str,
  enabled: bool,
) -> Vec<SearchWorkspaceChip> {
  if !enabled {
    return Vec::new();
  }
  let q = query.trim().to_lowercase();
  if q.is_empty() {
    return Vec::new();
  }
  workspaces
    .iter()
    .filter(|w| Some(w.id) != current_workspace_id && w.name.to_lowercase().contains(&q))
    .take(MAX_WORKSPACE_HITS)
    .cloned()
    .collect()
}

fn derive_recents_from_tree(tree: &PageTreeModel) -> Vec<SearchRecentItem> {
  let now = Utc::now();
  let mut pages = Vec::new();
  for row in tree.all_live_rows() {
    if row.is_folder {
      continue;
    }
    if let Some(v) = tree.get(row.view_id) {
      pages.push(v);
    }
  }
  pages.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then_with(|| a.name.cmp(&b.name)));
  pages
    .into_iter()
    .take(MAX_RECENT_ITEMS)
    .map(|v| SearchRecentItem {
      view_id: v.id,
      document_id: v.object_id,
      title: v.name.clone(),
      icon: v.icon.clone(),
      relative_time: format_relative_time(v.updated_at, now),
    })
    .collect()
}

fn ancestor_segments(
  tree: &PageTreeModel,
  view_id: Uuid,
  workspace_name: Option<&str>,
) -> Vec<String> {
  let mut segs = Vec::new();
  if let Some(ws) = workspace_name
    && !ws.trim().is_empty()
  {
    segs.push(ws.trim().to_string());
  }
  for crumb in tree.breadcrumbs(view_id) {
    if crumb.id != view_id {
      segs.push(crumb.name.clone());
    }
  }
  segs
}

/// Search both FOLDERS (by name) and PAGES (by title + body/board content),
/// returning enriched hits with full ancestor path (`Workspace / Folder`).
pub fn run_rich_workspace_search(
  tree: &PageTreeModel,
  docs: &HashMap<Uuid, String>,
  workspace_name: Option<&str>,
  query: &str,
) -> Vec<RichSearchHit> {
  let q_trimmed = query.trim();
  if q_trimmed.is_empty() {
    return Vec::new();
  }
  let q_lower = q_trimmed.to_lowercase();

  // Traverse ALL live rows including collapsed folders directly.
  let all_rows = tree.all_live_rows();

  let mut out = Vec::new();

  // 1. Folder hits (by name match)
  for row in &all_rows {
    if !row.is_folder {
      continue;
    }
    if row.name.to_lowercase().contains(&q_lower) {
      out.push(RichSearchHit {
        view_id: row.view_id,
        document_id: row.object_id,
        title: row.name.clone(),
        is_folder: true,
        title_match: true,
        snippet: String::new(),
        ancestor_path: ancestor_segments(tree, row.view_id, workspace_name),
      });
    }
  }

  // 2. Page hits (title + body + embedded board via `search_cached_documents`)
  let mut corpus = Vec::new();
  let mut view_by_doc: HashMap<Uuid, Uuid> = HashMap::new();
  for row in &all_rows {
    if row.is_folder {
      continue;
    }
    view_by_doc.insert(row.object_id, row.view_id);
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
    corpus.push((row.object_id, row.name.clone(), core_blocks));
  }

  let page_hits = search_cached_documents(&corpus, q_trimmed, 20);
  for hit in page_hits {
    let view_id = view_by_doc
      .get(&hit.document_id)
      .copied()
      .unwrap_or(hit.document_id);
    let title_match = hit.title.to_lowercase().contains(&q_lower);
    // Avoid duplicating title-only placeholder snippet if identical to title
    let snippet = if hit.snippet.trim() == hit.title.trim() {
      String::new()
    } else {
      hit.snippet
    };
    out.push(RichSearchHit {
      view_id,
      document_id: hit.document_id,
      title: hit.title,
      is_folder: false,
      title_match,
      snippet,
      ancestor_path: ancestor_segments(tree, view_id, workspace_name),
    });
  }

  out
}

/// Legacy page-only search helper kept for backward compatibility.
pub fn run_workspace_search(
  tree: &PageTreeModel,
  docs: &HashMap<Uuid, String>,
  query: &str,
) -> Vec<SearchHit> {
  if query.trim().is_empty() {
    return Vec::new();
  }
  let mut corpus = Vec::new();
  for row in tree.all_live_rows() {
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

#[cfg(test)]
mod tests {
  use super::*;
  use mica_api_types::View;

  fn mk_view(
    id: Uuid,
    obj: Uuid,
    parent: Option<Uuid>,
    object_type: &str,
    name: &str,
  ) -> View {
    let now = Utc::now();
    View {
      id,
      workspace_id: Uuid::new_v4(),
      parent_view_id: parent,
      object_id: obj,
      object_type: object_type.to_string(),
      name: name.to_string(),
      icon: None,
      position: "a0".to_string(),
      is_deleted: false,
      created_by: Uuid::new_v4(),
      created_at: now,
      updated_at: now,
      state_bytes: None,
    }
  }

  #[test]
  fn highlight_runs_splits_case_insensitive_matches() {
    let runs = highlight_runs("Hello Mica and MICA world", "mica");
    assert_eq!(
      runs,
      vec![
        HighlightRun { text: "Hello ".into(), hit: false },
        HighlightRun { text: "Mica".into(), hit: true },
        HighlightRun { text: " and ".into(), hit: false },
        HighlightRun { text: "MICA".into(), hit: true },
        HighlightRun { text: " world".into(), hit: false },
      ]
    );
  }

  #[test]
  fn wrapping_selection_navigation_wraps_both_ends() {
    assert_eq!(move_selection_wrap(0, 0, 1), None);
    assert_eq!(move_selection_wrap(0, 3, -1), Some(2));
    assert_eq!(move_selection_wrap(2, 3, 1), Some(0));
    assert_eq!(move_selection_wrap(1, 3, 1), Some(2));
  }

  #[test]
  fn rich_search_finds_folders_and_nested_pages_with_ancestor_paths() {
    let folder_id = Uuid::new_v4();
    let doc_view = Uuid::new_v4();
    let doc_obj = Uuid::new_v4();
    let tree = PageTreeModel::new(vec![
      mk_view(folder_id, folder_id, None, "folder", "架构设计"),
      mk_view(doc_view, doc_obj, Some(folder_id), "page", "纯Rust核心"),
    ]);
    let mut docs = HashMap::new();
    docs.insert(doc_obj, "这是关于纯Rust引擎的架构设计说明。".to_string());

    let hits = run_rich_workspace_search(&tree, &docs, Some("研发工作区"), "架构设计");
    assert_eq!(hits.len(), 2);
    let folder_hit = hits.iter().find(|h| h.is_folder).expect("folder hit");
    assert_eq!(folder_hit.title, "架构设计");
    assert_eq!(folder_hit.ancestor_path, vec!["研发工作区".to_string()]);

    let page_hit = hits.iter().find(|h| !h.is_folder).expect("page hit");
    assert_eq!(page_hit.title, "纯Rust核心");
    assert_eq!(
      page_hit.ancestor_path,
      vec!["研发工作区".to_string(), "架构设计".to_string()]
    );
  }

  #[test]
  fn matching_workspaces_excludes_current_and_caps_at_five() {
    let cur = Uuid::new_v4();
    let mut list = vec![SearchWorkspaceChip {
      id: cur,
      name: "Project Alpha".into(),
    }];
    for i in 0..8 {
      list.push(SearchWorkspaceChip {
        id: Uuid::new_v4(),
        name: format!("Project {i}"),
      });
    }
    let hits = matching_workspaces(&list, Some(cur), "project", true);
    assert_eq!(hits.len(), 5);
    assert!(hits.iter().all(|w| w.id != cur));
  }
}
