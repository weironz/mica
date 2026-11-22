//! 1:1 Settings Modal Dialog (`settings_shell.dart` + all 8 tabs):
//! - Responsive `1120×840` shell (`max_w(94%)`, `max_h(92%)`)
//! - Full-width top header (`设置` + close `×`) spanning both panes
//! - Left `200px` warm-gray nav rail with 4 groups (`通用` / `账户` / `工作区` / `其他`)
//!   + 8 tabs + pinned bottom `关于` footer behind a `1px` divider
//! - Right scrollable detail pane (`28px` padding, `24px w600` title, `13px` subtitle)
//! - Backward-compatible [`SettingsCallbacks`] + [`settings_modal_view`] alongside the
//!   full [`SettingsViewProps`] + [`settings_modal_view_with_props`] entrypoint.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use chrono::{DateTime, Utc};
use mica_api_types::View as MicaView;
use nsh_base::{button, input};
use nsh_core::{SubmitBehavior, geometry::px, prelude::*};
use uuid::Uuid;

use crate::{mica_icons, mica_theme::palette, transfer};

// ── Tab IDs (8 tabs across 4 groups) ─────────────────────────────────────────

const TAB_APPEARANCE: u8 = 0;
const TAB_SHORTCUTS: u8 = 1;
const TAB_ACCOUNT: u8 = 2;
const TAB_TOKENS: u8 = 3;
const TAB_DATA: u8 = 4;
const TAB_TRASH: u8 = 5;
const TAB_AI: u8 = 6;
const TAB_DIAGNOSTICS: u8 = 7;

/// 11 discrete page-width stops (680..=1880 px, step 120; last stop = 全宽).
pub const PAGE_WIDTH_STOPS: [u32; 11] = [
  680, 800, 920, 1040, 1160, 1280, 1400, 1520, 1640, 1760, 1880,
];
pub const PAGE_WIDTH_DEFAULT: u32 = 800;
pub const PAGE_WIDTH_FULL: u32 = 1880;
pub const EDITOR_FONT_PX_MIN: u32 = 13;
pub const EDITOR_FONT_PX_MAX: u32 = 22;

