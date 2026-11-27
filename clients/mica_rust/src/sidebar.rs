//! Mica L3 Workspace Sidebar (1:1 parity with Mica Flutter).
//!
//! - Reactive width (`220..=480px`, default `280px`) with right-edge drag-to-resize handle.
//! - Top header row: `WorkspaceSelector` + `关系图` (`open-graph-btn`) + `工作区设置` (`open-ws-settings-btn`).
//! - Recessed search pill (`搜索...  Ctrl+Shift+F`).
//! - Favorites section (`max_h(180px)` scroll, `30px` rows, `20px` left indent, Emoji/doc icon, right-click `取消收藏`).
//! - Virtualized page tree (`30px` rows, `2 + depth*16` indent, fixed `18px` chevron slot, `18×18` Emoji/folder/page icon, `14px` title):
//!   - Multi-select (`Shift+Click` same-parent range, `Ctrl/Cmd+Click` toggle, Esc/empty-area clear)
//!   - Double-click page row → `400px` centered `RenameDialog`
//!   - Right-click `.on_context_menu` AND hover `⋯` floating menus (Multi-select / Page / Folder)
//!   - Built-in `360×420` Emoji Picker modal & `400px` Rename modal
//!   - Mouse drag-and-drop reordering & reparenting (4px threshold, 30%/40%/30% Before/Into/After,
//!     horizontal reparenting, `DropIndicator`, 2px+8% `Into` highlight, single/deck ghost card, root drop zone)
//! - Quiet collapsible Recycle Bin (`回收站 (N)`).
//! - Account footer (`28×28` avatar `本` + `本地工作区`, plus `⚙` settings button).
//! - Exported `workspace_switcher_popover` (`320px`) & `rename_dialog_modal` (`400px`).

use std::{cell::RefCell, rc::Rc, time::Duration};

use mica_client::{PageTreeModel, TreeRow};
use nsh_base::{HandleEdge, ResizeHandleState, button, input, resize_handle};
use nsh_core::{
  AccessibilityRole, ContextMenuEvent, MouseButton, MouseEvent, PointerEvents, PressEvent,
  SubmitBehavior,
  color::Rgba,
  element::{Align, ElementRef, Placement, element_ref, overlay, set_interval},
  geometry::{Axis, Bounds, Point, Points, point},
  keymap::When,
  prelude::*,
};
use uuid::Uuid;

use crate::{
  emoji_picker::emoji_picker_modal,
  mica_icons,
  mica_theme::palette,
  tree_dnd::{
    ClickIntent, DropIndicatorState, DropMode, TREE_ROW_HEIGHT, TREE_ROW_PITCH, TreeDropPlanner,
    TreeSelection, compute_next_scroll_y, drop_indicator_inset, independent_selection, renamed_to,
    tree_click_intent, tree_drag_autoscroll_speed, tree_drag_row_at_y,
  },
};

pub const SIDEBAR_MIN_WIDTH: f32 = 220.0;
pub const SIDEBAR_MAX_WIDTH: f32 = 480.0;
pub const SIDEBAR_DEFAULT_WIDTH: f32 = 280.0;

const DRAG_THRESHOLD_PX: f32 = 4.0;
const WS_FILTER_THRESHOLD: usize = 8;

pub type OpenRenameModalCallback = Rc<dyn Fn(Uuid, String)>;
pub type OpenEmojiModalCallback = Rc<dyn Fn(Uuid, Option<String>)>;

/// Callbacks bridging sidebar user interactions back to the application shell.
///
/// Preserves all 12 original fields and extends cleanly with Domain 2 1:1 parity hooks.
pub struct SidebarCallbacks {
  // Existing fields (preserved 100%)
  pub on_select_doc: Rc<dyn Fn(Uuid, String)>,
  pub on_new_page: Rc<dyn Fn()>,
  pub on_new_folder: Rc<dyn Fn()>,
  pub on_rename_view: Rc<dyn Fn(Uuid, String)>,
  pub on_trash_view: Rc<dyn Fn(Uuid)>,
  pub on_move_view: Rc<dyn Fn(Uuid, i32)>,
  pub on_open_board: Rc<dyn Fn()>,
  pub on_restore_view: Rc<dyn Fn(Uuid)>,
  pub on_open_search: Rc<dyn Fn()>,
  pub on_open_settings: Rc<dyn Fn()>,
  pub on_toggle_workspace_menu: Rc<dyn Fn()>,
  pub on_toggle_account_menu: Rc<dyn Fn()>,
  // Domain 2 1:1 parity callbacks
  pub on_open_graph: Rc<dyn Fn()>,
  pub on_open_workspace_settings: Rc<dyn Fn()>,
  pub on_remove_favorite: Rc<dyn Fn(Uuid)>,
  pub on_open_in_new_tab: Rc<dyn Fn(Uuid, String)>,
  pub on_new_child_page: Rc<dyn Fn(Uuid)>,
  pub on_new_child_folder: Rc<dyn Fn(Uuid)>,
  pub on_set_icon: Rc<dyn Fn(Uuid, Option<String>)>,
  pub on_duplicate_view: Rc<dyn Fn(Uuid)>,
  pub on_export_folder_backup: Rc<dyn Fn(Uuid)>,
  pub on_export_folder_markdown: Rc<dyn Fn(Uuid)>,
  pub on_export_page_markdown: Rc<dyn Fn(Uuid)>,
  pub on_export_page_html: Rc<dyn Fn(Uuid)>,
  pub on_import_files_into_folder: Rc<dyn Fn(Uuid)>,
  pub on_import_folder_into_folder: Rc<dyn Fn(Uuid)>,
  pub on_open_rename_modal: Option<OpenRenameModalCallback>,
  pub on_open_emoji_modal: Option<OpenEmojiModalCallback>,
  pub on_transfer_move: Rc<dyn Fn(Vec<Uuid>)>,
  pub on_transfer_copy: Rc<dyn Fn(Vec<Uuid>)>,
  pub on_batch_trash: Rc<dyn Fn(Vec<Uuid>)>,
  pub on_reorder_views: Rc<dyn Fn(Option<Uuid>, Vec<Uuid>)>,
}

impl Default for SidebarCallbacks {
  fn default() -> Self {
    let noop0 = || Rc::new(|| {}) as Rc<dyn Fn()>;
    let noop_u = || Rc::new(|_: Uuid| {}) as Rc<dyn Fn(Uuid)>;
    let noop_us = || Rc::new(|_: Uuid, _: String| {}) as Rc<dyn Fn(Uuid, String)>;
    let noop_v = || Rc::new(|_: Vec<Uuid>| {}) as Rc<dyn Fn(Vec<Uuid>)>;
    Self {
      on_select_doc: noop_us(), on_new_page: noop0(), on_new_folder: noop0(),
      on_rename_view: noop_us(), on_trash_view: noop_u(),
      on_move_view: Rc::new(|_, _| {}), on_open_board: noop0(), on_restore_view: noop_u(),
      on_open_search: noop0(), on_open_settings: noop0(),
      on_toggle_workspace_menu: noop0(), on_toggle_account_menu: noop0(),
      on_open_graph: noop0(), on_open_workspace_settings: noop0(),
      on_remove_favorite: noop_u(), on_open_in_new_tab: noop_us(),
      on_new_child_page: noop_u(), on_new_child_folder: noop_u(),
      on_set_icon: Rc::new(|_, _| {}), on_duplicate_view: noop_u(),
      on_export_folder_backup: noop_u(), on_export_folder_markdown: noop_u(),
      on_export_page_markdown: noop_u(), on_export_page_html: noop_u(),
      on_import_files_into_folder: noop_u(), on_import_folder_into_folder: noop_u(),
      on_open_rename_modal: None, on_open_emoji_modal: None,
      on_transfer_move: noop_v(), on_transfer_copy: noop_v(), on_batch_trash: noop_v(),
      on_reorder_views: Rc::new(|_, _| {}),
    }
  }
}

/// Workspace item displayed in the `320px` [`workspace_switcher_popover`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceSwitcherItem {
  pub id: Uuid,
  pub name: String,
  pub is_local: bool,
  pub page_count: usize,
}

/// Callbacks for [`workspace_switcher_popover`].
pub struct WorkspaceSwitcherCallbacks {
  pub on_select: Rc<dyn Fn(Uuid)>,
  pub on_create: Rc<dyn Fn(String)>,
  pub on_rename: Rc<dyn Fn(Uuid, String)>,
  pub on_export_backup: Rc<dyn Fn(Uuid)>,
  pub on_export_markdown: Rc<dyn Fn(Uuid)>,
  pub on_import_files: Rc<dyn Fn(Uuid)>,
  pub on_import_folder: Rc<dyn Fn(Uuid)>,
  pub on_delete: Rc<dyn Fn(Uuid)>,
  pub on_import_workspace_zip: Rc<dyn Fn()>,
  pub on_import_workspace_notion: Rc<dyn Fn()>,
  pub on_close: Rc<dyn Fn()>,
}

impl Default for WorkspaceSwitcherCallbacks {
  fn default() -> Self {
    let noop0 = || Rc::new(|| {}) as Rc<dyn Fn()>;
    let noop_u = || Rc::new(|_: Uuid| {}) as Rc<dyn Fn(Uuid)>;
    Self {
      on_select: noop_u(), on_create: Rc::new(|_| {}), on_rename: Rc::new(|_, _| {}),
      on_export_backup: noop_u(), on_export_markdown: noop_u(),
      on_import_files: noop_u(), on_import_folder: noop_u(), on_delete: noop_u(),
      on_import_workspace_zip: noop0(), on_import_workspace_notion: noop0(), on_close: noop0(),
    }
  }
}

// ── Internal State ────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
enum ContextMenuTarget {
  TreeRow {
    view_id: Uuid,
    object_id: Uuid,
    name: String,
    icon: Option<String>,
    is_folder: bool,
    expanded: bool,
  },
  Batch { ids: Vec<Uuid> },
  Favorite { doc_id: Uuid },
}

#[derive(Clone, Debug, PartialEq)]
struct ContextMenuState {
  target: ContextMenuTarget,
  x: f32,
  y: f32,
}

#[derive(Clone, Debug, PartialEq)]
struct PendingDrag {
  source_view_id: Uuid,
  start_page_x: f32,
  start_page_y: f32,
}

#[derive(Clone, Debug, PartialEq)]
enum DragTarget {
  Row {
    target_idx: usize,
    target_view_id: Uuid,
    mode: DropMode,
    level: usize,
    allowed: bool,
  },
  Root,
}

