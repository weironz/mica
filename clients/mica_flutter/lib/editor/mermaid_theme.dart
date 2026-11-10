// Convert the app palette to merman host theme roles.

import 'package:flutter/painting.dart';

import '../src/rust/api/render.dart' show MermaidTheme;
import '../ui/theme_tokens.dart';

/// `#RRGGBB` for a [Color]. Alpha is dropped on purpose: these land in SVG fill
/// and stroke attributes, where a diagram is drawn on top of its own canvas rect
/// and a translucent role would let the page bleed through a node.
String cssHex(Color color) {
  String channel(double v) =>
      (v * 255).round().clamp(0, 255).toRadixString(16).padLeft(2, '0');
  return '#${channel(color.r)}${channel(color.g)}${channel(color.b)}';
}

/// The palette merman draws a diagram with, from the app's own tokens.
///
/// The role names are merman's, and the mapping is the obvious one: a diagram is
/// a small document laid on the page, so its canvas is the page, its nodes are
/// the raised surface, and its ink and hairlines are the same ones the editor
/// uses.
MermaidTheme mermaidThemeFor(MicaTokens tokens) => MermaidTheme(
  dark: tokens.dark,
  canvas: cssHex(tokens.surface.base),
  surface: cssHex(tokens.surface.raised),
  surfaceAlt: cssHex(tokens.surface.sunken),
  text: cssHex(tokens.text.primary),
  subtleText: cssHex(tokens.text.muted),
  border: cssHex(tokens.border.normal),
  line: cssHex(tokens.border.strong),
  error: cssHex(tokens.status.danger),
  warning: cssHex(tokens.status.warning),
  success: cssHex(tokens.status.success),
);
