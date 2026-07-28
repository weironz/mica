// The app's colours, by MEANING rather than by value.
//
// Everything used to be a literal: 278 `Color(0x…)` calls across 21 files, plus
// `EditorTheme`'s 11 `static const Color`s. Constants cannot change at runtime,
// which is precisely why dark mode was impossible — not because nobody had
// written the dark values, but because there was nowhere to put them.
//
// Shape borrowed from AppFlowy's `appflowy_ui` (the closest comparable: Flutter
// desktop, self-drawn editor): an inherited theme carrying one data object whose
// fields are grouped by ROLE — surface / text / border / accent / status — with a
// light and a dark instance and `lerp` so switching can animate. Deliberately
// smaller than theirs: they ship a full design system (their badge palette alone
// is 20KB); we need the roles this app actually uses, and nothing else.
//
// The rule that keeps this honest: a widget asks for a role, never for a colour.
// `tokens.text.muted`, not "the grey one" — so the dark palette is a second set
// of answers to the same questions rather than a parallel universe of literals.

import 'package:flutter/material.dart';

/// Backgrounds, from furthest back to nearest front.
class SurfaceTokens {
  const SurfaceTokens({
    required this.base,
    required this.raised,
    required this.sunken,
    required this.overlay,
    required this.hover,
  });

  /// The page itself.
  final Color base;

  /// Cards, panels, the sidebar — one step toward the viewer.
  final Color raised;

  /// Inputs, code blocks, quiet fills — one step away.
  final Color sunken;

  /// Menus and dialogs, which float above everything.
  final Color overlay;

  /// Row/button hover wash.
  final Color hover;

  static SurfaceTokens lerp(SurfaceTokens a, SurfaceTokens b, double t) =>
      SurfaceTokens(
        base: Color.lerp(a.base, b.base, t)!,
        raised: Color.lerp(a.raised, b.raised, t)!,
        sunken: Color.lerp(a.sunken, b.sunken, t)!,
        overlay: Color.lerp(a.overlay, b.overlay, t)!,
        hover: Color.lerp(a.hover, b.hover, t)!,
      );
}

/// Ink, by how loudly it should speak.
class TextTokens {
  const TextTokens({
    required this.primary,
    required this.muted,
    required this.faint,
  });

  /// Body copy and headings.
  final Color primary;

  /// Secondary lines: meta, descriptions, subtitles.
  final Color muted;

  /// Barely-there: timestamps, placeholders, disabled labels.
  ///
  /// There is deliberately no `inverse` here. Ink on a filled accent already has
  /// a name — [AccentTokens.onPrimary] — and having two names for it is what
  /// produced the first bug in this file: the dark accent was lightened so it
  /// would hold up on a dark page, while `text.inverse` stayed white, and white
  /// on that lighter blue misses WCAG AA. One name, one answer.
  final Color faint;

  static TextTokens lerp(TextTokens a, TextTokens b, double t) => TextTokens(
    primary: Color.lerp(a.primary, b.primary, t)!,
    muted: Color.lerp(a.muted, b.muted, t)!,
    faint: Color.lerp(a.faint, b.faint, t)!,
  );
}

/// Lines.
class BorderTokens {
  const BorderTokens({
    required this.subtle,
    required this.normal,
    required this.strong,
  });

  /// Hairlines inside a component (row separators).
  final Color subtle;

  /// The usual 1px outline around cards, inputs, menus.
  final Color normal;

  /// Focus and emphasis.
  final Color strong;

  static BorderTokens lerp(BorderTokens a, BorderTokens b, double t) =>
      BorderTokens(
        subtle: Color.lerp(a.subtle, b.subtle, t)!,
        normal: Color.lerp(a.normal, b.normal, t)!,
        strong: Color.lerp(a.strong, b.strong, t)!,
      );
}

/// The one blue.
class AccentTokens {
  const AccentTokens({
    required this.primary,
    required this.hover,
    required this.wash,
    required this.onPrimary,
  });

  /// Primary buttons, links, the caret, selection.
  final Color primary;
  final Color hover;

  /// A tint of [primary] for selected rows and icon chips.
  final Color wash;

  /// Ink on top of [primary].
  final Color onPrimary;

  static AccentTokens lerp(AccentTokens a, AccentTokens b, double t) =>
      AccentTokens(
        primary: Color.lerp(a.primary, b.primary, t)!,
        hover: Color.lerp(a.hover, b.hover, t)!,
        wash: Color.lerp(a.wash, b.wash, t)!,
        onPrimary: Color.lerp(a.onPrimary, b.onPrimary, t)!,
      );
}