#[derive(Clone, Debug, PartialEq)]
struct ActiveDrag {
  dragged_ids: Vec<Uuid>,
  lead_name: String,
  lead_icon: Option<String>,
  lead_is_folder: bool,
  target: Option<DragTarget>,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct DragState {
  pending: Option<PendingDrag>,
  active: Option<ActiveDrag>,
}

struct TreeRowContext {
  tree: Rc<RefCell<PageTreeModel>>,
  tree_rev: Signal<u64>,
  active_doc: Signal<Option<Uuid>>,
  dark_mode: Signal<bool>,
  sidebar_hovered: Signal<bool>,
  selection: Signal<TreeSelection>,
  context_menu: Signal<Option<ContextMenuState>>,
  page_to_local: Signal<(f32, f32)>,
  drag_state: Signal<DragState>,
  drag_pointer: Signal<(f32, f32)>,
  drag_page_pointer: Signal<(f32, f32)>,
  drag_autoscroll_speed: Signal<f32>,
  tree_list_ref: ElementRef,
  tree_scroll_y: Signal<f32>,
  tree_content_h: Signal<f32>,
  tree_viewport_h: Signal<f32>,
  tree_page_origin: Signal<(f32, f32)>,
  just_dragged: Signal<bool>,
  rename_modal_open: Signal<bool>,
  rename_modal_view: Signal<Option<Uuid>>,
  rename_modal_initial: Signal<String>,
  emoji_modal_open: Signal<bool>,
  emoji_modal_view: Signal<Option<Uuid>>,
  emoji_modal_current: Signal<Option<String>>,
  cb: Rc<SidebarCallbacks>,
}

// ── Main Sidebar View ─────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub fn sidebar_view(
  tree: Rc<RefCell<PageTreeModel>>,
  tree_rev: Signal<u64>,
  active_doc: Signal<Option<Uuid>>,
  favorite_pages: Signal<Vec<(Uuid, String)>>,
  sidebar_width: Signal<f32>,
  workspace_name: Signal<String>,
  account_name: Signal<String>,
  dark_mode: Signal<bool>,
  cb: Rc<SidebarCallbacks>,
) -> impl IntoElement {
  let (show_favorites, show_pages, sidebar_hovered) = (signal(true), signal(true), signal(false));
  let selection: Signal<TreeSelection> = signal(TreeSelection::new());
  let context_menu: Signal<Option<ContextMenuState>> = signal(None);
  let page_to_local: Signal<(f32, f32)> = signal((0.0_f32, 36.0_f32));
  let drag_state: Signal<DragState> = signal(DragState::default());
  let drag_pointer: Signal<(f32, f32)> = signal((0.0_f32, 0.0_f32));
  let drag_page_pointer: Signal<(f32, f32)> = signal((0.0_f32, 0.0_f32));
  let drag_autoscroll_speed: Signal<f32> = signal(0.0_f32);
  let tree_list_ref = element_ref();
  let tree_scroll_y: Signal<f32> = signal(0.0_f32);
  let tree_content_h: Signal<f32> = signal(0.0_f32);
  let tree_viewport_h: Signal<f32> = signal(0.0_f32);
  let tree_page_origin: Signal<(f32, f32)> = signal((0.0_f32, 0.0_f32));
  let just_dragged = signal(false);
  let (rename_modal_open, rename_modal_view, rename_modal_initial) =
    (signal(false), signal(None::<Uuid>), signal(String::new()));
  let (emoji_modal_open, emoji_modal_view, emoji_modal_current) =
    (signal(false), signal(None::<Uuid>), signal(None::<String>));

  // Esc closes context menu, cancels active drag, or clears multi-selection.
  effect(move || {
    let active_any = context_menu.get().is_some()
      || drag_state.get().active.is_some()
      || !selection.get().is_empty();
    if !active_any || rename_modal_open.get() || emoji_modal_open.get() { return; }
    on_key_with(
      "escape",
      KeyOptions { when: When::Always, repeat: false, pass_through: true, ..KeyOptions::default() },
      move |_| {
        if context_menu.get_untracked().is_some() {
          context_menu.set(None);
        } else if drag_state.get_untracked().active.is_some() {
          drag_autoscroll_speed.set(0.0);
          drag_state.set(DragState::default());
        } else {
          let mut s = selection.get_untracked();
          if s.clear() { selection.set(s); }
        }
      },
    );
  });

  {
    let tree_vis = tree.clone();
    effect(move || {
      tree_rev.get();
      let visible = tree_vis.borrow().visible_rows();
      let mut s = selection.get_untracked();
      if s.retain_visible(&visible) { selection.set(s); }
    });
  }

  let tree_ctx = Rc::new(TreeRowContext {
    tree: tree.clone(), tree_rev, active_doc, dark_mode, sidebar_hovered, selection,
    context_menu, page_to_local, drag_state, drag_pointer, drag_page_pointer,
    drag_autoscroll_speed, tree_list_ref, tree_scroll_y, tree_content_h, tree_viewport_h,
    tree_page_origin, just_dragged, rename_modal_open, rename_modal_view, rename_modal_initial,
    emoji_modal_open, emoji_modal_view, emoji_modal_current, cb: cb.clone(),
  });

  // Smooth 60fps edge auto-scroll while dragging tree rows near/past top or bottom edge.
  // Gated on a boolean memo so speed changes never reset the 16ms interval.
  {
    let ctx_auto = tree_ctx.clone();
    let autoscroll_active = computed(move || {
      drag_state.get().active.is_some() && drag_autoscroll_speed.get().abs() >= 0.1
    });
    effect(move || {
      if !autoscroll_active.get() {
        return;
      }
      let ctx_tick = ctx_auto.clone();
      let task = set_interval(Duration::from_millis(16), move || {
        let speed = ctx_tick.drag_autoscroll_speed.get_untracked();
        if speed.abs() < 0.1 || ctx_tick.drag_state.get_untracked().active.is_none() {
          ctx_tick.drag_autoscroll_speed.set(0.0);
          return;
        }
        let row_count = ctx_tick.tree.borrow().visible_rows().len();
        let cur_y = ctx_tick.tree_scroll_y.get_untracked();
        let ch = ctx_tick.tree_content_h.get_untracked();
        let vh = ctx_tick.tree_viewport_h.get_untracked();
        let next_y = compute_next_scroll_y(cur_y, speed, ch, vh, row_count);
        if (next_y - cur_y).abs() >= 0.5 {
          ctx_tick.tree_scroll_y.set(next_y);
          ctx_tick.tree_list_ref.scroll_to_offset(px(next_y), false);
          update_drag_target_after_autoscroll(&ctx_tick, next_y, ch, vh, row_count);
        }
      });
      cleanup(move || task.cancel());
    });
  }

  let (ctx_move, ctx_up, ctx_menu_overlay) = (tree_ctx.clone(), tree_ctx.clone(), tree_ctx.clone());
  let (cb_rename_submit, cb_emoji_pick) = (cb.clone(), cb.clone());

  view()
    .relative()
    .w(move || px(sidebar_width.get().clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH)))
    .h_full().flex_shrink_0().border_r_1()
    .border_color(move || palette(dark_mode.get()).border_subtle)
    .bg(move || palette(dark_mode.get()).surface_raised)
    .on_mouse_enter(move |_: &MouseEvent| sidebar_hovered.set(true))
    .on_mouse_leave(move |_: &MouseEvent| {
      sidebar_hovered.set(false);
      drag_autoscroll_speed.set(0.0);
    })
    .on_mouse_move(move |e: &MouseEvent| {
      ctx_move.page_to_local.set((e.page_position.x.0 - e.position.x.0, e.page_position.y.0 - e.position.y.0));
      let st = ctx_move.drag_state.get_untracked();
      if st.active.is_none() && st.pending.is_none() { return; }
      if e.pressed_button != Some(MouseButton::Left) {
        // Mouse was released outside the sidebar (so `on_mouse_up` here did not fire):
        // cancel any dangling drag cleanly rather than committing a stale drop.
        ctx_move.drag_autoscroll_speed.set(0.0);
        ctx_move.drag_state.set(DragState::default());
        return;
      }
      if st.active.is_some() {
        ctx_move.drag_pointer.set((e.position.x.0, e.position.y.0));
        update_drag_autoscroll(&ctx_move, e.page_position.x.0, e.page_position.y.0);
      } else if let Some(pending) = st.pending.clone() {
        let dx = e.page_position.x.0 - pending.start_page_x;
        let dy = e.page_position.y.0 - pending.start_page_y;
        if dx.hypot(dy) >= DRAG_THRESHOLD_PX {
          start_active_drag(
            &ctx_move,
            pending.source_view_id,
            e.position.x.0,
            e.position.y.0,
            e.page_position.x.0,
            e.page_position.y.0,
          );
        }
      }
    })
    .on_mouse_up(move |_: &MouseEvent| {
      ctx_up.drag_autoscroll_speed.set(0.0);
      let mut st = ctx_up.drag_state.get_untracked();
      if st.active.is_none() && st.pending.is_none() { return; }
      if let Some(active) = st.active.take() {
        ctx_up.drag_state.set(DragState::default());
        ctx_up.just_dragged.set(true);
        commit_tree_drop(&ctx_up, active);
      } else {
        ctx_up.drag_state.set(DragState::default());
      }
    })
    .child(workspace_header(workspace_name, dark_mode, cb.clone()))
    .child(search_pill(dark_mode, cb.clone()))
    .child(favorites_section(tree.clone(), tree_rev, favorite_pages, active_doc, show_favorites, dark_mode, context_menu, page_to_local, cb.clone()))
    .child(pages_section_header(show_pages, dark_mode, cb.clone()))
    .child(view().flex_1().min_h(px(0.0)).px_2().py_1().child(when(move || show_pages.get(), {
      let ctx = tree_ctx.clone();
      move || page_tree_area(ctx.clone())
    })))
    // Recycle Bin lives exclusively in `设置 → 回收站` (matching Flutter 1:1, hidden from sidebar footer).
    .child(account_footer(account_name, dark_mode, cb))
    .child(sidebar_resize_handle(sidebar_width, dark_mode))
    .child(context_menu_overlay(ctx_menu_overlay))
    .child(drag_ghost_overlay(drag_state, drag_pointer, dark_mode))
    .child(rename_dialog_modal(
      rename_modal_open, rename_modal_initial, "重命名", dark_mode,
      Rc::new(move |new_name: String| {
        if let Some(vid) = rename_modal_view.get_untracked() { (cb_rename_submit.on_rename_view)(vid, new_name); }
        rename_modal_open.set(false); rename_modal_view.set(None);
      }),
      Rc::new(move || { rename_modal_open.set(false); rename_modal_view.set(None); }),
    ))
    .child(emoji_picker_modal(
      emoji_modal_open, emoji_modal_current, dark_mode,
      Rc::new(move |picked: Option<String>| {
        if let Some(vid) = emoji_modal_view.get_untracked() { (cb_emoji_pick.on_set_icon)(vid, picked); }
        emoji_modal_open.set(false); emoji_modal_view.set(None);
      }),
      Rc::new(move || { emoji_modal_open.set(false); emoji_modal_view.set(None); }),
    ))
}

// ── Header Row: WorkspaceSelector + 关系图 + 工作区设置 ────────────────────────

fn workspace_header(
  workspace_name: Signal<String>,
  dark_mode: Signal<bool>,
  cb: Rc<SidebarCallbacks>,
) -> impl IntoElement {
  let cb_ws = cb.clone();
  let cb_graph = cb.clone();
  let cb_ws_set = cb;

  view()
    .h(px(40.0)).px_2().flex_row().items_center().justify_between().gap_1()
    .child(
      button()
        .test_id("workspace-switcher-btn")
        .accessibility_label("切换或新建工作区")
        .flex_1().min_w(px(0.0)).h(px(32.0)).px(px(6.0))
        .flex_row().items_center().gap_2().rounded(px(6.0))
        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
        .on_press(move || (cb_ws.on_toggle_workspace_menu)())
        .child(
          view()
            .w(px(22.0)).h(px(22.0)).flex_shrink_0().rounded(px(6.0))
            .items_center().justify_center()
            .bg(move || palette(dark_mode.get()).surface_hover)
            .child(dynamic(move || mica_icons::cloud(palette(dark_mode.get()).text_primary_hex))),
        )
        .child(
          text(move || workspace_name.get())
            .flex_1().text_size(px(14.0)).font_weight(FontWeight::MEDIUM)
            .text_color(move || palette(dark_mode.get()).text_primary)
            .truncate(),
        )
        .child(dynamic(move || {
          mica_icons::chevrons_up_down(palette(dark_mode.get()).text_faint_hex)
        })),
    )
    .child(
      view().flex_row().items_center().gap_1().flex_shrink_0()
        .child(crate::tooltip::with_tooltip(
          button()
            .test_id("open-graph-btn").accessibility_label("关系图")
            .w(px(28.0)).h(px(28.0)).items_center().justify_center().rounded(px(6.0))
            .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
            .on_press(move || {
              crate::tooltip::hide_tooltip();
              (cb_graph.on_open_graph)();
            })
            .child(dynamic(move || {
              mica_icons::graph_nodes(palette(dark_mode.get()).text_muted_hex)
            })),
          "关系图谱",
        ))
        .child(crate::tooltip::with_tooltip(
          button()
            .test_id("open-ws-settings-btn").accessibility_label("工作区设置")
            .w(px(28.0)).h(px(28.0)).items_center().justify_center().rounded(px(6.0))
            .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
            .on_press(move || {
              crate::tooltip::hide_tooltip();
              (cb_ws_set.on_open_workspace_settings)();
            })
            .child(dynamic(move || {
              mica_icons::settings_gear(palette(dark_mode.get()).text_muted_hex)
            })),
          "工作区设置",
        )),
    )
}

// ── Search Pill ───────────────────────────────────────────────────────────────

