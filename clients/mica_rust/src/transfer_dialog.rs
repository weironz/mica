//! 1:1 Cross-Workspace Move / Copy Dialog (`transfer_dialog.dart` + `destination_tree_picker.dart`).
//!
//! - `440px`-wide modal (`移动到工作区` / `复制到工作区`)
//! - Filterable target workspace picker
//! - `240px`-tall `DestinationTreePicker` (`工作区根目录` + searchable folder/page tree
//!   + placement segment bar: `放入文件夹内` / `插在此项之前` / `插在此项之后` or `根目录末尾` / `根目录顶部`)
//! - Dry-run summary (`将移动/复制 {docs} 篇文档、{folders} 个文件夹、{images} 张图片`)
//! - Dangling-link amber warning + version-history notice on move

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use mica_api_types::View as MicaView;
use nsh_base::{button, input};
use nsh_core::{geometry::px, prelude::*};
use uuid::Uuid;

use crate::{mica_icons, mica_theme::palette};

/// Target placement mode inside the destination workspace tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetPlacement {
  Inside,
  Before,
  After,
  RootEnd,
  RootTop,
}

/// Selected destination inside a target workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestinationPick {
  pub parent_view_id: Option<Uuid>,
  pub before_view_id: Option<Uuid>,
  pub after_view_id: Option<Uuid>,
  pub placement: TargetPlacement,
}

impl DestinationPick {
  /// Append at the end of the target workspace root (default).
  pub fn root() -> Self {
    Self {
      parent_view_id: None,
      before_view_id: None,
      after_view_id: None,
      placement: TargetPlacement::RootEnd,
    }
  }

  pub fn root_top(first_root_id: Uuid) -> Self {
    Self {
      parent_view_id: None,
      before_view_id: Some(first_root_id),
      after_view_id: None,
      placement: TargetPlacement::RootTop,
    }
  }

  pub fn inside_folder(folder_id: Uuid) -> Self {
    Self {
      parent_view_id: Some(folder_id),
      before_view_id: None,
      after_view_id: None,
      placement: TargetPlacement::Inside,
    }
  }

  pub fn before_node(parent_view_id: Option<Uuid>, anchor_id: Uuid) -> Self {
    Self {
      parent_view_id,
      before_view_id: Some(anchor_id),
      after_view_id: None,
      placement: TargetPlacement::Before,
    }
  }

  pub fn after_node(parent_view_id: Option<Uuid>, anchor_id: Uuid) -> Self {
    Self {
      parent_view_id,
      before_view_id: None,
      after_view_id: Some(anchor_id),
      placement: TargetPlacement::After,
    }
  }
}

/// Candidate target workspace item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferDestination {
  pub id: Uuid,
  pub name: String,
}

/// Lightweight tree node rendered inside `DestinationTreePicker`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferDestinationNode {
  pub id: Uuid,
  pub parent_view_id: Option<Uuid>,
  pub name: String,
  pub icon: Option<String>,
  pub is_folder: bool,
  pub position: String,
}

impl From<&MicaView> for TransferDestinationNode {
  fn from(v: &MicaView) -> Self {
    Self {
      id: v.id,
      parent_view_id: v.parent_view_id,
      name: v.name.clone(),
      icon: v.icon.clone(),
      is_folder: v.object_type == "folder",
      position: v.position.clone(),
    }
  }
}

/// Dry-run impact summary shown before confirming a cross-workspace transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TransferPreviewReport {
  pub documents: usize,
  pub folders: usize,
  pub images: usize,
  pub dangling_links: usize,
}

/// State & callbacks driving [`transfer_dialog_modal`].
pub struct TransferDialogState {
  /// `true` = copy (keep source); `false` = move (soft-delete source after copy).
  pub copy: bool,
  /// Number of root items being transferred (for header/confirm context).
  pub item_count: usize,
  /// Available target workspaces (source workspace already excluded).
  pub destinations: Vec<TransferDestination>,
  /// Load the live tree nodes of a selected destination workspace.
  pub on_load_tree: Rc<dyn Fn(Uuid) -> Vec<TransferDestinationNode>>,
  /// Dry-run preview against `(dest_workspace_id, pick)`.
  pub on_preview: Rc<dyn Fn(Uuid, DestinationPick) -> Result<TransferPreviewReport, String>>,
  /// Commit the transfer against `(dest_workspace_id, pick)`.
  pub on_confirm: Rc<dyn Fn(Uuid, DestinationPick) -> Result<(), String>>,
  /// Dismiss the modal.
  pub on_close: Rc<dyn Fn()>,
}