/// Outcomes. Each has ink and a wash to sit it on.
class StatusTokens {
  const StatusTokens({
    required this.danger,
    required this.dangerWash,
    required this.warning,
    required this.warningWash,
    required this.success,
    required this.successWash,
  });

  final Color danger;
  final Color dangerWash;
  final Color warning;
  final Color warningWash;
  final Color success;
  final Color successWash;

  static StatusTokens lerp(StatusTokens a, StatusTokens b, double t) =>
      StatusTokens(
        danger: Color.lerp(a.danger, b.danger, t)!,
        dangerWash: Color.lerp(a.dangerWash, b.dangerWash, t)!,
        warning: Color.lerp(a.warning, b.warning, t)!,
        warningWash: Color.lerp(a.warningWash, b.warningWash, t)!,
        success: Color.lerp(a.success, b.success, t)!,
        successWash: Color.lerp(a.successWash, b.successWash, t)!,
      );
}

/// What the self-drawn canvas needs beyond the shared roles above.
///
/// Separate because the editor paints these itself (it has no widgets to inherit
/// from) and because they are editor concepts — a quote bar or a comment wash
/// means nothing in a settings dialog.
class EditorTokens {
  const EditorTokens({
    required this.selection,
    required this.caret,
    required this.codeBg,
    required this.inlineCodeBg,
    required this.inlineCodeInk,
    required this.commentHighlight,
    required this.commentHighlightActive,
    required this.quoteBar,
    required this.dropLine,
    required this.alertAccents,
  });

  final Color selection;
  final Color caret;

  /// Fenced code block background.
  final Color codeBg;

  /// Inline `code` pill. Translucent so a selection tints through it.
  final Color inlineCodeBg;

  /// Ink ON that pill. A role of its own because it is deliberately not
  /// `text.primary`: inline code reads as a slightly cooler, lighter ink so the
  /// pill is legible as a pill. Pairing it with [inlineCodeBg] is what keeps the
  /// two from drifting until the text disappears into its own chip.
  final Color inlineCodeInk;

  /// Amber wash behind commented text; `active` is the focused thread.
  final Color commentHighlight;
  final Color commentHighlightActive;

  final Color quoteBar;
  final Color dropLine;

  /// GFM alert accent per type (note/tip/important/warning/caution) — the left
  /// bar and title ink. A tinted wash of each sits behind the block.
  final Map<String, Color> alertAccents;

  static EditorTokens lerp(EditorTokens a, EditorTokens b, double t) =>
      EditorTokens(
        selection: Color.lerp(a.selection, b.selection, t)!,
        caret: Color.lerp(a.caret, b.caret, t)!,
        codeBg: Color.lerp(a.codeBg, b.codeBg, t)!,
        inlineCodeBg: Color.lerp(a.inlineCodeBg, b.inlineCodeBg, t)!,
        inlineCodeInk: Color.lerp(a.inlineCodeInk, b.inlineCodeInk, t)!,
        commentHighlight: Color.lerp(
          a.commentHighlight,
          b.commentHighlight,
          t,
        )!,
        commentHighlightActive: Color.lerp(
          a.commentHighlightActive,
          b.commentHighlightActive,
          t,
        )!,
        quoteBar: Color.lerp(a.quoteBar, b.quoteBar, t)!,
        dropLine: Color.lerp(a.dropLine, b.dropLine, t)!,
        // Discrete keys, not a continuum: crossfading five accents would show
        // muddy intermediates for no benefit. Snap at the halfway point.
        alertAccents: t < 0.5 ? a.alertAccents : b.alertAccents,
      );
}

/// Syntax highlighting. One set per mode: a light palette on a dark code block is
/// unreadable, and the reverse is worse.
class CodeTokens {
  const CodeTokens({
    required this.keyword,
    required this.string,
    required this.number,
    required this.comment,
    required this.key,
    required this.function,
  });

  final Color keyword;
  final Color string;
  final Color number;
  final Color comment;

  /// Keys, properties, tags, attributes, sigils — the category the highlighter
  /// actually paints. There is no `type` role because no rule table produces
  /// one; naming a role nothing emits is how a palette starts lying.
  final Color key;
  final Color function;

  static CodeTokens lerp(CodeTokens a, CodeTokens b, double t) => CodeTokens(
    keyword: Color.lerp(a.keyword, b.keyword, t)!,
    string: Color.lerp(a.string, b.string, t)!,
    number: Color.lerp(a.number, b.number, t)!,
    comment: Color.lerp(a.comment, b.comment, t)!,
    key: Color.lerp(a.key, b.key, t)!,
    function: Color.lerp(a.function, b.function, t)!,
  );
}