fn search_pill(dark_mode: Signal<bool>, cb: Rc<SidebarCallbacks>) -> impl IntoElement {
  view().px_3().pb_2().child(
    button()
      .test_id("open-search-btn").accessibility_label("搜索工作区")
      .w_full().h(px(30.0)).px_2().flex_row().items_center().justify_between().rounded(px(6.0))
      .bg(move || palette(dark_mode.get()).surface_sunken)
      .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
      .on_press(move || (cb.on_open_search)())
      .child(
        view().flex_row().items_center().gap_2()
          .child(dynamic(move || mica_icons::search(palette(dark_mode.get()).text_ghost_hex)))
          .child(
            text(move || crate::mica_theme::tr("搜索...", "Search..."))
              .text_size(move || px((13.0 * crate::mica_theme::ui_scale_factor()).round()))
              .text_color(move || palette(dark_mode.get()).text_ghost),
          ),
      )
      .child(
        text("Ctrl+Shift+F")
          .text_size(move || px((11.0 * crate::mica_theme::ui_scale_factor()).round()))
          .text_color(move || palette(dark_mode.get()).text_faint),
      ),
  )
}

// ── Favorites Section (`max_h(180)`, `30px` rows, `pl(20)`, Emoji/doc, ctx) ───

#[allow(clippy::too_many_arguments)]
fn favorites_section(
  tree: Rc<RefCell<PageTreeModel>>,
  tree_rev: Signal<u64>,
  favorite_pages: Signal<Vec<(Uuid, String)>>,
  active_doc: Signal<Option<Uuid>>,
  show_favorites: Signal<bool>,
  dark_mode: Signal<bool>,
  context_menu: Signal<Option<ContextMenuState>>,
  page_to_local: Signal<(f32, f32)>,
  cb: Rc<SidebarCallbacks>,
) -> impl IntoElement {
  when(move || !favorite_pages.get().is_empty(), {
    let cb_outer = cb.clone();
    let tree_outer = tree.clone();
    move || {
      let cb_list = cb_outer.clone();
      let tree_list = tree_outer.clone();
      view().w_full().pt(px(6.0)).pb_1()
        .child(section_toggle_header("收藏", "toggle-favorites-section", "展开或收起收藏列表", show_favorites, dark_mode))
        .child(when(move || show_favorites.get(), {
          let cb_rows = cb_list.clone();
          let tree_rows = tree_list.clone();
          move || {
            let cb_dyn = cb_rows.clone();
            let tree_dyn = tree_rows.clone();
            scroll().w_full().max_h(px(180.0)).child(dynamic(move || {
              tree_rev.get();
              let items = favorite_pages.get();
              let curr = active_doc.get();
              let p = palette(dark_mode.get());
              let rows_snapshot = tree_dyn.borrow().visible_rows();
              let mut list = view().w_full().px_2();
              for (idx, (doc_id, raw_name)) in items.into_iter().enumerate() {
                let select_cb = cb_dyn.clone();
                let is_active = curr == Some(doc_id);
                let display_name = if raw_name.trim().is_empty() { "无标题".to_string() } else { raw_name.clone() };
                let icon_emoji = tree_dyn
                  .borrow()
                  .get(doc_id)
                  .and_then(|v| v.icon.clone())
                  .or_else(|| {
                    rows_snapshot
                      .iter()
                      .find(|r| r.object_id == doc_id || r.view_id == doc_id)
                      .and_then(|r| r.icon.clone())
                  })
                  .filter(|s| !s.trim().is_empty());
                let open_name = raw_name;
                list = list.child(
                  view()
                    .test_id(&format!("favorite-page-{idx}"))
                    .role(AccessibilityRole::Button)
                    .label(SharedString::from(format!("打开收藏页面 {display_name}")))
                    .w_full().h(px(TREE_ROW_HEIGHT)).pl(px(20.0)).pr(px(6.0))
                    .flex_row().items_center().rounded(px(6.0))
                    .bg(if is_active { p.surface_hover } else { p.surface_raised })
                    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                    .on_press(move |_: &PressEvent| {
                      context_menu.set(None);
                      (select_cb.on_select_doc)(doc_id, open_name.clone());
                    })
                    .on_context_menu(move |e: &ContextMenuEvent| {
                      let _ = page_to_local.get_untracked();
                      context_menu.set(Some(ContextMenuState {
                        target: ContextMenuTarget::Favorite { doc_id },
                        x: e.page_position.x.0.max(8.0),
                        y: e.page_position.y.0.max(8.0),
                      }));
                    })
                    .child(
                      view().w(px(18.0)).h(px(18.0)).flex_shrink_0().items_center().justify_center()
                        .child(if let Some(emoji) = icon_emoji {
                          view().child(text(emoji).text_size(px(15.0)))
                        } else {
                          view().child(mica_icons::file_doc(if is_active { p.text_primary_hex } else { p.text_muted_hex }))
                        }),
                    )
                    .child(
                      view().flex_1().min_w(px(0.0)).pl(px(6.0)).child(
                        text(display_name).text_size(px(14.0)).text_color(p.text_primary).truncate(),
                      ),
                    ),
                );
              }
              list
            }))
          }
        }))
    }
  })
}

fn section_toggle_header(
  label: &'static str,
  tid: &'static str,
  a11y: &'static str,
  expanded: Signal<bool>,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  view().h(px(26.0)).px_3().flex_row().items_center().justify_between().child(
    button()
      .test_id(tid).accessibility_label(a11y)
      .flex_row().items_center().gap_1().px_1().py_1().rounded(px(4.0))
      .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
      .on_press(move || expanded.set(!expanded.get_untracked()))
      .child(
        text(label).text_size(px(12.0)).font_weight(FontWeight::MEDIUM)
          .text_color(move || palette(dark_mode.get()).text_faint),
      )
      .child(dynamic(move || {
        let hex = palette(dark_mode.get()).text_faint_hex;
        if expanded.get() { mica_icons::chevron_down(hex) } else { mica_icons::chevron_right(hex) }
      })),
  )
}

// ── Pages Section Header ──────────────────────────────────────────────────────

fn pages_section_header(
  show_pages: Signal<bool>,
  dark_mode: Signal<bool>,
  cb: Rc<SidebarCallbacks>,
) -> impl IntoElement {
  let cb_p = cb.clone();
  let cb_f = cb;
  view()
    .h(px(28.0)).px_3().flex_row().items_center().justify_between()
    .child(
      button()
        .test_id("toggle-pages-section").accessibility_label("展开或收起页面列表")
        .flex_row().items_center().gap_1().px_1().py_1().rounded(px(4.0))
        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
        .on_press(move || show_pages.set(!show_pages.get_untracked()))
        .child(
          text(move || crate::mica_theme::tr("页面", "Pages"))
            .text_size(move || px((12.5 * crate::mica_theme::ui_scale_factor()).round()))
            .font_weight(FontWeight::MEDIUM)
            .text_color(move || palette(dark_mode.get()).text_faint),
        )
        .child(dynamic(move || {
          let hex = palette(dark_mode.get()).text_faint_hex;
          if show_pages.get() { mica_icons::chevron_down(hex) } else { mica_icons::chevron_right(hex) }
        })),
    )
    .child(
      view().flex_row().items_center().gap_1()
        .child(crate::tooltip::with_tooltip(
          button()
            .test_id("new-page-btn").accessibility_label("新建页面")
            .w(px(24.0)).h(px(24.0)).items_center().justify_center().rounded(px(6.0))
            .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
            .on_press(move || {
              crate::tooltip::hide_tooltip();
              (cb_p.on_new_page)();
            })
            .child(dynamic(move || mica_icons::plus(palette(dark_mode.get()).text_faint_hex))),
          "新建页面",
        ))
        .child(crate::tooltip::with_tooltip(
          button()
            .test_id("new-folder-btn").accessibility_label("新建文件夹")
            .w(px(24.0)).h(px(24.0)).items_center().justify_center().rounded(px(6.0))
            .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
            .on_press(move || {
              crate::tooltip::hide_tooltip();
              (cb_f.on_new_folder)();
            })
            .child(dynamic(move || mica_icons::folder_plus(palette(dark_mode.get()).text_faint_hex))),
          "新建文件夹",
        )),
    )
}

// ── Virtualized Page Tree + Root Drop Zone ────────────────────────────────────

fn page_tree_area(ctx: Rc<TreeRowContext>) -> impl IntoElement {
  let tree_count = ctx.tree.clone();
  let tree_keys = ctx.tree.clone();
  let tree_rev = ctx.tree_rev;
  let row_ctx = ctx.clone();
  let root_ctx = ctx.clone();
  let root_click_ctx = ctx.clone();
  let scroll_ctx = ctx.clone();
  let layout_ctx = ctx.clone();
  let dark_mode = ctx.dark_mode;
  let drag_state = ctx.drag_state;

  view()
    .flex_1().w_full().min_h(px(0.0))
    .child(
      virtual_list(
        move || {
          tree_rev.get();
          tree_count.borrow().visible_rows().len()
        },
        move |at: Position| render_tree_row(row_ctx.clone(), at),
      )
      .keys(move || {
        tree_rev.get();
        tree_keys
          .borrow()
          .visible_rows()
          .into_iter()
          .map(|r| r.view_id.to_string())
          .collect::<Vec<_>>()
      })
      .element_ref(&ctx.tree_list_ref)
      .estimated_item_size(px(TREE_ROW_HEIGHT))
      .on_scroll(move |e: &ScrollEvent| {
        scroll_ctx.tree_scroll_y.set(e.offset.y.0);
        if e.content_size.height.0 > 0.0 {
          scroll_ctx.tree_content_h.set(e.content_size.height.0);
        }
        if e.size.height.0 > 0.0 {
          scroll_ctx.tree_viewport_h.set(e.size.height.0);
        }
        if scroll_ctx.drag_state.get_untracked().active.is_some() {
          let row_count = scroll_ctx.tree.borrow().visible_rows().len();
          update_drag_target_after_autoscroll(
            &scroll_ctx,
            e.offset.y.0,
            scroll_ctx.tree_content_h.get_untracked(),
            scroll_ctx.tree_viewport_h.get_untracked(),
            row_count,
          );
        }
      })
      .on_layout(move |b: &Bounds<Points>| {
        if b.size.height.0 > 0.0 {
          layout_ctx.tree_viewport_h.set(b.size.height.0);
        }
        refresh_tree_list_geometry(&layout_ctx);
      })
      .flex_1(),
    )
    .child(
      view()
        .test_id("sidebar-root-drop-zone")
        .w_full().h(px(48.0)).flex_shrink_0().pt(px(6.0))
        .on_press(move |_: &PressEvent| {
          root_click_ctx.context_menu.set(None);
          let mut s = root_click_ctx.selection.get_untracked();
          if s.clear() {
            root_click_ctx.selection.set(s);
          }
        })
        .on_mouse_move(move |e: &MouseEvent| {
          if e.pressed_button != Some(MouseButton::Left) {
            return;
          }
          let mut st = root_ctx.drag_state.get_untracked();
          if let Some(active) = st.active.as_mut() {
            update_drag_autoscroll(&root_ctx, e.page_position.x.0, e.page_position.y.0);
            if active.target != Some(DragTarget::Root) {
              active.target = Some(DragTarget::Root);
              root_ctx.drag_state.set(st);
            }
          }
        })
        .child(dynamic(move || {
          if drag_state.get().active.as_ref().is_some_and(|a| a.target == Some(DragTarget::Root)) {
            view().child(drop_indicator(drop_indicator_inset(0), DropIndicatorState::Allowed, dark_mode))
          } else {
            view()
          }
        })),
    )
}

fn tree_row_bg(
  p: &crate::mica_theme::MicaPalette,
  is_multi_selected: bool,
  is_active: bool,
  is_menu_open: bool,
  is_hovered: bool,
) -> Rgba {
  if is_multi_selected {
    if is_hovered {
      Rgba {
        a: (p.selection_bg.a + 0.06).min(1.0),
        ..p.selection_bg
      }
    } else {
      p.selection_bg
    }
  } else if is_active || is_menu_open || is_hovered {
    p.surface_hover
  } else {
    p.surface_raised
  }
}