/// Flattened visible row inside the destination tree picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibleDestRow {
  pub node: TransferDestinationNode,
  pub depth: usize,
  pub has_children: bool,
  pub expanded: bool,
}

/// Compute flattened rows for `DestinationTreePicker`, honoring expansion and
/// case-insensitive substring filter (auto-expanding matching ancestors).
pub fn compute_destination_rows(
  nodes: &[TransferDestinationNode],
  expanded: &HashSet<Uuid>,
  filter_query: &str,
) -> Vec<VisibleDestRow> {
  let by_id: HashMap<Uuid, &TransferDestinationNode> = nodes.iter().map(|n| (n.id, n)).collect();
  let mut children_by_parent: HashMap<Option<Uuid>, Vec<&TransferDestinationNode>> = HashMap::new();
  for n in nodes {
    let effective_parent = n
      .parent_view_id
      .filter(|pid| by_id.get(pid).is_some_and(|p| p.is_folder));
    children_by_parent
      .entry(effective_parent)
      .or_default()
      .push(n);
  }
  for list in children_by_parent.values_mut() {
    list.sort_by(|a, b| a.position.cmp(&b.position).then_with(|| a.name.cmp(&b.name)));
  }

  let q = filter_query.trim().to_lowercase();
  let filtering = !q.is_empty();

  let (keep, effective_open): (Option<HashSet<Uuid>>, HashSet<Uuid>) = if filtering {
    let mut keep_set = HashSet::new();
    let mut open_set = HashSet::new();
    for n in nodes {
      if !n.name.to_lowercase().contains(&q) {
        continue;
      }
      keep_set.insert(n.id);
      let mut cursor = n.parent_view_id;
      let mut guard = HashSet::new();
      while let Some(pid) = cursor {
        if !guard.insert(pid) {
          break;
        }
        keep_set.insert(pid);
        open_set.insert(pid);
        cursor = by_id.get(&pid).and_then(|p| p.parent_view_id);
      }
    }
    (Some(keep_set), open_set)
  } else {
    (None, expanded.clone())
  };

  let mut out = Vec::new();
  let mut visited = HashSet::new();
  walk_dest_rows(
    None,
    0,
    &children_by_parent,
    &effective_open,
    keep.as_ref(),
    &mut visited,
    &mut out,
  );
  out
}

fn walk_dest_rows(
  parent: Option<Uuid>,
  depth: usize,
  children_by_parent: &HashMap<Option<Uuid>, Vec<&TransferDestinationNode>>,
  expanded: &HashSet<Uuid>,
  keep: Option<&HashSet<Uuid>>,
  visited: &mut HashSet<Uuid>,
  out: &mut Vec<VisibleDestRow>,
) {
  let Some(children) = children_by_parent.get(&parent) else {
    return;
  };
  for &node in children {
    if !visited.insert(node.id) {
      continue;
    }
    if let Some(filter_set) = keep
      && !filter_set.contains(&node.id)
    {
      continue;
    }
    let has_children = node.is_folder
      && children_by_parent
        .get(&Some(node.id))
        .is_some_and(|c| !c.is_empty());
    let is_open = node.is_folder && expanded.contains(&node.id);
    out.push(VisibleDestRow {
      node: node.clone(),
      depth,
      has_children,
      expanded: is_open,
    });
    if is_open {
      walk_dest_rows(
        Some(node.id),
        depth + 1,
        children_by_parent,
        expanded,
        keep,
        visited,
        out,
      );
    }
  }
}

/// First root-level node sorted by position (used for `根目录顶部` placement).
pub fn first_root_node(nodes: &[TransferDestinationNode]) -> Option<&TransferDestinationNode> {
  let mut roots: Vec<&TransferDestinationNode> =
    nodes.iter().filter(|n| n.parent_view_id.is_none()).collect();
  roots.sort_by(|a, b| a.position.cmp(&b.position).then_with(|| a.name.cmp(&b.name)));
  roots.into_iter().next()
}