/// Every colour the app is allowed to use, in one object.
class MicaTokens {
  const MicaTokens({
    required this.dark,
    required this.surface,
    required this.text,
    required this.border,
    required this.accent,
    required this.status,
    required this.editor,
    required this.code,
  });

  /// Which mode this instance IS. Widgets occasionally need to know (an image
  /// placeholder, a shadow's strength) — one honest flag beats each of them
  /// guessing from a luminance.
  final bool dark;

  final SurfaceTokens surface;
  final TextTokens text;
  final BorderTokens border;
  final AccentTokens accent;
  final StatusTokens status;
  final EditorTokens editor;
  final CodeTokens code;

  /// The palette the app already shipped with. Every value here was a literal
  /// somewhere — this is a re-labelling, not a redesign, so adopting the tokens
  /// cannot change how light mode looks.
  static const MicaTokens light = MicaTokens(
    dark: false,
    surface: SurfaceTokens(
      base: Color(0xFFFFFFFF),
      raised: Color(0xFFF8FAFC),
      sunken: Color(0xFFF1F5F9),
      overlay: Color(0xFFFFFFFF),
      hover: Color(0xFFF4F4F6),
    ),
    text: TextTokens(
      // GitHub's ink: calmer than slate-900, still ~13:1 on white.
      primary: Color(0xFF24292F),
      muted: Color(0xFF57606A),
      faint: Color(0xFF9AA4AF),
    ),
    border: BorderTokens(
      subtle: Color(0xFFF1F5F9),
      normal: Color(0xFFE2E8F0),
      strong: Color(0xFFCBD5E1),
    ),
    accent: AccentTokens(
      primary: Color(0xFF2563EB),
      hover: Color(0xFF1D4ED8),
      wash: Color(0xFFEFF6FF),
      onPrimary: Color(0xFFFFFFFF),
    ),
    status: StatusTokens(
      danger: Color(0xFFDC2626),
      dangerWash: Color(0xFFFEF2F2),
      warning: Color(0xFFB45309),
      warningWash: Color(0xFFFFFBEB),
      success: Color(0xFF16A34A),
      successWash: Color(0xFFF0FDF4),
    ),
    editor: EditorTokens(
      selection: Color(0x332563EB),
      caret: Color(0xFF2563EB),
      codeBg: Color(0xFFF4F4F6),
      inlineCodeBg: Color(0x1A64748B),
      inlineCodeInk: Color(0xFF334155),
      commentHighlight: Color(0x33F59E0B),
      commentHighlightActive: Color(0x59F59E0B),
      quoteBar: Color(0xFFCBD5E1),
      dropLine: Color(0xFF2563EB),
      alertAccents: {
        'note': Color(0xFF0969DA),
        'tip': Color(0xFF1A7F37),
        'important': Color(0xFF8250DF),
        'warning': Color(0xFF9A6700),
        'caution': Color(0xFFCF222E),
      },
    ),
    // The values the highlighter has always painted, moved here verbatim. They
    // were GitHub Primer's before, which nothing read — a palette that had
    // never been wired to the thing it claimed to describe.
    code: CodeTokens(
      keyword: Color(0xFF7C3AED),
      string: Color(0xFF0A7D34),
      number: Color(0xFFB45309),
      comment: Color(0xFF6B7280),
      key: Color(0xFF0F766E),
      function: Color(0xFF2563EB),
    ),
  );