fn render_tree_row(ctx: Rc<TreeRowContext>, at: Position) -> impl IntoElement {
  let tree_read = ctx.tree.clone();
  let tree_rev = ctx.tree_rev;
  let active_doc = ctx.active_doc;
  let dark_mode = ctx.dark_mode;
  let sidebar_hovered = ctx.sidebar_hovered;
  let selection = ctx.selection;
  let context_menu = ctx.context_menu;
  let drag_state = ctx.drag_state;
  let row_idx = at.index();
  let row_hovered = signal(false);

  let row_test_id = ctx
    .tree
    .borrow()
    .visible_rows()
    .get(row_idx)
    .map(|r| format!("tree-row-{}", r.view_id))
    .unwrap_or_else(|| format!("tree-row-idx-{row_idx}"));

  let row_info = computed(move || {
    tree_rev.get();
    tree_read.borrow().visible_rows().get(at.index()).cloned()
  });
  let row_view_id = computed(move || row_info.get().map(|r| r.view_id));
  let is_active = computed(move || {
    let curr = active_doc.get();
    row_info.get().is_some_and(|r| !r.is_folder && Some(r.object_id) == curr)
  });
  let is_multi_selected = computed(move || row_view_id.get().is_some_and(|vid| selection.get().contains(vid)));
  let is_menu_open = computed(move || {
    row_view_id.get().is_some_and(|vid| {
      matches!(
        context_menu.get(),
        Some(ContextMenuState { target: ContextMenuTarget::TreeRow { view_id, .. }, .. }) if view_id == vid
      )
    })
  });
  let is_dragged_source = computed(move || {
    row_view_id.get().is_some_and(|vid| {
      drag_state.get().active.as_ref().is_some_and(|a| a.dragged_ids.contains(&vid))
    })
  });
  let drop_overlay = computed(move || -> Option<(DropMode, usize, DropIndicatorState)> {
    let vid = row_view_id.get()?;
    let st = drag_state.get();
    let active = st.active.as_ref()?;
    match active.target {
      Some(DragTarget::Row { target_view_id, mode, level, allowed, .. }) if target_view_id == vid => {
        Some((mode, level, if allowed { DropIndicatorState::Allowed } else { DropIndicatorState::Blocked }))
      }
      _ => None,
    }
  });
  let row_label = computed(move || {
    row_info
      .get()
      .map(|r| if r.name.trim().is_empty() { "无标题".to_string() } else { r.name })
      .unwrap_or_default()
  });
  let indent_px = computed(move || row_info.get().map(|r| 2.0 + (r.depth as f32) * 16.0).unwrap_or(2.0));

  let ctx_press = ctx.clone();
  let ctx_down = ctx.clone();
  let ctx_row_down = ctx.clone();
  let ctx_move = ctx.clone();
  let ctx_context = ctx.clone();
  let ctx_chevron = ctx.clone();
  let ctx_more = ctx.clone();
  let ctx_add_page = ctx.clone();
  let ctx_add_folder = ctx.clone();

  view()
    .test_id(&row_test_id)
    .relative().w_full()
    .opacity(move || if is_dragged_source.get() { 0.4_f32 } else { 1.0_f32 })
    .on_mouse_enter(move |_: &MouseEvent| row_hovered.set(true))
    .on_mouse_leave(move |_: &MouseEvent| row_hovered.set(false))
    .on_press_down(move |e: &PressEvent| {
      ctx_down.just_dragged.set(false);
      if let Some(row) = row_info.get_untracked() {
        ctx_down.drag_state.set(DragState {
          pending: Some(PendingDrag {
            source_view_id: row.view_id,
            start_page_x: e.page_position.x.0,
            start_page_y: e.page_position.y.0,
          }),
          active: None,
        });
      }
    })
    .on_mouse_move(move |e: &MouseEvent| {
      if e.pressed_button != Some(MouseButton::Left) {
        return;
      }
      let Some(row) = row_info.get_untracked() else { return; };
      let st = ctx_move.drag_state.get_untracked();
      if st.active.is_none() {
        if let Some(pending) = st.pending.clone() {
          let dx = e.page_position.x.0 - pending.start_page_x;
          let dy = e.page_position.y.0 - pending.start_page_y;
          if dx.hypot(dy) >= DRAG_THRESHOLD_PX {
            let (ox, oy) = ctx_move.page_to_local.get_untracked();
            start_active_drag(
              &ctx_move,
              pending.source_view_id,
              e.page_position.x.0 - ox,
              e.page_position.y.0 - oy,
              e.page_position.x.0,
              e.page_position.y.0,
            );
          }
        }
        return;
      }
      update_drag_autoscroll(&ctx_move, e.page_position.x.0, e.page_position.y.0);
      update_drag_target_for_row(&ctx_move, row.view_id, e.position.x.0, e.position.y.0);
    })
    .on_context_menu(move |e: &ContextMenuEvent| {
      if let Some(row) = row_info.get_untracked() {
        open_tree_context_menu(&ctx_context, row, e.page_position.x.0.max(8.0), e.page_position.y.0.max(8.0));
      }
    })
    // Before indicator
    .child(dynamic(move || match drop_overlay.get() {
      Some((DropMode::Before, level, state)) => view()
        .absolute().top(px(-4.0)).left(px(0.0)).right(px(0.0)).z_index(30)
        .pointer_events(PointerEvents::None)
        .child(drop_indicator(drop_indicator_inset(level), state, dark_mode)),
      _ => view(),
    }))
    // Main row body
    .child(
      view()
        .test_id(&format!("sidebar-row-body-{row_idx}"))
        .w_full().h(px(TREE_ROW_HEIGHT)).my(px(1.0)).pl(move || px(indent_px.get())).pr(px(4.0))
        .flex_row().items_center().rounded(px(5.0))
        .bg(move || {
          let p = palette(dark_mode.get());
          tree_row_bg(
            &p,
            is_multi_selected.get(),
            is_active.get(),
            is_menu_open.get(),
            row_hovered.get(),
          )
        })
        // Fixed 18px chevron slot (clicking chevron preserves multi-select)
        .child(
          view().w(px(18.0)).h(px(TREE_ROW_HEIGHT)).flex_shrink_0().items_center().justify_center()
            .child(dynamic(move || {
              let info = row_info.get();
              let has_children = info.as_ref().is_some_and(|r| r.has_children);
              let expanded = info.as_ref().is_some_and(|r| r.expanded);
              let vid = info.as_ref().map(|r| r.view_id);
              if !has_children {
                return view().w(px(18.0)).h(px(TREE_ROW_HEIGHT));
              }
              let show = sidebar_hovered.get() || row_hovered.get();
              let p = palette(dark_mode.get());
              let ctx_ch = ctx_chevron.clone();
              view().w(px(18.0)).h(px(TREE_ROW_HEIGHT)).child(
                button()
                  .accessibility_label(if expanded { "收起" } else { "展开" })
                  .w(px(18.0)).h(px(TREE_ROW_HEIGHT)).items_center().justify_center()
                  .opacity(if show { 1.0 } else { 0.0 })
                  .on_press(move || {
                    if let Some(id) = vid {
                      ctx_ch.tree.borrow_mut().toggle_expanded(id);
                      ctx_ch.tree_rev.set(ctx_ch.tree_rev.get_untracked() + 1);
                    }
                  })
                  .child(if expanded { mica_icons::chevron_down(p.text_faint_hex) } else { mica_icons::chevron_right(p.text_faint_hex) }),
              )
            })),
        )
        // Row body (icon + title)
        .child(
          view()
            .test_id(&format!("sidebar-row-{row_idx}"))
            .role(AccessibilityRole::Button)
            .optional_label(move || {
              let label = row_label.get();
              if row_info.get().is_some_and(|r| r.is_folder) {
                Some(SharedString::from(format!("展开或收起文件夹 {label}")))
              } else {
                Some(SharedString::from(format!("打开页面 {label}")))
              }
            })
            .flex_1().min_w(px(0.0)).h_full().flex_row().items_center()
            // `nsh` dispatches press events only to the innermost pressable view,
            // so `on_press_down` MUST live right beside `on_press`.
            // Note: do NOT cancel drag in `on_press_up` because `nsh` fires
            // `cancel_press` -> `on_press_up` as soon as pointer movement exceeds `PRESS_SLOP`.
            .on_press_down(move |e: &PressEvent| {
              ctx_row_down.just_dragged.set(false);
              if let Some(row) = row_info.get_untracked() {
                ctx_row_down.drag_state.set(DragState {
                  pending: Some(PendingDrag {
                    source_view_id: row.view_id,
                    start_page_x: e.page_position.x.0,
                    start_page_y: e.page_position.y.0,
                  }),
                  active: None,
                });
              }
            })
            .on_press(move |e: &PressEvent| {
              if ctx_press.just_dragged.get_untracked() {
                ctx_press.just_dragged.set(false);
                return;
              }
              ctx_press.context_menu.set(None);
              let Some(row) = row_info.get_untracked() else { return; };
              match tree_click_intent(e.modifiers.shift, e.modifiers.ctrl || e.modifiers.cmd) {
                ClickIntent::Range => {
                  let visible = ctx_press.tree.borrow().visible_rows();
                  let mut sel = ctx_press.selection.get_untracked();
                  sel.select_range(row.view_id, &visible);
                  ctx_press.selection.set(sel);
                }
                ClickIntent::Toggle => {
                  let mut sel = ctx_press.selection.get_untracked();
                  sel.toggle(row.view_id);
                  ctx_press.selection.set(sel);
                }
                ClickIntent::Open => {
                  let mut sel = ctx_press.selection.get_untracked();
                  if sel.anchor_on(row.view_id) {
                    ctx_press.selection.set(sel);
                  }
                  if !row.is_folder && e.clicks >= 2 {
                    if let Some(open_r) = &ctx_press.cb.on_open_rename_modal {
                      open_r(row.view_id, row.name);
                    } else {
                      ctx_press.rename_modal_view.set(Some(row.view_id));
                      ctx_press.rename_modal_initial.set(row.name);
                      ctx_press.rename_modal_open.set(true);
                    }
                    return;
                  }
                  if row.is_folder {
                    ctx_press.tree.borrow_mut().toggle_expanded(row.view_id);
                    ctx_press.tree_rev.set(ctx_press.tree_rev.get_untracked() + 1);
                  } else {
                    (ctx_press.cb.on_select_doc)(row.object_id, row.name);
                  }
                }
              }
            })
            // 18×18 Emoji or Folder/Page icon
            .child(
              view().w(px(18.0)).h(px(18.0)).flex_shrink_0().items_center().justify_center()
                .child(dynamic(move || {
                  let p = palette(dark_mode.get());
                  let info = row_info.get();
                  let icon_hex = if is_active.get() { p.text_primary_hex } else { p.text_muted_hex };
                  if let Some(emoji) = info.as_ref().and_then(|r| r.icon.as_ref()).filter(|s| !s.trim().is_empty()) {
                    view().child(text(emoji.clone()).text_size(px(15.0)))
                  } else if info.as_ref().is_some_and(|r| r.is_folder) {
                    view().child(mica_icons::folder(icon_hex))
                  } else {
                    view().child(mica_icons::file_doc(icon_hex))
                  }
                })),
            )
            // 6px gap + 14px title
            .child(
              view().flex_1().min_w(px(0.0)).pl(px(6.0)).child(dynamic(move || {
                let p = palette(dark_mode.get());
                let active = is_active.get();
                text(row_label.get())
                  .text_size(px((14.0 * crate::mica_theme::ui_scale_factor()).round()))
                  .font_weight(if active { FontWeight::MEDIUM } else { FontWeight::NORMAL })
                  .text_color(if active { p.text_primary } else { p.text_muted })
                  .truncate()
              })),
            ),
        )
        // Hover actions: `⋯` + folder `+` / `folder+`
        .child(when(move || row_hovered.get() || is_menu_open.get(), {
          let ctx_m = ctx_more.clone();
          let ctx_p = ctx_add_page.clone();
          let ctx_f = ctx_add_folder.clone();
          move || {
            let ctx_m2 = ctx_m.clone();
            let ctx_p2 = ctx_p.clone();
            let ctx_f2 = ctx_f.clone();
            view().flex_row().items_center().flex_shrink_0()
              .child(
                view()
                  .test_id(&format!("sidebar-row-menu-{row_idx}"))
                  .role(AccessibilityRole::Button)
                  .label("更多操作")
                  .w(px(24.0)).h(px(24.0)).items_center().justify_center().rounded(px(4.0))
                  .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_sunken))
                  .on_press(move |e: &PressEvent| {
                    if let Some(row) = row_info.get_untracked() {
                      open_tree_context_menu(
                        &ctx_m2,
                        row,
                        e.page_position.x.0.max(8.0),
                        (e.page_position.y.0 + 14.0).max(8.0),
                      );
                    }
                  })
                  .child(dynamic(move || mica_icons::more_dots(palette(dark_mode.get()).text_muted_hex))),
              )
              .child(when(move || row_info.get().is_some_and(|r| r.is_folder), {
                let cp = ctx_p2.clone();
                let cf = ctx_f2.clone();
                move || {
                  let cp2 = cp.clone();
                  let cf2 = cf.clone();
                  view().flex_row().items_center()
                    .child(
                      button()
                        .accessibility_label("新建子页面")
                        .w(px(22.0)).h(px(22.0)).items_center().justify_center().rounded(px(4.0))
                        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_sunken))
                        .on_press(move || {
                          if let Some(vid) = row_view_id.get_untracked() {
                            (cp2.cb.on_new_child_page)(vid);
                          }
                        })
                        .child(dynamic(move || mica_icons::plus(palette(dark_mode.get()).text_faint_hex))),
                    )
                    .child(
                      button()
                        .accessibility_label("新建子文件夹")
                        .w(px(22.0)).h(px(22.0)).items_center().justify_center().rounded(px(4.0))
                        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_sunken))
                        .on_press(move || {
                          if let Some(vid) = row_view_id.get_untracked() {
                            (cf2.cb.on_new_child_folder)(vid);
                          }
                        })
                        .child(dynamic(move || mica_icons::folder_plus(palette(dark_mode.get()).text_faint_hex))),
                    )
                }
              }))
          }
        })),
    )
    // Into indicator (2px rounded border + 8% tint box)
    .child(dynamic(move || match drop_overlay.get() {
      Some((DropMode::Into, _, state)) => {
        let p = palette(dark_mode.get());
        let color = match state {
          DropIndicatorState::Allowed => p.accent,
          DropIndicatorState::Blocked => p.warning,
          DropIndicatorState::Idle => Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.0 },
        };
        view()
          .absolute().inset_0().rounded(px(6.0)).border_2().border_color(color)
          .bg(Rgba { a: 0.08, ..color }).z_index(28).pointer_events(PointerEvents::None)
      }
      _ => view(),
    }))
    // After indicator
    .child(dynamic(move || match drop_overlay.get() {
      Some((DropMode::After, level, state)) => view()
        .absolute().bottom(px(-4.0)).left(px(0.0)).right(px(0.0)).z_index(30)
        .pointer_events(PointerEvents::None)
        .child(drop_indicator(drop_indicator_inset(level), state, dark_mode)),
      _ => view(),
    }))
}