const AI_PRESETS: &[(&str, &str, &str, &str)] = &[
  ("deepseek", "DeepSeek", "openai", "https://api.deepseek.com"),
  ("zhipu", "智谱 GLM", "openai", "https://open.bigmodel.cn/api/coding/paas/v4"),
  ("kimi", "月之暗面 Kimi", "openai", "https://api.moonshot.cn/v1"),
  ("openai", "OpenAI", "openai", "https://api.openai.com/v1"),
  ("anthropic", "Anthropic (Claude)", "anthropic", "https://api.anthropic.com"),
  ("custom", "Local / Custom", "openai", "http://localhost:11434/v1"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeChoice {
  System,
  Light,
  Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanguageChoice {
  System,
  Chinese,
  English,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiFontScale {
  Small,
  Standard,
  Large,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorFontFamily {
  System,
  Serif,
  Mono,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowCloseBehavior {
  Quit,
  MinimizeToTray,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiTokenItem {
  pub id: String,
  pub name: String,
  pub can_write: bool,
  pub last_used_label: String,
  pub expires_label: String,
  pub expired: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiProviderEntry {
  pub id: String,
  pub protocol: String,
  pub base_url: String,
  pub model: String,
  pub has_key: bool,
  pub key_hint: String,
  pub active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LocalCacheStatsView {
  pub local_only_pages: usize,
  pub local_only_bytes: u64,
  pub mirrored_pages: usize,
  pub mirrored_bytes: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrashRootEntry {
  pub view: MicaView,
  pub original_path: String,
  pub child_folders: usize,
  pub child_pages: usize,
  pub relative_deleted: String,
}

/// Legacy callback bundle preserved 100% compatible with existing `lib.rs`.
pub struct SettingsCallbacks {
  pub dark_mode: Signal<bool>,
  pub locale_zh: Signal<bool>,
  pub server_url: Signal<String>,
  pub cloud_status: Signal<String>,
  pub export_preview: Signal<String>,
  pub account_email: Signal<String>,
  pub is_cloud_signed_in: Signal<bool>,
  pub on_toggle_theme: Rc<dyn Fn()>,
  pub on_toggle_locale: Rc<dyn Fn()>,
  pub on_check_server: Rc<dyn Fn(String)>,
  pub on_cloud_login: Rc<dyn Fn(String, String, String)>,
  pub on_cloud_logout: Rc<dyn Fn()>,
  pub on_sync_now: Rc<dyn Fn()>,
  pub on_export_markdown: Rc<dyn Fn()>,
  pub on_export_html: Rc<dyn Fn()>,
  pub on_export_bundle: Rc<dyn Fn()>,
  pub on_import_markdown: Rc<dyn Fn(String)>,
  pub on_save_snapshot: Rc<dyn Fn()>,
  pub on_close: Rc<dyn Fn()>,
}

/// Full 1:1 Settings props covering all 8 tabs + pinned About footer.
#[allow(clippy::type_complexity)]
pub struct SettingsViewProps {
  pub dark_mode: Signal<bool>,
  pub locale_zh: Signal<bool>,
  pub page_width_px: Signal<u32>,
  pub editor_font_px: Signal<u32>,
  pub editor_font_family: Signal<EditorFontFamily>,
  pub ui_font_scale: Signal<UiFontScale>,
  pub show_format_bar: Signal<bool>,
  pub show_page_title: Signal<bool>,
  pub rehost_images: Signal<bool>,
  pub close_behavior: Signal<WindowCloseBehavior>,
  pub on_toggle_theme: Rc<dyn Fn()>,
  pub on_toggle_locale: Rc<dyn Fn()>,
  pub on_set_dark_mode: Rc<dyn Fn(bool)>,
  pub on_set_locale_zh: Rc<dyn Fn(bool)>,

  pub server_url: Signal<String>,
  pub cloud_status: Signal<String>,
  pub account_email: Signal<String>,
  pub account_name: Signal<String>,
  pub is_cloud_signed_in: Signal<bool>,
  pub on_check_server: Rc<dyn Fn(String)>,
  pub on_cloud_login: Rc<dyn Fn(String, String, String)>,
  pub on_cloud_logout: Rc<dyn Fn()>,
  pub on_sync_now: Rc<dyn Fn()>,
  pub on_update_profile: Option<Rc<dyn Fn(String) -> Result<(), String>>>,
  pub on_change_password: Option<Rc<dyn Fn(String, String) -> Result<(), String>>>,
  pub on_delete_account: Option<Rc<dyn Fn(String) -> Result<(), String>>>,

  pub tokens: Signal<Vec<ApiTokenItem>>,
  pub on_create_token: Option<Rc<dyn Fn(String, bool, Option<u32>) -> Result<String, String>>>,
  pub on_revoke_token: Option<Rc<dyn Fn(String) -> Result<(), String>>>,

  pub export_preview: Signal<String>,
  pub cache_stats: Signal<LocalCacheStatsView>,
  pub on_export_markdown: Rc<dyn Fn()>,
  pub on_export_html: Rc<dyn Fn()>,
  pub on_export_bundle: Rc<dyn Fn()>,
  pub on_export_mica_zip: Option<Rc<dyn Fn()>>,
  pub on_export_markdown_zip: Option<Rc<dyn Fn()>>,
  pub on_import_markdown: Rc<dyn Fn(String)>,
  pub on_import_zip: Option<Rc<dyn Fn(bool)>>,
  pub on_import_folder: Option<Rc<dyn Fn()>>,
  pub on_save_snapshot: Rc<dyn Fn()>,
  pub on_clear_mirror_cache: Option<Rc<dyn Fn() -> Result<String, String>>>,

  pub trashed_views: Signal<Vec<MicaView>>,
  pub live_views: Signal<Vec<MicaView>>,
  pub on_restore_view: Option<Rc<dyn Fn(Uuid)>>,
  pub on_purge_view: Option<Rc<dyn Fn(Uuid)>>,
  pub on_purge_all_trash: Option<Rc<dyn Fn() -> usize>>,
  pub on_refresh_trash: Option<Rc<dyn Fn()>>,

  pub ai_enabled: Signal<bool>,
  pub ai_providers: Signal<Vec<AiProviderEntry>>,
  pub on_save_ai_provider: Option<Rc<dyn Fn(AiProviderEntry, Option<String>) -> Result<(), String>>>,
  pub on_delete_ai_provider: Option<Rc<dyn Fn(String) -> Result<(), String>>>,
  pub on_fetch_ai_models:
    Option<Rc<dyn Fn(String, String, String, Option<String>) -> Result<Vec<String>, String>>>,

  pub diagnostics_enabled: Signal<bool>,
  pub diagnostics_dir: String,
  pub diagnostics_dropped: Option<String>,
  pub on_open_diagnostics_folder: Option<Rc<dyn Fn()>>,
  pub on_open_about: Option<Rc<dyn Fn()>>,
  pub on_close: Rc<dyn Fn()>,
}

impl SettingsViewProps {
  /// Wrap a legacy [`SettingsCallbacks`] into a full [`SettingsViewProps`].
  pub fn from_legacy(cb: Rc<SettingsCallbacks>) -> Self {
    let dark_mode = cb.dark_mode;
    let locale_zh = cb.locale_zh;
    let on_toggle_theme = cb.on_toggle_theme.clone();
    let on_toggle_locale = cb.on_toggle_locale.clone();
    let default_providers = vec![
      AiProviderEntry {
        id: "deepseek".into(),
        protocol: "openai".into(),
        base_url: "https://api.deepseek.com".into(),
        model: String::new(),
        has_key: false,
        key_hint: String::new(),
        active: true,
      },
      AiProviderEntry {
        id: "openai".into(),
        protocol: "openai".into(),
        base_url: "https://api.openai.com/v1".into(),
        model: String::new(),
        has_key: false,
        key_hint: String::new(),
        active: false,
      },
    ];
    let initial_name = cb.account_email.get_untracked();
    Self {
      dark_mode,
      locale_zh,
      page_width_px: signal(PAGE_WIDTH_DEFAULT),
      editor_font_px: signal(16),
      editor_font_family: signal(EditorFontFamily::System),
      ui_font_scale: signal(UiFontScale::Standard),
      show_format_bar: signal(true),
      show_page_title: signal(true),
      rehost_images: signal(true),
      close_behavior: signal(WindowCloseBehavior::Quit),
      on_toggle_theme: on_toggle_theme.clone(),
      on_toggle_locale: on_toggle_locale.clone(),
      on_set_dark_mode: Rc::new(move |want_dark| {
        if dark_mode.get_untracked() != want_dark {
          on_toggle_theme();
        }
      }),
      on_set_locale_zh: Rc::new(move |want_zh| {
        if locale_zh.get_untracked() != want_zh {
          on_toggle_locale();
        }
      }),
      server_url: cb.server_url,
      cloud_status: cb.cloud_status,
      account_email: cb.account_email,
      account_name: signal(initial_name),
      is_cloud_signed_in: cb.is_cloud_signed_in,
      on_check_server: cb.on_check_server.clone(),
      on_cloud_login: cb.on_cloud_login.clone(),
      on_cloud_logout: cb.on_cloud_logout.clone(),
      on_sync_now: cb.on_sync_now.clone(),
      on_update_profile: None,
      on_change_password: None,
      on_delete_account: None,
      tokens: signal(Vec::new()),
      on_create_token: None,
      on_revoke_token: None,
      export_preview: cb.export_preview,
      cache_stats: signal(LocalCacheStatsView::default()),
      on_export_markdown: cb.on_export_markdown.clone(),
      on_export_html: cb.on_export_html.clone(),
      on_export_bundle: cb.on_export_bundle.clone(),
      on_export_mica_zip: None,
      on_export_markdown_zip: None,
      on_import_markdown: cb.on_import_markdown.clone(),
      on_import_zip: None,
      on_import_folder: None,
      on_save_snapshot: cb.on_save_snapshot.clone(),
      on_clear_mirror_cache: None,
      trashed_views: signal(Vec::new()),
      live_views: signal(Vec::new()),
      on_restore_view: None,
      on_purge_view: None,
      on_purge_all_trash: None,
      on_refresh_trash: None,
      ai_enabled: signal(true),
      ai_providers: signal(default_providers),
      on_save_ai_provider: None,
      on_delete_ai_provider: None,
      on_fetch_ai_models: None,
      diagnostics_enabled: signal(false),
      diagnostics_dir: default_diagnostics_dir(),
      diagnostics_dropped: None,
      on_open_diagnostics_folder: None,
      on_open_about: None,
      on_close: cb.on_close.clone(),
    }
  }
}

fn default_diagnostics_dir() -> String {
  #[cfg(not(target_arch = "wasm32"))]
  {
    dirs_next::data_local_dir()
      .or_else(dirs_next::cache_dir)
      .unwrap_or_else(std::env::temp_dir)
      .join("mica")
      .join("diagnostics")
      .display()
      .to_string()
  }
  #[cfg(target_arch = "wasm32")]
  {
    "browser://mica/diagnostics".to_string()
  }
}

// ── Pure Logic Helpers ───────────────────────────────────────────────────────

pub fn format_page_width_label(width_px: u32) -> String {
  if width_px >= PAGE_WIDTH_FULL {
    "全宽".to_string()
  } else {
    format!("{width_px} px")
  }
}

pub fn is_valid_ai_provider_id(id: &str) -> bool {
  let mut chars = id.chars();
  let Some(first) = chars.next() else {
    return false;
  };
  if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
    return false;
  }
  chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

pub fn format_bytes_human(bytes: u64) -> String {
  const KB: f64 = 1024.0;
  const MB: f64 = KB * 1024.0;
  const GB: f64 = MB * 1024.0;
  let b = bytes as f64;
  if b >= GB {
    format!("{:.1} GB", b / GB)
  } else if b >= MB {
    format!("{:.1} MB", b / MB)
  } else if b >= KB {
    format!("{:.1} KB", b / KB)
  } else {
    format!("{bytes} B")
  }
}

pub fn format_relative_time(ts: DateTime<Utc>, now: DateTime<Utc>) -> String {
  let secs = (now - ts).num_seconds().max(0);
  if secs < 60 {
    "刚刚".to_string()
  } else if secs < 3600 {
    format!("{} 分钟前", secs / 60)
  } else if secs < 86_400 {
    format!("{} 小时前", secs / 3600)
  } else {
    format!("{} 天前", secs / 86_400)
  }
}

pub fn build_trash_root_entries(
  deleted: &[MicaView],
  live: &[MicaView],
  now: DateTime<Utc>,
) -> Vec<TrashRootEntry> {
  let deleted_ids: HashSet<Uuid> = deleted.iter().map(|v| v.id).collect();
  let live_by_id: HashMap<Uuid, &MicaView> = live.iter().map(|v| (v.id, v)).collect();
  let mut children_by_parent: HashMap<Uuid, Vec<&MicaView>> = HashMap::new();
  for v in deleted {
    if let Some(pid) = v.parent_view_id
      && deleted_ids.contains(&pid)
    {
      children_by_parent.entry(pid).or_default().push(v);
    }
  }
  let mut roots: Vec<&MicaView> = deleted
    .iter()
    .filter(|v| v.parent_view_id.is_none_or(|pid| !deleted_ids.contains(&pid)))
    .collect();
  roots.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then_with(|| a.name.cmp(&b.name)));

  roots
    .into_iter()
    .map(|root| {
      let (mut child_folders, mut child_pages) = (0usize, 0usize);
      let mut stack = vec![root.id];
      let mut visited = HashSet::from([root.id]);
      while let Some(curr) = stack.pop() {
        if let Some(kids) = children_by_parent.get(&curr) {
          for &kid in kids {
            if visited.insert(kid.id) {
              if kid.object_type == "folder" {
                child_folders += 1;
              } else {
                child_pages += 1;
              }
              stack.push(kid.id);
            }
          }
        }
      }
      let mut path_parts = Vec::new();
      let mut cursor = root.parent_view_id;
      let mut guard = HashSet::new();
      while let Some(pid) = cursor {
        if !guard.insert(pid) {
          break;
        }
        if let Some(pv) = live_by_id.get(&pid) {
          path_parts.push(pv.name.clone());
          cursor = pv.parent_view_id;
        } else {
          break;
        }
      }
      path_parts.reverse();
      TrashRootEntry {
        view: root.clone(),
        original_path: path_parts.join(" / "),
        child_folders,
        child_pages,
        relative_deleted: format_relative_time(root.updated_at, now),
      }
    })
    .collect()
}

fn ai_preset_label(id: &str) -> String {
  AI_PRESETS
    .iter()
    .find(|&&(pid, _, _, _)| pid != "custom" && pid == id)
    .map(|&(_, label, _, _)| label.to_string())
    .unwrap_or_else(|| id.to_string())
}

// ── Public Entrypoints ───────────────────────────────────────────────────────

/// Backward-compatible entrypoint called by existing `lib.rs`.
pub fn settings_modal_view(cb: Rc<SettingsCallbacks>) -> impl IntoElement {
  settings_modal_view_with_props(Rc::new(SettingsViewProps::from_legacy(cb)))
}

/// Full 1:1 `1120×840` Settings modal dialog with all 8 tabs and pinned About footer.
pub fn settings_modal_view_with_props(props: Rc<SettingsViewProps>) -> impl IntoElement {
  let active_tab = signal(TAB_APPEARANCE);
  let about_open = signal(false);
  let theme_choice = signal(if props.dark_mode.get_untracked() {
    ThemeChoice::Dark
  } else {
    ThemeChoice::Light
  });
  let lang_choice = signal(if props.locale_zh.get_untracked() {
    LanguageChoice::Chinese
  } else {
    LanguageChoice::English
  });

  // Tab-local signals
  let email_draft = signal(props.account_email.get_untracked());
  let password_draft = signal(String::new());
  let display_name_draft = signal(props.account_name.get_untracked());
  let cur_pass_draft = signal(String::new());
  let new_pass_draft = signal(String::new());
  let account_msg = signal(String::new());
  let account_msg_err = signal(false);
  let delete_confirm_open = signal(false);
  let delete_pass_draft = signal(String::new());

  let token_name_draft = signal(String::new());
  let token_write_draft = signal(false);
  let token_expiry_days: Signal<Option<u32>> = signal(Some(90));
  let token_error = signal(String::new());
  let created_secret: Signal<Option<String>> = signal(None);
  let confirm_revoke_id: Signal<Option<String>> = signal(None);

  let import_path_draft = signal(String::new());
  let cache_msg = signal(String::new());
  let confirm_clear_cache = signal(false);

  let confirm_purge_id: Signal<Option<Uuid>> = signal(None);
  let confirm_empty_trash = signal(false);
  let trash_notice = signal(String::new());

  let initial_ai_id = props
    .ai_providers
    .get_untracked()
    .into_iter()
    .find(|p| p.active)
    .map(|p| p.id)
    .unwrap_or_else(|| "deepseek".into());
  let ai_selected_id = signal(initial_ai_id);
  let ai_adding = signal(false);
  let ai_add_preset = signal("zhipu".to_string());
  let ai_add_custom_name = signal(String::new());
  let ai_add_base_url = signal("https://open.bigmodel.cn/api/coding/paas/v4".to_string());
  let ai_add_err = signal(String::new());
  let ai_api_key_draft = signal(String::new());
  let ai_fetched_models: Signal<Vec<String>> = signal(Vec::new());
  let ai_status_msg = signal(String::new());
  let ai_status_err = signal(false);

  let dm = props.dark_mode;
  let close_bg = props.on_close.clone();
  let close_btn = props.on_close.clone();
  let on_about = props.on_open_about.clone();
  let p_body = props.clone();

  view()
    .test_id("settings-modal-backdrop")
    .absolute()
    .inset_0()
    .z_index(50)
    .items_center()
    .justify_center()
    .bg(move || palette(dm.get()).backdrop)
    .child(
      button()
        .test_id("settings-backdrop-dismiss")
        .accessibility_label("关闭设置弹窗")
        .absolute()
        .inset_0()
        .on_press(move || close_bg()),
    )
    .child(
      // Responsive 1120×840 shell matching Flutter `MicaSettingsShell`
      view()
        .test_id("settings-panel")
        .w(px(1120.0))
        .max_w(relative(0.94))
        .h(px(840.0))
        .max_h(relative(0.92))
        .rounded(px(12.0))
        .border_1()
        .border_color(move || palette(dm.get()).border_normal)
        .bg(move || palette(dm.get()).surface_overlay)
        .overflow_hidden()
        .z_index(51)
        .child(
          // Full-width top header spanning BOTH nav rail and detail pane
          view()
            .w_full()
            .h(px(52.0))
            .px_4()
            .flex_row()
            .items_center()
            .justify_between()
            .border_b_1()
            .border_color(move || palette(dm.get()).border_subtle)
            .child(
              view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(dynamic(move || mica_icons::settings_gear(palette(dm.get()).text_muted_hex)))
                .child(
                  text("设置")
                    .text_size(px(16.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(move || palette(dm.get()).text_primary),
                ),
            )
            .child(
              button()
                .test_id("settings-close-btn")
                .accessibility_label("关闭设置")
                .w(px(30.0))
                .h(px(30.0))
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .hover(move |s| s.bg(palette(dm.get_untracked()).surface_hover))
                .on_press(move || close_btn())
                .child(dynamic(move || mica_icons::close_sm(palette(dm.get()).text_faint_hex))),
            ),
        )
        .child(
          // Two-column body: 200px left rail + scrollable right detail pane
          view()
            .flex_1()
            .w_full()
            .flex_row()
            .overflow_hidden()
            .child(left_nav_rail(active_tab, about_open, dm, on_about))
            .child(
              scroll()
                .flex_1()
                .h_full()
                .bg(move || palette(dm.get()).surface_base)
                .p(px(28.0))
                .child(
                  view()
                    .w_full()
                    .child(
                      text(move || tab_title(active_tab.get()))
                        .text_size(px(24.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || palette(dm.get()).text_primary),
                    )
                    .child(
                      view().pt_2().pb(px(28.0)).child(
                        text(move || tab_description(active_tab.get()))
                          .text_size(px(13.0))
                          .text_color(move || palette(dm.get()).text_muted),
                      ),
                    )
                    .child(when(move || about_open.get(), move || {
                      about_inline_card(dm, about_open)
                    }))
                    .child(when(move || active_tab.get() == TAB_APPEARANCE, {
                      let p = p_body.clone();
                      move || appearance_tab(p.clone(), theme_choice, lang_choice)
                    }))
                    .child(when(move || active_tab.get() == TAB_SHORTCUTS, move || {
                      shortcuts_tab(dm)
                    }))
                    .child(when(move || active_tab.get() == TAB_ACCOUNT, {
                      let p = p_body.clone();
                      move || {
                        account_tab(
                          p.clone(),
                          email_draft,
                          password_draft,
                          display_name_draft,
                          cur_pass_draft,
                          new_pass_draft,
                          account_msg,
                          account_msg_err,
                          delete_confirm_open,
                          delete_pass_draft,
                        )
                      }
                    }))
                    .child(when(move || active_tab.get() == TAB_TOKENS, {
                      let p = p_body.clone();
                      move || {
                        tokens_tab(
                          p.clone(),
                          token_name_draft,
                          token_write_draft,
                          token_expiry_days,
                          token_error,
                          created_secret,
                          confirm_revoke_id,
                        )
                      }
                    }))
                    .child(when(move || active_tab.get() == TAB_DATA, {
                      let p = p_body.clone();
                      move || data_tab(p.clone(), import_path_draft, cache_msg, confirm_clear_cache)
                    }))
                    .child(when(move || active_tab.get() == TAB_TRASH, {
                      let p = p_body.clone();
                      move || {
                        trash_tab(p.clone(), confirm_purge_id, confirm_empty_trash, trash_notice)
                      }
                    }))
                    .child(when(move || active_tab.get() == TAB_AI, {
                      let p = p_body.clone();
                      move || {
                        ai_tab(
                          p.clone(),
                          ai_selected_id,
                          ai_adding,
                          ai_add_preset,
                          ai_add_custom_name,
                          ai_add_base_url,
                          ai_add_err,
                          ai_api_key_draft,
                          ai_fetched_models,
                          ai_status_msg,
                          ai_status_err,
                        )
                      }
                    }))
                    .child(when(move || active_tab.get() == TAB_DIAGNOSTICS, {
                      let p = p_body.clone();
                      move || diagnostics_tab(p.clone())
                    })),
                ),
            ),
        ),
    )
}

fn left_nav_rail(
  active_tab: Signal<u8>,
  about_open: Signal<bool>,
  dm: Signal<bool>,
  on_about: Option<Rc<dyn Fn()>>,
) -> impl IntoElement {
  view()
    .w(px(200.0))
    .h_full()
    .flex_shrink_0()
    .border_r_1()
    .border_color(move || palette(dm.get()).border_subtle)
    .bg(move || palette(dm.get()).surface_raised)
    .child(
      scroll().flex_1().p_3().child(
        view()
          .w_full()
          .gap_1()
          .child(nav_group("通用", true, dm))
          .child(nav_item("settings-tab-appearance", "外观", TAB_APPEARANCE, active_tab, dm, mica_icons::sun_icon))
          .child(nav_item("settings-tab-about", "快捷键", TAB_SHORTCUTS, active_tab, dm, mica_icons::keyboard_icon))
          .child(nav_group("账户", false, dm))
          .child(nav_item("settings-tab-account", "账户", TAB_ACCOUNT, active_tab, dm, mica_icons::cloud))
          .child(nav_item("settings-tab-tokens", "API 令牌", TAB_TOKENS, active_tab, dm, mica_icons::link_icon))
          .child(nav_group("工作区", false, dm))
          .child(nav_item("settings-tab-data", "数据", TAB_DATA, active_tab, dm, mica_icons::database_icon))
          .child(nav_item("settings-tab-trash", "回收站", TAB_TRASH, active_tab, dm, mica_icons::trash_icon))
          .child(nav_group("其他", false, dm))
          .child(nav_item("settings-tab-ai", "AI 服务商", TAB_AI, active_tab, dm, mica_icons::sparkles_icon))
          .child(nav_item("settings-tab-diagnostics", "诊断", TAB_DIAGNOSTICS, active_tab, dm, mica_icons::outline_icon)),
      ),
    )
    .child(
      view()
        .w_full()
        .border_t_1()
        .border_color(move || palette(dm.get()).border_subtle)
        .px_3()
        .py_2()
        .child(
          button()
            .test_id("settings-about-footer")
            .accessibility_label("关于 Mica")
            .w_full()
            .h(px(34.0))
            .px_2()
            .flex_row()
            .items_center()
            .justify_between()
            .rounded(px(6.0))
            .hover(move |s| s.bg(palette(dm.get_untracked()).surface_hover))
            .on_press(move || {
              if let Some(cb) = &on_about {
                cb();
              } else {
                about_open.set(!about_open.get_untracked());
              }
            })
            .child(
              view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(dynamic(move || mica_icons::info_icon(palette(dm.get()).text_faint_hex)))
                .child(text("关于").text_size(px(13.0)).text_color(move || palette(dm.get()).text_muted)),
            )
            .child(
              text(format!("v{}", env!("CARGO_PKG_VERSION")))
                .text_size(px(11.5))
                .text_color(move || palette(dm.get()).text_faint),
            ),
        ),
    )
}

fn tab_title(tab: u8) -> &'static str {
  match tab {
    TAB_APPEARANCE => "外观",
    TAB_SHORTCUTS => "快捷键",
    TAB_ACCOUNT => "账户",
    TAB_TOKENS => "API 令牌",
    TAB_DATA => "数据",
    TAB_TRASH => "回收站",
    TAB_AI => "AI 服务商",
    TAB_DIAGNOSTICS => "诊断",
    _ => "外观",
  }
}

fn tab_description(tab: u8) -> &'static str {
  match tab {
    TAB_APPEARANCE => "调整外观与语言、界面与正文阅读体验，修改会立即生效。",
    TAB_SHORTCUTS => "查看编辑、导航和格式操作的快捷键。",
    TAB_ACCOUNT => "管理你的云端账户、头像与登录信息。",
    TAB_TOKENS => "管理脚本与外部工具访问当前账户的 API 令牌。",
    TAB_DATA => "导入、导出与管理本机缓存。可用操作取决于当前模式。",
    TAB_TRASH => "查看已删除的页面，并在需要时恢复或永久删除。",
    TAB_AI => "个人 AI 开关与当前服务器的服务商配置。服务商配置仅管理员可修改。",
    TAB_DIAGNOSTICS => "本机诊断选项，用于定位问题与收集排查信息。",
    _ => "",
  }
}

fn nav_group(label: &'static str, first: bool, dm: Signal<bool>) -> impl IntoElement {
  view()
    .pt(px(if first { 2.0 } else { 14.0 }))
    .pb_1()
    .px_2()
    .child(
      text(label)
        .text_size(px(12.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(move || palette(dm.get()).text_muted),
    )
}

fn nav_item<F, I>(
  tid: &'static str,
  label: &'static str,
  tab: u8,
  active_tab: Signal<u8>,
  dm: Signal<bool>,
  icon_fn: F,
) -> impl IntoElement
where
  F: Fn(&'static str) -> I + 'static,
  I: IntoElement + 'static,
{
  button()
    .test_id(tid)
    .accessibility_label(label)
    .w_full()
    .h(px(34.0))
    .px_2()
    .flex_row()
    .items_center()
    .gap_2()
    .rounded(px(6.0))
    .bg(move || {
      let p = palette(dm.get());
      if active_tab.get() == tab { p.surface_hover } else { p.surface_raised }
    })
    .hover(move |s| s.bg(palette(dm.get_untracked()).surface_hover))
    .on_press(move || active_tab.set(tab))
    .child(dynamic(move || {
      let p = palette(dm.get());
      icon_fn(if active_tab.get() == tab { p.text_primary_hex } else { p.text_faint_hex })
    }))
    .child(
      text(label)
        .text_size(px(13.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(move || {
          let p = palette(dm.get());
          if active_tab.get() == tab { p.text_primary } else { p.text_muted }
        }),
    )
}

fn about_inline_card(dm: Signal<bool>, open: Signal<bool>) -> impl IntoElement {
  setting_card(dm)
    .mb_4()
    .flex_row()
    .items_center()
    .justify_between()
    .gap_3()
    .child(
      view()
        .flex_row()
        .items_center()
        .gap_3()
        .child(mica_icons::mica_logo())
        .child(
          view()
            .gap_1()
            .child(
              text(format!("Mica v{}", env!("CARGO_PKG_VERSION")))
                .text_size(px(14.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || palette(dm.get()).text_primary),
            )
            .child(
              text("100% 全栈纯 Rust 笔记与白板（mica-core + mica-board + nsh 原生壳）")
                .text_size(px(12.0))
                .text_color(move || palette(dm.get()).text_muted),
            ),
        ),
    )
    .child(action_pill("about-inline-close", "收起", dm, move || open.set(false)))
}

// ── Tab 0: 外观 ──────────────────────────────────────────────────────────────

fn appearance_tab(
  props: Rc<SettingsViewProps>,
  theme_choice: Signal<ThemeChoice>,
  lang_choice: Signal<LanguageChoice>,
) -> impl IntoElement {
  let dm = props.dark_mode;
  let locale_zh = props.locale_zh;
  let page_width = props.page_width_px;
  let font_px = props.editor_font_px;
  let font_family = props.editor_font_family;
  let ui_scale = props.ui_font_scale;
  let on_toggle_theme = props.on_toggle_theme.clone();
  let on_toggle_locale = props.on_toggle_locale.clone();
  let (set_dark_s, set_dark_l, set_dark_d) = (
    props.on_set_dark_mode.clone(),
    props.on_set_dark_mode.clone(),
    props.on_set_dark_mode.clone(),
  );
  let (set_zh_s, set_zh_c, set_zh_e) = (
    props.on_set_locale_zh.clone(),
    props.on_set_locale_zh.clone(),
    props.on_set_locale_zh.clone(),
  );

  let mut col = view()
    .w_full()
    .gap_4()
    .child(pref_group_heading("界面", dm))
    .child(pref_row(
      "语言",
      dm,
      view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_2()
        .child(
          segment_group(dm)
            .child(segment_btn("lang-system", "跟随系统", move || lang_choice.get() == LanguageChoice::System, dm, move || {
              lang_choice.set(LanguageChoice::System);
              set_zh_s(true);
            }))
            .child(segment_btn("lang-zh", "中文", move || lang_choice.get() == LanguageChoice::Chinese && locale_zh.get(), dm, move || {
              lang_choice.set(LanguageChoice::Chinese);
              set_zh_c(true);
            }))
            .child(segment_btn("lang-en", "English", move || lang_choice.get() == LanguageChoice::English || !locale_zh.get(), dm, move || {
              lang_choice.set(LanguageChoice::English);
              set_zh_e(false);
            })),
        )
        .child(
          button()
            .test_id("settings-toggle-locale")
            .accessibility_label("切换界面语言")
            .px_3()
            .py_1()
            .rounded(px(6.0))
            .border_1()
            .border_color(move || palette(dm.get()).border_normal)
            .bg(move || palette(dm.get()).surface_sunken)
            .hover(move |s| s.bg(palette(dm.get_untracked()).surface_hover))
            .on_press(move || {
              on_toggle_locale();
              lang_choice.set(if locale_zh.get_untracked() {
                LanguageChoice::Chinese
              } else {
                LanguageChoice::English
              });
            })
            .child(
              text(move || if locale_zh.get() { "语言：简体中文" } else { "Language: English" })
                .text_size(px(12.0))
                .text_color(move || palette(dm.get()).text_primary),
            ),
        ),
    ))
    .child(pref_row(
      "主题",
      dm,
      view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_2()
        .child(
          segment_group(dm)
            .child(segment_btn("theme-system", "跟随系统", move || theme_choice.get() == ThemeChoice::System, dm, move || {
              theme_choice.set(ThemeChoice::System);
              set_dark_s(false);
            }))
            .child(segment_btn("theme-light", "浅色", move || theme_choice.get() == ThemeChoice::Light && !dm.get(), dm, move || {
              theme_choice.set(ThemeChoice::Light);
              set_dark_l(false);
            }))
            .child(segment_btn("theme-dark", "深色", move || theme_choice.get() == ThemeChoice::Dark || dm.get(), dm, move || {
              theme_choice.set(ThemeChoice::Dark);
              set_dark_d(true);
            })),
        )
        .child(
          button()
            .test_id("settings-toggle-theme")
            .accessibility_label("切换浅色或深色主题")
            .px_3()
            .py_1()
            .rounded(px(6.0))
            .border_1()
            .border_color(move || palette(dm.get()).border_normal)
            .bg(move || palette(dm.get()).surface_sunken)
            .hover(move |s| s.bg(palette(dm.get_untracked()).surface_hover))
            .on_press(move || {
              on_toggle_theme();
              theme_choice.set(if dm.get_untracked() {
                ThemeChoice::Dark
              } else {
                ThemeChoice::Light
              });
            })
            .child(
              text(move || if dm.get() { "当前：深色模式" } else { "当前：浅色模式" })
                .text_size(px(12.0))
                .text_color(move || palette(dm.get()).text_primary),
            ),
        ),
    ))
    .child(pref_row(
      "界面字号",
      dm,
      segment_group(dm)
        .child(segment_btn("ui-font-small", "小", move || ui_scale.get() == UiFontScale::Small, dm, move || ui_scale.set(UiFontScale::Small)))
        .child(segment_btn("ui-font-standard", "标准", move || ui_scale.get() == UiFontScale::Standard, dm, move || ui_scale.set(UiFontScale::Standard)))
        .child(segment_btn("ui-font-large", "大", move || ui_scale.get() == UiFontScale::Large, dm, move || ui_scale.set(UiFontScale::Large))),
    ))
    .child(pref_group_heading("正文编辑", dm))
    .child(page_width_control_row(page_width, dm))
    .child(editor_font_size_row(font_px, dm))
    .child(pref_row(
      "字体",
      dm,
      segment_group(dm)
        .child(segment_btn("font-family-system", "系统", move || font_family.get() == EditorFontFamily::System, dm, move || font_family.set(EditorFontFamily::System)))
        .child(segment_btn("font-family-serif", "Serif", move || font_family.get() == EditorFontFamily::Serif, dm, move || font_family.set(EditorFontFamily::Serif)))
        .child(segment_btn("font-family-mono", "Mono", move || font_family.get() == EditorFontFamily::Mono, dm, move || font_family.set(EditorFontFamily::Mono))),
    ))
    .child(switch_row("settings-switch-format-bar", "格式工具栏", "在页面上方显示常用 Markdown 操作的工具栏。", props.show_format_bar, dm))
    .child(switch_row("settings-switch-page-title", "页面标题", "在页面顶部显示页面标题。隐藏后，页面直接从第一行开始。", props.show_page_title, dm))
    .child(appearance_preview_card(page_width, font_px, font_family, dm))
    .child(pref_group_heading("粘贴与图片", dm))
    .child(switch_row("settings-switch-rehost-images", "转存网络图片", "粘贴的图片链接会被保存到 Mica 存储中。关闭时：保留为标准的外部 Markdown 链接。", props.rehost_images, dm));

  #[cfg(not(target_arch = "wasm32"))]
  {
    let cb = props.close_behavior;
    col = col
      .child(pref_group_heading("关闭窗口时", dm))
      .child(
        view()
          .w_full()
          .gap_2()
          .child(radio_row("close-behavior-quit", "退出 Mica", "关闭主窗口时直接退出应用", move || cb.get() == WindowCloseBehavior::Quit, dm, move || cb.set(WindowCloseBehavior::Quit)))
          .child(radio_row("close-behavior-tray", "最小化到系统托盘", "关闭主窗口时隐藏到系统托盘，保持后台同步", move || cb.get() == WindowCloseBehavior::MinimizeToTray, dm, move || cb.set(WindowCloseBehavior::MinimizeToTray))),
      );
  }
  col
}

fn page_width_control_row(page_width: Signal<u32>, dm: Signal<bool>) -> impl IntoElement {
  let mut stops = view().flex_1().flex_row().items_center().gap_1();
  for &stop in &PAGE_WIDTH_STOPS {
    stops = stops.child(
      button()
        .test_id(&format!("settings-page-width-stop-{stop}"))
        .accessibility_label(format!("页面宽度 {}", format_page_width_label(stop)))
        .flex_1()
        .h(px(22.0))
        .items_center()
        .justify_center()
        .on_press(move || page_width.set(stop))
        .child(view().w_full().h(px(6.0)).rounded(px(3.0)).bg(move || {
          let p = palette(dm.get());
          if page_width.get() >= stop { p.accent } else { p.border_normal }
        })),
    );
  }
  pref_row(
    "页面宽度",
    dm,
    view()
      .w_full()
      .flex_row()
      .items_center()
      .gap_2()
      .child(step_btn("settings-page-width-dec", "−", "减小页面宽度", dm, move || {
        let cur = page_width.get_untracked();
        if let Some(&prev) = PAGE_WIDTH_STOPS.iter().rev().find(|&&s| s < cur) {
          page_width.set(prev);
        }
      }))
      .child(stops)
      .child(step_btn("settings-page-width-inc", "+", "增大页面宽度", dm, move || {
        let cur = page_width.get_untracked();
        if let Some(&next) = PAGE_WIDTH_STOPS.iter().find(|&&s| s > cur) {
          page_width.set(next);
        }
      }))
      .child(
        view().w(px(58.0)).items_end().child(
          text(move || format_page_width_label(page_width.get()))
            .test_id("settings-page-width-label")
            .text_size(px(12.5))
            .text_color(move || palette(dm.get()).text_muted),
        ),
      )
      .child(action_pill("settings-page-width-reset", "重置", dm, move || {
        page_width.set(PAGE_WIDTH_DEFAULT);
      })),
  )
}

fn editor_font_size_row(font_px: Signal<u32>, dm: Signal<bool>) -> impl IntoElement {
  let mut track = view().flex_1().flex_row().items_center().gap_1();
  for sz in EDITOR_FONT_PX_MIN..=EDITOR_FONT_PX_MAX {
    track = track.child(
      button()
        .test_id(&format!("settings-font-px-{sz}"))
        .accessibility_label(format!("正文字号 {sz} px"))
        .flex_1()
        .h(px(22.0))
        .items_center()
        .justify_center()
        .on_press(move || font_px.set(sz))
        .child(view().w_full().h(px(6.0)).rounded(px(3.0)).bg(move || {
          let p = palette(dm.get());
          if font_px.get() >= sz { p.accent } else { p.border_normal }
        })),
    );
  }
  pref_row(
    "字号",
    dm,
    view()
      .w_full()
      .flex_row()
      .items_center()
      .gap_2()
      .child(step_btn("settings-font-px-dec", "−", "减小正文字号", dm, move || {
        let cur = font_px.get_untracked();
        if cur > EDITOR_FONT_PX_MIN {
          font_px.set(cur - 1);
        }
      }))
      .child(track)
      .child(step_btn("settings-font-px-inc", "+", "增大正文字号", dm, move || {
        let cur = font_px.get_untracked();
        if cur < EDITOR_FONT_PX_MAX {
          font_px.set(cur + 1);
        }
      }))
      .child(
        view().w(px(58.0)).items_end().child(
          text(move || format!("{} px", font_px.get()))
            .text_size(px(12.5))
            .text_color(move || palette(dm.get()).text_muted),
        ),
      ),
  )
}

fn appearance_preview_card(
  page_width: Signal<u32>,
  font_px: Signal<u32>,
  font_family: Signal<EditorFontFamily>,
  dm: Signal<bool>,
) -> impl IntoElement {
  view()
    .test_id("settings-appearance-preview")
    .w_full()
    .p_4()
    .rounded(px(8.0))
    .border_1()
    .border_color(move || palette(dm.get()).border_normal)
    .bg(move || palette(dm.get()).surface_base)
    .gap_4()
    .child(
      text(move || {
        format!("排版预览 · {} · {} px", format_page_width_label(page_width.get()), font_px.get())
      })
      .text_size(px(12.0))
      .text_color(move || palette(dm.get()).text_muted),
    )
    .child(view().w_full().items_center().child(dynamic(move || {
      let p = palette(dm.get());
      let base = font_px.get() as f32;
      let frac = ((page_width.get() as f32) / 1200.0).clamp(0.55, 1.0);
      let family = match font_family.get() {
        EditorFontFamily::System => "system-ui",
        EditorFontFamily::Serif => "Georgia",
        EditorFontFamily::Mono => "Consolas",
      };
      view()
        .w(relative(frac))
        .gap_3()
        .child(
          text("让想法自然展开")
            .text_size(px((base * 1.45).round()))
            .font_weight(FontWeight::SEMIBOLD)
            .font_family(family)
            .text_color(p.text_primary),
        )
        .child(
          text("清晰的文字与从容的留白，让阅读和记录更轻松。Keep your ideas clear, one paragraph at a time.")
            .text_size(px(base))
            .font_family(family)
            .text_color(p.text_primary),
        )
        .child(
          view().w_full().p_3().rounded(px(8.0)).bg(p.code_bg).child(
            text(r#"const idea = "Hello, Mica";"#)
              .text_size(px((base - 1.5).max(11.5)))
              .font_family("Consolas")
              .text_color(p.text_primary),
          ),
        )
    })))
}

// ── Tab 1: 快捷键 (25 rows in 3 groups) ──────────────────────────────────────

fn shortcuts_tab(dm: Signal<bool>) -> impl IntoElement {
  const APP_ROWS: &[(&str, &str)] = &[
    ("Ctrl + N", "新建页面"),
    ("Ctrl + F", "页内查找"),
    ("F3 / Shift + F3", "查找下一个 / 上一个（在查找栏内）"),
    ("Ctrl + Shift + F", "搜索工作区"),
    ("Ctrl + ,", "打开设置"),
    ("F2", "重命名选中的页面/文件夹"),
    ("Ctrl + T", "在新标签页打开页面"),
    ("Ctrl + W", "关闭当前标签页"),
    ("Ctrl + Tab / Ctrl + Shift + Tab", "下一个 / 上一个标签页"),
    ("Ctrl + Click", "侧栏：把这一行加进多选／取出"),
    ("Shift + Click", "侧栏：选中同一父级下的一段连续行"),
    ("Esc", "侧栏：清空多选（点侧栏外任意处同样清空）"),
  ];
  const FMT_ROWS: &[(&str, &str)] = &[
    ("Ctrl + B", "加粗"),
    ("Ctrl + I", "斜体"),
    ("Ctrl + E", "行内代码"),
    ("Ctrl + K", "链接"),
    ("Ctrl + Alt + 1…6", "一至六级标题"),
    ("Ctrl + Alt + 0", "转为正文（段落）"),
    ("Tab / Shift + Tab", "增加 / 减少列表或代码缩进"),
  ];
  const EDIT_ROWS: &[(&str, &str)] = &[
    ("Ctrl + Z", "撤销"),
    ("Ctrl + Shift + Z", "重做"),
    ("Ctrl + A", "全选"),
    ("Ctrl + C / X / V", "复制 / 剪切 / 粘贴"),
    ("Ctrl + Shift + V", "粘贴为纯文本"),
    ("/", "斜杠命令菜单"),
  ];

  view()
    .w_full()
    .gap_4()
    .child(
      text("常用快捷键")
        .text_size(px(13.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(move || palette(dm.get()).text_primary),
    )
    .child(shortcut_group("应用", APP_ROWS, dm))
    .child(shortcut_group("编辑器 —— 格式", FMT_ROWS, dm))
    .child(shortcut_group("编辑器 —— 编辑", EDIT_ROWS, dm))
    .child(
      text("在 macOS 上用 ⌘ 代替 Ctrl。标题快捷键遵循 Notion/Word 约定（Ctrl+Alt+N）—— Web 版无法捕获单独的 Ctrl+N，它被浏览器用于切换标签页。\n注意：Ctrl+, 可能被中文输入法吞掉（标点切换）；若无响应，请切换到英文输入。")
        .text_size(px(12.0))
        .text_color(move || palette(dm.get()).text_faint),
    )
}

fn shortcut_group(
  title: &'static str,
  rows: &[(&'static str, &'static str)],
  dm: Signal<bool>,
) -> impl IntoElement {
  let mut col = setting_card(dm).gap_2().child(
    text(title)
      .text_size(px(12.5))
      .font_weight(FontWeight::MEDIUM)
      .text_color(move || palette(dm.get()).text_muted),
  );
  for &(keys, desc) in rows {
    col = col.child(
      view()
        .w_full()
        .py_1()
        .flex_row()
        .items_center()
        .gap_4()
        .child(
          view()
            .px_2()
            .py_1()
            .rounded(px(6.0))
            .bg(move || palette(dm.get()).surface_sunken)
            .border_1()
            .border_color(move || palette(dm.get()).border_strong)
            .child(
              text(keys)
                .text_size(px(11.5))
                .font_family("Consolas")
                .text_color(move || palette(dm.get()).text_primary),
            ),
        )
        .child(text(desc).flex_1().text_size(px(13.0)).text_color(move || palette(dm.get()).text_primary)),
    );
  }
  col
}

// ── Tab 2: 账户 ──────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn account_tab(
  props: Rc<SettingsViewProps>,
  email_draft: Signal<String>,
  password_draft: Signal<String>,
  display_name_draft: Signal<String>,
  cur_pass_draft: Signal<String>,
  new_pass_draft: Signal<String>,
  account_msg: Signal<String>,
  account_msg_err: Signal<bool>,
  delete_confirm_open: Signal<bool>,
  delete_pass_draft: Signal<String>,
) -> impl IntoElement {
  let signed = props.is_cloud_signed_in;
  let (p1, p2) = (props.clone(), props);
  view()
    .w_full()
    .gap_4()
    .child(when(move || !signed.get(), move || {
      cloud_login_card(p1.clone(), email_draft, password_draft)
    }))
    .child(when(move || signed.get(), move || {
      cloud_signed_in_section(
        p2.clone(),
        password_draft,
        display_name_draft,
        cur_pass_draft,
        new_pass_draft,
        account_msg,
        account_msg_err,
        delete_confirm_open,
        delete_pass_draft,
      )
    }))
}

fn cloud_login_card(
  props: Rc<SettingsViewProps>,
  email_draft: Signal<String>,
  password_draft: Signal<String>,
) -> impl IntoElement {
  let dm = props.dark_mode;
  let server_url = props.server_url;
  let cloud_status = props.cloud_status;
  let on_check = props.on_check_server.clone();
  let on_login_btn = props.on_cloud_login.clone();
  let on_login_sub = props.on_cloud_login.clone();

  setting_card(dm)
    .gap_3()
    .child(
      view()
        .gap_1()
        .child(text("登录 Mica 云端账号").text_size(px(14.0)).font_weight(FontWeight::MEDIUM).text_color(move || palette(dm.get()).text_primary))
        .child(text("连接自托管或官方 Mica 同步服务，在多设备间实时同步 CRDT 文档").text_size(px(12.0)).text_color(move || palette(dm.get()).text_muted)),
    )
    .child(labeled_box(
      "服务器地址",
      dm,
      view()
        .flex_row()
        .items_center()
        .gap_2()
        .child(
          styled_input(dm)
            .test_id("settings-server-url")
            .placeholder("https://your-mica-server.example.com")
            .flex_1()
            .value(move || server_url.get())
            .on_change(move |v: &str| server_url.set(v.to_owned())),
        )
        .child(action_pill("settings-check-server", "检测连接", dm, move || {
          on_check(server_url.get_untracked());
        })),
    ))
    .child(labeled_box(
      "账号邮箱",
      dm,
      styled_input(dm)
        .test_id("settings-email-input")
        .placeholder("you@example.com")
        .w_full()
        .value(move || email_draft.get())
        .on_change(move |v: &str| email_draft.set(v.to_owned())),
    ))
    .child(labeled_box(
      "登录密码",
      dm,
      styled_input(dm)
        .test_id("settings-password-input")
        .placeholder("输入账号密码")
        .masked(true)
        .submit_behavior(SubmitBehavior::Submit)
        .w_full()
        .value(move || password_draft.get())
        .on_change(move |v: &str| password_draft.set(v.to_owned()))
        .on_submit(move |_: &str| {
          on_login_sub(server_url.get_untracked(), email_draft.get_untracked(), password_draft.get_untracked());
        }),
    ))
    .child(
      view()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_3()
        .child(primary_btn("settings-login-btn", "登录并启用同步", dm, move || {
          on_login_btn(server_url.get_untracked(), email_draft.get_untracked(), password_draft.get_untracked());
        }))
        .child(text(move || cloud_status.get()).flex_1().text_size(px(12.0)).text_color(move || palette(dm.get()).text_muted).truncate()),
    )
}

#[allow(clippy::too_many_arguments)]
fn cloud_signed_in_section(
  props: Rc<SettingsViewProps>,
  password_draft: Signal<String>,
  display_name_draft: Signal<String>,
  cur_pass_draft: Signal<String>,
  new_pass_draft: Signal<String>,
  account_msg: Signal<String>,
  account_msg_err: Signal<bool>,
  delete_confirm_open: Signal<bool>,
  delete_pass_draft: Signal<String>,
) -> impl IntoElement {
  let dm = props.dark_mode;
  let email = props.account_email;
  let name = props.account_name;
  let server_url = props.server_url;
  let cloud_status = props.cloud_status;
  let on_sync = props.on_sync_now.clone();
  let on_logout = props.on_cloud_logout.clone();
  let on_prof = props.on_update_profile.clone();
  let on_pass = props.on_change_password.clone();
  let on_del = props.on_delete_account.clone();

  view()
    .w_full()
    .gap_4()
    .child(
      setting_card(dm)
        .gap_3()
        .child(
          view()
            .flex_row()
            .items_center()
            .justify_between()
            .child(
              view()
                .flex_row()
                .items_center()
                .gap_3()
                .child(
                  view()
                    .w(px(44.0))
                    .h(px(44.0))
                    .rounded(px(22.0))
                    .items_center()
                    .justify_center()
                    .bg(move || palette(dm.get()).accent_soft)
                    .child(
                      text(move || {
                        let s = name.get();
                        let src = if s.trim().is_empty() { email.get() } else { s };
                        src.trim().chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_else(|| "M".into())
                      })
                      .text_size(px(18.0))
                      .font_weight(FontWeight::SEMIBOLD)
                      .text_color(move || palette(dm.get()).accent),
                    ),
                )
                .child(
                  view()
                    .gap_1()
                    .child(
                      text(move || {
                        let e = email.get();
                        if e.trim().is_empty() { "已登录云端账号".into() } else { e }
                      })
                      .text_size(px(14.0))
                      .font_weight(FontWeight::SEMIBOLD)
                      .text_color(move || palette(dm.get()).text_primary),
                    )
                    .child(text(move || format!("服务器：{}", server_url.get())).text_size(px(12.0)).text_color(move || palette(dm.get()).text_muted)),
                ),
            )
            .child(
              view().px_2().py_1().rounded(px(6.0)).bg(move || palette(dm.get()).success_wash).child(
                text("云端已连接").text_size(px(11.5)).font_weight(FontWeight::MEDIUM).text_color(move || palette(dm.get()).success),
              ),
            ),
        )
        .child(
          view().px_3().py_2().rounded(px(6.0)).bg(move || palette(dm.get()).surface_base).border_1().border_color(move || palette(dm.get()).border_subtle).child(
            text(move || cloud_status.get()).text_size(px(12.0)).text_color(move || palette(dm.get()).text_muted),
          ),
        )
        .child(
          view()
            .flex_row()
            .items_center()
            .gap_2()
            .child(primary_btn("settings-sync-now-btn", "立即同步", dm, move || on_sync()))
            .child(danger_outline_btn("settings-logout-btn", "退出登录", dm, move || {
              password_draft.set(String::new());
              on_logout();
            })),
        ),
    )
    .child(
      setting_card(dm)
        .gap_3()
        .child(labeled_box(
          "显示名称",
          dm,
          view()
            .flex_row()
            .items_center()
            .gap_2()
            .child(
              styled_input(dm)
                .test_id("settings-display-name-input")
                .placeholder("输入显示名称")
                .flex_1()
                .value(move || display_name_draft.get())
                .on_change(move |v: &str| display_name_draft.set(v.to_owned())),
            )
            .child(action_pill("settings-save-profile-btn", "保存名称", dm, move || {
              let next = display_name_draft.get_untracked().trim().to_string();
              if let Some(cb) = &on_prof {
                match cb(next.clone()) {
                  Ok(()) => { name.set(next); account_msg.set("已保存".into()); account_msg_err.set(false); }
                  Err(e) => { account_msg.set(e); account_msg_err.set(true); }
                }
              } else {
                name.set(next);
                account_msg.set("已更新本地显示名称".into());
                account_msg_err.set(false);
              }
            })),
        ))
        .child(
          view()
            .gap_2()
            .pt_2()
            .border_t_1()
            .border_color(move || palette(dm.get()).border_subtle)
            .child(text("修改密码").text_size(px(12.5)).font_weight(FontWeight::MEDIUM).text_color(move || palette(dm.get()).text_primary))
            .child(styled_input(dm).test_id("settings-cur-password-input").placeholder("当前密码").masked(true).w_full().value(move || cur_pass_draft.get()).on_change(move |v: &str| cur_pass_draft.set(v.to_owned())))
            .child(styled_input(dm).test_id("settings-new-password-input").placeholder("新密码（至少 8 位）").masked(true).w_full().value(move || new_pass_draft.get()).on_change(move |v: &str| new_pass_draft.set(v.to_owned())))
            .child(action_pill("settings-change-password-btn", "修改密码", dm, move || {
              let np = new_pass_draft.get_untracked();
              if np.len() < 8 {
                account_msg.set("新密码至少需要 8 个字符".into());
                account_msg_err.set(true);
                return;
              }
              if let Some(cb) = &on_pass {
                match cb(cur_pass_draft.get_untracked(), np) {
                  Ok(()) => { cur_pass_draft.set(String::new()); new_pass_draft.set(String::new()); account_msg.set("密码已修改，请重新登录所有设备".into()); account_msg_err.set(false); }
                  Err(e) => { account_msg.set(e); account_msg_err.set(true); }
                }
              } else {
                account_msg.set("当前处于本地模式，连接云端后可修改密码".into());
                account_msg_err.set(false);
              }
            })),
        )
        .child(when(move || !account_msg.get().is_empty(), move || {
          text(move || account_msg.get()).text_size(px(12.0)).text_color(move || {
            let p = palette(dm.get());
            if account_msg_err.get() { p.danger } else { p.success }
          })
        })),
    )
    .child(
      setting_card(dm)
        .gap_2()
        .child(
          view()
            .flex_row()
            .items_center()
            .justify_between()
            .child(text("危险操作").text_size(px(12.5)).font_weight(FontWeight::MEDIUM).text_color(move || palette(dm.get()).danger))
            .child(danger_outline_btn("settings-delete-account-btn", "注销账号", dm, move || {
              delete_confirm_open.set(!delete_confirm_open.get_untracked());
            })),
        )
        .child(when(move || delete_confirm_open.get(), move || {
          let del_cb = on_del.clone();
          view()
            .p_3()
            .rounded(px(6.0))
            .bg(move || palette(dm.get()).danger_wash)
            .gap_2()
            .child(text("此操作不可撤销，请输入当前密码确认永久注销账号：").text_size(px(12.0)).text_color(move || palette(dm.get()).danger))
            .child(
              view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(styled_input(dm).test_id("settings-delete-account-pass").placeholder("输入当前密码确认").masked(true).flex_1().value(move || delete_pass_draft.get()).on_change(move |v: &str| delete_pass_draft.set(v.to_owned())))
                .child(danger_solid_btn("settings-delete-account-confirm", "确认注销", dm, move || {
                  if let Some(cb) = &del_cb {
                    match cb(delete_pass_draft.get_untracked()) {
                      Ok(()) => { delete_pass_draft.set(String::new()); delete_confirm_open.set(false); }
                      Err(e) => { account_msg.set(e); account_msg_err.set(true); }
                    }
                  } else {
                    account_msg.set("需要连接云端账号后才能执行注销操作".into());
                    account_msg_err.set(true);
                  }
                })),
            )
        })),
    )
}

// ── Tab 3: API 令牌 ──────────────────────────────────────────────────────────

fn tokens_tab(
  props: Rc<SettingsViewProps>,
  name_draft: Signal<String>,
  write_draft: Signal<bool>,
  expiry_days: Signal<Option<u32>>,
  token_err: Signal<String>,
  created_secret: Signal<Option<String>>,
  confirm_revoke_id: Signal<Option<String>>,
) -> impl IntoElement {
  let dm = props.dark_mode;
  let tokens = props.tokens;
  let on_create = props.on_create_token.clone();
  let on_revoke = props.on_revoke_token.clone();

  view()
    .w_full()
    .gap_4()
    .child(text("用于 API、CLI 和定时备份的长期令牌。密钥只显示一次 —— 请当场复制。默认只读。").text_size(px(12.5)).text_color(move || palette(dm.get()).text_muted))
    .child(
      setting_card(dm)
        .gap_3()
        .child(labeled_box(
          "名称",
          dm,
          styled_input(dm)
            .test_id("settings-token-name-input")
            .placeholder("例如 backup")
            .w_full()
            .value(move || name_draft.get())
            .on_change(move |v: &str| name_draft.set(v.to_owned())),
        ))
        .child(
          view()
            .flex_row()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
              view().flex_row().items_center().gap_2()
                .child(text("权限").text_size(px(12.5)).text_color(move || palette(dm.get()).text_muted))
                .child(
                  segment_group(dm)
                    .child(segment_btn("token-scope-read", "只读", move || !write_draft.get(), dm, move || write_draft.set(false)))
                    .child(segment_btn("token-scope-write", "读写", move || write_draft.get(), dm, move || write_draft.set(true))),
                ),
            )
            .child(
              view().flex_row().items_center().gap_2()
                .child(text("有效期").text_size(px(12.5)).text_color(move || palette(dm.get()).text_muted))
                .child(
                  segment_group(dm)
                    .child(segment_btn("token-exp-never", "永不", move || expiry_days.get().is_none(), dm, move || expiry_days.set(None)))
                    .child(segment_btn("token-exp-30", "30 天", move || expiry_days.get() == Some(30), dm, move || expiry_days.set(Some(30))))
                    .child(segment_btn("token-exp-90", "90 天", move || expiry_days.get() == Some(90), dm, move || expiry_days.set(Some(90))))
                    .child(segment_btn("token-exp-365", "365 天", move || expiry_days.get() == Some(365), dm, move || expiry_days.set(Some(365)))),
                ),
            ),
        )
        .child(primary_btn("settings-token-create-btn", "创建令牌", dm, move || {
          let name = name_draft.get_untracked().trim().to_string();
          if name.is_empty() {
            token_err.set("名称不能为空".into());
            return;
          }
          token_err.set(String::new());
          let (w, exp) = (write_draft.get_untracked(), expiry_days.get_untracked());
          if let Some(cb) = &on_create {
            match cb(name, w, exp) {
              Ok(sec) => { name_draft.set(String::new()); write_draft.set(false); created_secret.set(Some(sec)); }
              Err(e) => token_err.set(e),
            }
          } else {
            let id = Uuid::new_v4().to_string();
            let sec = format!("mica_pat_{}", &id.replace('-', "")[..24]);
            let mut list = tokens.get_untracked();
            list.insert(0, ApiTokenItem {
              id,
              name,
              can_write: w,
              last_used_label: "从未".into(),
              expires_label: exp.map(|d| format!("{d} 天后")).unwrap_or_else(|| "永不".into()),
              expired: false,
            });
            tokens.set(list);
            name_draft.set(String::new());
            write_draft.set(false);
            created_secret.set(Some(sec));
          }
        }))
        .child(when(move || !token_err.get().is_empty(), move || {
          text(move || token_err.get()).text_size(px(12.0)).text_color(move || palette(dm.get()).danger)
        })),
    )
    .child(when(move || created_secret.get().is_some(), move || {
      view()
        .test_id("settings-token-secret-box")
        .w_full()
        .p_4()
        .rounded(px(8.0))
        .bg(move || palette(dm.get()).success_wash)
        .border_1()
        .border_color(move || palette(dm.get()).success)
        .gap_2()
        .child(text("令牌已创建 · 立即复制 —— 密钥不会再次显示。").text_size(px(12.5)).font_weight(FontWeight::MEDIUM).text_color(move || palette(dm.get()).success))
        .child(
          view().w_full().px_3().py_2().rounded(px(6.0)).bg(move || palette(dm.get()).surface_base).border_1().border_color(move || palette(dm.get()).border_normal).child(
            text(move || created_secret.get().unwrap_or_default()).text_size(px(12.5)).font_family("Consolas").text_color(move || palette(dm.get()).text_primary),
          ),
        )
        .child(
          view().flex_row().gap_2()
            .child(action_pill("settings-token-copy", "复制密钥", dm, move || {
              if let Some(sec) = created_secret.get_untracked() { set_clipboard(sec); }
            }))
            .child(action_pill("settings-token-done", "完成", dm, move || created_secret.set(None))),
        )
    }))
    .child(dynamic(move || {
      let list = tokens.get();
      let p = palette(dm.get());
      let confirming = confirm_revoke_id.get();
      if list.is_empty() {
        return setting_card(dm).items_center().py_6().gap_1()
          .child(text("还没有令牌。").text_size(px(13.5)).font_weight(FontWeight::MEDIUM).text_color(p.text_primary))
          .child(text("创建一个只读令牌，就能用 CLI 定时备份这个云端工作区的全部内容。").text_size(px(12.0)).text_color(p.text_muted));
      }
      let mut col = setting_card(dm).gap_2();
      for tok in list {
        let tid = tok.id.clone();
        let is_conf = confirming.as_deref() == Some(tid.as_str());
        let (tid_ask, tid_do, rcb) = (tid.clone(), tid.clone(), on_revoke.clone());
        col = col.child(
          view().w_full().py_2().border_b_1().border_color(p.border_subtle).gap_2()
            .child(
              view().w_full().flex_row().items_center().justify_between().gap_2()
                .child(
                  view().flex_1().gap_1()
                    .child(
                      view().flex_row().items_center().gap_2()
                        .child(text(if tok.name.is_empty() { "(未命名)".into() } else { tok.name.clone() }).text_size(px(13.0)).font_weight(FontWeight::MEDIUM).text_color(if tok.expired { p.text_faint } else { p.text_primary }))
                        .child(
                          view().px_2().py_1().rounded(px(6.0)).bg(if tok.can_write { p.warning_wash } else { p.surface_hover }).child(
                            text(if tok.can_write { "读写" } else { "只读" }).text_size(px(11.0)).text_color(if tok.can_write { p.warning } else { p.text_muted }),
                          ),
                        ),
                    )
                    .child(text(format!("最近使用 {}  ·  过期 {}", tok.last_used_label, tok.expires_label)).text_size(px(11.5)).text_color(p.text_faint)),
                )
                .child(danger_outline_btn("settings-token-revoke", "吊销", dm, move || {
                  confirm_revoke_id.set(Some(tid_ask.clone()));
                })),
            )
            .child(when(move || is_conf, move || {
              let (rcb2, tdo2) = (rcb.clone(), tid_do.clone());
              inline_danger_confirm(
                "使用这个令牌的脚本会立刻开始失败（401），不可撤销。",
                "确认吊销",
                dm,
                move || confirm_revoke_id.set(None),
                move || {
                  if let Some(cb) = &rcb2 { let _ = cb(tdo2.clone()); } else {
                    let mut next = tokens.get_untracked();
                    next.retain(|x| x.id != tdo2);
                    tokens.set(next);
                  }
                  confirm_revoke_id.set(None);
                },
              )
            })),
        );
      }
      col
    }))
}

// ── Tab 4: 数据 ──────────────────────────────────────────────────────────────

fn data_tab(
  props: Rc<SettingsViewProps>,
  import_path_draft: Signal<String>,
  cache_msg: Signal<String>,
  confirm_clear_cache: Signal<bool>,
) -> impl IntoElement {
  let dm = props.dark_mode;
  let export_preview = props.export_preview;
  let cache_stats = props.cache_stats;
  let (on_md, on_html, on_bundle, on_snap) = (
    props.on_export_markdown.clone(),
    props.on_export_html.clone(),
    props.on_export_bundle.clone(),
    props.on_save_snapshot.clone(),
  );
  let (on_mica_zip, on_md_zip) = (props.on_export_mica_zip.clone(), props.on_export_markdown_zip.clone());
  let (on_imp_btn, on_imp_sub, on_imp_pick) = (
    props.on_import_markdown.clone(),
    props.on_import_markdown.clone(),
    props.on_import_markdown.clone(),
  );
  let (on_imp_zip, on_imp_notion, on_imp_folder) = (
    props.on_import_zip.clone(),
    props.on_import_zip.clone(),
    props.on_import_folder.clone(),
  );
  let on_clear_cache = props.on_clear_mirror_cache.clone();

  view()
    .w_full()
    .gap_4()
    .child(text("从 ZIP 导入工作区 —— Mica 导出或 Notion「Markdown & CSV」导出。页面树、排序和图片都会被重建。").text_size(px(12.5)).text_color(move || palette(dm.get()).text_muted))
    .child(
      button()
        .test_id("settings-import-zip-zone")
        .accessibility_label("导入工作区(ZIP)")
        .w_full()
        .p_5()
        .rounded(px(10.0))
        .border_1()
        .border_dashed()
        .border_color(move || palette(dm.get()).border_strong)
        .bg(move || palette(dm.get()).surface_raised)
        .hover(move |s| s.bg(palette(dm.get_untracked()).surface_hover))
        .items_center()
        .justify_center()
        .gap_2()
        .on_press(move || {
          if let Some(cb) = &on_imp_zip {
            cb(false);
          } else {
            spawn_zip_import_preview(false, export_preview);
          }
        })
        .child(dynamic(move || mica_icons::folder_plus(palette(dm.get()).accent_hex)))
        .child(text("导入工作区(ZIP)").text_size(px(14.0)).font_weight(FontWeight::MEDIUM).text_color(move || palette(dm.get()).text_primary))
        .child(text("选择 Mica 导出的 ZIP，或 Notion 的「Markdown & CSV」导出包").text_size(px(12.0)).text_color(move || palette(dm.get()).text_muted)),
    )
    .child(
      view().flex_row().flex_wrap().gap_2()
        .child(action_pill("settings-import-notion-btn", "从 Notion 导入 (Markdown & CSV ZIP)", dm, move || {
          if let Some(cb) = &on_imp_notion { cb(true); } else { spawn_zip_import_preview(true, export_preview); }
        }))
        .child(action_pill("settings-import-folder-btn", "导入本地 Markdown 文件夹", dm, move || {
          if let Some(cb) = &on_imp_folder { cb(); } else {
            spawn(async move {
              match transfer::pick_import_folder().await {
                Ok(Some(dir)) => match transfer::import_folder_from_disk(&dir, true) {
                  Ok(pages) => export_preview.set(format!("已扫描文件夹：共 {} 篇 Markdown 页面", pages.len())),
                  Err(e) => export_preview.set(e),
                },
                Ok(None) => {}
                Err(e) => export_preview.set(e),
              }
            });
          }
        })),
    )
    .child(
      setting_card(dm)
        .gap_3()
        .child(text("文档导出与工作区备份（单一 Rust 引擎）").text_size(px(13.5)).font_weight(FontWeight::MEDIUM).text_color(move || palette(dm.get()).text_primary))
        .child(
          view().flex_row().flex_wrap().gap_2()
            .child(action_pill("settings-export-mica-zip", "备份工作区 (.mica.zip)", dm, {
              let fb = on_bundle.clone();
              move || if let Some(cb) = &on_mica_zip { cb(); } else { fb(); }
            }))
            .child(action_pill("settings-export-md-zip", "导出工作区 (Markdown ZIP)", dm, {
              let fb = on_bundle.clone();
              move || if let Some(cb) = &on_md_zip { cb(); } else { fb(); }
            }))
            .child(action_pill("settings-export-md", "导出 Markdown", dm, move || on_md()))
            .child(action_pill("settings-export-html", "导出 HTML", dm, move || on_html()))
            .child(action_pill("settings-export-bundle", "导出工作区备份 (.mica.json)", dm, move || on_bundle()))
            .child(action_pill("settings-save-snapshot", "保存版本快照", dm, move || on_snap())),
        )
        .child(
          text(move || {
            let p = export_preview.get();
            if p.is_empty() { "本地数据以 SQLite Vault 实时落盘，离线可用。".into() } else { p }
          })
          .text_size(px(12.0))
          .text_color(move || palette(dm.get()).text_muted),
        ),
    )
    .child(
      setting_card(dm)
        .gap_2()
        .child(text("导入本地 Markdown 文件").text_size(px(13.5)).font_weight(FontWeight::MEDIUM).text_color(move || palette(dm.get()).text_primary))
        .child(text("选择或输入本地 .md 文件路径，导入为当前工作区的新页面").text_size(px(12.0)).text_color(move || palette(dm.get()).text_muted))
        .child(
          view().flex_row().items_center().gap_2()
            .child(
              styled_input(dm)
                .test_id("settings-import-path-input")
                .placeholder("输入本地 .md 文件路径，例如 C:\\notes\\demo.md")
                .submit_behavior(SubmitBehavior::Submit)
                .flex_1()
                .value(move || import_path_draft.get())
                .on_change(move |v: &str| import_path_draft.set(v.to_owned()))
                .on_submit(move |_: &str| on_imp_sub(import_path_draft.get_untracked())),
            )
            .child(action_pill("settings-pick-md-btn", "选择文件…", dm, move || {
              let imp = on_imp_pick.clone();
              spawn(async move {
                match transfer::pick_import_files("Markdown", &["md", "markdown"], false).await {
                  Ok(paths) if !paths.is_empty() => {
                    let s = paths[0].display().to_string();
                    import_path_draft.set(s.clone());
                    imp(s);
                  }
                  Ok(_) => {}
                  Err(e) => export_preview.set(e),
                }
              });
            }))
            .child(primary_btn("settings-import-md-btn", "导入 Markdown", dm, move || {
              on_imp_btn(import_path_draft.get_untracked());
            })),
        ),
    )
    .child(
      setting_card(dm)
        .gap_2()
        .child(text("本机存储与镜像缓存").text_size(px(13.0)).font_weight(FontWeight::MEDIUM).text_color(move || palette(dm.get()).text_primary))
        .child(dynamic(move || {
          let st = cache_stats.get();
          let p = palette(dm.get());
          let clear_cb = on_clear_cache.clone();
          let mut block = view().w_full().gap_2().child(
            text(format!("本地独有：{} 篇页面 · {}", st.local_only_pages, format_bytes_human(st.local_only_bytes)))
              .text_size(px(12.0))
              .text_color(p.text_muted),
          );
          if st.mirrored_pages > 0 || st.mirrored_bytes > 0 || clear_cb.is_some() {
            block = block.child(
              view().flex_row().items_center().justify_between()
                .child(text(format!("云端离线镜像：{} 篇页面 · {}", st.mirrored_pages, format_bytes_human(st.mirrored_bytes))).text_size(px(12.0)).text_color(p.text_muted))
                .child(action_pill("settings-clear-cache-btn", "清理镜像缓存", dm, move || confirm_clear_cache.set(true))),
            );
          }
          if confirm_clear_cache.get() {
            block = block.child(inline_danger_confirm(
              "仅清理可从云端重新拉取的镜像，本地独有页面与未同步离线编辑不受影响。",
              "确认清理",
              dm,
              move || confirm_clear_cache.set(false),
              move || {
                if let Some(cb) = &clear_cb {
                  match cb() { Ok(m) | Err(m) => cache_msg.set(m) }
                } else {
                  cache_msg.set("已清理 0 B 镜像缓存".into());
                }
                confirm_clear_cache.set(false);
              },
            ));
          }
          let msg = cache_msg.get();
          if !msg.is_empty() {
            block = block.child(text(msg).text_size(px(12.0)).text_color(p.text_muted));
          }
          block
        })),
    )
    .child(
      view().flex_row().items_center().gap_2()
        .child(dynamic(move || mica_icons::info_icon(palette(dm.get()).text_faint_hex)))
        .child(text("提示：可从页面菜单（▾）导出单个页面，或从工作区的 ⋯ 菜单导出整个工作区。").flex_1().text_size(px(12.0)).text_color(move || palette(dm.get()).text_faint)),
    )
}

fn spawn_zip_import_preview(notion_mode: bool, preview: Signal<String>) {
  spawn(async move {
    let label = if notion_mode { "Notion ZIP" } else { "Mica / Markdown ZIP" };
    match transfer::pick_import_files(label, &["zip"], false).await {
      Ok(paths) if !paths.is_empty() => {
        #[cfg(not(target_arch = "wasm32"))]
        match std::fs::read(&paths[0])
          .map_err(|e| format!("读取失败：{e}"))
          .and_then(|b| transfer::parse_zip_archive_to_pages(&b, notion_mode))
        {
          Ok(pages) => preview.set(format!("已解析归档：共 {} 篇页面", pages.len())),
          Err(e) => preview.set(e),
        }
      }
      Ok(_) => {}
      Err(e) => preview.set(e),
    }
  });
}

// ── Tab 5: 回收站 ────────────────────────────────────────────────────────────

fn trash_tab(
  props: Rc<SettingsViewProps>,
  confirm_purge_id: Signal<Option<Uuid>>,
  confirm_empty_trash: Signal<bool>,
  trash_notice: Signal<String>,
) -> impl IntoElement {
  let dm = props.dark_mode;
  let (trashed, live) = (props.trashed_views, props.live_views);
  let (on_restore, on_purge, on_purge_all, on_refresh) = (
    props.on_restore_view.clone(),
    props.on_purge_view.clone(),
    props.on_purge_all_trash.clone(),
    props.on_refresh_trash.clone(),
  );

  view()
    .w_full()
    .gap_3()
    .child(
      view().w_full().flex_row().items_center().justify_between()
        .child(
          text(move || {
            let roots = build_trash_root_entries(&trashed.get(), &live.get(), Utc::now());
            format!("共 {} 项已删除内容", roots.len())
          })
          .text_size(px(12.5))
          .text_color(move || palette(dm.get()).text_muted),
        )
        .child(
          view().flex_row().items_center().gap_2()
            .child(action_pill("settings-trash-refresh", "刷新", dm, move || {
              if let Some(cb) = &on_refresh { cb(); }
            }))
            .child(when(move || !trashed.get().is_empty(), move || {
              danger_outline_btn("settings-trash-empty-all", "清空回收站", dm, move || {
                confirm_empty_trash.set(true);
              })
            })),
        ),
    )
    .child(when(move || confirm_empty_trash.get(), move || {
      let pa = on_purge_all.clone();
      inline_danger_confirm(
        "永久删除回收站里的全部项目？所有子页面、版本历史与评论都将无法恢复。",
        "清空回收站",
        dm,
        move || confirm_empty_trash.set(false),
        move || {
          let n = if let Some(cb) = &pa { cb() } else {
            let count = trashed.get_untracked().len();
            trashed.set(Vec::new());
            count
          };
          trash_notice.set(format!("已永久删除 {n} 项"));
          confirm_empty_trash.set(false);
        },
      )
    }))
    .child(when(move || !trash_notice.get().is_empty(), move || {
      text(move || trash_notice.get()).text_size(px(12.0)).text_color(move || palette(dm.get()).text_muted)
    }))
    .child(dynamic(move || {
      let entries = build_trash_root_entries(&trashed.get(), &live.get(), Utc::now());
      let p = palette(dm.get());
      let confirming_id = confirm_purge_id.get();
      if entries.is_empty() {
        return setting_card(dm).items_center().py_8().gap_1()
          .child(mica_icons::trash_icon(p.text_faint_hex))
          .child(text("回收站是空的").text_size(px(14.0)).font_weight(FontWeight::MEDIUM).text_color(p.text_primary))
          .child(text("删除的页面会出现在这里，可以恢复。").text_size(px(12.0)).text_color(p.text_muted));
      }
      let mut list = setting_card(dm).gap_2();
      for entry in entries {
        let vid = entry.view.id;
        let is_folder = entry.view.object_type == "folder";
        let name = entry.view.name.clone();
        let is_conf = confirming_id == Some(vid);
        let mut meta = Vec::new();
        if !entry.original_path.is_empty() { meta.push(entry.original_path.clone()); }
        let mut counts = Vec::new();
        if entry.child_folders > 0 { counts.push(format!("{} 个文件夹", entry.child_folders)); }
        if entry.child_pages > 0 { counts.push(format!("{} 篇笔记", entry.child_pages)); }
        if !counts.is_empty() { meta.push(format!("含 {}", counts.join("、"))); }
        meta.push(entry.relative_deleted.clone());
        let (rcb, pcb) = (on_restore.clone(), on_purge.clone());

        list = list.child(
          view().w_full().py_2().border_b_1().border_color(p.border_subtle).gap_2()
            .child(
              view().w_full().flex_row().items_center().justify_between().gap_3()
                .child(
                  view().flex_1().flex_row().items_center().gap_2()
                    .child(if is_folder { mica_icons::folder(p.text_muted_hex) } else { mica_icons::file_doc(p.text_muted_hex) })
                    .child(
                      view().flex_1().gap_1()
                        .child(text(name.clone()).text_size(px(13.5)).font_weight(FontWeight::MEDIUM).text_color(p.text_primary).truncate())
                        .child(text(meta.join("  ·  ")).text_size(px(11.5)).text_color(p.text_faint).truncate()),
                    ),
                )
                .child(
                  view().flex_row().items_center().gap_2()
                    .child(action_pill("trash-restore-btn", "恢复", dm, move || {
                      if let Some(cb) = &rcb { cb(vid); } else {
                        let mut next = trashed.get_untracked();
                        next.retain(|v| v.id != vid);
                        trashed.set(next);
                      }
                    }))
                    .child(danger_outline_btn("trash-purge-ask", "永久删除", dm, move || {
                      confirm_purge_id.set(Some(vid));
                    })),
                ),
            )
            .child(when(move || is_conf, {
              let cname = name.clone();
              move || {
                let pcb2 = pcb.clone();
                inline_danger_confirm(
                  format!("永久删除「{cname}」？它和它的所有子页面、版本历史、评论都会被立即删除，无法恢复。"),
                  "永久删除",
                  dm,
                  move || confirm_purge_id.set(None),
                  move || {
                    if let Some(cb) = &pcb2 { cb(vid); } else {
                      let mut next = trashed.get_untracked();
                      next.retain(|v| v.id != vid);
                      trashed.set(next);
                    }
                    confirm_purge_id.set(None);
                  },
                )
              }
            })),
        );
      }
      list
    }))
}

// ── Tab 6: AI 服务商 ─────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn ai_tab(
  props: Rc<SettingsViewProps>,
  selected_id: Signal<String>,
  adding: Signal<bool>,
  add_preset: Signal<String>,
  add_custom_name: Signal<String>,
  add_base_url: Signal<String>,
  add_err: Signal<String>,
  api_key_draft: Signal<String>,
  fetched_models: Signal<Vec<String>>,
  status_msg: Signal<String>,
  status_err: Signal<bool>,
) -> impl IntoElement {
  let dm = props.dark_mode;
  let providers = props.ai_providers;
  let (on_save, on_del, on_fetch) = (
    props.on_save_ai_provider.clone(),
    props.on_delete_ai_provider.clone(),
    props.on_fetch_ai_models.clone(),
  );

  view()
    .w_full()
    .gap_4()
    .child(switch_row("settings-ai-enable-switch", "启用 AI 功能", "显示「Ask AI」按钮和 /ai 命令。仅当开启此项且下方配置了服务商时才会出现。", props.ai_enabled, dm))
    .child(
      view().w_full().flex_row().items_start().gap_4()
        .child(
          view().w(px(172.0)).flex_shrink_0().gap_2()
            .child(text("服务商").text_size(px(11.5)).font_weight(FontWeight::MEDIUM).text_color(move || palette(dm.get()).text_faint))
            .child(dynamic(move || {
              let list = providers.get();
              let cur = selected_id.get();
              let is_add = adding.get();
              let p = palette(dm.get());
              let mut col = view().w_full().gap_1();
              if list.is_empty() {
                col = col.child(text("还没有配置任何一家。").text_size(px(12.0)).text_color(p.text_muted));
              }
              for row in list {
                let rid = row.id.clone();
                let is_sel = !is_add && cur == rid;
                let host = row.base_url.strip_prefix("https://").or_else(|| row.base_url.strip_prefix("http://")).unwrap_or(&row.base_url).split('/').next().unwrap_or(&row.base_url).to_string();
                let rid_c = rid.clone();
                col = col.child(
                  button()
                    .test_id(&format!("ai-provider-row-{rid}"))
                    .accessibility_label(ai_preset_label(&rid))
                    .w_full()
                    .px_2()
                    .py_2()
                    .rounded(px(6.0))
                    .bg(if is_sel { p.surface_hover } else { p.surface_base })
                    .hover(move |s| s.bg(palette(dm.get_untracked()).surface_hover))
                    .on_press(move || {
                      adding.set(false);
                      selected_id.set(rid_c.clone());
                      api_key_draft.set(String::new());
                      fetched_models.set(Vec::new());
                      status_msg.set(String::new());
                      let mut next = providers.get_untracked();
                      for it in &mut next { it.active = it.id == rid_c; }
                      providers.set(next);
                    })
                    .child(
                      view().w_full().gap_1()
                        .child(
                          view().w_full().flex_row().items_center().justify_between()
                            .child(text(ai_preset_label(&rid)).flex_1().text_size(px(12.5)).font_weight(if is_sel { FontWeight::MEDIUM } else { FontWeight::NORMAL }).text_color(p.text_primary).truncate())
                            .child(when(move || row.active, move || {
                              view().px_1().rounded(px(4.0)).bg(p.success_wash).child(text("在用").text_size(px(10.5)).text_color(p.success))
                            })),
                        )
                        .child(text(host).text_size(px(11.0)).text_color(p.text_faint).truncate()),
                    ),
                );
              }
              col
            }))
            .child(action_pill("ai-add-provider-open", "+ 添加供应商", dm, move || {
              adding.set(true);
              add_err.set(String::new());
            })),
        )
        .child(
          view().flex_1()
            .child(when(move || adding.get(), move || {
              ai_add_provider_form(dm, providers, selected_id, adding, add_preset, add_custom_name, add_base_url, add_err)
            }))
            .child(when(move || !adding.get(), move || {
              ai_provider_detail_form(dm, providers, selected_id, api_key_draft, fetched_models, status_msg, status_err, on_save.clone(), on_del.clone(), on_fetch.clone())
            })),
        ),
    )
}

#[allow(clippy::too_many_arguments)]
fn ai_add_provider_form(
  dm: Signal<bool>,
  providers: Signal<Vec<AiProviderEntry>>,
  selected_id: Signal<String>,
  adding: Signal<bool>,
  add_preset: Signal<String>,
  add_custom_name: Signal<String>,
  add_base_url: Signal<String>,
  add_err: Signal<String>,
) -> impl IntoElement {
  let mut chips = view().flex_row().flex_wrap().gap_1();
  for &(pid, label, _, url) in AI_PRESETS {
    chips = chips.child(
      button()
        .test_id(&format!("ai-add-preset-{pid}"))
        .accessibility_label(label)
        .px_2()
        .py_1()
        .rounded(px(6.0))
        .border_1()
        .border_color(move || palette(dm.get()).border_normal)
        .bg(move || {
          let p = palette(dm.get());
          if add_preset.get() == pid { p.surface_hover } else { p.surface_base }
        })
        .on_press(move || {
          add_preset.set(pid.to_string());
          add_base_url.set(url.to_string());
          add_err.set(String::new());
        })
        .child(text(label).text_size(px(12.0)).text_color(move || palette(dm.get()).text_primary)),
    );
  }
  setting_card(dm)
    .gap_3()
    .child(text("添加供应商").text_size(px(14.0)).font_weight(FontWeight::MEDIUM).text_color(move || palette(dm.get()).text_primary))
    .child(chips)
    .child(when(move || add_preset.get() == "custom", move || {
      labeled_box(
        "名称（小写字母、数字及 - _ .）",
        dm,
        styled_input(dm).test_id("ai-add-custom-name").placeholder("ollama").w_full().value(move || add_custom_name.get()).on_change(move |v: &str| add_custom_name.set(v.to_owned())),
      )
    }))
    .child(labeled_box(
      "API 基础 URL",
      dm,
      styled_input(dm).test_id("ai-add-base-url").placeholder("https://api.example.com/v1").w_full().value(move || add_base_url.get()).on_change(move |v: &str| add_base_url.set(v.to_owned())),
    ))
    .child(when(move || !add_err.get().is_empty(), move || {
      text(move || add_err.get()).text_size(px(12.0)).text_color(move || palette(dm.get()).danger)
    }))
    .child(
      view().flex_row().gap_2()
        .child(primary_btn("ai-add-confirm-btn", "添加供应商", dm, move || {
          let preset = add_preset.get_untracked();
          let id = if preset == "custom" { add_custom_name.get_untracked().trim().to_lowercase() } else { preset.clone() };
          if !is_valid_ai_provider_id(&id) {
            add_err.set("请填名称：以小写字母或数字开头，其余可用字母、数字和 - _ .".into());
            return;
          }
          let base = add_base_url.get_untracked().trim().to_string();
          if base.is_empty() { add_err.set("必须填写 API 基础 URL。".into()); return; }
          let mut list = providers.get_untracked();
          if list.iter().any(|p| p.id == id) {
            add_err.set(format!("{} 已经配置过了 —— 在左边列表里选中它即可修改。", ai_preset_label(&id)));
            return;
          }
          for it in &mut list { it.active = false; }
          list.push(AiProviderEntry {
            id: id.clone(),
            protocol: if preset == "anthropic" { "anthropic".into() } else { "openai".into() },
            base_url: base,
            model: String::new(),
            has_key: false,
            key_hint: String::new(),
            active: true,
          });
          providers.set(list);
          selected_id.set(id);
          add_custom_name.set(String::new());
          adding.set(false);
        }))
        .child(action_pill("ai-add-cancel-btn", "取消", dm, move || adding.set(false))),
    )
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn ai_provider_detail_form(
  dm: Signal<bool>,
  providers: Signal<Vec<AiProviderEntry>>,
  selected_id: Signal<String>,
  api_key_draft: Signal<String>,
  fetched_models: Signal<Vec<String>>,
  status_msg: Signal<String>,
  status_err: Signal<bool>,
  on_save: Option<Rc<dyn Fn(AiProviderEntry, Option<String>) -> Result<(), String>>>,
  on_delete: Option<Rc<dyn Fn(String) -> Result<(), String>>>,
  on_fetch: Option<Rc<dyn Fn(String, String, String, Option<String>) -> Result<Vec<String>, String>>>,
) -> impl IntoElement {
  dynamic(move || {
    let cur = selected_id.get();
    let p = palette(dm.get());
    let Some(entry) = providers.get().into_iter().find(|x| x.id == cur) else {
      return setting_card(dm).child(
        text("本实例还没有配置 AI 服务商。点击左侧「添加供应商」后「问 AI」才会出现。").text_size(px(12.5)).text_color(p.text_muted),
      );
    };
    let base_url = signal(entry.base_url.clone());
    let model = signal(entry.model.clone());
    let proto = signal(entry.protocol.clone());
    let (id_s, id_f, id_d) = (entry.id.clone(), entry.id.clone(), entry.id.clone());
    let (has_key, key_hint) = (entry.has_key, entry.key_hint.clone());
    let (sv, ft, dl) = (on_save.clone(), on_fetch.clone(), on_delete.clone());

    setting_card(dm)
      .gap_3()
      .child(
        view().gap_1()
          .child(text(ai_preset_label(&entry.id)).text_size(px(16.0)).font_weight(FontWeight::SEMIBOLD).text_color(p.text_primary))
          .child(text(entry.id.clone()).text_size(px(11.5)).font_family("Consolas").text_color(p.text_faint)),
      )
      .child(labeled_box("API 基础 URL", dm, styled_input(dm).test_id("ai-base-url-input").placeholder("https://api.example.com/v1").w_full().value(move || base_url.get()).on_change(move |v: &str| base_url.set(v.to_owned()))))
      .child(labeled_box(
        "协议",
        dm,
        segment_group(dm)
          .child(segment_btn("ai-proto-openai", "Chat Completions (OpenAI 兼容)", move || proto.get() != "anthropic", dm, move || proto.set("openai".into())))
          .child(segment_btn("ai-proto-anthropic", "Anthropic Messages", move || proto.get() == "anthropic", dm, move || proto.set("anthropic".into()))),
      ))
      .child(
        view().gap_1()
          .child(
            text(if has_key {
              if key_hint.is_empty() { "✓ 已配置 API 密钥".into() } else { format!("✓ 已配置 API 密钥（尾号 {key_hint}）") }
            } else { "尚未配置 API 密钥".into() })
            .text_size(px(12.0))
            .text_color(if has_key { p.success } else { p.text_muted }),
          )
          .child(
            styled_input(dm)
              .test_id("ai-api-key-input")
              .placeholder(if has_key { "••••••••••••••••••••（留空保持不变）" } else { "sk-..." })
              .masked(true)
              .w_full()
              .value(move || api_key_draft.get())
              .on_change(move |v: &str| api_key_draft.set(v.to_owned())),
          ),
      )
      .child(
        view().gap_1()
          .child(text("模型").text_size(px(12.0)).text_color(p.text_muted))
          .child(
            view().flex_row().items_center().gap_2()
              .child(styled_input(dm).test_id("ai-model-input").placeholder("点右侧拉取列表，或直接填模型 id").flex_1().value(move || model.get()).on_change(move |v: &str| model.set(v.to_owned())))
              .child(action_pill("ai-fetch-models-btn", "拉取模型", dm, move || {
                if let Some(cb) = &ft {
                  let k = api_key_draft.get_untracked().trim().to_string();
                  match cb(proto.get_untracked(), id_f.clone(), base_url.get_untracked().trim().into(), (!k.is_empty()).then_some(k)) {
                    Ok(ms) => {
                      status_err.set(ms.is_empty());
                      status_msg.set(if ms.is_empty() { "服务商没有返回任何模型。".into() } else { format!("已拉取 {} 个可用模型", ms.len()) });
                      fetched_models.set(ms);
                    }
                    Err(e) => { status_msg.set(e); status_err.set(true); }
                  }
                } else {
                  status_msg.set("请先连接云端服务或手动输入模型 ID".into());
                  status_err.set(false);
                }
              })),
          )
          .child(dynamic(move || {
            let ms = fetched_models.get();
            if ms.is_empty() { return view(); }
            let mut chips = view().flex_row().flex_wrap().gap_1().pt_1();
            for m in ms {
              let mc = m.clone();
              let sel = model.get() == m;
              chips = chips.child(
                button().accessibility_label(format!("选择模型 {m}")).px_2().py_1().rounded(px(6.0)).border_1()
                  .border_color(if sel { p.accent } else { p.border_normal })
                  .bg(if sel { p.accent_soft } else { p.surface_base })
                  .on_press(move || model.set(mc.clone()))
                  .child(text(m).text_size(px(11.5)).text_color(p.text_primary)),
              );
            }
            chips
          })),
      )
      .child(
        view().flex_row().items_center().gap_2().pt_1()
          .child(primary_btn("ai-save-btn", "保存配置", dm, move || {
            let k = api_key_draft.get_untracked().trim().to_string();
            let tail = if k.len() >= 4 { k[k.len() - 4..].to_string() } else { key_hint.clone() };
            let upd = AiProviderEntry {
              id: id_s.clone(),
              protocol: proto.get_untracked(),
              base_url: base_url.get_untracked().trim().into(),
              model: model.get_untracked().trim().into(),
              has_key: has_key || !k.is_empty(),
              key_hint: tail,
              active: true,
            };
            let res = if let Some(cb) = &sv { cb(upd.clone(), (!k.is_empty()).then_some(k)) } else { Ok(()) };
            match res {
              Ok(()) => {
                let mut list = providers.get_untracked();
                for it in &mut list { if it.id == upd.id { *it = upd.clone(); } else { it.active = false; } }
                providers.set(list);
                api_key_draft.set(String::new());
                status_msg.set(format!("已保存 {} 配置（{}）", upd.id, upd.base_url));
                status_err.set(false);
              }
              Err(e) => { status_msg.set(e); status_err.set(true); }
            }
          }))
          .child(danger_outline_btn("ai-delete-provider-btn", "删除供应商", dm, move || {
            let ok = if let Some(cb) = &dl { cb(id_d.clone()).is_ok() } else { true };
            if ok {
              let mut list = providers.get_untracked();
              list.retain(|x| x.id != id_d);
              selected_id.set(list.first().map(|x| x.id.clone()).unwrap_or_default());
              providers.set(list);
              status_msg.set("已删除供应商".into());
              status_err.set(false);
            }
          })),
      )
      .child(when(move || !status_msg.get().is_empty(), move || {
        text(move || status_msg.get()).text_size(px(12.0)).text_color(move || {
          let pal = palette(dm.get());
          if status_err.get() { pal.danger } else { pal.text_muted }
        })
      }))
      .child(text("本地模型（Ollama、LM Studio）通常无需密钥。每家服务商各自保存自己的地址、密钥和模型。").text_size(px(11.5)).text_color(p.text_faint))
  })
}

// ── Tab 7: 诊断 ──────────────────────────────────────────────────────────────

fn diagnostics_tab(props: Rc<SettingsViewProps>) -> impl IntoElement {
  let dm = props.dark_mode;
  let (dir, dir_open, on_open) = (
    props.diagnostics_dir.clone(),
    props.diagnostics_dir.clone(),
    props.on_open_diagnostics_folder.clone(),
  );
  let mut col = view()
    .w_full()
    .gap_4()
    .child(switch_row(
      "settings-diagnostics-switch",
      "记录问题复现",
      "默认关闭。打开后复现一次问题，把下面的文件夹发来即可——它记录 app 实际收到了什么（例如粘贴时的原始剪贴板内容），那通常正是缺的那块。",
      props.diagnostics_enabled,
      dm,
    ))
    .child(
      setting_card(dm)
        .flex_row()
        .items_center()
        .gap_3()
        .child(action_pill("settings-diagnostics-open-folder", "打开文件夹", dm, move || {
          if let Some(cb) = &on_open { cb(); } else {
            let d = dir_open.clone();
            spawn(async move { let _ = open_path(d).await; });
          }
        }))
        .child(text(dir).flex_1().text_size(px(12.0)).font_family("Consolas").text_color(move || palette(dm.get()).text_muted).truncate()),
    );
  if let Some(dropped) = props.diagnostics_dropped.clone() {
    col = col.child(setting_card(dm).child(
      text(format!("本次运行被有意丢弃的错误：{dropped}")).text_size(px(12.0)).text_color(move || palette(dm.get()).text_muted),
    ));
  }
  col.child(text("记录内容包含你粘贴或打开的正文。只保留最近 20 条，可随时删除该文件夹。").text_size(px(12.0)).text_color(move || palette(dm.get()).text_faint))
}

// ── Shared Controls & Primitives ─────────────────────────────────────────────

fn pref_group_heading(label: &'static str, dm: Signal<bool>) -> impl IntoElement {
  view().pt_2().child(text(label).text_size(px(14.0)).font_weight(FontWeight::MEDIUM).text_color(move || palette(dm.get()).text_primary))
}

fn pref_row(label: &'static str, dm: Signal<bool>, control: impl IntoElement) -> impl IntoElement {
  view().w_full().py_1().flex_row().items_center().gap_4()
    .child(view().w(px(108.0)).flex_shrink_0().child(text(label).text_size(px(13.0)).text_color(move || palette(dm.get()).text_primary)))
    .child(view().flex_1().child(control))
}

fn labeled_box(label: &'static str, dm: Signal<bool>, child: impl IntoElement) -> impl IntoElement {
  view().w_full().gap_1()
    .child(text(label).text_size(px(12.0)).text_color(move || palette(dm.get()).text_muted))
    .child(child)
}

fn segment_group(dm: Signal<bool>) -> View {
  view().flex_row().items_center().gap_1().p_1().rounded(px(8.0)).bg(move || palette(dm.get()).surface_sunken)
}

fn segment_btn(
  tid: &'static str,
  label: &'static str,
  selected: impl Fn() -> bool + Copy + 'static,
  dm: Signal<bool>,
  on_press: impl Fn() + 'static,
) -> impl IntoElement {
  button()
    .test_id(tid)
    .accessibility_label(label)
    .px_3()
    .py_1()
    .rounded(px(6.0))
    .bg(move || { let p = palette(dm.get()); if selected() { p.surface_base } else { p.surface_sunken } })
    .on_press(on_press)
    .child(
      text(label).text_size(px(12.5)).font_weight(FontWeight::MEDIUM)
        .text_color(move || { let p = palette(dm.get()); if selected() { p.text_primary } else { p.text_muted } }),
    )
}

fn step_btn(
  tid: &'static str,
  glyph: &'static str,
  a11y: &'static str,
  dm: Signal<bool>,
  on_press: impl Fn() + 'static,
) -> impl IntoElement {
  button()
    .test_id(tid)
    .accessibility_label(a11y)
    .w(px(26.0))
    .h(px(26.0))
    .items_center()
    .justify_center()
    .rounded(px(6.0))
    .border_1()
    .border_color(move || palette(dm.get()).border_normal)
    .bg(move || palette(dm.get()).surface_base)
    .on_press(on_press)
    .child(text(glyph).text_size(px(13.0)).text_color(move || palette(dm.get()).text_primary))
}

fn switch_row(
  tid: &'static str,
  title: &'static str,
  subtitle: &'static str,
  value: Signal<bool>,
  dm: Signal<bool>,
) -> impl IntoElement {
  button()
    .test_id(tid)
    .accessibility_label(title)
    .w_full()
    .py_2()
    .flex_row()
    .items_center()
    .justify_between()
    .gap_4()
    .on_press(move || value.set(!value.get_untracked()))
    .child(
      view().flex_1().gap_1()
        .child(text(title).text_size(px(13.5)).font_weight(FontWeight::MEDIUM).text_color(move || palette(dm.get()).text_primary))
        .child(text(subtitle).text_size(px(12.0)).text_color(move || palette(dm.get()).text_muted)),
    )
    .child(
      view().w(px(40.0)).h(px(22.0)).p(px(2.0)).rounded(px(11.0)).flex_row().items_center().justify_between()
        .bg(move || { let p = palette(dm.get()); if value.get() { p.accent } else { p.border_strong } })
        .child(when(move || value.get(), move || view().w(px(18.0))))
        .child(view().w(px(18.0)).h(px(18.0)).rounded(px(9.0)).bg(nsh_core::color::white()))
        .child(when(move || !value.get(), move || view().w(px(18.0)))),
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn radio_row(
  tid: &'static str,
  title: &'static str,
  subtitle: &'static str,
  selected: impl Fn() -> bool + Copy + 'static,
  dm: Signal<bool>,
  on_select: impl Fn() + 'static,
) -> impl IntoElement {
  button()
    .test_id(tid)
    .accessibility_label(title)
    .w_full()
    .p_3()
    .rounded(px(8.0))
    .border_1()
    .border_color(move || { let p = palette(dm.get()); if selected() { p.accent } else { p.border_normal } })
    .bg(move || palette(dm.get()).surface_raised)
    .flex_row()
    .items_center()
    .gap_3()
    .on_press(on_select)
    .child(
      view().w(px(16.0)).h(px(16.0)).rounded(px(8.0)).border_2()
        .border_color(move || { let p = palette(dm.get()); if selected() { p.accent } else { p.border_strong } })
        .items_center()
        .justify_center()
        .child(when(selected, move || view().w(px(8.0)).h(px(8.0)).rounded(px(4.0)).bg(move || palette(dm.get()).accent))),
    )
    .child(
      view().flex_1().gap_1()
        .child(text(title).text_size(px(13.0)).font_weight(FontWeight::MEDIUM).text_color(move || palette(dm.get()).text_primary))
        .child(text(subtitle).text_size(px(12.0)).text_color(move || palette(dm.get()).text_muted)),
    )
}

fn inline_danger_confirm(
  prompt: impl Into<String>,
  confirm_label: &'static str,
  dm: Signal<bool>,
  on_cancel: impl Fn() + 'static,
  on_confirm: impl Fn() + 'static,
) -> impl IntoElement {
  let msg = prompt.into();
  view()
    .p_2()
    .rounded(px(6.0))
    .bg(move || palette(dm.get()).danger_wash)
    .border_1()
    .border_color(move || palette(dm.get()).danger)
    .flex_row()
    .items_center()
    .justify_between()
    .gap_2()
    .child(text(msg).flex_1().text_size(px(11.5)).text_color(move || palette(dm.get()).danger))
    .child(
      view().flex_row().gap_1()
        .child(action_pill("confirm-cancel", "取消", dm, on_cancel))
        .child(danger_solid_btn("confirm-ok", confirm_label, dm, on_confirm)),
    )
}

fn styled_input(dm: Signal<bool>) -> Input {
  input()
    .px_3()
    .py_2()
    .border_1()
    .border_color(move || palette(dm.get()).border_normal)
    .rounded(px(6.0))
    .text_size(px(13.0))
    .text_color(move || palette(dm.get()).text_primary)
    .bg(move || palette(dm.get()).surface_base)
}

fn setting_card(dm: Signal<bool>) -> View {
  view()
    .w_full()
    .p_4()
    .rounded(px(8.0))
    .border_1()
    .border_color(move || palette(dm.get()).border_normal)
    .bg(move || palette(dm.get()).surface_raised)
}

fn action_pill(tid: &'static str, label: &'static str, dm: Signal<bool>, on_click: impl Fn() + 'static) -> impl IntoElement {
  button()
    .test_id(tid)
    .accessibility_label(label)
    .px_3()
    .py_1()
    .rounded(px(6.0))
    .border_1()
    .border_color(move || palette(dm.get()).border_normal)
    .bg(move || palette(dm.get()).surface_base)
    .hover(move |s| s.bg(palette(dm.get_untracked()).surface_hover))
    .on_press(on_click)
    .child(text(label).text_size(px(12.5)).text_color(move || palette(dm.get()).text_primary))
}

fn primary_btn(tid: &'static str, label: &'static str, dm: Signal<bool>, on_click: impl Fn() + 'static) -> impl IntoElement {
  button()
    .test_id(tid)
    .accessibility_label(label)
    .px_4()
    .py_2()
    .rounded(px(6.0))
    .bg(move || palette(dm.get()).accent)
    .on_press(on_click)
    .child(text(label).text_size(px(12.5)).font_weight(FontWeight::MEDIUM).text_color(nsh_core::color::white()))
}

fn danger_outline_btn(tid: &'static str, label: &'static str, dm: Signal<bool>, on_click: impl Fn() + 'static) -> impl IntoElement {
  button()
    .test_id(tid)
    .accessibility_label(label)
    .px_3()
    .py_1()
    .rounded(px(6.0))
    .border_1()
    .border_color(move || palette(dm.get()).danger)
    .hover(move |s| s.bg(palette(dm.get_untracked()).danger_wash))
    .on_press(on_click)
    .child(text(label).text_size(px(12.0)).text_color(move || palette(dm.get()).danger))
}

fn danger_solid_btn(tid: &'static str, label: &'static str, dm: Signal<bool>, on_click: impl Fn() + 'static) -> impl IntoElement {
  button()
    .test_id(tid)
    .accessibility_label(label)
    .px_3()
    .py_1()
    .rounded(px(6.0))
    .bg(move || palette(dm.get()).danger)
    .on_press(on_click)
    .child(text(label).text_size(px(12.0)).text_color(nsh_core::color::white()))
}

#[cfg(test)]
mod tests {
  use super::*;
  use chrono::Duration;

  fn mk_view(
    id: Uuid,
    parent: Option<Uuid>,
    object_type: &str,
    name: &str,
    deleted: bool,
    updated_at: DateTime<Utc>,
  ) -> MicaView {
    MicaView {
      id,
      workspace_id: Uuid::new_v4(),
      parent_view_id: parent,
      object_id: id,
      object_type: object_type.to_string(),
      name: name.to_string(),
      icon: None,
      position: "a0".to_string(),
      is_deleted: deleted,
      created_by: Uuid::new_v4(),
      created_at: updated_at,
      updated_at,
      state_bytes: None,
    }
  }

  #[test]
  fn page_width_stops_match_flutter_11_stops() {
    assert_eq!(PAGE_WIDTH_STOPS.len(), 11);
    assert_eq!(PAGE_WIDTH_STOPS[0], 680);
    assert_eq!(PAGE_WIDTH_STOPS[1], 800);
    assert_eq!(PAGE_WIDTH_STOPS[10], 1880);
    for w in PAGE_WIDTH_STOPS.windows(2) {
      assert_eq!(w[1] - w[0], 120);
    }
    assert_eq!(format_page_width_label(800), "800 px");
    assert_eq!(format_page_width_label(1880), "全宽");
  }

  #[test]
  fn ai_provider_id_validation_rules() {
    assert!(is_valid_ai_provider_id("deepseek"));
    assert!(is_valid_ai_provider_id("nvidia-glm5.2"));
    assert!(is_valid_ai_provider_id("local_v1"));
    assert!(!is_valid_ai_provider_id(""));
    assert!(!is_valid_ai_provider_id("DeepSeek"));
    assert!(!is_valid_ai_provider_id("-bad"));
    assert!(!is_valid_ai_provider_id("bad/id"));
    assert!(!is_valid_ai_provider_id("bad id"));
  }

  #[test]
  fn format_bytes_and_relative_time_helpers() {
    assert_eq!(format_bytes_human(512), "512 B");
    assert_eq!(format_bytes_human(2048), "2.0 KB");
    assert_eq!(format_bytes_human(5 * 1024 * 1024), "5.0 MB");

    let now = Utc::now();
    assert_eq!(format_relative_time(now - Duration::seconds(20), now), "刚刚");
    assert_eq!(format_relative_time(now - Duration::minutes(12), now), "12 分钟前");
    assert_eq!(format_relative_time(now - Duration::hours(3), now), "3 小时前");
    assert_eq!(format_relative_time(now - Duration::days(2), now), "2 天前");
  }

  #[test]
  fn trash_root_entries_collapse_subtrees_and_resolve_live_path() {
    let now = Utc::now();
    let live_root = Uuid::new_v4();
    let del_folder = Uuid::new_v4();
    let del_sub = Uuid::new_v4();
    let del_page = Uuid::new_v4();

    let live = vec![mk_view(live_root, None, "folder", "项目", false, now)];
    let deleted = vec![
      mk_view(del_folder, Some(live_root), "folder", "归档", true, now - Duration::minutes(5)),
      mk_view(del_sub, Some(del_folder), "folder", "子目录", true, now - Duration::minutes(5)),
      mk_view(del_page, Some(del_sub), "page", "旧文档", true, now - Duration::minutes(5)),
    ];

    let entries = build_trash_root_entries(&deleted, &live, now);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].view.id, del_folder);
    assert_eq!(entries[0].original_path, "项目");
    assert_eq!(entries[0].child_folders, 1);
    assert_eq!(entries[0].child_pages, 1);
    assert_eq!(entries[0].relative_deleted, "5 分钟前");
  }
}
