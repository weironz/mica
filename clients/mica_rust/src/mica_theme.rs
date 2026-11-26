//! Mica 1:1 warm-gray Notion-inspired design tokens synced against
//! `main:clients/mica_flutter/lib/ui/theme_tokens.dart`.
//!
//! Surface hierarchy:
//! - Light: base `#FFFFFF`, raised `#F7F7F5`, sunken `#F1F1EF`, overlay `#FFFFFF`, hover `#EFEFED`,
//!   text `#37352F` / `#5F5E5B` / `#6C6B66` / `#9B9A97`,
//!   borders `#F1F1EF` / `#E9E9E7` / `#D3D2CE`, code `#F7F6F3`, accent `#2563EB`.
//! - Dark: base `#191919`, raised `#202020`, sunken `#262626`, overlay `#252525`, hover `#2C2C2C`,
//!   text `#D4D4D4` / `#A6A6A6` / `#949494` / `#6B6B6B`,
//!   borders `#2A2A2A` / `#2F2F2F` / `#3F3F3F`, code `#202020`, accent `#4C8DF6`.

use nsh_core::{
  color::{Hsla, Rgba, hsla},
  prelude::*,
};

/// Ambient UI appearance signals shared across shell, sidebar, editor, and settings.
/// Changing any of these in Settings updates the live UI immediately.
#[derive(Clone, Copy)]
pub struct UiAppearance {
  /// UI chrome scale multiplier (`0.90` Small / `1.0` Standard / `1.15` Large).
  pub ui_scale: Signal<f32>,
  /// Editor body base font size in px (`13..=22`, default `16`).
  pub editor_font_px: Signal<u32>,
  /// Editor font family (`""` System, `"Georgia, serif"` Serif, `"monospace"` Mono).
  pub editor_font_family: Signal<&'static str>,
  /// `true` = 简体中文, `false` = English.
  pub locale_zh: Signal<bool>,
}

impl UiAppearance {
  pub fn scale(&self) -> f32 {
    self.ui_scale.get().clamp(0.85, 1.3)
  }

  pub fn scaled(&self, base_px: f32) -> f32 {
    (base_px * self.scale()).round()
  }

  pub fn is_zh(&self) -> bool {
    self.locale_zh.get()
  }
}

/// Read ambient UI scale factor (`1.0` fallback when no context is mounted).
pub fn ui_scale_factor() -> f32 {
  use_context::<UiAppearance>()
    .map(|a| a.scale())
    .unwrap_or(1.0)
}

/// Read ambient `locale_zh` (`true` = Chinese fallback when no context is mounted).
pub fn is_locale_zh() -> bool {
  use_context::<UiAppearance>()
    .map(|a| a.is_zh())
    .unwrap_or(true)
}

/// Convenience bilingual selector (`zh` when Chinese, `en` when English).
pub fn tr(zh: &'static str, en: &'static str) -> &'static str {
  if is_locale_zh() { zh } else { en }
}

#[derive(Clone, Copy)]
pub struct MicaPalette {
  pub surface_base: Rgba,
  pub surface_raised: Rgba,
  pub surface_sunken: Rgba,
  pub surface_hover: Rgba,
  pub surface_overlay: Rgba,
  pub text_primary: Rgba,
  pub text_muted: Rgba,
  pub text_faint: Rgba,
  pub text_ghost: Rgba,
  pub border_subtle: Rgba,
  pub border_normal: Rgba,
  pub border_strong: Rgba,
  pub code_bg: Rgba,
  pub inline_code_bg: Rgba,
  pub inline_code_ink: Rgba,
  pub highlight_bg: Rgba,
  pub quote_bar: Rgba,
  pub accent: Rgba,
  pub accent_hover: Rgba,
  pub accent_soft: Rgba,
  pub on_primary: Rgba,
  pub selection_hsla: Hsla,
  pub selection_bg: Rgba,
  pub comment_highlight: Rgba,
  pub comment_highlight_active: Rgba,
  pub favorite_amber: Rgba,
  pub copy_done: Rgba,
  pub danger: Rgba,
  pub danger_wash: Rgba,
  pub warning: Rgba,
  pub warning_wash: Rgba,
  pub success: Rgba,
  pub success_wash: Rgba,
  pub alert_note: Rgba,
  pub alert_tip: Rgba,
  pub alert_important: Rgba,
  pub alert_warning: Rgba,
  pub alert_caution: Rgba,
  pub code_keyword: Rgba,
  pub code_string: Rgba,
  pub code_comment: Rgba,
  pub code_number: Rgba,
  pub code_function: Rgba,
  pub code_key: Rgba,
  pub backdrop: Rgba,
  pub text_primary_hex: &'static str,
  pub text_muted_hex: &'static str,
  pub text_faint_hex: &'static str,
  pub text_ghost_hex: &'static str,
  pub accent_hex: &'static str,
  pub favorite_hex: &'static str,
  pub danger_hex: &'static str,
  pub warning_hex: &'static str,
  pub success_hex: &'static str,
}

