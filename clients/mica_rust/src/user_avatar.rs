//! Shared user avatar badge + reactive `image(...)` loader with initials fallback.
//!
//! Used consistently across all 4 surfaces:
//! - Left sidebar account footer (`sidebar.rs`)
//! - Account popover menu Card 1 (`app_menu.rs`)
//! - Settings Account tab profile card (`settings_view.rs`)
//! - Workspace members list (`workspace_views.rs`)

use nsh_core::{
  ResizeMode,
  element::ImageSrc,
  prelude::*,
};

/// Server-enforced raw avatar payload limit (`4 MiB`).
pub const MAX_AVATAR_BYTES: usize = 4 * 1024 * 1024;

/// Context wrapper exposing the current signed-in user's reactive avatar URL
/// and email (for initials fallback) without altering component signatures.
#[derive(Clone)]
pub struct AccountAvatarSignal {
  pub url: Signal<Option<String>>,
  pub email: Signal<String>,
}

/// Accepted raster avatar MIME types (matches server `routes/avatar.rs`).
pub fn is_supported_avatar_mime(mime: &str) -> bool {
  matches!(
    mime.trim().to_ascii_lowercase().as_str(),
    "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/avif" | "image/bmp"
  )
}

/// Build an effective avatar URL (adding `&cb={rev}` or `?cb={rev}` after local
/// avatar upload to bypass browser/texture cache even if version stays unchanged).
pub fn effective_avatar_url(
  origin: &str,
  user_id: &str,
  version: Option<&str>,
  cache_bust_rev: u64,
) -> Option<String> {
  let mut url = mica_client::build_avatar_url(origin, user_id, version)?;
  if cache_bust_rev > 0 {
    let sep = if url.contains('?') { '&' } else { '?' };
    url.push_str(&format!("{sep}cb={cache_bust_rev}"));
  }
  Some(url)
}

/// Single-character uppercase fallback initial derived from display name or email.
pub fn avatar_initials(display_name: &str, email: &str) -> String {
  let trimmed = display_name.trim();
  if trimmed == "未登录" {
    return "M".to_string();
  }
  if trimmed == "本地工作区" {
    return if crate::web_runtime::is_web() {
      "M".to_string()
    } else {
      "本".to_string()
    };
  }
  trimmed
    .chars()
    .next()
    .or_else(|| email.trim().chars().next())
    .map(|c| c.to_uppercase().to_string())
    .unwrap_or_else(|| "M".to_string())
}

/// Reactive circular user avatar: renders `url` when present and not failed,
/// otherwise displays uppercase initials. Automatically retries if `url` changes.
pub fn user_avatar(
  dark_mode: Signal<bool>,
  url: Signal<Option<String>>,
  display_name: Signal<String>,
  email: Signal<String>,
  size_px: f32,
  font_px: f32,
  bg_override: Option<Rgba>,
) -> impl IntoElement {
  let failed_url: Signal<Option<String>> = signal(None);

  view()
    .w(px(size_px))
    .h(px(size_px))
    .flex_shrink_0()
    .rounded_full()
    .overflow_hidden()
    .items_center()
    .justify_center()
    .bg(move || {
      bg_override.unwrap_or_else(|| crate::mica_theme::palette(dark_mode.get()).surface_hover)
    })
    .child(
      text(move || avatar_initials(&display_name.get(), &email.get()))
        .text_size(px(font_px))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(move || crate::mica_theme::palette(dark_mode.get()).text_primary),
    )
    .child(dynamic(move || {
      let active = url.get().filter(|u| {
        let trimmed = u.trim();
        !trimmed.is_empty() && failed_url.get().as_deref() != Some(trimmed)
      });
      if let Some(u) = active {
        let u_err = u.clone();
        view()
          .absolute()
          .inset_0()
          .child(
            image(ImageSrc::from(u))
              .w_full()
              .h_full()
              .object_fit(ResizeMode::Cover)
              .on_error(move |_msg: &str| {
                failed_url.set(Some(u_err.clone()));
              }),
          )
      } else {
        view()
      }
    }))
}

/// Non-reactive circular user avatar for static list items (e.g. workspace members).
pub fn user_avatar_static(
  dark_mode: Signal<bool>,
  url: Option<String>,
  display_name: &str,
  email: &str,
  size_px: f32,
  font_px: f32,
  bg_override: Option<Rgba>,
) -> impl IntoElement {
  user_avatar(
    dark_mode,
    signal(url),
    signal(display_name.to_string()),
    signal(email.to_string()),
    size_px,
    font_px,
    bg_override,
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn avatar_initials_prefers_display_name_then_email() {
    assert_eq!(avatar_initials("alice", "bob@x.com"), "A");
    assert_eq!(avatar_initials("   ", "bob@x.com"), "B");
    assert_eq!(avatar_initials("", ""), "M");
  }

  #[test]
  fn effective_avatar_url_appends_cache_bust_cleanly() {
    let url = effective_avatar_url("https://mica.cloudcele.com", "u1", Some("v1"), 3).unwrap();
    assert_eq!(url, "https://mica.cloudcele.com/api/users/u1/avatar?v=v1&cb=3");
    assert!(effective_avatar_url("https://mica.cloudcele.com", "u1", None, 2).is_none());
  }

  #[test]
  fn supported_mime_allows_bitmaps_only() {
    assert!(is_supported_avatar_mime("image/png"));
    assert!(is_supported_avatar_mime("image/AVIF"));
    assert!(!is_supported_avatar_mime("image/svg+xml"));
    assert!(!is_supported_avatar_mime("application/pdf"));
  }
}
