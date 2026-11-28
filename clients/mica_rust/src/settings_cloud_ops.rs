//! Cloud settings & usage-statistics orchestration kept separate from `lib.rs`.
//!
//! Binds real L2 `CloudConnector` endpoints (profile, password, account deletion,
//! avatar upload/delete, Personal Access Tokens, cloud AI providers/models, Web
//! Markdown upload, server health & usage stats) to `SettingsViewProps` using
//! `cloud_sync::with_auto_refresh` and stale-generation guards.

use std::{cell::RefCell, rc::Rc};

use chrono::{DateTime, Local, Utc};
use nsh_core::prelude::*;
use uuid::Uuid;

use crate::{
  cloud_sync::{self, CloudSessionHandles},
  clipboard_platform,
  net::{AiProviderConfig, CloudConfig, CloudConnector},
  settings_view::{
    self, AiProviderEntry, ApiTokenItem, EditorFontFamily, SettingsCallbacks, SettingsModelsCb,
    SettingsResultCb, SettingsStringCb, SettingsViewProps, UiFontScale,
  },
  storage::VaultStore,
  transfer::{self, ArchiveExportFormat},
  usage_stats_view, user_avatar,
};

pub struct SettingsCloudOpsCtx {
  pub app_owner: Owner,
  pub allow_local: bool,
  pub vault: Rc<VaultStore>,
  pub cloud_connector: Rc<RefCell<CloudConnector>>,
  pub cloud_session: CloudSessionHandles,
  pub dark_mode: Signal<bool>,
  pub locale_zh: Signal<bool>,
  pub page_width_px: Signal<u32>,
  pub ui_font_scale: Signal<UiFontScale>,
  pub editor_font_px: Signal<u32>,
  pub editor_font_family: Signal<EditorFontFamily>,
  pub show_format_bar: Signal<bool>,
  pub show_page_title: Signal<bool>,
  pub ai_enabled: Signal<bool>,
  pub rehost_images: Signal<bool>,
  pub active_origin: Signal<String>,
  pub active_folder_id: Signal<Uuid>,
  pub server_url: Signal<String>,
  pub cloud_status: Signal<String>,
  pub export_preview: Signal<String>,
  pub account_id: Signal<String>,
  pub account_email: Signal<String>,
  pub account_name: Signal<String>,
  pub account_avatar_version: Signal<Option<String>>,
  pub avatar_cache_bust_rev: Signal<u64>,
  pub account_avatar_url: Signal<Option<String>>,
  pub is_cloud_signed_in: Signal<bool>,
  pub trashed_views: Signal<Vec<mica_api_types::View>>,
  pub live_views: Signal<Vec<mica_api_types::View>>,
  pub auth_gen: Signal<u64>,
  pub check_gen: Signal<u64>,
  pub settings_modal_open: Signal<bool>,
  pub account_menu_open: Signal<bool>,
  pub about_modal_open: Signal<bool>,
  pub on_toggle_theme: Rc<dyn Fn()>,
  pub on_cloud_login: Rc<dyn Fn(String, String, String)>,
  pub on_cloud_logout: Rc<dyn Fn()>,
  pub on_export_markdown: Rc<dyn Fn()>,
  pub on_export_html: Rc<dyn Fn()>,
  pub on_export_bundle: Rc<dyn Fn()>,
  pub export_workspace_zip: Rc<dyn Fn(Option<Uuid>, ArchiveExportFormat)>,
  pub on_import_markdown: Rc<dyn Fn(String)>,
  pub on_import_zip_picker: Rc<dyn Fn(Option<Uuid>, bool)>,
  pub on_import_folder_picker: Rc<dyn Fn(Option<Uuid>)>,
  pub on_import_files_picker: Rc<dyn Fn(Option<Uuid>)>,
  pub on_save_snapshot: Rc<dyn Fn()>,
  pub on_restore_view: Rc<dyn Fn(Uuid)>,
  pub on_purge_view: Rc<dyn Fn(Uuid)>,
  pub on_purge_all_trash: Rc<dyn Fn() -> usize>,
  pub on_refresh_trash: Rc<dyn Fn()>,
}

