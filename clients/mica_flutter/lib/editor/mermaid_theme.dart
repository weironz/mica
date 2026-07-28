// The app's palette, in the form the two mermaid engines want.
//
// Both backends can be themed by the host — merman through its
// `HostThemeProfile` (the seam its docs name for exactly this), mermaid.js
// through `initialize({theme, themeVariables})`. Neither needs the CSS flattener
// in `mermaid_svg_inline.dart` to learn about colours: that file's job is
// getting merman's stylesheet into a form flutter_svg can draw AT ALL, which is
// orthogonal to which colours are in it.
//
// The values come from [MicaTokens] rather than being written out again here.
// A diagram that keeps its own idea of "background" would sit on the page like a
// sticker the first time either palette is adjusted.

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

/// mermaid.js's built-in theme to start from (web).
///
/// A base theme still matters even though every variable below is supplied: the
/// built-ins differ in things `themeVariables` does not cover, and starting from
/// the light one and painting it dark leaves those parts light.
String mermaidJsBaseTheme(MicaTokens tokens) =>
    tokens.dark ? 'dark' : 'neutral';

/// `themeVariables` for mermaid.js, the web counterpart of [mermaidThemeFor].
Map<String, String> mermaidJsThemeVariables(MicaTokens tokens) => {
  'background': cssHex(tokens.surface.base),
  'primaryColor': cssHex(tokens.surface.raised),
  'primaryTextColor': cssHex(tokens.text.primary),
  'primaryBorderColor': cssHex(tokens.border.normal),
  'secondaryColor': cssHex(tokens.surface.sunken),
  'tertiaryColor': cssHex(tokens.surface.sunken),
  'lineColor': cssHex(tokens.border.strong),
  'textColor': cssHex(tokens.text.primary),
  'errorBkgColor': cssHex(tokens.status.dangerWash),
  'errorTextColor': cssHex(tokens.status.danger),
};
