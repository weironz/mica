//! Workspace Settings & Members/Invitations modal (`440px`),
//! Workspace/Folder OverviewPane (`卡片视图` / `列表视图`), and
//! HomeScreen empty-tab start pane (`max_w 440px`).
//!
//! 1:1 parity with:
//! - `main:clients/mica_flutter/lib/main_workspace_view.dart` (`_openWorkspaceSettingsDialog`)
//! - `main:clients/mica_flutter/lib/ui/workspace_invitations.dart`
//! - `main:clients/mica_flutter/lib/ui/workspace_overview.dart`
//! - `main:clients/mica_flutter/lib/ui/overview_pane.dart`
//! - `main:clients/mica_flutter/lib/ui/home_screen.dart`

use std::rc::Rc;

use nsh_base::{button, input};
use nsh_core::{
  element::{ImageSrc, Svg, svg},
  geometry::px,
  prelude::*,
};
use uuid::Uuid;

use crate::{
  mica_icons,
  mica_theme::{MicaPalette, palette},
};

const INVITE_ROLES: &[&str] = &["admin", "editor", "viewer"];

// ============================================================================
// 1. Workspace Settings & Members / Invitations Modal (440px)
// ============================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceMemberEntry {
  pub user_id: String,
  pub display_name: String,
  pub email: String,
  pub role: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingInvitationEntry {
  pub id: String,
  pub email: String,
  pub role: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceStorageUsage {
  pub used_bytes: u64,
  pub quota_bytes: u64,
}

#[derive(Clone)]
pub struct WorkspaceSettingsState {
  pub open: Signal<bool>,
  pub workspace_id: Signal<String>,
  pub workspace_name: Signal<String>,
  pub role: Signal<String>,
  pub is_cloud: Signal<bool>,
  pub can_manage: Signal<bool>,
  pub storage_usage: Signal<Option<WorkspaceStorageUsage>>,
  pub members: Signal<Vec<WorkspaceMemberEntry>>,
  pub pending_invitations: Signal<Vec<PendingInvitationEntry>>,
  pub invite_status: Signal<Option<(String, bool)>>,
  pub on_rename: Rc<dyn Fn(String)>,
  pub on_invite: Rc<dyn Fn(String, String)>,
  pub on_revoke_invitation: Rc<dyn Fn(String)>,
  pub on_change_member_role: Rc<dyn Fn(String, String)>,
  pub on_remove_member: Rc<dyn Fn(String)>,
  pub on_delete_workspace: Rc<dyn Fn()>,
  pub on_close: Rc<dyn Fn()>,
}

/// Binary byte formatting matching Flutter `formatBytesBinary` (`B / KiB / MiB / GiB`).
pub fn format_bytes_binary(bytes: u64) -> String {
  const KIB: f64 = 1024.0;
  const MIB: f64 = 1024.0 * 1024.0;
  const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
  let b = bytes as f64;
  if b >= GIB {
    format!("{:.1} GiB", b / GIB)
  } else if b >= MIB {
    format!("{:.1} MiB", b / MIB)
  } else if b >= KIB {
    format!("{:.1} KiB", b / KIB)
  } else {
    format!("{bytes} B")
  }
}

/// Centered `440px` Workspace Settings modal (`工作区设置`).
pub fn workspace_settings_modal(
  state: WorkspaceSettingsState,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  when(move || state.open.get(), {
    let state = state.clone();
    move || {
      let rename_draft = signal(state.workspace_name.get_untracked());
      let invite_email = signal(String::new());
      let invite_role = signal("editor".to_string());
      let confirm_delete = signal(false);

      let st_bg = state.clone();
      let st_close_top = state.clone();
      let st_close_bottom = state.clone();
      let st_details = state.clone();
      let st_rename = state.clone();
      let st_members = state.clone();
      let st_del = state.clone();

      view()
        .test_id("workspace-settings-backdrop")
        .absolute()
        .inset_0()
        .z_index(60)
        .items_center()
        .justify_center()
        .bg(move || palette(dark_mode.get()).backdrop)
        .child(
          button()
            .test_id("workspace-settings-backdrop-dismiss")
            .accessibility_label("关闭工作区设置")
            .absolute()
            .inset_0()
            .on_press(move || (st_bg.on_close)()),
        )
        .child(
          view()
            .test_id("workspace-settings-modal")
            .w(px(440.0))
            .max_w(relative(0.92))
            .max_h(relative(0.86))
            .rounded(px(12.0))
            .border_1()
            .border_color(move || palette(dark_mode.get()).border_normal)
            .bg(move || palette(dark_mode.get()).surface_overlay)
            .overflow_hidden()
            .shadow_md()
            .z_index(61)
            .child(
              // Header
              view()
                .h(px(50.0))
                .px(px(20.0))
                .flex_row()
                .items_center()
                .justify_between()
                .border_b_1()
                .border_color(move || palette(dark_mode.get()).border_subtle)
                .child(
                  text("工作区设置")
                    .text_size(px(16.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(move || palette(dark_mode.get()).text_primary),
                )
                .child(
                  button()
                    .test_id("ws-settings-close-icon")
                    .accessibility_label("关闭")
                    .w(px(26.0))
                    .h(px(26.0))
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                    .on_press(move || (st_close_top.on_close)())
                    .child(dynamic(move || {
                      mica_icons::close_sm(palette(dark_mode.get()).text_faint_hex)
                    })),
                ),
            )
            .child(
              // Scrollable settings body
              scroll().flex_1().min_h(px(0.0)).child(
                view()
                  .w_full()
                  .px(px(20.0))
                  .py(px(16.0))
                  .gap(px(16.0))
                  .child(ws_details_section(st_details, dark_mode))
                  .child(ws_rename_section(st_rename, rename_draft, dark_mode))
                  .child(ws_divider(dark_mode))
                  .child(ws_members_section(
                    st_members,
                    invite_email,
                    invite_role,
                    dark_mode,
                  ))
                  .child(ws_divider(dark_mode))
                  .child(ws_delete_section(st_del, confirm_delete, dark_mode)),
              ),
            )
            .child(
              // Footer
              view()
                .px(px(20.0))
                .py(px(12.0))
                .flex_row()
                .items_center()
                .justify_end()
                .border_t_1()
                .border_color(move || palette(dark_mode.get()).border_subtle)
                .child(
                  button()
                    .test_id("ws-settings-close")
                    .accessibility_label("关闭工作区设置")
                    .px_3()
                    .py_1()
                    .rounded(px(6.0))
                    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                    .on_press(move || (st_close_bottom.on_close)())
                    .child(
                      text("关闭")
                        .text_size(px(13.0))
                        .text_color(move || palette(dark_mode.get()).text_primary),
                    ),
                ),
            ),
        )
    }
  })
}

fn ws_divider(dark_mode: Signal<bool>) -> impl IntoElement {
  view()
    .w_full()
    .h(px(1.0))
    .bg(move || palette(dark_mode.get()).border_subtle)
}

fn ws_details_section(
  state: WorkspaceSettingsState,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  view().w_full().gap(px(8.0)).child(dynamic(move || {
    let p = palette(dark_mode.get());
    let role = state.role.get();
    let ws_id = state.workspace_id.get();
    let usage = state.storage_usage.get();

    let mut col = view()
      .w_full()
      .gap(px(6.0))
      .child(detail_row("角色", role, p))
      .child(detail_row("ID", ws_id, p));

    if let Some(u) = usage {
      let label = if u.quota_bytes > 0 {
        format!(
          "{} / {}",
          format_bytes_binary(u.used_bytes),
          format_bytes_binary(u.quota_bytes)
        )
      } else {
        format_bytes_binary(u.used_bytes)
      };
      col = col.child(detail_row("存储", label, p));
      // Mirror Flutter: nothing stored -> omit progress bar so empty track doesn't read as a stray rule.
      if u.quota_bytes > 0 && u.used_bytes > 0 {
        let ratio = (u.used_bytes as f32 / u.quota_bytes as f32).clamp(0.0, 1.0);
        col = col.child(
          view()
            .w_full()
            .h(px(6.0))
            .rounded(px(3.0))
            .bg(p.surface_sunken)
            .overflow_hidden()
            .child(
              view()
                .h_full()
                .w(relative(ratio))
                .rounded(px(3.0))
                .bg(p.accent),
            ),
        );
      }
    }
    col
  }))
}

fn detail_row(label: &'static str, value: String, p: MicaPalette) -> impl IntoElement {
  view()
    .w_full()
    .flex_row()
    .items_center()
    .justify_between()
    .gap_3()
    .child(text(label).text_size(px(12.5)).text_color(p.text_muted))
    .child(
      text(value)
        .text_size(px(12.5))
        .text_color(p.text_primary)
        .truncate(),
    )
}

fn ws_rename_section(
  state: WorkspaceSettingsState,
  draft: Signal<String>,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  let on_rename_submit = state.on_rename.clone();
  let on_rename_btn = state.on_rename.clone();

  view()
    .w_full()
    .gap(px(8.0))
    .child(
      text("重命名工作区")
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
        .child(
          input()
            .test_id("ws-settings-rename-input")
            .flex_1()
            .h(px(34.0))
            .px_3()
            .rounded(px(6.0))
            .border_1()
            .border_color(move || palette(dark_mode.get()).border_normal)
            .bg(move || palette(dark_mode.get()).surface_base)
            .text_size(px(13.0))
            .text_color(move || palette(dark_mode.get()).text_primary)
            .placeholder("工作区名称")
            .value(move || draft.get())
            .on_change(move |v: &str| draft.set(v.to_owned()))
            .on_submit(move |v: &str| {
              let trimmed = v.trim();
              if !trimmed.is_empty() {
                on_rename_submit(trimmed.to_string());
              }
            }),
        )
        .child(
          button()
            .test_id("ws-settings-rename-save")
            .accessibility_label("保存工作区名称")
            .h(px(34.0))
            .px_3()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .bg(move || palette(dark_mode.get()).accent)
            .on_press(move || {
              let trimmed = draft.get_untracked().trim().to_string();
              if !trimmed.is_empty() {
                on_rename_btn(trimmed);
              }
            })
            .child(
              text("保存")
                .text_size(px(13.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || palette(dark_mode.get()).on_primary),
            ),
        ),
    )
}

fn ws_members_section(
  state: WorkspaceSettingsState,
  invite_email: Signal<String>,
  invite_role: Signal<String>,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  view().w_full().child(dynamic(move || {
    let p = palette(dark_mode.get());
    if !state.is_cloud.get() {
      return view()
        .test_id("ws-members-local-empty")
        .w_full()
        .p(px(14.0))
        .rounded(px(8.0))
        .bg(p.surface_raised)
        .border_1()
        .border_color(p.border_subtle)
        .items_center()
        .gap(px(6.0))
        .child(users_icon(p.text_faint_hex))
        .child(
          text("本地工作区没有成员")
            .text_size(px(13.0))
            .font_weight(FontWeight::MEDIUM)
            .text_color(p.text_primary),
        )
        .child(
          text("它只存在这台设备上，不需要账号，也不能共享。用工作区菜单里的「上云…」把它变成云端工作区，就能邀请协作者。")
            .text_size(px(12.0))
            .text_color(p.text_faint),
        )
        .into_element();
    }

    let can_manage = state.can_manage.get();
    let members = state.members.get();
    let pending = state.pending_invitations.get();
    let status = state.invite_status.get();

    let mut col = view().w_full().gap(px(10.0)).child(
      view()
        .flex_row()
        .items_center()
        .gap_2()
        .child(users_icon(p.text_muted_hex))
        .child(
          text("成员")
            .text_size(px(14.0))
            .font_weight(FontWeight::MEDIUM)
            .text_color(p.text_primary),
        ),
    );

    if can_manage {
      let cur_role = invite_role.get();
      let on_inv = state.on_invite.clone();
      let mut role_row = view().flex_row().items_center().gap_1();
      for role_opt in INVITE_ROLES {
        let active = cur_role == *role_opt;
        let r_str = *role_opt;
        role_row = role_row.child(
          button()
            .test_id(&format!("ws-invite-role-{r_str}"))
            .accessibility_label(format!("角色 {r_str}"))
            .px_2()
            .py_1()
            .rounded(px(6.0))
            .border_1()
            .border_color(if active { p.border_strong } else { p.border_normal })
            .bg(if active { p.surface_hover } else { p.surface_base })
            .on_press(move || invite_role.set(r_str.to_string()))
            .child(
              text(r_str)
                .text_size(px(12.0))
                .font_weight(if active { FontWeight::MEDIUM } else { FontWeight::NORMAL })
                .text_color(if active { p.text_primary } else { p.text_muted }),
            ),
        );
      }

      col = col.child(
        view()
          .test_id("ws-member-invite-section")
          .w_full()
          .gap(px(8.0))
          .child(
            input()
              .test_id("ws-invite-email-input")
              .w_full()
              .h(px(34.0))
              .px_3()
              .rounded(px(6.0))
              .border_1()
              .border_color(p.border_normal)
              .bg(p.surface_base)
              .text_size(px(13.0))
              .text_color(p.text_primary)
              .placeholder("成员邮箱")
              .value(move || invite_email.get())
              .on_change(move |v: &str| invite_email.set(v.to_owned())),
          )
          .child(
            view()
              .w_full()
              .flex_row()
              .items_center()
              .justify_between()
              .gap_2()
              .child(role_row)
              .child(
                button()
                  .test_id("ws-invite-submit-btn")
                  .accessibility_label("邀请成员")
                  .h(px(30.0))
                  .px_3()
                  .items_center()
                  .justify_center()
                  .rounded(px(6.0))
                  .bg(p.accent)
                  .on_press(move || {
                    let email = invite_email.get_untracked().trim().to_string();
                    if !email.is_empty() {
                      on_inv(email, invite_role.get_untracked());
                      invite_email.set(String::new());
                    }
                  })
                  .child(
                    text("邀请")
                      .text_size(px(12.5))
                      .font_weight(FontWeight::MEDIUM)
                      .text_color(p.on_primary),
                  ),
              ),
          ),
      );

      if let Some((msg, is_err)) = status {
        col = col.child(
          text(msg)
            .text_size(px(12.0))
            .text_color(if is_err { p.danger } else { p.success }),
        );
      }

      if !pending.is_empty() {
        let mut p_list = view().w_full().gap(px(6.0)).child(
          text("待接受的邀请")
            .text_size(px(12.0))
            .font_weight(FontWeight::MEDIUM)
            .text_color(p.text_muted),
        );
        for (i, inv) in pending.into_iter().enumerate() {
          let revoke = state.on_revoke_invitation.clone();
          let inv_id = inv.id;
          p_list = p_list.child(
            view()
              .test_id(&format!("ws-pending-invite-{i}"))
              .w_full()
              .px_2()
              .py_1()
              .flex_row()
              .items_center()
              .gap_2()
              .rounded(px(6.0))
              .bg(p.surface_raised)
              .child(mail_icon(p.text_faint_hex))
              .child(
                text(format!("{} · {}", inv.email, inv.role))
                  .flex_1()
                  .text_size(px(12.5))
                  .text_color(p.text_primary)
                  .truncate(),
              )
              .child(
                view()
                  .px(px(6.0))
                  .py(px(2.0))
                  .rounded(px(6.0))
                  .bg(p.warning_wash)
                  .child(text("待接受").text_size(px(11.0)).text_color(p.warning)),
              )
              .child(
                button()
                  .test_id(&format!("ws-revoke-invite-{i}"))
                  .accessibility_label("撤回邀请")
                  .w(px(24.0))
                  .h(px(24.0))
                  .items_center()
                  .justify_center()
                  .rounded(px(6.0))
                  .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                  .on_press(move || revoke(inv_id.clone()))
                  .child(mica_icons::close_sm(p.text_faint_hex)),
              ),
          );
        }
        col = col.child(p_list);
      }
    }

    if members.is_empty() {
      col = col.child(text("未加载成员。").text_size(px(12.5)).text_color(p.text_faint));
    } else {
      for (i, m) in members.into_iter().enumerate() {
        let initial = m
          .display_name
          .chars()
          .next()
          .or_else(|| m.email.chars().next())
          .unwrap_or('U')
          .to_uppercase()
          .to_string();
        let can_edit_member = can_manage && m.role != "owner";
        let on_cycle_role = state.on_change_member_role.clone();
        let on_rm = state.on_remove_member.clone();
        let uid_role = m.user_id.clone();
        let uid_rm = m.user_id.clone();
        let next_role = match m.role.as_str() {
          "admin" => "editor",
          "editor" => "viewer",
          _ => "admin",
        };
        let label_name = if m.display_name.trim().is_empty() {
          m.email.clone()
        } else {
          m.display_name.clone()
        };

        let mut row = view()
          .test_id(&format!("ws-member-row-{i}"))
          .w_full()
          .px_2()
          .py_2()
          .flex_row()
          .items_center()
          .gap_2()
          .rounded(px(8.0))
          .border_1()
          .border_color(p.border_subtle)
          .bg(p.surface_base)
          .child(
            view()
              .w(px(28.0))
              .h(px(28.0))
              .rounded_full()
              .bg(p.surface_sunken)
              .items_center()
              .justify_center()
              .flex_shrink_0()
              .child(
                text(initial)
                  .text_size(px(12.0))
                  .font_weight(FontWeight::MEDIUM)
                  .text_color(p.text_primary),
              ),
          )
          .child(
            view()
              .flex_1()
              .min_w(px(0.0))
              .child(
                text(label_name)
                  .text_size(px(13.0))
                  .font_weight(FontWeight::MEDIUM)
                  .text_color(p.text_primary)
                  .truncate(),
              )
              .child(
                text(m.email)
                  .text_size(px(11.5))
                  .text_color(p.text_faint)
                  .truncate(),
              ),
          );

        if can_edit_member {
          row = row
            .child(
              button()
                .test_id(&format!("ws-member-role-{i}"))
                .accessibility_label("切换成员角色")
                .px_2()
                .py_1()
                .rounded(px(6.0))
                .bg(p.surface_sunken)
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                .on_press(move || on_cycle_role(uid_role.clone(), next_role.to_string()))
                .child(text(m.role).text_size(px(11.5)).text_color(p.text_primary)),
            )
            .child(
              button()
                .test_id(&format!("ws-member-remove-{i}"))
                .accessibility_label("移除成员")
                .w(px(24.0))
                .h(px(24.0))
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).danger_wash))
                .on_press(move || on_rm(uid_rm.clone()))
                .child(mica_icons::trash_icon(p.danger_hex)),
            );
        } else {
          row = row.child(
            view()
              .px_2()
              .py_1()
              .rounded(px(6.0))
              .bg(p.surface_sunken)
              .child(text(m.role).text_size(px(11.5)).text_color(p.text_muted)),
          );
        }
        col = col.child(row);
      }
    }

    col.into_element()
  }))
}

fn ws_delete_section(
  state: WorkspaceSettingsState,
  confirm_delete: Signal<bool>,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  view().w_full().child(dynamic(move || {
    let p = palette(dark_mode.get());
    let ws_name = state.workspace_name.get();
    if !confirm_delete.get() {
      return button()
        .test_id("ws-settings-delete-btn")
        .accessibility_label("删除工作区")
        .w_full()
        .px_3()
        .py_2()
        .flex_row()
        .items_center()
        .justify_center()
        .gap_2()
        .rounded(px(6.0))
        .border_1()
        .border_color(p.border_normal)
        .bg(p.danger_wash)
        .on_press(move || confirm_delete.set(true))
        .child(mica_icons::trash_icon(p.danger_hex))
        .child(
          text("删除工作区")
            .text_size(px(13.0))
            .font_weight(FontWeight::MEDIUM)
            .text_color(p.danger),
        )
        .into_element();
    }

    let on_del = state.on_delete_workspace.clone();
    view()
      .test_id("ws-settings-delete-confirm")
      .w_full()
      .p_3()
      .rounded(px(8.0))
      .border_1()
      .border_color(p.danger)
      .bg(p.danger_wash)
      .gap(px(10.0))
      .child(
        text(format!("删除“{ws_name}”及其所有页面吗？此操作不可撤销。"))
          .text_size(px(12.5))
          .text_color(p.danger),
      )
      .child(
        view()
          .flex_row()
          .items_center()
          .justify_end()
          .gap_2()
          .child(
            button()
              .test_id("ws-settings-cancel-delete")
              .px_3()
              .py_1()
              .rounded(px(6.0))
              .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
              .on_press(move || confirm_delete.set(false))
              .child(text("取消").text_size(px(12.5)).text_color(p.text_primary)),
          )
          .child(
            button()
              .test_id("ws-settings-confirm-delete")
              .px_3()
              .py_1()
              .rounded(px(6.0))
              .bg(p.danger)
              .on_press(move || {
                confirm_delete.set(false);
                on_del();
              })
              .child(
                text("确认删除")
                  .text_size(px(12.5))
                  .font_weight(FontWeight::MEDIUM)
                  .text_color(p.on_primary),
              ),
          ),
      )
      .into_element()
  }))
}

// ============================================================================
// 2. OverviewPane (Breadcrumb + Cards / List Mode + Dashed Leader + Empty State)
// ============================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverviewItem {
  pub view_id: Uuid,
  pub object_id: Uuid,
  pub name: String,
  pub icon: Option<String>,
  pub is_folder: bool,
  pub child_count: usize,
  pub updated_label: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverviewMode {
  Cards,
  List,
}

/// Workspace / folder overview pane with breadcrumb (`全部页面 › Folder`),
/// `内容` header with Cards / List toggle (`卡片视图` / `列表视图`),
/// responsive card grid vs dashed-leader directory list, and empty state.
///
/// `on_open_item` receives `(id, is_folder)` where `id` is `view_id` for folders
/// and `object_id` for pages.
pub fn overview_pane_view(
  breadcrumb: Signal<Vec<(Option<Uuid>, String)>>,
  items: Signal<Vec<OverviewItem>>,
  dark_mode: Signal<bool>,
  on_open_item: Rc<dyn Fn(Uuid, bool)>,
  on_nav_folder: Rc<dyn Fn(Option<Uuid>)>,
  on_new_page: Rc<dyn Fn()>,
) -> impl IntoElement {
  let mode = signal(OverviewMode::Cards);
  let pane_width = signal(960.0_f32);

  view()
    .test_id("overview-pane")
    .flex_1()
    .min_h(px(0.0))
    .w_full()
    .bg(move || palette(dark_mode.get()).surface_base)
    .on_layout(move |b: &Bounds<Points>| {
      let w = f32::from(b.size.width).max(260.0);
      if (pane_width.get_untracked() - w).abs() > 2.0 {
        pane_width.set(w);
      }
    })
    .child(overview_breadcrumb(breadcrumb, dark_mode, on_nav_folder))
    .child(
      scroll().flex_1().min_h(px(0.0)).child(
        view()
          .w_full()
          .px(px(24.0))
          .pb(px(24.0))
          .gap(px(18.0))
          .child(overview_header(mode, dark_mode))
          .child(dynamic(move || {
            let list = items.get();
            let cur_mode = mode.get();
            let p = palette(dark_mode.get());
            if list.is_empty() {
              return overview_empty_state(dark_mode, p, on_new_page.clone()).into_element();
            }
            let content_w = (pane_width.get() - 48.0).max(220.0);
            match cur_mode {
              OverviewMode::Cards => {
                overview_card_grid(list, content_w, dark_mode, p, on_open_item.clone()).into_element()
              }
              OverviewMode::List => {
                overview_directory_list(list, content_w, dark_mode, p, on_open_item.clone())
                  .into_element()
              }
            }
          })),
      ),
    )
}

fn overview_breadcrumb(
  breadcrumb: Signal<Vec<(Option<Uuid>, String)>>,
  dark_mode: Signal<bool>,
  on_nav_folder: Rc<dyn Fn(Option<Uuid>)>,
) -> impl IntoElement {
  view()
    .test_id("overview-breadcrumb")
    .w_full()
    .px(px(24.0))
    .pt(px(18.0))
    .pb(px(10.0))
    .child(dynamic(move || {
      let p = palette(dark_mode.get());
      let raw = breadcrumb.get();
      let mut trail: Vec<(Option<Uuid>, String)> = Vec::with_capacity(raw.len() + 1);
      if raw.first().is_none_or(|(id, _)| id.is_some()) {
        trail.push((None, "全部页面".to_string()));
      }
      trail.extend(raw);

      let last_idx = trail.len().saturating_sub(1);
      let mut row = view().flex_row().flex_wrap().items_center().gap(px(4.0));
      for (i, (folder_id, label)) in trail.into_iter().enumerate() {
        if i > 0 {
          row = row.child(
            view()
              .px(px(2.0))
              .child(mica_icons::chevron_right(p.text_faint_hex)),
          );
        }
        let is_current = i == last_idx;
        if is_current {
          row = row.child(
            text(label)
              .text_size(px(13.0))
              .font_weight(FontWeight::MEDIUM)
              .text_color(p.text_primary),
          );
        } else {
          let nav = on_nav_folder.clone();
          row = row.child(
            button()
              .test_id(&format!("overview-crumb-{i}"))
              .accessibility_label(format!("返回 {label}"))
              .px_1()
              .py(px(2.0))
              .rounded(px(6.0))
              .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
              .on_press(move || nav(folder_id))
              .child(text(label).text_size(px(13.0)).text_color(p.text_muted)),
          );
        }
      }
      row
    }))
}

fn overview_header(mode: Signal<OverviewMode>, dark_mode: Signal<bool>) -> impl IntoElement {
  view()
    .w_full()
    .flex_row()
    .items_center()
    .justify_between()
    .child(
      text("内容")
        .text_size(px(13.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(move || palette(dark_mode.get()).text_muted),
    )
    .child(dynamic(move || {
      let p = palette(dark_mode.get());
      let cur = mode.get();
      view()
        .p(px(2.0))
        .flex_row()
        .items_center()
        .gap(px(2.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(p.border_normal)
        .child(mode_toggle_btn(
          "overview-mode-cards",
          "卡片视图",
          cur == OverviewMode::Cards,
          dark_mode,
          p,
          grid_icon(if cur == OverviewMode::Cards {
            p.text_primary_hex
          } else {
            p.text_faint_hex
          }),
          move || mode.set(OverviewMode::Cards),
        ))
        .child(mode_toggle_btn(
          "overview-mode-list",
          "列表视图",
          cur == OverviewMode::List,
          dark_mode,
          p,
          mica_icons::list_bullet(if cur == OverviewMode::List {
            p.text_primary_hex
          } else {
            p.text_faint_hex
          }),
          move || mode.set(OverviewMode::List),
        ))
    }))
}

#[allow(clippy::too_many_arguments)]
fn mode_toggle_btn(
  tid: &'static str,
  label: &'static str,
  selected: bool,
  dark_mode: Signal<bool>,
  p: MicaPalette,
  icon: Svg,
  on_press: impl Fn() + 'static,
) -> impl IntoElement {
  button()
    .test_id(tid)
    .accessibility_label(label)
    .w(px(32.0))
    .h(px(28.0))
    .items_center()
    .justify_center()
    .rounded(px(6.0))
    .bg(if selected { p.surface_hover } else { p.surface_base })
    .hover(move |s| {
      let pal = palette(dark_mode.get_untracked());
      s.bg(if selected { pal.surface_hover } else { pal.surface_sunken })
    })
    .on_press(on_press)
    .child(icon)
}

fn overview_card_grid(
  items: Vec<OverviewItem>,
  width: f32,
  dark_mode: Signal<bool>,
  p: MicaPalette,
  on_open_item: Rc<dyn Fn(Uuid, bool)>,
) -> impl IntoElement {
  let columns = if width >= 720.0 {
    3
  } else if width >= 460.0 {
    2
  } else {
    1
  };
  let card_w = ((width - 12.0 * (columns as f32 - 1.0)) / columns as f32)
    .floor()
    .max(180.0);

  let mut grid = view().w_full().flex_row().flex_wrap().gap(px(12.0));
  for (i, item) in items.into_iter().enumerate() {
    let open = on_open_item.clone();
    let target_id = if item.is_folder { item.view_id } else { item.object_id };
    let is_folder = item.is_folder;
    let meta = if !item.updated_label.is_empty() {
      item.updated_label.clone()
    } else if is_folder {
      format!("{} 项", item.child_count)
    } else {
      String::new()
    };

    let has_meta = !meta.is_empty();
    let mut card_row = view()
      .w_full()
      .flex_row()
      .items_center()
      .gap(px(12.0))
      .child(item_glyph(&item, p, 20.0))
      .child(
        view()
          .flex_1()
          .min_w(px(0.0))
          .gap(px(2.0))
          .child(
            text(item.name.clone())
              .text_size(px(14.0))
              .font_weight(FontWeight::MEDIUM)
              .text_color(p.text_primary)
              .truncate(),
          )
          .child(when(move || has_meta, {
            let meta_s = meta.clone();
            move || {
              text(meta_s.clone())
                .text_size(px(12.0))
                .text_color(p.text_faint)
                .truncate()
            }
          })),
      );

    if is_folder {
      card_row = card_row.child(
        text(format!("{}", item.child_count))
          .text_size(px(12.0))
          .text_color(p.text_faint),
      );
    }

    grid = grid.child(
      button()
        .test_id(&format!("overview-card-{i}"))
        .accessibility_label(item.name)
        .w(px(card_w))
        .px(px(16.0))
        .py(px(14.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(p.border_subtle)
        .bg(p.surface_base)
        .hover(move |s| {
          let pal = palette(dark_mode.get_untracked());
          s.border_color(pal.border_strong).bg(pal.surface_hover)
        })
        .on_press(move || open(target_id, is_folder))
        .child(card_row),
    );
  }
  grid
}

fn overview_directory_list(
  items: Vec<OverviewItem>,
  width: f32,
  dark_mode: Signal<bool>,
  p: MicaPalette,
  on_open_item: Rc<dyn Fn(Uuid, bool)>,
) -> impl IntoElement {
  let name_max_w = (width * 0.55).clamp(80.0, 520.0);
  let mut col = view().w_full().gap(px(2.0));

  for (i, item) in items.into_iter().enumerate() {
    let open = on_open_item.clone();
    let target_id = if item.is_folder { item.view_id } else { item.object_id };
    let is_folder = item.is_folder;
    let trailing = if is_folder {
      format!("{}", item.child_count)
    } else {
      item.updated_label.clone()
    };

    col = col.child(
      button()
        .test_id(&format!("overview-row-{i}"))
        .accessibility_label(item.name.clone())
        .w_full()
        .px(px(8.0))
        .py(px(7.0))
        .flex_row()
        .items_center()
        .rounded(px(6.0))
        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_sunken))
        .on_press(move || open(target_id, is_folder))
        .child(item_glyph(&item, p, 16.0))
        .child(view().w(px(8.0)))
        .child(
          view().max_w(px(name_max_w)).child(
            text(item.name)
              .text_size(px(13.0))
              .font_weight(if is_folder {
                FontWeight::MEDIUM
              } else {
                FontWeight::NORMAL
              })
              .text_color(p.text_primary)
              .truncate(),
          ),
        )
        .child(dashed_leader(dark_mode))
        .child(
          text(trailing)
            .text_size(px(12.0))
            .text_color(p.text_faint),
        ),
    );
  }
  col
}

fn dashed_leader(dark_mode: Signal<bool>) -> impl IntoElement {
  let leader_w = signal(120.0_f32);
  view()
    .flex_1()
    .min_w(px(16.0))
    .h(px(1.0))
    .mx(px(10.0))
    .child(
      canvas(move || {
        let w = leader_w.get().max(1.0);
        let col = palette(dark_mode.get()).border_normal;
        let mut p = Painter::new()
          .stroke_color(col)
          .line_width(px(1.0))
          .line_dash(&[px(3.0), px(3.0)]);
        p = p
          .begin_path()
          .move_to(px(0.0), px(0.5))
          .line_to(px(w), px(0.5))
          .stroke();
        p
      })
      .w_full()
      .h_full()
      .on_layout(move |b: &Bounds<Points>| {
        let w = f32::from(b.size.width).max(1.0);
        if (leader_w.get_untracked() - w).abs() > 1.0 {
          leader_w.set(w);
        }
      }),
    )
}

fn item_glyph(item: &OverviewItem, p: MicaPalette, size_px: f32) -> impl IntoElement {
  if let Some(emoji) = item.icon.as_deref()
    && !emoji.is_empty()
  {
    return text(emoji.to_string())
      .text_size(px(size_px))
      .into_element();
  }
  if item.is_folder {
    mica_icons::folder(p.text_faint_hex).into_element()
  } else {
    mica_icons::file_doc(p.text_faint_hex).into_element()
  }
}

fn overview_empty_state(
  dark_mode: Signal<bool>,
  p: MicaPalette,
  on_new_page: Rc<dyn Fn()>,
) -> impl IntoElement {
  view()
    .test_id("overview-empty")
    .w_full()
    .px(px(26.0))
    .py(px(40.0))
    .items_center()
    .gap(px(10.0))
    .child(
      view()
        .w(px(46.0))
        .h(px(46.0))
        .rounded(px(8.0))
        .bg(p.surface_sunken)
        .items_center()
        .justify_center()
        .child(mica_icons::folder(p.text_faint_hex)),
    )
    .child(
      text("这里还没有内容")
        .text_size(px(14.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(p.text_primary),
    )
    .child(
      text("新建一篇页面，或从侧栏把已有页面移进来。")
        .text_size(px(13.0))
        .text_color(p.text_faint),
    )
    .child(
      button()
        .test_id("overview-empty-new-page")
        .accessibility_label("新建页面")
        .mt(px(4.0))
        .px(px(14.0))
        .py(px(7.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(p.border_strong)
        .bg(p.surface_hover)
        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_sunken))
        .on_press(move || on_new_page())
        .child(
          text("新建页面")
            .text_size(px(13.0))
            .font_weight(FontWeight::MEDIUM)
            .text_color(p.text_primary),
        ),
    )
}

// ============================================================================
// 3. HomeScreen Empty-Tab Start Pane (max_w 440px)
// ============================================================================

/// Centered `max_w(440px)` empty-tab start pane:
/// `新建页面 (Ctrl+N)`, `搜索页面 (Ctrl+Shift+F)`, divider, and `最近编辑` list.
pub fn home_screen_view(
  recent_pages: Signal<Vec<(Uuid, String, String)>>,
  dark_mode: Signal<bool>,
  on_new_page: Rc<dyn Fn()>,
  on_open_search: Rc<dyn Fn()>,
  on_open_page: Rc<dyn Fn(Uuid)>,
) -> impl IntoElement {
  view()
    .test_id("home-screen")
    .flex_1()
    .w_full()
    .h_full()
    .items_center()
    .justify_center()
    .bg(move || palette(dark_mode.get()).surface_base)
    .child(
      scroll().w_full().child(
        view()
          .w_full()
          .items_center()
          .px(px(32.0))
          .py(px(48.0))
          .child(
            view()
              .w_full()
              .max_w(px(440.0))
              .gap(px(4.0))
              .child(home_action_row(
                "home-new-page-btn",
                "新建页面",
                "Ctrl+N",
                dark_mode,
                move |p| mica_icons::plus(p.text_muted_hex),
                on_new_page,
              ))
              .child(home_action_row(
                "home-search-btn",
                "搜索页面",
                "Ctrl+Shift+F",
                dark_mode,
                move |p| mica_icons::search(p.text_muted_hex),
                on_open_search,
              ))
              .child(dynamic(move || {
                let recents = recent_pages.get();
                if recents.is_empty() {
                  return view().into_element();
                }
                let p = palette(dark_mode.get());
                let mut col = view()
                  .w_full()
                  .pt(px(12.0))
                  .gap(px(4.0))
                  .child(view().w_full().h(px(1.0)).bg(p.border_subtle))
                  .child(
                    view().px_3().pt(px(10.0)).pb(px(2.0)).child(
                      text("最近编辑")
                        .text_size(px(12.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.text_faint),
                    ),
                  );
                for (i, (doc_id, title, meta)) in recents.into_iter().enumerate() {
                  let open = on_open_page.clone();
                  let display_title = if title.trim().is_empty() {
                    "无标题".to_string()
                  } else {
                    title
                  };
                  col = col.child(
                    button()
                      .test_id(&format!("home-recent-{i}"))
                      .accessibility_label(format!("打开最近编辑 {display_title}"))
                      .w_full()
                      .px_3()
                      .py(px(10.0))
                      .flex_row()
                      .items_center()
                      .gap_3()
                      .rounded(px(8.0))
                      .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                      .on_press(move || open(doc_id))
                      .child(mica_icons::history_icon(p.text_muted_hex))
                      .child(
                        text(display_title)
                          .flex_1()
                          .text_size(px(14.0))
                          .text_color(p.text_primary)
                          .truncate(),
                      )
                      .child(text(meta).text_size(px(12.0)).text_color(p.text_faint)),
                  );
                }
                col.into_element()
              })),
          ),
      ),
    )
}

fn home_action_row(
  tid: &'static str,
  label: &'static str,
  hint: &'static str,
  dark_mode: Signal<bool>,
  icon_fn: impl Fn(MicaPalette) -> Svg + 'static,
  on_press: Rc<dyn Fn()>,
) -> impl IntoElement {
  button()
    .test_id(tid)
    .accessibility_label(label)
    .w_full()
    .px_3()
    .py(px(12.0))
    .flex_row()
    .items_center()
    .gap_3()
    .rounded(px(8.0))
    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
    .on_press(move || on_press())
    .child(dynamic(move || icon_fn(palette(dark_mode.get()))))
    .child(
      text(label)
        .flex_1()
        .text_size(px(14.0))
        .text_color(move || palette(dark_mode.get()).text_primary)
        .truncate(),
    )
    .child(
      text(hint)
        .text_size(px(12.0))
        .text_color(move || palette(dark_mode.get()).text_faint),
    )
}

// ============================================================================
// Inline 1.5px Stroke SVG Helpers
// ============================================================================

fn stroked_svg(size_px: f32, color: &str, body: &str) -> Svg {
  let markup = format!(
    r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="none" stroke="{color}" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">{body}</svg>"##
  );
  svg(ImageSrc::svg_markup(&markup))
    .w(px(size_px))
    .h(px(size_px))
}

fn grid_icon(color: &str) -> Svg {
  stroked_svg(
    15.0,
    color,
    r##"<rect x="3.5" y="3.5" width="5.2" height="5.2" rx="1"/><rect x="11.3" y="3.5" width="5.2" height="5.2" rx="1"/><rect x="3.5" y="11.3" width="5.2" height="5.2" rx="1"/><rect x="11.3" y="11.3" width="5.2" height="5.2" rx="1"/>"##,
  )
}

fn users_icon(color: &str) -> Svg {
  stroked_svg(
    16.0,
    color,
    r##"<circle cx="8" cy="7.2" r="2.6"/><path d="M3.2 15.8C3.6 13.1 5.6 11.8 8 11.8C10.4 11.8 12.4 13.1 12.8 15.8"/><circle cx="13.8" cy="7.8" r="2"/><path d="M13.6 12C15.3 12.2 16.6 13.4 17 15.5"/>"##,
  )
}

fn mail_icon(color: &str) -> Svg {
  stroked_svg(
    15.0,
    color,
    r##"<rect x="3" y="5" width="14" height="10" rx="1.6"/><path d="M3.5 5.8L10 10.6L16.5 5.8"/>"##,
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn format_bytes_binary_matches_expected_units() {
    assert_eq!(format_bytes_binary(512), "512 B");
    assert_eq!(format_bytes_binary(1024), "1.0 KiB");
    assert_eq!(format_bytes_binary(5 * 1024 * 1024), "5.0 MiB");
    assert_eq!(format_bytes_binary(2 * 1024 * 1024 * 1024), "2.0 GiB");
  }
}