// ── Drag-and-Drop Engine & Indicators ─────────────────────────────────────────

fn refresh_tree_list_geometry(ctx: &TreeRowContext) {
  let lref = ctx.tree_list_ref;
  let origin_sig = ctx.tree_page_origin;
  let vh_sig = ctx.tree_viewport_h;
  let drag_state = ctx.drag_state;
  let drag_page_pointer = ctx.drag_page_pointer;
  let drag_autoscroll_speed = ctx.drag_autoscroll_speed;
  spawn(async move {
    if let Some(m) = lref.measure().await {
      origin_sig.set((m.page.x.0, m.page.y.0));
      if m.frame.size.height.0 > 0.0 {
        vh_sig.set(m.frame.size.height.0);
      }
      if drag_state.get_untracked().active.is_some() {
        let (_, page_y) = drag_page_pointer.get_untracked();
        let speed = tree_drag_autoscroll_speed(page_y - m.page.y.0, vh_sig.get_untracked());
        if (drag_autoscroll_speed.get_untracked() - speed).abs() >= 0.05 {
          drag_autoscroll_speed.set(speed);
        }
      }
    }
  });
}

fn update_drag_autoscroll(ctx: &TreeRowContext, page_x: f32, page_y: f32) {
  ctx.drag_page_pointer.set((page_x, page_y));
  let (_, origin_y) = ctx.tree_page_origin.get_untracked();
  let vh = ctx.tree_viewport_h.get_untracked();
  let local_y = page_y - origin_y;
  let speed = tree_drag_autoscroll_speed(local_y, vh);
  if (ctx.drag_autoscroll_speed.get_untracked() - speed).abs() >= 0.05 {
    ctx.drag_autoscroll_speed.set(speed);
  }
}

fn update_drag_target_for_row(
  ctx: &TreeRowContext,
  target_view_id: Uuid,
  row_local_x: f32,
  row_local_y: f32,
) {
  let mut st = ctx.drag_state.get_untracked();
  let Some(active) = st.active.as_mut() else {
    return;
  };
  let visible = ctx.tree.borrow().visible_rows();
  let Some(cur_idx) = visible.iter().position(|r| r.view_id == target_view_id) else {
    return;
  };
  let row = &visible[cur_idx];
  let mode = TreeDropPlanner::drop_zone_for_local_y(row_local_y, TREE_ROW_HEIGHT);
  let (level, allowed) = match mode {
    DropMode::Before => (
      row.depth,
      TreeDropPlanner::can_drop(&active.dragged_ids, row, DropMode::Before, &visible),
    ),
    DropMode::Into => (
      row.depth + 1,
      TreeDropPlanner::can_drop(&active.dragged_ids, row, DropMode::Into, &visible),
    ),
    DropMode::After => {
      let next_depth = visible.get(cur_idx + 1).map(|r| r.depth);
      let lvl = TreeDropPlanner::reparent_level_for(row_local_x, row.depth, next_depth);
      let resolved_id = TreeDropPlanner::resolve_after_target(cur_idx, lvl, &visible);
      let resolved_row = visible.iter().find(|r| r.view_id == resolved_id).unwrap_or(row);
      (
        lvl,
        TreeDropPlanner::can_drop(&active.dragged_ids, resolved_row, DropMode::After, &visible),
      )
    }
  };
  let next_target = Some(DragTarget::Row {
    target_idx: cur_idx,
    target_view_id,
    mode,
    level,
    allowed,
  });
  if active.target != next_target {
    active.target = next_target;
    ctx.drag_state.set(st);
  }
}

fn update_drag_target_after_autoscroll(
  ctx: &TreeRowContext,
  next_scroll_y: f32,
  content_h: f32,
  viewport_h: f32,
  row_count: usize,
) {
  if row_count == 0 || viewport_h <= 1.0 {
    return;
  }
  let (page_x, page_y) = ctx.drag_page_pointer.get_untracked();
  let (origin_x, origin_y) = ctx.tree_page_origin.get_untracked();
  let local_y = page_y - origin_y;
  if local_y < 0.0 || local_y > viewport_h {
    return;
  }
  let row_pitch = if content_h > 0.0 {
    content_h / (row_count as f32)
  } else {
    TREE_ROW_PITCH
  };
  let Some((idx, row_local_y)) = tree_drag_row_at_y(next_scroll_y + local_y, row_pitch, row_count)
  else {
    return;
  };
  let target_view_id = {
    let visible = ctx.tree.borrow().visible_rows();
    visible.get(idx).map(|r| r.view_id)
  };
  if let Some(vid) = target_view_id {
    let row_local_x = (page_x - origin_x).max(0.0);
    update_drag_target_for_row(ctx, vid, row_local_x, row_local_y);
  }
}

fn start_active_drag(
  ctx: &TreeRowContext,
  source_view_id: Uuid,
  local_x: f32,
  local_y: f32,
  page_x: f32,
  page_y: f32,
) {
  let visible = ctx.tree.borrow().visible_rows();
  let sel = ctx.selection.get_untracked();
  let dragged_ids = if sel.acts_on_selection(source_view_id) {
    independent_selection(&sel.ids, &visible)
  } else {
    vec![source_view_id]
  };
  let lead_id = dragged_ids.first().copied().unwrap_or(source_view_id);
  let lead = visible.iter().find(|r| r.view_id == lead_id);
  let lead_name = lead
    .map(|r| if r.name.trim().is_empty() { "无标题".to_string() } else { r.name.clone() })
    .unwrap_or_default();

  refresh_tree_list_geometry(ctx);
  ctx.context_menu.set(None);
  ctx.drag_pointer.set((local_x, local_y));
  ctx.drag_page_pointer.set((page_x, page_y));
  ctx.drag_state.set(DragState {
    pending: None,
    active: Some(ActiveDrag {
      dragged_ids,
      lead_name,
      lead_icon: lead.and_then(|r| r.icon.clone()),
      lead_is_folder: lead.is_some_and(|r| r.is_folder),
      target: None,
    }),
  });
  update_drag_autoscroll(ctx, page_x, page_y);
}

fn commit_tree_drop(ctx: &TreeRowContext, active: ActiveDrag) {
  let Some(target) = active.target else { return; };
  if active.dragged_ids.is_empty() {
    return;
  }
  let visible = ctx.tree.borrow().visible_rows();

  match target {
    DragTarget::Root => {
      let roots: Vec<Uuid> = visible.iter().filter(|r| r.parent_view_id.is_none()).map(|r| r.view_id).collect();
      (ctx.cb.on_reorder_views)(None, TreeDropPlanner::root_drop_order(&roots, &active.dragged_ids));
    }
    DragTarget::Row { target_idx, target_view_id, mode, level, allowed } => {
      if !allowed {
        return;
      }
      match mode {
        DropMode::Into => {
          ctx.tree.borrow_mut().set_expanded(target_view_id, true);
          let expanded = ctx.tree.borrow().visible_rows();
          let children: Vec<Uuid> = expanded
            .iter()
            .filter(|r| r.parent_view_id == Some(target_view_id))
            .map(|r| r.view_id)
            .collect();
          let ordered = TreeDropPlanner::root_drop_order(&children, &active.dragged_ids);
          ctx.tree_rev.set(ctx.tree_rev.get_untracked() + 1);
          (ctx.cb.on_reorder_views)(Some(target_view_id), ordered);
        }
        DropMode::Before => {
          let Some(row) = visible.iter().find(|r| r.view_id == target_view_id) else { return; };
          let pid = row.parent_view_id;
          let sibs: Vec<Uuid> = visible.iter().filter(|r| r.parent_view_id == pid).map(|r| r.view_id).collect();
          let ordered = TreeDropPlanner::sibling_order_after_drop(&sibs, &active.dragged_ids, target_view_id, true);
          (ctx.cb.on_reorder_views)(pid, ordered);
        }
        DropMode::After => {
          let idx = visible.iter().position(|r| r.view_id == target_view_id).unwrap_or(target_idx);
          if idx >= visible.len() {
            return;
          }
          let resolved_id = TreeDropPlanner::resolve_after_target(idx, level, &visible);
          let Some(row) = visible.iter().find(|r| r.view_id == resolved_id) else { return; };
          let pid = row.parent_view_id;
          let sibs: Vec<Uuid> = visible.iter().filter(|r| r.parent_view_id == pid).map(|r| r.view_id).collect();
          let ordered = TreeDropPlanner::sibling_order_after_drop(&sibs, &active.dragged_ids, resolved_id, false);
          (ctx.cb.on_reorder_views)(pid, ordered);
        }
      }
    }
  }

  let mut sel = ctx.selection.get_untracked();
  if sel.clear() {
    ctx.selection.set(sel);
  }
}

/// 8px hollow circle + 2px horizontal line drop indicator.
fn drop_indicator(inset: f32, state: DropIndicatorState, dark_mode: Signal<bool>) -> impl IntoElement {
  let p = palette(dark_mode.get());
  let color = match state {
    DropIndicatorState::Idle => Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.0 },
    DropIndicatorState::Allowed => p.accent,
    DropIndicatorState::Blocked => p.warning,
  };
  view()
    .w_full().h(px(8.0)).pl(px(inset)).pr(px(4.0)).flex_row().items_center()
    .child(view().w(px(8.0)).h(px(8.0)).flex_shrink_0().rounded_full().border_2().border_color(color))
    .child(view().flex_1().h(px(2.0)).bg(color))
}