pub struct SettingsCloudBundle {
  pub settings_props: Rc<SettingsViewProps>,
  pub on_open_usage_stats: Rc<dyn Fn()>,
}

fn format_token_expiry(expires_at: Option<&str>, now: DateTime<Utc>) -> (String, bool) {
  let Some(raw) = expires_at.map(str::trim).filter(|s| !s.is_empty()) else {
    return ("永不".to_string(), false);
  };
  if let Ok(dt) = DateTime::parse_from_rfc3339(raw) {
    let utc = dt.with_timezone(&Utc);
    let expired = utc < now;
    let date = utc.format("%Y-%m-%d").to_string();
    let label = if expired {
      format!("已过期 ({date})")
    } else {
      date
    };
    return (label, expired);
  }
  (raw.to_string(), false)
}

fn format_last_used(last_used_at: Option<&str>, now: DateTime<Utc>) -> String {
  let Some(raw) = last_used_at.map(str::trim).filter(|s| !s.is_empty()) else {
    return "从未".to_string();
  };
  if let Ok(dt) = DateTime::parse_from_rfc3339(raw) {
    return settings_view::format_relative_time(dt.with_timezone(&Utc), now);
  }
  raw.to_string()
}

fn map_token_item(info: crate::net::ApiTokenInfo, now: DateTime<Utc>) -> ApiTokenItem {
  let (expires_label, expired) = format_token_expiry(info.expires_at.as_deref(), now);
  ApiTokenItem {
    id: info.id,
    name: info.name,
    can_write: info.can_write,
    last_used_label: format_last_used(info.last_used_at.as_deref(), now),
    expires_label,
    expired,
  }
}

fn map_ai_entries(list: Vec<AiProviderConfig>) -> Vec<AiProviderEntry> {
  list
    .into_iter()
    .map(|c| AiProviderEntry {
      id: c.id,
      protocol: c.protocol,
      base_url: c.base_url,
      model: c.model,
      has_key: c.has_key,
      key_hint: c.key_tail,
      active: c.active,
    })
    .collect()
}

fn apply_rotation(
  allow_local: bool,
  vault: &VaultStore,
  cc: &RefCell<CloudConnector>,
  srv_url: &str,
  email: &str,
  conn: CloudConnector,
  rot: Option<(String, String)>,
) {
  if rot.is_some() {
    *cc.borrow_mut() = conn;
    if allow_local {
      cloud_sync::persist_refreshed_tokens(vault, srv_url, email, rot);
    }
  }
}