  /// The dark answers to the same questions.
  ///
  /// Not an inversion. Pure black backgrounds and pure white ink are the two
  /// mistakes every first dark theme makes: the page is a dark slate (#0F1419)
  /// and the ink is off-white (#E6EDF3), which is roughly where GitHub, AFFiNE
  /// and AppFlowy all land — enough contrast to read, not enough to glare.
  ///
  /// The washes change KIND, not just value: a light theme's `#EFF6FF` on a dark
  /// page reads as a bright rectangle, so on dark the same role becomes the accent
  /// at low alpha — it sits ON the page instead of punching a hole in it.
  static const MicaTokens dark_ = MicaTokens(
    dark: true,
    surface: SurfaceTokens(
      base: Color(0xFF0F1419),
      raised: Color(0xFF161B22),
      sunken: Color(0xFF1C2128),
      overlay: Color(0xFF1C2128),
      hover: Color(0xFF222A33),
    ),
    text: TextTokens(
      primary: Color(0xFFE6EDF3),
      muted: Color(0xFF9BA7B4),
      faint: Color(0xFF6E7A88),
    ),
    border: BorderTokens(
      subtle: Color(0xFF21262D),
      normal: Color(0xFF2D333B),
      strong: Color(0xFF444C56),
    ),
    accent: AccentTokens(
      // A touch lighter than the light-mode blue so it holds up on a dark page.
      primary: Color(0xFF4C8DF6),
      hover: Color(0xFF6BA1F8),
      wash: Color(0x1F4C8DF6),
      onPrimary: Color(0xFF0F1419),
    ),
    status: StatusTokens(
      danger: Color(0xFFF87171),
      dangerWash: Color(0x1FF87171),
      warning: Color(0xFFE3B341),
      warningWash: Color(0x1FE3B341),
      success: Color(0xFF56D364),
      successWash: Color(0x1F56D364),
    ),
    editor: EditorTokens(
      selection: Color(0x4C4C8DF6),
      caret: Color(0xFF4C8DF6),
      codeBg: Color(0xFF161B22),
      inlineCodeBg: Color(0x338B949E),
      inlineCodeInk: Color(0xFFC9D1D9),
      commentHighlight: Color(0x3DE3B341),
      commentHighlightActive: Color(0x66E3B341),
      quoteBar: Color(0xFF444C56),
      dropLine: Color(0xFF4C8DF6),
      alertAccents: {
        'note': Color(0xFF58A6FF),
        'tip': Color(0xFF3FB950),
        'important': Color(0xFFA371F7),
        'warning': Color(0xFFD29922),
        'caution': Color(0xFFF85149),
      },
    ),
    // Each role keeps its LIGHT hue and is lifted for a dark page, rather than
    // adopting a ready-made dark scheme: swapping in another palette would make
    // keywords violet in one mode and red in the other, so the same file would
    // read as two different languages depending on the time of day.
    code: CodeTokens(
      keyword: Color(0xFFC4A0FF),
      string: Color(0xFF7EE787),
      number: Color(0xFFE3B341),
      comment: Color(0xFF8B949E),
      key: Color(0xFF39C5CF),
      function: Color(0xFF79C0FF),
    ),
  );

  static MicaTokens lerp(MicaTokens a, MicaTokens b, double t) => MicaTokens(
    // A boolean cannot be half-true; it flips with the crossfade.
    dark: t < 0.5 ? a.dark : b.dark,
    surface: SurfaceTokens.lerp(a.surface, b.surface, t),
    text: TextTokens.lerp(a.text, b.text, t),
    border: BorderTokens.lerp(a.border, b.border, t),
    accent: AccentTokens.lerp(a.accent, b.accent, t),
    status: StatusTokens.lerp(a.status, b.status, t),
    editor: EditorTokens.lerp(a.editor, b.editor, t),
    code: CodeTokens.lerp(a.code, b.code, t),
  );

  /// The Material theme that matches, so the widgets we do NOT hand-colour
  /// (dialogs, text fields, buttons) come out right instead of staying light.
  ThemeData toMaterialTheme() {
    final scheme =
        ColorScheme.fromSeed(
          seedColor: accent.primary,
          brightness: dark ? Brightness.dark : Brightness.light,
        ).copyWith(
          surface: surface.base,
          onSurface: text.primary,
          primary: accent.primary,
          onPrimary: accent.onPrimary,
          error: status.danger,
          outline: border.normal,
        );
    return ThemeData(
      useMaterial3: true,
      colorScheme: scheme,
      scaffoldBackgroundColor: surface.base,
      dialogTheme: DialogThemeData(backgroundColor: surface.overlay),
      canvasColor: surface.base,
      dividerColor: border.normal,
    );
  }
}

/// Puts [MicaTokens] in the tree. Read it with `MicaTheme.of(context)`.
class MicaTheme extends InheritedWidget {
  const MicaTheme({required this.tokens, required super.child, super.key});

  final MicaTokens tokens;

  /// Falls back to [MicaTokens.light] rather than throwing.
  ///
  /// A widget pumped in a test with no theme above it should look like the app,
  /// not crash — and during the migration most of the tree still has no provider,
  /// so a hard failure would mean the app cannot boot half-done.
  static MicaTokens of(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<MicaTheme>()?.tokens ??
      MicaTokens.light;

  @override
  bool updateShouldNotify(MicaTheme old) => old.tokens != tokens;
}