/// Floating drag ghost card (single card or 3-layer stacked deck + `{N} 项` badge).
fn drag_ghost_overlay(
  drag_state: Signal<DragState>,
  drag_pointer: Signal<(f32, f32)>,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  when(move || drag_state.get().active.is_some(), move || {
    view()
      .absolute()
      .inset_0()
      .z_index(70)
      .pointer_events(PointerEvents::None)
      .child(
        view()
          .absolute()
          .left(move || px((drag_pointer.get().0 + 12.0).max(4.0)))
          .top(move || px((drag_pointer.get().1 + 10.0).max(4.0)))
          .pointer_events(PointerEvents::None)
          .child(dynamic(move || {
            let Some(active) = drag_state.get().active else { return view(); };
            let p = palette(dark_mode.get());
            let count = active.dragged_ids.len();

            let lead_card = view()
              .min_w(px(132.0)).max_w(px(240.0)).px_3().py_2().flex_row().items_center().gap_2()
              .rounded(px(6.0)).border_1().border_color(p.border_normal).bg(p.surface_overlay)
              .shadow_md()
              .child(
                view().w(px(18.0)).h(px(18.0)).flex_shrink_0().items_center().justify_center()
                  .child(if let Some(emoji) = active.lead_icon.filter(|s| !s.trim().is_empty()) {
                    view().child(text(emoji).text_size(px(15.0)))
                  } else if active.lead_is_folder {
                    view().child(mica_icons::folder(p.text_muted_hex))
                  } else {
                    view().child(mica_icons::file_doc(p.text_muted_hex))
                  }),
              )
              .child(text(active.lead_name).flex_1().text_size(px(13.0)).text_color(p.text_primary).truncate());

            if count <= 1 {
              return view().test_id("tree-drag-ghost").child(lead_card);
            }

            view()
              .test_id("tree-drag-ghost-deck").gap_1()
              .child(
                view().relative()
                  .child(
                    view().absolute().top(px(0.0)).left(px(0.0)).w(px(180.0)).h(px(34.0))
                      .rounded(px(6.0)).border_1().border_color(p.border_normal).bg(p.surface_overlay).opacity(0.65),
                  )
                  .child(
                    view().absolute().top(px(4.0)).left(px(4.0)).w(px(180.0)).h(px(34.0))
                      .rounded(px(6.0)).border_1().border_color(p.border_normal).bg(p.surface_overlay).opacity(0.82),
                  )
                  .child(view().pl(px(8.0)).pt(px(8.0)).child(lead_card)),
              )
              .child(
                view().pl(px(8.0)).child(
                  view().px_2().py(px(2.0)).rounded(px(10.0)).bg(p.text_primary).child(
                    text(format!("{count} 项")).text_size(px(11.0)).font_weight(FontWeight::MEDIUM).text_color(p.on_primary),
                  ),
                ),
              )
          })),
      )
  })
}

// ── Floating Context Menu Overlay ─────────────────────────────────────────────

fn open_tree_context_menu(ctx: &TreeRowContext, row: TreeRow, x: f32, y: f32) {
  let sel = ctx.selection.get_untracked();
  let target = if sel.acts_on_selection(row.view_id) {
    let visible = ctx.tree.borrow().visible_rows();
    ContextMenuTarget::Batch { ids: independent_selection(&sel.ids, &visible) }
  } else {
    ContextMenuTarget::TreeRow {
      view_id: row.view_id,
      object_id: row.object_id,
      name: row.name,
      icon: row.icon,
      is_folder: row.is_folder,
      expanded: row.expanded,
    }
  };
  ctx.context_menu.set(Some(ContextMenuState { target, x, y }));
}

fn context_menu_overlay(ctx: Rc<TreeRowContext>) -> impl IntoElement {
  let dark_mode = ctx.dark_mode;
  let context_menu = ctx.context_menu;

  when(move || context_menu.get().is_some(), {
    let ctx_outer = ctx.clone();
    move || {
      let ctx_in = ctx_outer.clone();
      let Some(menu) = context_menu.get_untracked() else {
        return view().into_element();
      };
      let p = palette(dark_mode.get());
      let mut panel = view()
        .test_id("sidebar-context-menu")
        .w(px(220.0))
        .py_1()
        .rounded(px(8.0))
        .border_1()
        .border_color(p.border_normal)
        .bg(p.surface_overlay);

      match menu.target {
        ContextMenuTarget::Favorite { doc_id } => {
          let cb_rm = ctx_in.cb.clone();
          let cb_tab = ctx_in.cb.clone();
          let fav_title = ctx_in
            .tree
            .borrow()
            .visible_rows()
            .into_iter()
            .find(|r| !r.is_folder && (r.object_id == doc_id || r.view_id == doc_id))
            .map(|r| r.name)
            .unwrap_or_else(|| "未命名页面".to_string());
          panel = panel
            .child(menu_item_row("ctx-favorite-open-new-tab", "在新标签页打开", dark_mode, false, move || {
              context_menu.set(None); (cb_tab.on_open_in_new_tab)(doc_id, fav_title.clone());
            }))
            .child(menu_divider(dark_mode))
            .child(menu_item_row("ctx-favorite-remove", "取消收藏", dark_mode, false, move || {
              context_menu.set(None); (cb_rm.on_remove_favorite)(doc_id);
            }));
        }
        ContextMenuTarget::Batch { ids } => {
          let n = ids.len();
          let (cb_mv, cb_cp, cb_del) = (ctx_in.cb.clone(), ctx_in.cb.clone(), ctx_in.cb.clone());
          let (ids_mv, ids_cp, ids_del) = (ids.clone(), ids.clone(), ids);
          let sel_sig = ctx_in.selection;
          panel = panel
            .child(menu_item_row("ctx-batch-move", format!("移动 {n} 项到工作区…"), dark_mode, false, move || {
              context_menu.set(None); (cb_mv.on_transfer_move)(ids_mv.clone());
            }))
            .child(menu_item_row("ctx-batch-copy", format!("复制 {n} 项到工作区…"), dark_mode, false, move || {
              context_menu.set(None); (cb_cp.on_transfer_copy)(ids_cp.clone());
            }))
            .child(menu_divider(dark_mode))
            .child(menu_item_row("ctx-batch-delete", format!("删除 {n} 项"), dark_mode, true, move || {
              context_menu.set(None); (cb_del.on_batch_trash)(ids_del.clone());
              let mut s = sel_sig.get_untracked();
              if s.clear() { sel_sig.set(s); }
            }));
        }
        ContextMenuTarget::TreeRow { view_id, object_id, name, icon, is_folder, expanded } => {
          let (r_open, r_view, r_init) = (ctx_in.rename_modal_open, ctx_in.rename_modal_view, ctx_in.rename_modal_initial);
          let (e_open, e_view, e_cur) = (ctx_in.emoji_modal_open, ctx_in.emoji_modal_view, ctx_in.emoji_modal_current);
          let rename_name = name.clone();
          let cb_rename_host = ctx_in.cb.on_open_rename_modal.clone();
          let cb_emoji_host = ctx_in.cb.on_open_emoji_modal.clone();
          let (cb_dup, cb_mv, cb_cp, cb_del) = (ctx_in.cb.clone(), ctx_in.cb.clone(), ctx_in.cb.clone(), ctx_in.cb.clone());

          if !is_folder {
            let cb_tab = ctx_in.cb.clone();
            panel = panel.child(menu_item_row("ctx-open-new-tab", "在新标签页打开", dark_mode, false, move || {
              context_menu.set(None); (cb_tab.on_open_in_new_tab)(object_id, name.clone());
            }));
          } else {
            let (cb_np, cb_nf) = (ctx_in.cb.clone(), ctx_in.cb.clone());
            panel = panel
              .child(menu_item_row("ctx-new-child-page", "新建子页面", dark_mode, false, move || {
                context_menu.set(None); (cb_np.on_new_child_page)(view_id);
              }))
              .child(menu_item_row("ctx-new-child-folder", "新建子文件夹", dark_mode, false, move || {
                context_menu.set(None); (cb_nf.on_new_child_folder)(view_id);
              }));
          }

          panel = panel
            .child(menu_item_row("ctx-rename", "重命名", dark_mode, false, move || {
              context_menu.set(None);
              if let Some(open_r) = &cb_rename_host {
                open_r(view_id, rename_name.clone());
              } else {
                r_view.set(Some(view_id)); r_init.set(rename_name.clone()); r_open.set(true);
              }
            }))
            .child(menu_item_row("ctx-set-icon", "设置图标", dark_mode, false, move || {
              context_menu.set(None);
              if let Some(open_e) = &cb_emoji_host {
                open_e(view_id, icon.clone());
              } else {
                e_view.set(Some(view_id)); e_cur.set(icon.clone()); e_open.set(true);
              }
            }))
            .child(menu_item_row("ctx-duplicate", "创建副本", dark_mode, false, move || {
              context_menu.set(None); (cb_dup.on_duplicate_view)(view_id);
            }));

          if is_folder {
            let (tree_tog, rev_tog) = (ctx_in.tree.clone(), ctx_in.tree_rev);
            let (cb_bk, cb_md, cb_if, cb_id) = (ctx_in.cb.clone(), ctx_in.cb.clone(), ctx_in.cb.clone(), ctx_in.cb.clone());
            panel = panel
              .child(menu_item_row("ctx-toggle-children", if expanded { "收起子项" } else { "展开子项" }, dark_mode, false, move || {
                context_menu.set(None); tree_tog.borrow_mut().toggle_expanded(view_id); rev_tog.set(rev_tog.get_untracked() + 1);
              }))
              .child(menu_divider(dark_mode))
              .child(menu_item_row("ctx-export-backup", "备份(Mica 格式)", dark_mode, false, move || {
                context_menu.set(None); (cb_bk.on_export_folder_backup)(view_id);
              }))
              .child(menu_item_row("ctx-export-markdown", "导出为 Markdown", dark_mode, false, move || {
                context_menu.set(None); (cb_md.on_export_folder_markdown)(view_id);
              }))
              .child(menu_item_row("ctx-import-files", "导入文件(.md/.zip)", dark_mode, false, move || {
                context_menu.set(None); (cb_if.on_import_files_into_folder)(view_id);
              }))
              .child(menu_item_row("ctx-import-folder", "导入文件夹", dark_mode, false, move || {
                context_menu.set(None); (cb_id.on_import_folder_into_folder)(view_id);
              }));
          } else {
            let (cb_pmd, cb_phtml) = (ctx_in.cb.clone(), ctx_in.cb.clone());
            panel = panel
              .child(menu_divider(dark_mode))
              .child(menu_item_row("ctx-export-page-md", "导出为 Markdown", dark_mode, false, move || {
                context_menu.set(None); (cb_pmd.on_export_page_markdown)(object_id);
              }))
              .child(menu_item_row("ctx-export-page-html", "导出为 HTML", dark_mode, false, move || {
                context_menu.set(None); (cb_phtml.on_export_page_html)(object_id);
              }));
          }

          panel = panel
            .child(menu_divider(dark_mode))
            .child(menu_item_row("ctx-transfer-move", "移动到工作区", dark_mode, false, move || {
              context_menu.set(None); (cb_mv.on_transfer_move)(vec![view_id]);
            }))
            .child(menu_item_row("ctx-transfer-copy", "复制到工作区", dark_mode, false, move || {
              context_menu.set(None); (cb_cp.on_transfer_copy)(vec![view_id]);
            }))
            .child(menu_divider(dark_mode))
            .child(menu_item_row("ctx-delete", "删除", dark_mode, true, move || {
              context_menu.set(None); (cb_del.on_trash_view)(view_id);
            }));
        }
      }

      overlay()
        .anchor_point(move || point(px(menu.x), px(menu.y)))
        .placement(Placement::Bottom)
        .align(Align::Start)
        .offset(px(2.0))
        .margin(px(8.0))
        .on_press_outside(move |_: &Point<Points>| context_menu.set(None))
        .on_cancel(move || context_menu.set(None))
        .child(panel)
        .into_element()
    }
  })
}

fn menu_item_row(
  tid: &'static str,
  label: impl Into<String>,
  dark_mode: Signal<bool>,
  danger: bool,
  on_press: impl Fn() + 'static,
) -> impl IntoElement {
  let label_str = label.into();
  button()
    .test_id(tid).accessibility_label(label_str.clone())
    .w_full().h(px(TREE_ROW_HEIGHT)).px_3().flex_row().items_center()
    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
    .on_press(on_press)
    .child(
      text(label_str).text_size(px(13.0))
        .text_color(move || {
          let p = palette(dark_mode.get());
          if danger { p.danger } else { p.text_primary }
        })
        .truncate(),
    )
}