pub fn build_settings_cloud_bundle(ctx: SettingsCloudOpsCtx) -> SettingsCloudBundle {
  let app_owner = ctx.app_owner;
  let allow_local = ctx.allow_local;
  let vault = ctx.vault.clone();
  let cc = ctx.cloud_connector.clone();
  let cs = ctx.cloud_session.clone();
  let auth_gen = ctx.auth_gen;
  let check_gen = ctx.check_gen;
  let server_url = ctx.server_url;
  let account_email = ctx.account_email;
  let account_name = ctx.account_name;
  let account_id = ctx.account_id;
  let account_avatar_version = ctx.account_avatar_version;
  let avatar_cache_bust_rev = ctx.avatar_cache_bust_rev;
  let is_cloud_signed_in = ctx.is_cloud_signed_in;
  let cloud_status = ctx.cloud_status;
  let active_origin = ctx.active_origin;
  let active_folder_id = ctx.active_folder_id;
  let settings_modal_open = ctx.settings_modal_open;
  let account_menu_open = ctx.account_menu_open;
  let about_modal_open = ctx.about_modal_open;

  let on_check_server: Rc<dyn Fn(String)> = Rc::new(move |url: String| {
    let epoch = check_gen.get_untracked().wrapping_add(1);
    check_gen.set(epoch);
    cloud_status.set(format!("正在检测服务器：{url}…"));
    let url_req = url.clone();
    app_owner.run(move || {
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
    });
  });

  let on_sync_now: Rc<dyn Fn()> = {
    let cc_sync = cc.clone();
    let cs_sync = cs.clone();
    Rc::new(move || {
      let orig = active_origin.get_untracked();
      if cloud_sync::is_cloud_origin(&orig) && is_cloud_signed_in.get_untracked() {
        let srv = server_url.get_untracked();
        let cs_run = cs_sync.clone();
        app_owner.run(move || cs_run.enter_origin(srv, false));
        return;
      }
      let epoch = check_gen.get_untracked().wrapping_add(1);
      check_gen.set(epoch);
      let conn = cc_sync.borrow().clone();
      let phase = conn.sync_phase_label();
      cloud_status.set("正在检查同步连接…".to_string());
      app_owner.run(move || {
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
      });
    })
  };

  let settings_legacy = Rc::new(SettingsCallbacks {
    dark_mode: ctx.dark_mode,
    locale_zh: ctx.locale_zh,
    server_url,
    cloud_status,
    export_preview: ctx.export_preview,
    account_email,
    is_cloud_signed_in,
    on_toggle_theme: ctx.on_toggle_theme.clone(),
    on_toggle_locale: {
      let v = vault.clone();
      let locale_zh = ctx.locale_zh;
      Rc::new(move || {
        let next = !locale_zh.get_untracked();
        locale_zh.set(next);
        let _ = v.set_bool_pref("localeZh", next);
      })
    },
    on_check_server,
    on_cloud_login: ctx.on_cloud_login.clone(),
    on_cloud_logout: ctx.on_cloud_logout.clone(),
    on_sync_now,
    on_export_markdown: ctx.on_export_markdown.clone(),
    on_export_html: ctx.on_export_html.clone(),
    on_export_bundle: ctx.on_export_bundle.clone(),
    on_import_markdown: ctx.on_import_markdown.clone(),
    on_save_snapshot: ctx.on_save_snapshot.clone(),
    on_close: Rc::new(move || settings_modal_open.set(false)),
  });

  let mut props = SettingsViewProps::from_legacy(settings_legacy);
  props.allow_local = allow_local;
  props.page_width_px = ctx.page_width_px;
  props.ui_font_scale = ctx.ui_font_scale;
  props.editor_font_px = ctx.editor_font_px;
  props.editor_font_family = ctx.editor_font_family;
  props.show_format_bar = ctx.show_format_bar;
  props.show_page_title = ctx.show_page_title;
  props.account_name = account_name;
  props.avatar_url = ctx.account_avatar_url;
  let avatar_busy = signal(false);
  props.avatar_busy = avatar_busy;
  props.ai_enabled = ctx.ai_enabled;
  props.rehost_images = ctx.rehost_images;
  props.trashed_views = ctx.trashed_views;
  props.live_views = ctx.live_views;

  props.on_export_mica_zip = Some({
    let exp = ctx.export_workspace_zip.clone();
    Rc::new(move || exp(None, ArchiveExportFormat::BackupMicaZip))
  });
  props.on_export_markdown_zip = Some({
    let exp = ctx.export_workspace_zip.clone();
    Rc::new(move || exp(None, ArchiveExportFormat::MarkdownZip))
  });
  props.on_import_zip = Some({
    let imp = ctx.on_import_zip_picker.clone();
    Rc::new(move |notion: bool| imp(None, notion))
  });
  props.on_import_folder = allow_local.then(|| {
    let imp = ctx.on_import_folder_picker.clone();
    Rc::new(move || imp(None)) as Rc<dyn Fn()>
  });
  props.on_import_markdown_web = Some({
    let imp = ctx.on_import_files_picker.clone();
    Rc::new(move || imp(Some(active_folder_id.get_untracked())))
  });
  props.on_restore_view = Some(ctx.on_restore_view.clone());
  props.on_purge_view = Some(ctx.on_purge_view.clone());
  props.on_purge_all_trash = Some(ctx.on_purge_all_trash.clone());
  props.on_refresh_trash = Some(ctx.on_refresh_trash.clone());

  let tokens_sig = props.tokens;
  let ai_providers_sig = props.ai_providers;

  props.on_upload_avatar = Some({
    let cc_a = cc.clone();
    let v_a = vault.clone();
    Rc::new(move |done: SettingsResultCb| {
      if !is_cloud_signed_in.get_untracked() {
        done(Err("请先登录云端账号".to_string()));
        return;
      }
      if avatar_busy.get_untracked() {
        return;
      }
      let auth_epoch = auth_gen.get_untracked();
      let srv = server_url.get_untracked();
      let email = account_email.get_untracked();
      let mut conn = cc_a.borrow().clone();
      let cc_w = cc_a.clone();
      let v_w = v_a.clone();
      avatar_busy.set(true);
      app_owner.run(move || {
        spawn(async move {
          let picked = transfer::pick_import_file_bytes(
            "头像图片",
            &["png", "jpg", "jpeg", "gif", "webp", "avif", "bmp"],
            false,
          )
          .await;
          let files = match picked {
            Ok(f) => f,
            Err(e) => {
              avatar_busy.set(false);
              done(Err(e));
              return;
            }
          };
          let Some(file) = files.into_iter().next() else {
            avatar_busy.set(false);
            return;
          };
          if file.bytes.len() > user_avatar::MAX_AVATAR_BYTES {
            avatar_busy.set(false);
            done(Err("头像文件不能超过 4 MB".to_string()));
            return;
          }
          let mime = clipboard_platform::mime_for_image(&file.bytes, &file.name);
          if !user_avatar::is_supported_avatar_mime(mime) {
            avatar_busy.set(false);
            done(Err("不支持的图片格式，请使用 PNG / JPEG / GIF / WebP / AVIF / BMP".to_string()));
            return;
          }
          let bytes = file.bytes;
          let res = cloud_sync::with_auto_refresh(&mut conn, move |c| {
            let b = bytes.clone();
            async move { c.upload_avatar(&b, mime).await }
          })
          .await;
          avatar_busy.set(false);
          if auth_gen.get_untracked() != auth_epoch || !is_cloud_signed_in.get_untracked() {
            return;
          }
          match res {
            Ok((ver, rot)) => {
              let next_conn = conn.with_avatar_version(ver.clone());
              *cc_w.borrow_mut() = next_conn;
              if allow_local && rot.is_some() {
                cloud_sync::persist_refreshed_tokens(&v_w, &srv, &email, rot);
              }
              account_avatar_version.set(ver);
              avatar_cache_bust_rev.set(avatar_cache_bust_rev.get_untracked().wrapping_add(1));
              done(Ok(()));
            }
            Err(e) => done(Err(e)),
          }
        });
      });
    })
  });

  props.on_delete_avatar = Some({
    let cc_a = cc.clone();
    let v_a = vault.clone();
    Rc::new(move |done: SettingsResultCb| {
      if !is_cloud_signed_in.get_untracked() {
        done(Err("请先登录云端账号".to_string()));
        return;
      }
      if avatar_busy.get_untracked() {
        return;
      }
      let auth_epoch = auth_gen.get_untracked();
      let srv = server_url.get_untracked();
      let email = account_email.get_untracked();
      let mut conn = cc_a.borrow().clone();
      let cc_w = cc_a.clone();
      let v_w = v_a.clone();
      avatar_busy.set(true);
      app_owner.run(move || {
        spawn(async move {
          let res = cloud_sync::with_auto_refresh(&mut conn, |c| async move {
            c.delete_avatar().await
          })
          .await;
          avatar_busy.set(false);
          if auth_gen.get_untracked() != auth_epoch || !is_cloud_signed_in.get_untracked() {
            return;
          }
          match res {
            Ok(((), rot)) => {
              let next_conn = conn.with_avatar_version(None);
              *cc_w.borrow_mut() = next_conn;
              if allow_local && rot.is_some() {
                cloud_sync::persist_refreshed_tokens(&v_w, &srv, &email, rot);
              }
              account_avatar_version.set(None);
              avatar_cache_bust_rev.set(avatar_cache_bust_rev.get_untracked().wrapping_add(1));
              done(Ok(()));
            }
            Err(e) => done(Err(e)),
          }
        });
      });
    })
  });

  props.on_update_profile = Some({
    let cc_a = cc.clone();
    let v_a = vault.clone();
    Rc::new(move |next_name: String, done: SettingsResultCb| {
      if !is_cloud_signed_in.get_untracked() {
        done(Err("需要连接云端账号后才能修改个人资料".to_string()));
        return;
      }
      let auth_epoch = auth_gen.get_untracked();
      let srv = server_url.get_untracked();
      let email = account_email.get_untracked();
      let mut conn = cc_a.borrow().clone();
      let cc_w = cc_a.clone();
      let v_w = v_a.clone();
      app_owner.run(move || {
        spawn(async move {
          let res = cloud_sync::with_auto_refresh(&mut conn, move |c| {
            let n = next_name.clone();
            async move { c.update_me(&n).await }
          })
          .await;
          if auth_gen.get_untracked() != auth_epoch || !is_cloud_signed_in.get_untracked() {
            return;
          }
          match res {
            Ok((user, rot)) => {
              let trimmed = user.display_name.trim();
              let dname = if !trimmed.is_empty() {
                trimmed.to_string()
              } else {
                crate::sidebar::format_footer_account_label(&email)
              };
              *cc_w.borrow_mut() = conn.with_display_name(Some(dname.clone()));
              if allow_local && rot.is_some() {
                cloud_sync::persist_refreshed_tokens(&v_w, &srv, &email, rot);
              }
              account_name.set(dname);
              done(Ok(()));
            }
            Err(e) => done(Err(e)),
          }
        });
      });
    })
  });

  props.on_change_password = Some({
    let cc_a = cc.clone();
    let v_a = vault.clone();
    Rc::new(move |cur_pw: String, new_pw: String, done: SettingsResultCb| {
      if !is_cloud_signed_in.get_untracked() {
        done(Err("需要连接云端账号后才能修改密码".to_string()));
        return;
      }
      let auth_epoch = auth_gen.get_untracked();
      let srv = server_url.get_untracked();
      let email = account_email.get_untracked();
      let mut conn = cc_a.borrow().clone();
      let cc_w = cc_a.clone();
      let v_w = v_a.clone();
      app_owner.run(move || {
        spawn(async move {
          let res = cloud_sync::with_auto_refresh(&mut conn, move |c| {
            let (cp, np) = (cur_pw.clone(), new_pw.clone());
            async move { c.change_password(&cp, &np).await }
          })
          .await;
          if auth_gen.get_untracked() != auth_epoch || !is_cloud_signed_in.get_untracked() {
            return;
          }
          match res {
            Ok(((), rot)) => {
              apply_rotation(allow_local, &v_w, &cc_w, &srv, &email, conn, rot);
              done(Ok(()));
            }
            Err(e) => done(Err(e)),
          }
        });
      });
    })
  });

  props.on_delete_account = Some({
    let cc_a = cc.clone();
    let v_a = vault.clone();
    let logout = ctx.on_cloud_logout.clone();
    Rc::new(move |password: String, done: SettingsResultCb| {
      if !is_cloud_signed_in.get_untracked() {
        done(Err("需要连接云端账号后才能执行注销操作".to_string()));
        return;
      }
      let auth_epoch = auth_gen.get_untracked();
      let srv = server_url.get_untracked();
      let email = account_email.get_untracked();
      let mut conn = cc_a.borrow().clone();
      let cc_w = cc_a.clone();
      let v_w = v_a.clone();
      let logout_cb = logout.clone();
      app_owner.run(move || {
        spawn(async move {
          let res = cloud_sync::with_auto_refresh(&mut conn, move |c| {
            let pw = password.clone();
            async move { c.delete_account(&pw).await }
          })
          .await;
          if auth_gen.get_untracked() != auth_epoch || !is_cloud_signed_in.get_untracked() {
            return;
          }
          match res {
            Ok(((), rot)) => {
              apply_rotation(allow_local, &v_w, &cc_w, &srv, &email, conn, rot);
              done(Ok(()));
              logout_cb();
            }
            Err(e) => done(Err(e)),
          }
        });
      });
    })
  });

  props.on_create_token = Some({
    let cc_a = cc.clone();
    let v_a = vault.clone();
    Rc::new(move |name: String, can_write: bool, exp: Option<u32>, done: SettingsStringCb| {
      if !is_cloud_signed_in.get_untracked() {
        done(Err("请先登录云端账号后创建 API 令牌".to_string()));
        return;
      }
      let auth_epoch = auth_gen.get_untracked();
      let srv = server_url.get_untracked();
      let email = account_email.get_untracked();
      let mut conn = cc_a.borrow().clone();
      let cc_w = cc_a.clone();
      let v_w = v_a.clone();
      app_owner.run(move || {
        spawn(async move {
          let res = cloud_sync::with_auto_refresh(&mut conn, move |c| {
            let n = name.clone();
            async move { c.create_token(&n, can_write, exp.map(|d| d as i64)).await }
          })
          .await;
          if auth_gen.get_untracked() != auth_epoch || !is_cloud_signed_in.get_untracked() {
            return;
          }
          match res {
            Ok((created, rot)) => {
              apply_rotation(allow_local, &v_w, &cc_w, &srv, &email, conn, rot);
              let item = map_token_item(created.info, Utc::now());
              let mut list = tokens_sig.get_untracked();
              list.insert(0, item);
              tokens_sig.set(list);
              done(Ok(created.token));
            }
            Err(e) => done(Err(e)),
          }
        });
      });
    })
  });

  props.on_revoke_token = Some({
    let cc_a = cc.clone();
    let v_a = vault.clone();
    Rc::new(move |token_id: String, done: SettingsResultCb| {
      if !is_cloud_signed_in.get_untracked() {
        done(Err("请先登录云端账号".to_string()));
        return;
      }
      let auth_epoch = auth_gen.get_untracked();
      let srv = server_url.get_untracked();
      let email = account_email.get_untracked();
      let mut conn = cc_a.borrow().clone();
      let cc_w = cc_a.clone();
      let v_w = v_a.clone();
      app_owner.run(move || {
        spawn(async move {
          let tid = token_id.clone();
          let res = cloud_sync::with_auto_refresh(&mut conn, move |c| {
            let id = tid.clone();
            async move { c.revoke_token(&id).await }
          })
          .await;
          if auth_gen.get_untracked() != auth_epoch || !is_cloud_signed_in.get_untracked() {
            return;
          }
          match res {
            Ok(((), rot)) => {
              apply_rotation(allow_local, &v_w, &cc_w, &srv, &email, conn, rot);
              let mut next = tokens_sig.get_untracked();
              next.retain(|t| t.id != token_id);
              tokens_sig.set(next);
              done(Ok(()));
            }
            Err(e) => done(Err(e)),
          }
        });
      });
    })
  });

  props.on_save_ai_provider = Some({
    let cc_a = cc.clone();
    let v_a = vault.clone();
    Rc::new(move |entry: AiProviderEntry, key: Option<String>, done: SettingsResultCb| {
      if !is_cloud_signed_in.get_untracked() {
        done(Err("请先登录云端账号后保存 AI 服务商".to_string()));
        return;
      }
      let auth_epoch = auth_gen.get_untracked();
      let srv = server_url.get_untracked();
      let email = account_email.get_untracked();
      let mut conn = cc_a.borrow().clone();
      let cc_w = cc_a.clone();
      let v_w = v_a.clone();
      let cfg = AiProviderConfig {
        id: entry.id.clone(),
        name: entry.id,
        base_url: entry.base_url,
        protocol: entry.protocol,
        has_key: entry.has_key,
        key_tail: entry.key_hint,
        model: entry.model,
        active: entry.active,
      };
      app_owner.run(move || {
        spawn(async move {
          let res = cloud_sync::with_auto_refresh(&mut conn, move |c| {
            let cfg_c = cfg.clone();
            let k_c = key.clone();
            async move { c.save_ai_provider(&cfg_c, k_c.as_deref()).await }
          })
          .await;
          if auth_gen.get_untracked() != auth_epoch || !is_cloud_signed_in.get_untracked() {
            return;
          }
          match res {
            Ok((list, rot)) => {
              apply_rotation(allow_local, &v_w, &cc_w, &srv, &email, conn, rot);
              ai_providers_sig.set(map_ai_entries(list));
              done(Ok(()));
            }
            Err(e) => done(Err(e)),
          }
        });
      });
    })
  });

  props.on_delete_ai_provider = Some({
    let cc_a = cc.clone();
    let v_a = vault.clone();
    Rc::new(move |provider_id: String, done: SettingsResultCb| {
      if !is_cloud_signed_in.get_untracked() {
        done(Err("请先登录云端账号".to_string()));
        return;
      }
      let auth_epoch = auth_gen.get_untracked();
      let srv = server_url.get_untracked();
      let email = account_email.get_untracked();
      let mut conn = cc_a.borrow().clone();
      let cc_w = cc_a.clone();
      let v_w = v_a.clone();
      app_owner.run(move || {
        spawn(async move {
          let res = cloud_sync::with_auto_refresh(&mut conn, move |c| {
            let pid = provider_id.clone();
            async move { c.delete_ai_provider(&pid).await }
          })
          .await;
          if auth_gen.get_untracked() != auth_epoch || !is_cloud_signed_in.get_untracked() {
            return;
          }
          match res {
            Ok((list, rot)) => {
              apply_rotation(allow_local, &v_w, &cc_w, &srv, &email, conn, rot);
              ai_providers_sig.set(map_ai_entries(list));
              done(Ok(()));
            }
            Err(e) => done(Err(e)),
          }
        });
      });
    })
  });

  props.on_fetch_ai_models = Some({
    let cc_a = cc.clone();
    let v_a = vault.clone();
    Rc::new(
      move |proto: String, pid: String, base_url: String, key: Option<String>, done: SettingsModelsCb| {
        if !is_cloud_signed_in.get_untracked() {
          done(Err("请先连接云端服务或手动输入模型 ID".to_string()));
          return;
        }
        let auth_epoch = auth_gen.get_untracked();
        let srv = server_url.get_untracked();
        let email = account_email.get_untracked();
        let mut conn = cc_a.borrow().clone();
        let cc_w = cc_a.clone();
        let v_w = v_a.clone();
        app_owner.run(move || {
          spawn(async move {
            let res = cloud_sync::with_auto_refresh(&mut conn, move |c| {
              let (pr, id, bu, k) = (proto.clone(), pid.clone(), base_url.clone(), key.clone());
              async move { c.fetch_ai_models(Some(id.as_str()), &bu, &pr, k.as_deref()).await }
            })
            .await;
            if auth_gen.get_untracked() != auth_epoch || !is_cloud_signed_in.get_untracked() {
              return;
            }
            match res {
              Ok((models, rot)) => {
                apply_rotation(allow_local, &v_w, &cc_w, &srv, &email, conn, rot);
                done(Ok(models));
              }
              Err(e) => done(Err(e)),
            }
          });
        });
      },
    )
  });

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
    let cc_u = cc.clone();
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
      let mut conn = cc_u.borrow().clone();
      let cc_write = cc_u.clone();
      let v_save = v.clone();
      let orig_expect = orig.clone();
      let srv_url = server_url.get_untracked();
      let email_cur = account_email.get_untracked();
      let uid_cur = account_id.get_untracked();
      app_owner.run(move || {
        spawn(async move {
          let mut rotated_tokens: Option<(String, String)> = None;
          let ov_res =
            cloud_sync::with_auto_refresh(&mut conn, |c| async move { c.get_usage_overview().await })
              .await;
          let res = match ov_res {
            Ok((ov, rot1)) => {
              if rot1.is_some() {
                rotated_tokens = rot1;
              }
              match cloud_sync::with_auto_refresh(&mut conn, |c| {
                let r = range.clone();
                let wf = ws_filter.clone();
                async move { c.get_usage_activity(&r, tz_offset, wf.as_deref(), inc_imp).await }
              })
              .await
              {
                Ok((act, rot2)) => {
                  if rot2.is_some() {
                    rotated_tokens = rot2;
                  }
                  Ok((ov, act))
                }
                Err(e) => Err(e),
              }
            }
            Err(e) => Err(e),
          };
          if stats_fetch_gen.get_untracked() != epoch
            || active_origin.get_untracked() != orig_expect
          {
            return;
          }
          if let Some((ref new_tok, ref new_refresh)) = rotated_tokens {
            *cc_write.borrow_mut() = conn.clone();
            if allow_local && !new_tok.is_empty() {
              let _ = v_save.save_server_credential_full(
                &srv_url,
                &email_cur,
                &uid_cur,
                new_tok,
                new_refresh,
              );
            }
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
    dark_mode: ctx.dark_mode,
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

  let refresh_gen = signal(0u64);
  let on_refresh_cloud_settings: Rc<dyn Fn()> = {
    let cc_r = cc.clone();
    let v_r = vault.clone();
    Rc::new(move || {
      if !is_cloud_signed_in.get_untracked() {
        return;
      }
      let auth_epoch = auth_gen.get_untracked();
      let req_epoch = refresh_gen.get_untracked().wrapping_add(1);
      refresh_gen.set(req_epoch);
      let srv = server_url.get_untracked();
      let email = account_email.get_untracked();
      let mut conn = cc_r.borrow().clone();
      let cc_w = cc_r.clone();
      let v_w = v_r.clone();
      app_owner.run(move || {
        spawn(async move {
          let tok_res = cloud_sync::with_auto_refresh(&mut conn, |c| async move {
            c.list_tokens().await
          })
          .await;
          let ai_res = cloud_sync::with_auto_refresh(&mut conn, |c| async move {
            c.list_ai_providers().await
          })
          .await;
          if auth_gen.get_untracked() != auth_epoch
            || refresh_gen.get_untracked() != req_epoch
            || !is_cloud_signed_in.get_untracked()
          {
            return;
          }
          if let Ok((toks, rot)) = tok_res {
            apply_rotation(allow_local, &v_w, &cc_w, &srv, &email, conn.clone(), rot);
            let now = Utc::now();
            tokens_sig.set(toks.into_iter().map(|t| map_token_item(t, now)).collect());
          }
          if let Ok((provs, rot)) = ai_res {
            apply_rotation(allow_local, &v_w, &cc_w, &srv, &email, conn, rot);
            if !provs.is_empty() {
              ai_providers_sig.set(map_ai_entries(provs));
            }
          }
        });
      });
    })
  };

  {
    let was_open = signal(false);
    let ref_cb = on_refresh_cloud_settings.clone();
    effect(move || {
      let open = settings_modal_open.get();
      if open && !was_open.get_untracked() {
        ref_cb();
      }
      was_open.set(open);
    });
  }

  props.on_open_about = Some(Rc::new(move || {
    settings_modal_open.set(false);
    about_modal_open.set(true);
  }));
  props.active_tab = Some(settings_active_tab);
  props.usage_stats = Some(usage_stats_props);

  SettingsCloudBundle {
    settings_props: Rc::new(props),
    on_open_usage_stats,
  }
}
