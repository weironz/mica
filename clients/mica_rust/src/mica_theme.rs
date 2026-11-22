//! Mica 1:1 warm-gray Notion-inspired design tokens (`docs/plans/ui-visual-refresh.md`).
//!
//! Surface hierarchy:
//! - Light: base `#FFFFFF`, raised `#F7F7F5` (sidebar/right panel), sunken `#F1F1EF` (tab strip / search pill),
//!   hover/selected `#EFEFED`, text `#37352F` / `#5F5E5B` / `#6C6B66` / `#9B9A97`,
//!   borders `#F1F1EF` / `#E9E9E7` / `#D3D2CE`, code `#F7F6F3`, accent `#2563EB`,
//!   brand logo `#5B6EF5` (from `assets/mica-logo.svg`).
//! - Dark: base `#191919`, raised `#202020`, sunken `#262626`, hover `#2C2C2C`,
//!   text `#D4D4D4` / `#A6A6A6` / `#949494` / `#6B6B6B`,
//!   borders `#2A2A2A` / `#2F2F2F` / `#3F3F3F`, code `#252525`, accent `#60A5FA`.

use nsh_core::{
  color::{Hsla, Rgba, hsla},
  prelude::*,
};

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
  pub quote_bar: Rgba,
  pub accent: Rgba,
  pub accent_soft: Rgba,
  pub selection_hsla: Hsla,
  pub selection_bg: Rgba,
  pub favorite_amber: Rgba,
  pub danger: Rgba,
  pub backdrop: Rgba,
  pub text_primary_hex: &'static str,
  pub text_muted_hex: &'static str,
  pub text_faint_hex: &'static str,
  pub text_ghost_hex: &'static str,
  pub accent_hex: &'static str,
  pub favorite_hex: &'static str,
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
    quote_bar: rgb_hex(0xD3D2CE),
    accent: rgb_hex(0x2563EB),
    accent_soft: rgb_hex(0xEFF6FF),
    selection_hsla: hsla(217.0 / 360.0, 0.91, 0.60, 0.24),
    selection_bg: rgba_hex(0x2563EB, 0.14),
    favorite_amber: rgb_hex(0xEAB308),
    danger: rgb_hex(0xDC2626),
    backdrop: rgba_hex(0x000000, 0.34),
    text_primary_hex: "#37352f",
    text_muted_hex: "#5f5e5b",
    text_faint_hex: "#6c6b66",
    text_ghost_hex: "#9b9a97",
    accent_hex: "#2563eb",
    favorite_hex: "#eab308",
  }
}

pub fn dark_palette() -> MicaPalette {
  MicaPalette {
    surface_base: rgb_hex(0x191919),
    surface_raised: rgb_hex(0x202020),
    surface_sunken: rgb_hex(0x262626),
    surface_hover: rgb_hex(0x2C2C2C),
    surface_overlay: rgb_hex(0x242424),
    text_primary: rgb_hex(0xD4D4D4),
    text_muted: rgb_hex(0xA6A6A6),
    text_faint: rgb_hex(0x949494),
    text_ghost: rgb_hex(0x6B6B6B),
    border_subtle: rgb_hex(0x2A2A2A),
    border_normal: rgb_hex(0x2F2F2F),
    border_strong: rgb_hex(0x3F3F3F),
    code_bg: rgb_hex(0x252525),
    quote_bar: rgb_hex(0x3F3F3F),
    accent: rgb_hex(0x60A5FA),
    accent_soft: rgb_hex(0x1E293B),
    selection_hsla: hsla(213.0 / 360.0, 0.94, 0.68, 0.28),
    selection_bg: rgba_hex(0x60A5FA, 0.18),
    favorite_amber: rgb_hex(0xFACC15),
    danger: rgb_hex(0xF87171),
    backdrop: rgba_hex(0x000000, 0.56),
    text_primary_hex: "#d4d4d4",
    text_muted_hex: "#a6a6a6",
    text_faint_hex: "#949494",
    text_ghost_hex: "#6b6b6b",
    accent_hex: "#60a5fa",
    favorite_hex: "#facc15",
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
      "popover": "#242424",
      "popover-foreground": "#d4d4d4",
      "primary": "#60a5fa",
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
      "ring": "#60a5fa",
      "sidebar": "#202020",
      "sidebar-foreground": "#d4d4d4",
      "sidebar-primary": "#d4d4d4",
      "sidebar-primary-foreground": "#191919",
      "sidebar-accent": "#2c2c2c",
      "sidebar-accent-foreground": "#d4d4d4",
      "sidebar-border": "#2a2a2a",
      "sidebar-ring": "#60a5fa"
    }
  }
}"##;

/// Installs Mica's warm-gray design tokens as the active `nsh_base` theme.
pub fn install_mica_theme() {
  if let Ok(tokens) = nsh_base::tokens_of(Some(MICA_NSH_THEME_JSON)) {
    set_tokens(tokens);
  }
}