fn menu_divider(dark_mode: Signal<bool>) -> impl IntoElement {
  view().w_full().py_1().child(view().w_full().h(px(1.0)).bg(move || palette(dark_mode.get()).border_subtle))
}

// ── Account Footer (`28×28` avatar `本` + `本地工作区` + gear button) ─────────

/// Format the resident bottom-left account footer label without ever exposing
/// a full email address (`v0.14.11` privacy/compactness rule: details belong
/// inside the account popover, while the resident row shows only the owner name).
pub(crate) fn format_footer_account_label(raw: &str) -> String {
  let t = raw.trim();
  if t.is_empty() || t.eq_ignore_ascii_case("local") {
    return if crate::web_runtime::is_web() {
      "未登录".to_string()
    } else {
      "本地工作区".to_string()
    };
  }
  if let Some((local_part, domain)) = t.split_once('@')
    && !local_part.trim().is_empty()
    && domain.contains('.')
  {
    return local_part.trim().to_string();
  }
  t.to_string()
}

fn account_footer(account_name: Signal<String>, dark_mode: Signal<bool>, cb: Rc<SidebarCallbacks>) -> impl IntoElement {
  let cb_acct = cb.clone();
  let cb_set = cb;
  view()
    .h(px(44.0)).px_2().flex_row().items_center().justify_between().gap_1()
    .border_t_1().border_color(move || palette(dark_mode.get()).border_subtle)
    .child(
      button()
        .test_id("account-menu-btn").accessibility_label("打开账号菜单")
        .flex_1().min_w(px(0.0)).h(px(34.0)).px_1().flex_row().items_center().gap_2().rounded(px(6.0))
        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
        .on_press(move || (cb_acct.on_toggle_account_menu)())
        .child(
          view()
            .w(px(28.0)).h(px(28.0)).flex_shrink_0().rounded_full().items_center().justify_center()
            .bg(move || palette(dark_mode.get()).surface_hover)
            .child(
              text(move || {
                let label = format_footer_account_label(&account_name.get());
                if label == "未登录" {
                  "M".to_string()
                } else if label == "本地工作区" {
                  if crate::web_runtime::is_web() { "M".to_string() } else { "本".to_string() }
                } else {
                  label.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_else(|| "M".to_string())
                }
              })
              .text_size(px(12.0)).font_weight(FontWeight::MEDIUM)
              .text_color(move || palette(dark_mode.get()).text_primary),
            ),
        )
        .child(
          text(move || format_footer_account_label(&account_name.get()))
          .text_size(px(14.0)).font_weight(FontWeight::MEDIUM)
          .text_color(move || palette(dark_mode.get()).text_primary)
          .truncate(),
        ),
    )
    .child(
      button()
        .test_id("open-settings-btn").accessibility_label("打开设置")
        .w(px(28.0)).h(px(28.0)).flex_shrink_0().items_center().justify_center().rounded(px(6.0))
        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
        .on_press(move || (cb_set.on_open_settings)())
        .child(dynamic(move || mica_icons::settings_gear(palette(dark_mode.get()).text_muted_hex))),
    )
}

// ── Resize Handle ─────────────────────────────────────────────────────────────

fn sidebar_resize_handle(sidebar_width: Signal<f32>, dark_mode: Signal<bool>) -> impl IntoElement {
  let drag_start_w = signal(SIDEBAR_DEFAULT_WIDTH);
  let handle_state = signal(ResizeHandleState::Idle);
  resize_handle()
    .axis(Axis::Horizontal).edge(HandleEdge::Trailing).accessibility_label("拖拽调整侧边栏宽度")
    .on_drag_start(move || drag_start_w.set(sidebar_width.get_untracked()))
    .on_drag_move(move |offset: &Points| {
      sidebar_width.set((drag_start_w.get_untracked() + offset.0).clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH));
    })
    .on_drag_end(move |offset: &Points| {
      sidebar_width.set((drag_start_w.get_untracked() + offset.0).clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH));
    })
    .on_state_change(move |st: &ResizeHandleState| handle_state.set(*st))
    .child(
      view().test_id("sidebar-resize-handle").w(px(2.0)).h_full().bg(move || {
        let p = palette(dark_mode.get());
        match handle_state.get() {
          ResizeHandleState::Pressed | ResizeHandleState::Dragging => p.accent,
          ResizeHandleState::Hovered => p.border_strong,
          ResizeHandleState::Idle => Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.0 },
        }
      }),
    )
}

// ── Exported 320px Workspace Switcher Popover ─────────────────────────────────

/// 320px Workspace Switcher Popover (1:1 parity with Flutter `WorkspaceSelector`).
pub fn workspace_switcher_popover(
  open: Signal<bool>,
  items: Signal<Vec<WorkspaceSwitcherItem>>,
  active_workspace_id: Signal<Uuid>,
  dark_mode: Signal<bool>,
  cb: Rc<WorkspaceSwitcherCallbacks>,
) -> impl IntoElement {
  let filter_query = signal(String::new());
  let expanded_ws_menu: Signal<Option<Uuid>> = signal(None);
  let creating_new = signal(false);
  let new_ws_name = signal(String::new());
  let rename_ws_open = signal(false);
  let rename_ws_id: Signal<Option<Uuid>> = signal(None);
  let rename_ws_initial = signal(String::new());

  effect(move || {
    if !open.get() {
      return;
    }
    filter_query.set(String::new());
    expanded_ws_menu.set(None);
    creating_new.set(false);
    new_ws_name.set(String::new());
  });

  let cb_rename_commit = cb.clone();

  when(move || open.get() || rename_ws_open.get(), {
    let cb_outer = cb.clone();
    let cb_rename_outer = cb_rename_commit.clone();
    move || {
      let cb_dismiss = cb_outer.clone();
      let cb_close_btn = cb_outer.clone();
      let cb_filter_submit = cb_outer.clone();
      let cb_list = cb_outer.clone();
      let cb_create_submit = cb_outer.clone();
      let cb_create_btn = cb_outer.clone();
      let cb_imp_zip = cb_outer.clone();
      let cb_imp_notion = cb_outer.clone();
      let cb_ren = cb_rename_outer.clone();

      view()
        .absolute().inset_0().z_index(44)
        .child(when(move || open.get(), move || {
          let cb_dismiss = cb_dismiss.clone();
          let cb_close_btn = cb_close_btn.clone();
          let cb_filter_submit = cb_filter_submit.clone();
          let cb_list = cb_list.clone();
          let cb_create_submit = cb_create_submit.clone();
          let cb_create_btn = cb_create_btn.clone();
          let cb_imp_zip = cb_imp_zip.clone();
          let cb_imp_notion = cb_imp_notion.clone();

          view()
            .absolute().inset_0().z_index(44)
            .child(
              button()
                .test_id("workspace-switcher-backdrop").accessibility_label("关闭工作区切换菜单")
                .absolute().inset_0().on_press(move || (cb_dismiss.on_close)()),
            )
            .child(
              view()
                .test_id("workspace-switcher-popover")
                .absolute().top(px(78.0)).left(px(10.0)).w(px(320.0)).py(px(6.0))
                .rounded(px(8.0)).border_1()
                .border_color(move || palette(dark_mode.get()).border_normal)
                .bg(move || palette(dark_mode.get()).surface_overlay)
                .z_index(45)
                .child(
                  view().px_3().pb(px(4.0)).flex_row().items_center().justify_between()
                    .child(
                      text("切换工作区").text_size(px(12.0)).font_weight(FontWeight::MEDIUM)
                        .text_color(move || palette(dark_mode.get()).text_faint),
                    )
                    .child(
                      button()
                        .test_id("workspace-menu-close").accessibility_label("关闭工作区菜单")
                        .w(px(22.0)).h(px(22.0)).items_center().justify_center().rounded(px(4.0))
                        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                        .on_press(move || (cb_close_btn.on_close)())
                        .child(dynamic(move || mica_icons::close_sm(palette(dark_mode.get()).text_faint_hex))),
                    ),
                )
                .child(when(move || items.get().len() > WS_FILTER_THRESHOLD, {
                  let cb_sub = cb_filter_submit.clone();
                  move || {
                    let cb_s = cb_sub.clone();
                    view().px_3().pb_2().child(
                    view()
                      .h(px(32.0)).px_2().flex_row().items_center().gap_2().rounded(px(6.0))
                      .border_1().border_color(move || palette(dark_mode.get()).border_normal)
                      .bg(move || palette(dark_mode.get()).surface_sunken)
                      .child(dynamic(move || mica_icons::search(palette(dark_mode.get()).text_faint_hex)))
                      .child(
                        input()
                          .test_id("workspace-filter-input").auto_focus(true)
                          .submit_behavior(SubmitBehavior::Submit).placeholder("筛选工作区…")
                          .flex_1().min_w(px(0.0)).text_size(px(13.0))
                          .text_color(move || palette(dark_mode.get()).text_primary)
                          .bg(move || palette(dark_mode.get()).surface_sunken)
                          .value(move || filter_query.get())
                          .on_change(move |v: &str| filter_query.set(v.to_owned()))
                          .on_submit(move |q: &str| {
                            let needle = q.trim().to_lowercase();
                            let first = items.get_untracked().into_iter().find(|w| {
                              needle.is_empty() || w.name.to_lowercase().contains(&needle)
                            });
                            if let Some(ws) = first {
                              (cb_s.on_select)(ws.id);
                              (cb_s.on_close)();
                            }
                          }),
                      ),
                  )
                }
              }))
              .child(scroll().w_full().max_h(px(320.0)).child(dynamic(move || {
                let all = items.get();
                let q = filter_query.get().trim().to_lowercase();
                let filtered: Vec<WorkspaceSwitcherItem> = if all.len() > WS_FILTER_THRESHOLD && !q.is_empty() {
                  all.into_iter().filter(|w| w.name.to_lowercase().contains(&q)).collect()
                } else {
                  all
                };
                let cur_id = active_workspace_id.get();
                let open_sub = expanded_ws_menu.get();
                let p = palette(dark_mode.get());
                if filtered.is_empty() {
                  return view().w_full().px_3().py_2().child(
                    text("没有匹配的工作区").text_size(px(12.0)).text_color(p.text_faint),
                  );
                }
                let mut col = view().w_full();
                for (idx, ws) in filtered.into_iter().enumerate() {
                  let ws_id = ws.id;
                  let ws_name = ws.name.clone();
                  let is_cur = ws_id == cur_id;
                  let cb_sel = cb_list.clone();
                  let cb_close_sel = cb_list.clone();
                  let mut row_wrap = view().w_full().child(
                    view().w_full().px_2().flex_row().items_center()
                      .child(
                        button()
                          .test_id(&format!("workspace-item-{idx}"))
                          .accessibility_label(format!("切换到工作区 {ws_name}"))
                          .flex_1().min_w(px(0.0)).px_2().py_2().flex_row().items_center().gap_2().rounded(px(6.0))
                          .bg(if is_cur { p.surface_hover } else { p.surface_overlay })
                          .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                          .on_press(move || {
                            (cb_sel.on_select)(ws_id);
                            (cb_close_sel.on_close)();
                          })
                          .child(if is_cur { mica_icons::check_icon(p.text_primary_hex) } else { mica_icons::cloud(p.text_faint_hex) })
                          .child(
                            view().flex_1().min_w(px(0.0))
                              .child(
                                text(ws.name.clone()).text_size(px(14.0))
                                  .font_weight(if is_cur { FontWeight::MEDIUM } else { FontWeight::NORMAL })
                                  .text_color(p.text_primary).truncate(),
                              )
                              .child(if ws.page_count > 0 {
                                view().pt(px(2.0)).child(
                                  text(format!("{} 个页面", ws.page_count)).text_size(px(12.0)).text_color(p.text_faint),
                                )
                              } else {
                                view()
                              }),
                          ),
                      )
                      .child(
                        button()
                          .test_id(&format!("ws-row-menu-{idx}"))
                          .accessibility_label(format!("工作区 {ws_name} 菜单"))
                          .w(px(28.0)).h(px(28.0)).flex_shrink_0().items_center().justify_center().rounded(px(6.0))
                          .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                          .on_press(move || {
                            expanded_ws_menu.set(if expanded_ws_menu.get_untracked() == Some(ws_id) { None } else { Some(ws_id) });
                          })
                          .child(mica_icons::more_dots(p.text_muted_hex)),
                      ),
                  );

                  if open_sub == Some(ws_id) {
                    let (cb_bk, cb_md, cb_if, cb_id, cb_del) = (cb_list.clone(), cb_list.clone(), cb_list.clone(), cb_list.clone(), cb_list.clone());
                    let rename_init = ws.name.clone();
                    row_wrap = row_wrap.child(
                      view().mx_3().mb_1().py_1().rounded(px(6.0)).border_1().border_color(p.border_normal).bg(p.surface_sunken)
                        .child(menu_item_row("ws-action-rename", "重命名", dark_mode, false, move || {
                          expanded_ws_menu.set(None); rename_ws_id.set(Some(ws_id)); rename_ws_initial.set(rename_init.clone()); rename_ws_open.set(true);
                        }))
                        .child(menu_item_row("ws-action-export-backup", "导出：备份(Mica 格式)", dark_mode, false, move || {
                          expanded_ws_menu.set(None); (cb_bk.on_close)(); (cb_bk.on_export_backup)(ws_id);
                        }))
                        .child(menu_item_row("ws-action-export-md", "导出：Markdown", dark_mode, false, move || {
                          expanded_ws_menu.set(None); (cb_md.on_close)(); (cb_md.on_export_markdown)(ws_id);
                        }))
                        .child(menu_item_row("ws-action-import-files", "导入：文件(.md/.zip)", dark_mode, false, move || {
                          expanded_ws_menu.set(None); (cb_if.on_close)(); (cb_if.on_import_files)(ws_id);
                        }))
                        .child(menu_item_row("ws-action-import-folder", "导入：文件夹", dark_mode, false, move || {
                          expanded_ws_menu.set(None); (cb_id.on_close)(); (cb_id.on_import_folder)(ws_id);
                        }))
                        .child(menu_divider(dark_mode))
                        .child(menu_item_row("ws-action-delete", "删除", dark_mode, true, move || {
                          expanded_ws_menu.set(None); (cb_del.on_close)(); (cb_del.on_delete)(ws_id);
                        })),
                    );
                  }
                  col = col.child(row_wrap);
                }
                col
              })))
              .child(menu_divider(dark_mode))
              .child(when(move || !creating_new.get(), move || {
                button()
                  .test_id("ws-create-trigger").accessibility_label("新建工作区")
                  .w_full().h(px(36.0)).px_3().flex_row().items_center().gap_2()
                  .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                  .on_press(move || {
                    new_ws_name.set(String::new());
                    creating_new.set(true);
                  })
                  .child(dynamic(move || mica_icons::plus(palette(dark_mode.get()).text_primary_hex)))
                  .child(
                    text("新建工作区").text_size(px(14.0)).font_weight(FontWeight::MEDIUM)
                      .text_color(move || palette(dark_mode.get()).text_primary),
                  )
              }))
              .child(when(move || creating_new.get(), {
                let cb_sub = cb_create_submit.clone();
                let cb_btn = cb_create_btn.clone();
                move || {
                  let cb_s = cb_sub.clone();
                  let cb_b = cb_btn.clone();
                  view().px_3().py_1().flex_row().items_center().gap_2()
                    .child(
                      input()
                        .test_id("ws-create-input").auto_focus(true)
                        .submit_behavior(SubmitBehavior::Submit).placeholder("新工作区名称…")
                        .flex_1().min_w(px(0.0)).h(px(30.0)).px_2().rounded(px(6.0))
                        .border_1().border_color(move || palette(dark_mode.get()).accent)
                        .bg(move || palette(dark_mode.get()).surface_base)
                        .text_size(px(13.0)).text_color(move || palette(dark_mode.get()).text_primary)
                        .value(move || new_ws_name.get())
                        .on_change(move |v: &str| new_ws_name.set(v.to_owned()))
                        .on_submit(move |v: &str| {
                          let name = v.trim();
                          if !name.is_empty() {
                            (cb_s.on_create)(name.to_string());
                            creating_new.set(false);
                            (cb_s.on_close)();
                          }
                        }),
                    )
                    .child(
                      button()
                        .test_id("ws-create-confirm").accessibility_label("确认新建工作区")
                        .px_2().h(px(30.0)).items_center().justify_center().rounded(px(6.0))
                        .bg(move || palette(dark_mode.get()).accent)
                        .on_press(move || {
                          let raw = new_ws_name.get_untracked();
                          let name = raw.trim();
                          if !name.is_empty() {
                            (cb_b.on_create)(name.to_string());
                            creating_new.set(false);
                            (cb_b.on_close)();
                          }
                        })
                        .child(
                          text("创建").text_size(px(12.0)).font_weight(FontWeight::MEDIUM)
                            .text_color(move || palette(dark_mode.get()).on_primary),
                        ),
                    )
                }
              }))
              .child(
                view().w_full().pt(px(2.0))
                  .child(
                    view().px_3().py(px(4.0)).child(
                      text("导入工作区").text_size(px(12.0)).font_weight(FontWeight::MEDIUM)
                        .text_color(move || palette(dark_mode.get()).text_faint),
                    ),
                  )
                  .child(menu_item_row("ws-import-zip", "从 ZIP 导入(Mica 备份或 Markdown)", dark_mode, false, move || {
                    (cb_imp_zip.on_close)();
                    (cb_imp_zip.on_import_workspace_zip)();
                  }))
                  .child(menu_item_row("ws-import-notion", "从 Notion 导入(Markdown 和 CSV ZIP)", dark_mode, false, move || {
                    (cb_imp_notion.on_close)();
                    (cb_imp_notion.on_import_workspace_notion)();
                  })),
              ),
          )
        }))
        .child(rename_dialog_modal(
          rename_ws_open,
          rename_ws_initial,
          "重命名工作区",
          dark_mode,
          Rc::new(move |new_name: String| {
            if let Some(id) = rename_ws_id.get_untracked() {
              (cb_ren.on_rename)(id, new_name);
            }
            rename_ws_open.set(false);
            rename_ws_id.set(None);
          }),
          Rc::new(move || {
            rename_ws_open.set(false);
            rename_ws_id.set(None);
          }),
        ))
    }
  })
}