const fn rgb_hex(hex: u32) -> Rgba {
  Rgba {
    r: ((hex >> 16) & 0xff) as f32 / 255.0,
    g: ((hex >> 8) & 0xff) as f32 / 255.0,
    b: (hex & 0xff) as f32 / 255.0,
    a: 1.0,
  }
}

const fn rgba_hex(hex: u32, a: f32) -> Rgba {
  Rgba {
    r: ((hex >> 16) & 0xff) as f32 / 255.0,
    g: ((hex >> 8) & 0xff) as f32 / 255.0,
    b: (hex & 0xff) as f32 / 255.0,
    a,
  }
}

pub fn light_palette() -> MicaPalette {
  MicaPalette {
    surface_base: rgb_hex(0xFFFFFF),
    surface_raised: rgb_hex(0xF7F7F5),
    surface_sunken: rgb_hex(0xF1F1EF),
    surface_hover: rgb_hex(0xEFEFED),
    surface_overlay: rgb_hex(0xFFFFFF),
    text_primary: rgb_hex(0x37352F),
    text_muted: rgb_hex(0x5F5E5B),
    text_faint: rgb_hex(0x6C6B66),
    text_ghost: rgb_hex(0x9B9A97),
    border_subtle: rgb_hex(0xF1F1EF),
    border_normal: rgb_hex(0xE9E9E7),
    border_strong: rgb_hex(0xD3D2CE),
    code_bg: rgb_hex(0xF7F6F3),
    inline_code_bg: rgba_hex(0x878378, 0.15),
    inline_code_ink: rgb_hex(0x44423D),
    highlight_bg: rgba_hex(0xFDE047, 0.42),
    quote_bar: rgb_hex(0xD3D2CE),
    accent: rgb_hex(0x2563EB),
    accent_hover: rgb_hex(0x1D4ED8),
    accent_soft: rgb_hex(0xEFF6FF),
    on_primary: rgb_hex(0xFFFFFF),
    selection_hsla: hsla(210.0 / 360.0, 0.77, 0.51, 0.14),
    selection_bg: rgba_hex(0x2383E2, 0.14),
    comment_highlight: rgba_hex(0xF59E0B, 0.20),
    comment_highlight_active: rgba_hex(0xF59E0B, 0.35),
    favorite_amber: rgb_hex(0xF5B400),
    copy_done: rgb_hex(0x16A34A),
    danger: rgb_hex(0xDC2626),
    danger_wash: rgb_hex(0xFEF2F2),
    warning: rgb_hex(0xB45309),
    warning_wash: rgb_hex(0xFFFBEB),
    success: rgb_hex(0x16A34A),
    success_wash: rgb_hex(0xF0FDF4),
    alert_note: rgb_hex(0x0969DA),
    alert_tip: rgb_hex(0x1A7F37),
    alert_important: rgb_hex(0x8250DF),
    alert_warning: rgb_hex(0x9A6700),
    alert_caution: rgb_hex(0xCF222E),
    code_keyword: rgb_hex(0x7C3AED),
    code_string: rgb_hex(0x0A7D34),
    code_comment: rgb_hex(0x6B7280),
    code_number: rgb_hex(0xB45309),
    code_function: rgb_hex(0x2563EB),
    code_key: rgb_hex(0x0F766E),
    backdrop: rgba_hex(0x000000, 0.34),
    text_primary_hex: "#37352f",
    text_muted_hex: "#5f5e5b",
    text_faint_hex: "#6c6b66",
    text_ghost_hex: "#9b9a97",
    accent_hex: "#2563eb",
    favorite_hex: "#f5b400",
    danger_hex: "#dc2626",
    warning_hex: "#b45309",
    success_hex: "#16a34a",
  }
}