/// Render the centered 440px Cross-Workspace Move/Copy modal dialog.
pub fn transfer_dialog_modal(
  state: TransferDialogState,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  let state = Rc::new(state);
  let ws_filter = signal(String::new());
  let selected_dest: Signal<Option<Uuid>> = signal(None);
  let dest_tree: Signal<Vec<TransferDestinationNode>> = signal(Vec::new());
  let tree_filter = signal(String::new());
  let expanded_folders: Signal<HashSet<Uuid>> = signal(HashSet::new());
  let selection: Signal<DestinationPick> = signal(DestinationPick::root());
  let preview: Signal<Option<TransferPreviewReport>> = signal(None);
  let error_msg: Signal<Option<String>> = signal(None);

  let close_bg = state.on_close.clone();
  let close_x = state.on_close.clone();
  let close_cancel = state.on_close.clone();

  let run_preview: Rc<dyn Fn(Uuid, DestinationPick)> = {
    let st = state.clone();
    Rc::new(move |dest_id: Uuid, pick: DestinationPick| match (st.on_preview)(dest_id, pick) {
      Ok(report) => {
        preview.set(Some(report));
        error_msg.set(None);
      }
      Err(err) => {
        preview.set(None);
        error_msg.set(Some(format!("转移失败：{err}")));
      }
    })
  };

  let select_workspace: Rc<dyn Fn(Uuid)> = {
    let st = state.clone();
    let run_p = run_preview.clone();
    Rc::new(move |dest_id: Uuid| {
      selected_dest.set(Some(dest_id));
      let nodes = (st.on_load_tree)(dest_id);
      dest_tree.set(nodes);
      tree_filter.set(String::new());
      expanded_folders.set(HashSet::new());
      let root_pick = DestinationPick::root();
      selection.set(root_pick.clone());
      run_p(dest_id, root_pick);
    })
  };

  // Auto-select the sole destination workspace so the tree & preview render immediately.
  if state.destinations.len() == 1 {
    select_workspace(state.destinations[0].id);
  }

  let select_pick: Rc<dyn Fn(DestinationPick)> = {
    let run_p = run_preview.clone();
    Rc::new(move |pick: DestinationPick| {
      selection.set(pick.clone());
      if let Some(dest_id) = selected_dest.get_untracked() {
        run_p(dest_id, pick);
      }
    })
  };

  let on_confirm_press: Rc<dyn Fn()> = {
    let st = state.clone();
    Rc::new(move || {
      let Some(dest_id) = selected_dest.get_untracked() else {
        return;
      };
      let pick = selection.get_untracked();
      match (st.on_confirm)(dest_id, pick) {
        Ok(()) => (st.on_close)(),
        Err(err) => error_msg.set(Some(format!("转移失败：{err}"))),
      }
    })
  };

  let title = if state.copy {
    if state.item_count > 1 {
      format!("复制 {} 项到工作区", state.item_count)
    } else {
      "复制到工作区".to_string()
    }
  } else if state.item_count > 1 {
    format!("移动 {} 项到工作区", state.item_count)
  } else {
    "移动到工作区".to_string()
  };

  let confirm_label = if state.copy { "复制" } else { "移动" };
  let is_copy = state.copy;
  let destinations = state.destinations.clone();

  view()
    .test_id("transfer-modal-backdrop")
    .absolute()
    .inset_0()
    .z_index(55)
    .items_center()
    .justify_center()
    .bg(move || palette(dark_mode.get()).backdrop)
    .child(
      button()
        .test_id("transfer-backdrop-dismiss")
        .accessibility_label("关闭转移弹窗")
        .absolute()
        .inset_0()
        .on_press(move || close_bg()),
    )
    .child(
      view()
        .test_id("transfer-dialog-panel")
        .w(px(440.0))
        .max_w(relative(0.92))
        .max_h(relative(0.88))
        .rounded(px(12.0))
        .border_1()
        .border_color(move || palette(dark_mode.get()).border_normal)
        .bg(move || palette(dark_mode.get()).surface_overlay)
        .overflow_hidden()
        .z_index(56)
        .child(
          // Dialog header
          view()
            .px_5()
            .py_4()
            .flex_row()
            .items_center()
            .justify_between()
            .border_b_1()
            .border_color(move || palette(dark_mode.get()).border_subtle)
            .child(
              text(title)
                .text_size(px(16.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || palette(dark_mode.get()).text_primary),
            )
            .child(
              button()
                .test_id("transfer-close-btn")
                .accessibility_label("关闭")
                .w(px(28.0))
                .h(px(28.0))
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                .on_press(move || close_x())
                .child(dynamic(move || {
                  mica_icons::close_sm(palette(dark_mode.get()).text_faint_hex)
                })),
            ),
        )
        .child(
          // Scrollable content body (explicit `max_h` instead of `flex_1()` so
          // Taffy never collapses the scroll region to 0px inside an auto-height modal!)
          scroll()
            .w_full()
            .max_h(px(460.0))
            .px_5()
            .py_4()
            .child(if destinations.is_empty() {
              view()
                .w_full()
                .py_4()
                .child(
                  text("当前没有其他目标工作区，请先在左上角工作区切换器中新建一个工作区。")
                    .text_size(px(13.5))
                    .text_color(move || palette(dark_mode.get()).text_muted),
                )
            } else {
              let dests_for_list = destinations.clone();
              let select_ws = select_workspace.clone();
              let select_p = select_pick.clone();

              view()
                .w_full()
                .gap_3()
                .child(
                  // Workspace picker label + filter box + compact list
                  view()
                    .w_full()
                    .gap_2()
                    .child(
                      text("目标工作区")
                        .text_size(px(12.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || palette(dark_mode.get()).text_muted),
                    )
                    .child(
                      view()
                        .w_full()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .h(px(34.0))
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(move || palette(dark_mode.get()).border_normal)
                        .bg(move || palette(dark_mode.get()).surface_base)
                        .child(dynamic(move || {
                          mica_icons::search(palette(dark_mode.get()).text_faint_hex)
                        }))
                        .child(
                          input()
                            .test_id("transfer-workspace-filter")
                            .placeholder("输入名称筛选")
                            .flex_1()
                            .text_size(px(13.0))
                            .text_color(move || palette(dark_mode.get()).text_primary)
                            .bg(move || palette(dark_mode.get()).surface_base)
                            .value(move || ws_filter.get())
                            .on_change(move |v: &str| ws_filter.set(v.to_owned())),
                        ),
                    )
                    .child(
                      scroll()
                        .w_full()
                        .max_h(px(132.0))
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(move || palette(dark_mode.get()).border_normal)
                        .bg(move || palette(dark_mode.get()).surface_base)
                        .p_1()
                        .child(dynamic(move || {
                          let q = ws_filter.get().trim().to_lowercase();
                          let active_id = selected_dest.get();
                          let p = palette(dark_mode.get());
                          let mut col = view().w_full().gap_1();
                          let mut matched = 0usize;
                          for d in &dests_for_list {
                            if !q.is_empty() && !d.name.to_lowercase().contains(&q) {
                              continue;
                            }
                            matched += 1;
                            let is_sel = active_id == Some(d.id);
                            let id = d.id;
                            let on_pick = select_ws.clone();
                            col = col.child(
                              button()
                                .test_id(&format!("transfer-dest-ws-{}", d.id))
                                .accessibility_label(format!("选择目标工作区 {}", d.name))
                                .w_full()
                                .h(px(32.0))
                                .px_2()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .rounded(px(6.0))
                                .bg(if is_sel {
                                  p.surface_hover
                                } else {
                                  p.surface_base
                                })
                                .hover(move |s| {
                                  s.bg(palette(dark_mode.get_untracked()).surface_hover)
                                })
                                .on_press(move || on_pick(id))
                                .child(
                                  text(d.name.clone())
                                    .flex_1()
                                    .text_size(px(13.0))
                                    .font_weight(if is_sel {
                                      FontWeight::MEDIUM
                                    } else {
                                      FontWeight::NORMAL
                                    })
                                    .text_color(p.text_primary)
                                    .truncate(),
                                )
                                .child(when(
                                  move || is_sel,
                                  move || mica_icons::check_icon(p.accent_hex),
                                )),
                            );
                          }
                          if matched == 0 {
                            col = col.child(
                              view().py_3().items_center().child(
                                text("没有匹配的工作区")
                                  .text_size(px(12.0))
                                  .text_color(p.text_faint),
                              ),
                            );
                          }
                          col
                        })),
                    ),
                )
                .child(when(
                  move || selected_dest.get().is_some() && !dest_tree.get().is_empty(),
                  {
                    let sp = select_p.clone();
                    move || {
                      destination_tree_picker(
                        dest_tree,
                        tree_filter,
                        expanded_folders,
                        selection,
                        dark_mode,
                        sp.clone(),
                      )
                    }
                  },
                ))
                .child(dynamic(move || {
                  let p = palette(dark_mode.get());
                  let mut block = view().w_full().gap_2();
                  if let Some(rep) = preview.get() {
                    let summary = if is_copy {
                      format!(
                        "将复制 {} 篇文档、{} 个文件夹、{} 张图片",
                        rep.documents, rep.folders, rep.images
                      )
                    } else {
                      format!(
                        "将移动 {} 篇文档、{} 个文件夹、{} 张图片",
                        rep.documents, rep.folders, rep.images
                      )
                    };
                    block = block.child(
                      text(summary)
                        .test_id("transfer-preview-summary")
                        .text_size(px(13.0))
                        .text_color(p.text_primary),
                    );
                    if rep.dangling_links > 0 {
                      block = block.child(
                        view()
                          .test_id("transfer-dangling-warning")
                          .w_full()
                          .px_3()
                          .py_2()
                          .rounded(px(6.0))
                          .bg(p.warning_wash)
                          .border_1()
                          .border_color(p.warning)
                          .flex_row()
                          .items_center()
                          .gap_2()
                          .child(mica_icons::link_icon(p.warning_hex))
                          .child(
                            text(format!(
                              "有 {} 处指向源工作区其他页面的链接会失效",
                              rep.dangling_links
                            ))
                            .flex_1()
                            .text_size(px(12.0))
                            .text_color(p.warning),
                          ),
                      );
                    }
                  }
                  if !is_copy {
                    block = block.child(
                      view()
                        .test_id("transfer-version-notice")
                        .w_full()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(mica_icons::info_icon(p.text_muted_hex))
                        .child(
                          text("版本历史不会跟随（移动后新建，原检查点留在源）。")
                            .flex_1()
                            .text_size(px(12.0))
                            .text_color(p.text_muted),
                        ),
                    );
                  }
                  if let Some(err) = error_msg.get() {
                    block = block.child(
                      view()
                        .w_full()
                        .px_3()
                        .py_2()
                        .rounded(px(6.0))
                        .bg(p.danger_wash)
                        .child(text(err).text_size(px(12.0)).text_color(p.danger)),
                    );
                  }
                  block
                }))
            }),
        )
        .child(
          // Footer action bar
          view()
            .px_5()
            .py_3()
            .flex_row()
            .items_center()
            .justify_end()
            .gap_2()
            .border_t_1()
            .border_color(move || palette(dark_mode.get()).border_subtle)
            .child(
              button()
                .test_id("transfer-cancel-btn")
                .accessibility_label("取消")
                .px_4()
                .py_2()
                .rounded(px(6.0))
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                .on_press(move || close_cancel())
                .child(
                  text("取消")
                    .text_size(px(13.0))
                    .text_color(move || palette(dark_mode.get()).text_muted),
                ),
            )
            .child(
              button()
                .test_id("transfer-confirm-btn")
                .accessibility_label(confirm_label)
                .px_4()
                .py_2()
                .rounded(px(6.0))
                .bg(move || {
                  let p = palette(dark_mode.get());
                  if selected_dest.get().is_some() {
                    p.accent
                  } else {
                    p.surface_sunken
                  }
                })
                .on_press(move || on_confirm_press())
                .child(
                  text(confirm_label)
                    .text_size(px(13.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || {
                      let p = palette(dark_mode.get());
                      if selected_dest.get().is_some() {
                        p.on_primary
                      } else {
                        p.text_ghost
                      }
                    }),
                ),
            ),
        ),
    )
}

/// 240px-tall `DestinationTreePicker` matching Flutter `destination_tree_picker.dart`.
fn destination_tree_picker(
  dest_tree: Signal<Vec<TransferDestinationNode>>,
  tree_filter: Signal<String>,
  expanded_folders: Signal<HashSet<Uuid>>,
  selection: Signal<DestinationPick>,
  dark_mode: Signal<bool>,
  on_select: Rc<dyn Fn(DestinationPick)>,
) -> impl IntoElement {
  let on_select_tree = on_select.clone();
  let on_select_bar = on_select;

  view()
    .test_id("destination-tree-picker")
    .w_full()
    .gap_2()
    .child(
      text("目标位置（可选）")
        .text_size(px(12.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(move || palette(dark_mode.get()).text_muted),
    )
    .child(
      view()
        .w_full()
        .flex_row()
        .items_center()
        .gap_2()
        .px_2()
        .h(px(32.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(move || palette(dark_mode.get()).border_normal)
        .bg(move || palette(dark_mode.get()).surface_base)
        .child(dynamic(move || {
          mica_icons::search(palette(dark_mode.get()).text_faint_hex)
        }))
        .child(
          input()
            .test_id("destination-tree-filter")
            .placeholder("输入名称筛选目录或页面")
            .flex_1()
            .text_size(px(12.5))
            .text_color(move || palette(dark_mode.get()).text_primary)
            .bg(move || palette(dark_mode.get()).surface_base)
            .value(move || tree_filter.get())
            .on_change(move |v: &str| tree_filter.set(v.to_owned())),
        ),
    )
    .child(
      // 240px scrollable tree container
      scroll()
        .w_full()
        .h(px(240.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(move || palette(dark_mode.get()).border_normal)
        .bg(move || palette(dark_mode.get()).surface_base)
        .p_1()
        .child(dynamic(move || {
          let nodes = dest_tree.get();
          let q = tree_filter.get();
          let filtering = !q.trim().is_empty();
          let exp = expanded_folders.get();
          let sel = selection.get();
          let rows = compute_destination_rows(&nodes, &exp, &q);
          let p = palette(dark_mode.get());
          let is_root_end = sel.parent_view_id.is_none()
            && sel.before_view_id.is_none()
            && sel.after_view_id.is_none();

          let mut list = view().w_full().gap_1();

          if !filtering {
            let pick_root = on_select_tree.clone();
            list = list.child(
              button()
                .test_id("destination-root")
                .accessibility_label("工作区根目录")
                .w_full()
                .h(px(30.0))
                .px_2()
                .flex_row()
                .items_center()
                .gap_2()
                .rounded(px(6.0))
                .bg(if is_root_end {
                  p.surface_hover
                } else {
                  p.surface_base
                })
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                .on_press(move || pick_root(DestinationPick::root()))
                .child(mica_icons::folder(p.text_muted_hex))
                .child(
                  text("工作区根目录")
                    .flex_1()
                    .text_size(px(13.0))
                    .font_weight(if is_root_end {
                      FontWeight::MEDIUM
                    } else {
                      FontWeight::NORMAL
                    })
                    .text_color(p.text_primary)
                    .truncate(),
                )
                .child(when(
                  move || is_root_end,
                  move || mica_icons::check_icon(p.text_primary_hex),
                )),
            );
          }

          for row in rows {
            let v = row.node;
            let is_inside = v.is_folder
              && sel.parent_view_id == Some(v.id)
              && sel.before_view_id.is_none()
              && sel.after_view_id.is_none();
            let is_before = sel.before_view_id == Some(v.id);
            let is_after = sel.after_view_id == Some(v.id);
            let is_selected = is_inside || is_before || is_after;
            let indent_px = 4.0 + (row.depth + 1) as f32 * 16.0;
            let node_id = v.id;
            let node_parent = v.parent_view_id;
            let is_folder = v.is_folder;
            let has_children = row.has_children;
            let is_open = filtering || row.expanded;
            let on_row_tap = on_select_tree.clone();

            let mut row_wrap = view().w_full();
            if is_before {
              row_wrap = row_wrap.child(
                view()
                  .pl(px(indent_px))
                  .pr_2()
                  .child(view().w_full().h(px(2.0)).rounded(px(1.0)).bg(p.text_primary)),
              );
            }

            let chevron_slot = if is_folder && has_children && !filtering {
              view().w(px(20.0)).items_center().justify_center().child(
                button()
                  .test_id(&format!("destination-toggle-{node_id}"))
                  .accessibility_label("展开或收起文件夹")
                  .w(px(18.0))
                  .h(px(18.0))
                  .items_center()
                  .justify_center()
                  .rounded(px(4.0))
                  .on_press(move || {
                    let mut next = expanded_folders.get_untracked();
                    if !next.remove(&node_id) {
                      next.insert(node_id);
                    }
                    expanded_folders.set(next);
                  })
                  .child(if is_open {
                    mica_icons::chevron_down(p.text_faint_hex)
                  } else {
                    mica_icons::chevron_right(p.text_faint_hex)
                  }),
              )
            } else {
              view().w(px(20.0))
            };

            let icon_el = if is_folder {
              mica_icons::folder(p.text_muted_hex)
            } else {
              mica_icons::file_doc(p.text_muted_hex)
            };

            row_wrap = row_wrap.child(
              button()
                .test_id(&format!("destination-{node_id}"))
                .accessibility_label(v.name.clone())
                .w_full()
                .h(px(30.0))
                .pl(px(indent_px))
                .pr_2()
                .flex_row()
                .items_center()
                .gap_1()
                .rounded(px(6.0))
                .bg(if is_selected {
                  p.surface_hover
                } else {
                  p.surface_base
                })
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                .on_press(move || {
                  if is_folder {
                    if has_children && !filtering {
                      let mut next = expanded_folders.get_untracked();
                      if next.insert(node_id) {
                        expanded_folders.set(next);
                      }
                    }
                    on_row_tap(DestinationPick::inside_folder(node_id));
                  } else {
                    on_row_tap(DestinationPick::after_node(node_parent, node_id));
                  }
                })
                .child(chevron_slot)
                .child(icon_el)
                .child(
                  text(v.name)
                    .flex_1()
                    .text_size(px(13.0))
                    .font_weight(if is_selected {
                      FontWeight::MEDIUM
                    } else {
                      FontWeight::NORMAL
                    })
                    .text_color(p.text_primary)
                    .truncate(),
                )
                .child(when(
                  move || is_inside,
                  move || mica_icons::check_icon(p.text_primary_hex),
                ))
                .child(when(
                  move || is_before,
                  move || text("↑").text_size(px(12.0)).text_color(p.text_primary),
                ))
                .child(when(
                  move || is_after,
                  move || text("↓").text_size(px(12.0)).text_color(p.text_primary),
                )),
            );

            if is_after {
              row_wrap = row_wrap.child(
                view()
                  .pl(px(indent_px))
                  .pr_2()
                  .child(view().w_full().h(px(2.0)).rounded(px(1.0)).bg(p.text_primary)),
              );
            }

            list = list.child(row_wrap);
          }

          list
        })),
    )
    .child(dynamic(move || {
      // Placement segment bar matching `destination_tree_picker.dart`
      let nodes = dest_tree.get();
      let sel = selection.get();
      let p = palette(dark_mode.get());
      let by_id: HashMap<Uuid, &TransferDestinationNode> = nodes.iter().map(|n| (n.id, n)).collect();

      if sel.placement != TargetPlacement::RootTop
        && let Some(anchor_id) = sel.before_view_id.or(sel.after_view_id)
        && let Some(anchor) = by_id.get(&anchor_id)
      {
            let is_folder = anchor.is_folder;
            let anchor_parent = anchor.parent_view_id;
            let mut bar = view()
              .test_id("destination-placement-bar")
              .w_full()
              .flex_row()
              .gap_1()
              .p_1()
              .rounded(px(8.0))
              .bg(p.surface_sunken);

            if is_folder {
              let cb = on_select_bar.clone();
              bar = bar.child(placement_chip(
                "placement-inside",
                "放入文件夹内",
                false,
                dark_mode,
                move || cb(DestinationPick::inside_folder(anchor_id)),
              ));
            }
            let cb_b = on_select_bar.clone();
            let cb_a = on_select_bar.clone();
            return bar
              .child(placement_chip(
                "placement-before",
                "插在此项之前",
                sel.before_view_id == Some(anchor_id),
                dark_mode,
                move || cb_b(DestinationPick::before_node(anchor_parent, anchor_id)),
              ))
              .child(placement_chip(
                "placement-after",
                "插在此项之后",
                sel.after_view_id == Some(anchor_id),
                dark_mode,
                move || cb_a(DestinationPick::after_node(anchor_parent, anchor_id)),
              ));
      }

      if let Some(folder_id) = sel.parent_view_id
        && let Some(folder) = by_id.get(&folder_id)
      {
          let folder_parent = folder.parent_view_id;
          let cb_i = on_select_bar.clone();
          let cb_b = on_select_bar.clone();
          let cb_a = on_select_bar.clone();
          return view()
            .test_id("destination-placement-bar")
            .w_full()
            .flex_row()
            .gap_1()
            .p_1()
            .rounded(px(8.0))
            .bg(p.surface_sunken)
            .child(placement_chip(
              "placement-inside",
              "放入文件夹内",
              true,
              dark_mode,
              move || cb_i(DestinationPick::inside_folder(folder_id)),
            ))
            .child(placement_chip(
              "placement-before",
              "插在此项之前",
              false,
              dark_mode,
              move || cb_b(DestinationPick::before_node(folder_parent, folder_id)),
            ))
            .child(placement_chip(
              "placement-after",
              "插在此项之后",
              false,
              dark_mode,
              move || cb_a(DestinationPick::after_node(folder_parent, folder_id)),
            ));
      }

      if let Some(first_root) = first_root_node(&nodes) {
        let first_id = first_root.id;
        let is_root_top = sel.placement == TargetPlacement::RootTop;
        let cb_end = on_select_bar.clone();
        let cb_top = on_select_bar.clone();
        return view()
          .test_id("destination-placement-bar")
          .w_full()
          .flex_row()
          .gap_1()
          .p_1()
          .rounded(px(8.0))
          .bg(p.surface_sunken)
          .child(placement_chip(
            "placement-root-end",
            "根目录末尾",
            !is_root_top,
            dark_mode,
            move || cb_end(DestinationPick::root()),
          ))
          .child(placement_chip(
            "placement-root-top",
            "根目录顶部",
            is_root_top,
            dark_mode,
            move || cb_top(DestinationPick::root_top(first_id)),
          ));
      }

      view()
    }))
}

fn placement_chip(
  tid: &'static str,
  label: &'static str,
  selected: bool,
  dark_mode: Signal<bool>,
  on_press: impl Fn() + 'static,
) -> impl IntoElement {
  let p = palette(dark_mode.get_untracked());
  button()
    .test_id(tid)
    .accessibility_label(label)
    .flex_1()
    .h(px(28.0))
    .items_center()
    .justify_center()
    .rounded(px(6.0))
    .bg(if selected {
      p.surface_base
    } else {
      p.surface_sunken
    })
    .on_press(on_press)
    .child(
      text(label)
        .text_size(px(12.0))
        .font_weight(if selected {
          FontWeight::MEDIUM
        } else {
          FontWeight::NORMAL
        })
        .text_color(if selected {
          p.text_primary
        } else {
          p.text_muted
        }),
    )
}

#[cfg(test)]
mod tests {
  use super::*;

  fn mk_node(
    name: &str,
    parent: Option<Uuid>,
    is_folder: bool,
    position: &str,
  ) -> TransferDestinationNode {
    TransferDestinationNode {
      id: Uuid::new_v4(),
      parent_view_id: parent,
      name: name.to_string(),
      icon: None,
      is_folder,
      position: position.to_string(),
    }
  }

  #[test]
  fn destination_tree_filter_auto_expands_matching_ancestors() {
    let f1 = mk_node("工程", None, true, "a0");
    let f2 = mk_node("后端", Some(f1.id), true, "a0");
    let doc = mk_node("数据库迁移规范", Some(f2.id), false, "a0");
    let other = mk_node("市场方案", None, false, "a1");
    let nodes = vec![f1.clone(), f2.clone(), doc.clone(), other.clone()];

    // Collapsed by default -> only root items visible
    let empty_exp = HashSet::new();
    let collapsed = compute_destination_rows(&nodes, &empty_exp, "");
    assert_eq!(collapsed.len(), 2);
    assert_eq!(collapsed[0].node.id, f1.id);
    assert!(collapsed[0].has_children);
    assert_eq!(collapsed[1].node.id, other.id);

    // Filtering matches leaf and auto-expands both parent folders
    let filtered = compute_destination_rows(&nodes, &empty_exp, "迁移");
    assert_eq!(filtered.len(), 3);
    assert_eq!(filtered[0].node.id, f1.id);
    assert_eq!(filtered[0].depth, 0);
    assert_eq!(filtered[1].node.id, f2.id);
    assert_eq!(filtered[1].depth, 1);
    assert_eq!(filtered[2].node.id, doc.id);
    assert_eq!(filtered[2].depth, 2);

    assert_eq!(first_root_node(&nodes).map(|n| n.id), Some(f1.id));
  }

  #[test]
  fn destination_pick_constructors_set_expected_placement() {
    let f = Uuid::new_v4();
    let p = Uuid::new_v4();
    assert_eq!(DestinationPick::root().placement, TargetPlacement::RootEnd);
    assert_eq!(
      DestinationPick::root_top(p).placement,
      TargetPlacement::RootTop
    );
    assert_eq!(
      DestinationPick::inside_folder(f).placement,
      TargetPlacement::Inside
    );
    let before = DestinationPick::before_node(Some(f), p);
    assert_eq!(before.placement, TargetPlacement::Before);
    assert_eq!(before.before_view_id, Some(p));
    let after = DestinationPick::after_node(Some(f), p);
    assert_eq!(after.placement, TargetPlacement::After);
    assert_eq!(after.after_view_id, Some(p));
  }
}