// ── Exported 400px Centered Rename Dialog Modal ───────────────────────────────

/// 400px centered Rename Dialog modal (1:1 parity with Flutter `RenameDialog`).
pub fn rename_dialog_modal(
  open: Signal<bool>,
  initial_name: Signal<String>,
  title: &'static str,
  dark_mode: Signal<bool>,
  on_submit: Rc<dyn Fn(String)>,
  on_close: Rc<dyn Fn()>,
) -> impl IntoElement {
  let draft = signal(String::new());

  let close_esc = on_close.clone();
  effect(move || {
    if !open.get() {
      return;
    }
    draft.set(initial_name.get_untracked());
    let close_key = close_esc.clone();
    on_key_with(
      "escape",
      KeyOptions { when: When::Always, repeat: false, pass_through: true, ..KeyOptions::default() },
      move |_| close_key(),
    );
  });

  when(move || open.get(), {
    let submit_outer = on_submit.clone();
    let close_outer = on_close.clone();
    move || {
      let submit_enter = submit_outer.clone();
      let close_enter = close_outer.clone();
      let submit_btn = submit_outer.clone();
      let close_btn = close_outer.clone();
      let close_cancel = close_outer.clone();
      let close_bg = close_outer.clone();
      let close_key = close_outer.clone();

      view()
        .test_id("rename-dialog-backdrop")
        .absolute().inset_0().z_index(60).items_center().justify_center()
        .bg(move || palette(dark_mode.get()).backdrop)
        .child(
          button()
            .test_id("rename-dialog-dismiss").accessibility_label("关闭重命名弹窗")
            .absolute().inset_0().on_press(move || close_bg()),
        )
        .child(
          view()
            .test_id("rename-dialog-modal")
            .w(px(400.0)).p(px(20.0)).gap(px(14.0)).rounded(px(10.0))
            .border_1().border_color(move || palette(dark_mode.get()).border_normal)
            .bg(move || palette(dark_mode.get()).surface_overlay)
            .z_index(61)
            .child(
              text(title).text_size(px(16.0)).font_weight(FontWeight::SEMIBOLD)
                .text_color(move || palette(dark_mode.get()).text_primary),
            )
            .child(
              view().w_full().gap(px(6.0))
                .child(
                  text("名称").text_size(px(12.0)).font_weight(FontWeight::MEDIUM)
                    .text_color(move || palette(dark_mode.get()).text_faint),
                )
                .child(
                  input()
                    .test_id("rename-dialog-input").auto_focus(true)
                    .submit_behavior(SubmitBehavior::Submit).placeholder("输入名称…")
                    .w_full().h(px(36.0)).px_3().rounded(px(6.0))
                    .border_1().border_color(move || palette(dark_mode.get()).border_normal)
                    .bg(move || palette(dark_mode.get()).surface_base)
                    .text_size(px(14.0)).text_color(move || palette(dark_mode.get()).text_primary)
                    .value(move || draft.get())
                    .on_change(move |v: &str| draft.set(v.to_owned()))
                    .on_submit(move |submitted: &str| {
                      let cur = initial_name.get_untracked();
                      if let Some(next) = renamed_to(submitted, &cur) {
                        submit_enter(next);
                      } else {
                        close_enter();
                      }
                    })
                    .on_key_press(move |k: &str| {
                      if k == "Escape" {
                        close_key();
                      }
                    }),
                ),
            )
            .child(
              view().w_full().flex_row().items_center().justify_end().gap_2()
                .child(
                  button()
                    .test_id("rename-dialog-cancel").accessibility_label("取消")
                    .h(px(32.0)).px_3().items_center().justify_center().rounded(px(6.0))
                    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                    .on_press(move || close_cancel())
                    .child(text("取消").text_size(px(13.0)).text_color(move || palette(dark_mode.get()).text_muted)),
                )
                .child(
                  button()
                    .test_id("rename-dialog-save").accessibility_label("保存")
                    .h(px(32.0)).px_4().items_center().justify_center().rounded(px(6.0))
                    .bg(move || palette(dark_mode.get()).accent)
                    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).accent_hover))
                    .on_press(move || {
                      let cur = initial_name.get_untracked();
                      let val = draft.get_untracked();
                      if let Some(next) = renamed_to(&val, &cur) {
                        submit_btn(next);
                      } else {
                        close_btn();
                      }
                    })
                    .child(
                      text("保存").text_size(px(13.0)).font_weight(FontWeight::MEDIUM)
                        .text_color(move || palette(dark_mode.get()).on_primary),
                    ),
                ),
            ),
        )
    }
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn footer_account_label_never_exposes_full_email() {
    assert_eq!(format_footer_account_label("willzhmic@outlook.com"), "willzhmic");
    assert_eq!(format_footer_account_label("Will Zhang"), "Will Zhang");
    assert_eq!(format_footer_account_label("本地工作区"), "本地工作区");
    assert_eq!(format_footer_account_label(""), "本地工作区");
  }

  #[test]
  fn tree_row_bg_preserves_blue_selection_tint_on_hover() {
    for dark in [false, true] {
      let p = palette(dark);
      let sel_idle = tree_row_bg(&p, true, false, false, false);
      assert_eq!(sel_idle, p.selection_bg);

      let sel_hover = tree_row_bg(&p, true, false, false, true);
      assert_eq!((sel_hover.r, sel_hover.g, sel_hover.b), (p.selection_bg.r, p.selection_bg.g, p.selection_bg.b));
      assert!(sel_hover.a > p.selection_bg.a, "hovered multi-selected row should keep stronger blue tint");
      assert_ne!(sel_hover, p.surface_hover);

      assert_eq!(tree_row_bg(&p, false, false, false, true), p.surface_hover);
      assert_eq!(tree_row_bg(&p, false, true, false, false), p.surface_hover);
      assert_eq!(tree_row_bg(&p, false, false, true, false), p.surface_hover);
      assert_eq!(tree_row_bg(&p, false, false, false, false), p.surface_raised);
    }
  }
}