pub fn dark_palette() -> MicaPalette {
  MicaPalette {
    surface_base: rgb_hex(0x191919),
    surface_raised: rgb_hex(0x202020),
    surface_sunken: rgb_hex(0x262626),
    surface_hover: rgb_hex(0x2C2C2C),
    surface_overlay: rgb_hex(0x252525),
    text_primary: rgb_hex(0xD4D4D4),
    text_muted: rgb_hex(0xA6A6A6),
    text_faint: rgb_hex(0x949494),
    text_ghost: rgb_hex(0x6B6B6B),
    border_subtle: rgb_hex(0x2A2A2A),
    border_normal: rgb_hex(0x2F2F2F),
    border_strong: rgb_hex(0x3F3F3F),
    code_bg: rgb_hex(0x202020),
    inline_code_bg: rgba_hex(0x949494, 0.20),
    inline_code_ink: rgb_hex(0xD4D4D4),
    highlight_bg: rgba_hex(0xEAB308, 0.30),
    quote_bar: rgb_hex(0x3F3F3F),
    accent: rgb_hex(0x4C8DF6),
    accent_hover: rgb_hex(0x6BA1F8),
    accent_soft: rgba_hex(0x4C8DF6, 0.12),
    on_primary: rgb_hex(0x191919),
    selection_hsla: hsla(217.0 / 360.0, 0.91, 0.60, 0.22),
    selection_bg: rgba_hex(0x3B82F6, 0.22),
    comment_highlight: rgba_hex(0xE3B341, 0.24),
    comment_highlight_active: rgba_hex(0xE3B341, 0.40),
    favorite_amber: rgb_hex(0xF5B400),
    copy_done: rgb_hex(0x56D364),
    danger: rgb_hex(0xF87171),
    danger_wash: rgba_hex(0xF87171, 0.12),
    warning: rgb_hex(0xE3B341),
    warning_wash: rgba_hex(0xE3B341, 0.12),
    success: rgb_hex(0x56D364),
    success_wash: rgba_hex(0x56D364, 0.12),
    alert_note: rgb_hex(0x58A6FF),
    alert_tip: rgb_hex(0x3FB950),
    alert_important: rgb_hex(0xA371F7),
    alert_warning: rgb_hex(0xD29922),
    alert_caution: rgb_hex(0xF85149),
    code_keyword: rgb_hex(0xC4A0FF),
    code_string: rgb_hex(0x7EE787),
    code_comment: rgb_hex(0x8B949E),
    code_number: rgb_hex(0xE3B341),
    code_function: rgb_hex(0x79C0FF),
    code_key: rgb_hex(0x39C5CF),
    backdrop: rgba_hex(0x000000, 0.56),
    text_primary_hex: "#d4d4d4",
    text_muted_hex: "#a6a6a6",
    text_faint_hex: "#949494",
    text_ghost_hex: "#6b6b6b",
    accent_hex: "#4c8df6",
    favorite_hex: "#f5b400",
    danger_hex: "#f87171",
    warning_hex: "#e3b341",
    success_hex: "#56d364",
  }
}

pub fn palette(dark_mode: bool) -> MicaPalette {
  if dark_mode {
    dark_palette()
  } else {
    light_palette()
  }
}

const MICA_NSH_THEME_JSON: &str = r##"{
  "cssVars": {
    "theme": {
      "radius": "6px"
    },
    "light": {
      "background": "#ffffff",
      "foreground": "#37352f",
      "card": "#ffffff",
      "card-foreground": "#37352f",
      "popover": "#ffffff",
      "popover-foreground": "#37352f",
      "primary": "#2563eb",
      "primary-foreground": "#ffffff",
      "secondary": "#f1f1ef",
      "secondary-foreground": "#37352f",
      "muted": "#f7f7f5",
      "muted-foreground": "#5f5e5b",
      "accent": "#efefed",
      "accent-foreground": "#37352f",
      "destructive": "#dc2626",
      "destructive-foreground": "#ffffff",
      "border": "#e9e9e7",
      "input": "#e9e9e7",
      "ring": "#2563eb",
      "sidebar": "#f7f7f5",
      "sidebar-foreground": "#37352f",
      "sidebar-primary": "#37352f",
      "sidebar-primary-foreground": "#ffffff",
      "sidebar-accent": "#efefed",
      "sidebar-accent-foreground": "#37352f",
      "sidebar-border": "#f1f1ef",
      "sidebar-ring": "#2563eb"
    },
    "dark": {
      "background": "#191919",
      "foreground": "#d4d4d4",
      "card": "#202020",
      "card-foreground": "#d4d4d4",
      "popover": "#252525",
      "popover-foreground": "#d4d4d4",
      "primary": "#4c8df6",
      "primary-foreground": "#191919",
      "secondary": "#262626",
      "secondary-foreground": "#d4d4d4",
      "muted": "#202020",
      "muted-foreground": "#a6a6a6",
      "accent": "#2c2c2c",
      "accent-foreground": "#d4d4d4",
      "destructive": "#f87171",
      "destructive-foreground": "#191919",
      "border": "#2f2f2f",
      "input": "#2f2f2f",
      "ring": "#4c8df6",
      "sidebar": "#202020",
      "sidebar-foreground": "#d4d4d4",
      "sidebar-primary": "#d4d4d4",
      "sidebar-primary-foreground": "#191919",
      "sidebar-accent": "#2c2c2c",
      "sidebar-accent-foreground": "#d4d4d4",
      "sidebar-border": "#2a2a2a",
      "sidebar-ring": "#4c8df6"
    }
  }
}"##;

/// Installs Mica's warm-gray design tokens as the active `nsh_base` theme.
pub fn install_mica_theme() {
  if let Ok(tokens) = nsh_base::tokens_of(Some(MICA_NSH_THEME_JSON)) {
    set_tokens(tokens);
  }
}
