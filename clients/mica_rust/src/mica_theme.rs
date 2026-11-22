//! Exact Mica warm-gray design tokens ported 1:1 from `docs/plans/ui-visual-refresh.md`
//! and original `MicaTokens` (`lib/ui/theme_tokens.dart`).
//!
//! Light palette: quiet warm paper (`#FFFFFF` canvas, `#F7F7F5` sidebar,
//! `#F1F1EF` sunken/tab strip, `#EFEFED` hover/selection, `#37352F` ink).
//! Dark palette: warm charcoal (`#191919` canvas, `#202020` sidebar,
//! `#262626` sunken, `#2C2C2C` hover/selection, `#D4D4D4` ink).

use nsh_core::{color::rgba, prelude::*};

#[derive(Clone, Copy)]
pub struct MicaPalette {
  pub surface_base: Rgba,
  pub surface_raised: Rgba,
  pub surface_sunken: Rgba,
  pub surface_overlay: Rgba,
  pub surface_hover: Rgba,
  pub text_primary: Rgba,
  pub text_muted: Rgba,
  pub text_faint: Rgba,
  pub text_ghost: Rgba,
  pub border_subtle: Rgba,
  pub border_normal: Rgba,
  pub border_strong: Rgba,
  pub accent_primary: Rgba,
  pub accent_on_primary: Rgba,
  pub code_bg: Rgba,
  pub quote_bar: Rgba,
  pub brand_logo: Rgba,
  pub text_primary_hex: &'static str,
  pub text_muted_hex: &'static str,
  pub text_faint_hex: &'static str,
  pub text_ghost_hex: &'static str,
  pub brand_hex: &'static str,
}

pub const LIGHT: MicaPalette = MicaPalette {
  surface_base: rgb_hex(0xFFFFFF),
  surface_raised: rgb_hex(0xF7F7F5),
  surface_sunken: rgb_hex(0xF1F1EF),
  surface_overlay: rgb_hex(0xFFFFFF),
  surface_hover: rgb_hex(0xEFEFED),
  text_primary: rgb_hex(0x37352F),
  text_muted: rgb_hex(0x5F5E5B),
  text_faint: rgb_hex(0x6C6B66),
  text_ghost: rgb_hex(0x9B9A97),
  border_subtle: rgb_hex(0xF1F1EF),
  border_normal: rgb_hex(0xE9E9E7),
  border_strong: rgb_hex(0xD3D2CE),
  accent_primary: rgb_hex(0x2563EB),
  accent_on_primary: rgb_hex(0xFFFFFF),
  code_bg: rgb_hex(0xF7F6F3),
  quote_bar: rgb_hex(0xD3D2CE),
  brand_logo: rgb_hex(0x2F7D6F),
  text_primary_hex: "#37352F",
  text_muted_hex: "#5F5E5B",
  text_faint_hex: "#6C6B66",
  text_ghost_hex: "#9B9A97",
  brand_hex: "#2F7D6F",
};

pub const DARK: MicaPalette = MicaPalette {
  surface_base: rgb_hex(0x191919),
  surface_raised: rgb_hex(0x202020),
  surface_sunken: rgb_hex(0x262626),
  surface_overlay: rgb_hex(0x252525),
  surface_hover: rgb_hex(0x2C2C2C),
  text_primary: rgb_hex(0xD4D4D4),
  text_muted: rgb_hex(0xA6A6A6),
  text_faint: rgb_hex(0x949494),
  text_ghost: rgb_hex(0x6B6B6B),
  border_subtle: rgb_hex(0x2A2A2A),
  border_normal: rgb_hex(0x2F2F2F),
  border_strong: rgb_hex(0x3F3F3F),
  accent_primary: rgb_hex(0x4C8DF6),
  accent_on_primary: rgb_hex(0x191919),
  code_bg: rgb_hex(0x202020),
  quote_bar: rgb_hex(0x3F3F3F),
  brand_logo: rgb_hex(0x3D9988),
  text_primary_hex: "#D4D4D4",
  text_muted_hex: "#A6A6A6",
  text_faint_hex: "#949494",
  text_ghost_hex: "#6B6B6B",
  brand_hex: "#3D9988",
};

const fn rgb_hex(hex: u32) -> Rgba {
  rgba((hex << 8) | 0xFF)
}

pub fn palette(dark: bool) -> MicaPalette {
  if dark { DARK } else { LIGHT }
}

/// JSON theme matching `nsh_base::tokens_of` so even base NSH widgets
/// inherit Mica's exact warm-gray light & dark design tokens.
pub const MICA_NSH_THEME_JSON: &str = r##"{
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
      "muted": "#f7f6f3",
      "muted-foreground": "#6c6b66",
      "accent": "#efefed",
      "accent-foreground": "#37352f",
      "destructive": "#dc2626",
      "border": "#e9e9e7",
      "input": "#e9e9e7",
      "ring": "#2563eb",
      "sidebar": "#f7f7f5",
      "sidebar-foreground": "#5f5e5b",
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
      "muted-foreground": "#949494",
      "accent": "#2c2c2c",
      "accent-foreground": "#d4d4d4",
      "destructive": "#ef4444",
      "border": "#2f2f2f",
      "input": "#2f2f2f",
      "ring": "#4c8df6",
      "sidebar": "#202020",
      "sidebar-foreground": "#a6a6a6",
      "sidebar-primary": "#d4d4d4",
      "sidebar-primary-foreground": "#191919",
      "sidebar-accent": "#2c2c2c",
      "sidebar-accent-foreground": "#d4d4d4",
      "sidebar-border": "#2a2a2a",
      "sidebar-ring": "#4c8df6"
    }
  }
}"##;

/// Applies Mica's warm-gray design tokens to the active NSH surface.
pub fn install_mica_theme() {
  if let Ok(tokens) = nsh_base::tokens_of(Some(MICA_NSH_THEME_JSON)) {
    set_tokens(tokens);
  }
}
