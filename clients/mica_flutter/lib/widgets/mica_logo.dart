import 'package:flutter/material.dart';

/// The Mica mark: the letter M, its two inner strokes cut so they read as
/// folded sheets — the mineral the product is named for.
///
/// ONE MARK, ONE SIZE (2026-09-02). This replaces the isometric-cube wireframe
/// and, with it, the whole two-tier structure that mark required: a crystal
/// raster at >= 128px and a hand-drawn, reduced wireframe below it, because the
/// artwork smeared into a featureless blue dot in a 16px tray slot.
///
/// The new mark does not smear. Measured at 16/24/32/48 before the tiers were
/// deleted, not assumed — at 16px the M and its inner notch are both still
/// there. So there is nothing left to switch between, and ~110 lines of
/// geometry that existed only to survive small sizes are gone, along with the
/// class of bug they carried: a silent branch handing one context a different
/// logo than another.
///
/// A RASTER rather than the SVG, deliberately: `flutter_svg` is a non-web
/// dependency (CLAUDE.md's exemption table) and this widget also renders on
/// web. `assets/logo_mark.png` is 512 — every size the mark is shown at in-app
/// — and it is generated from `assets/mica-logo.svg` by
/// `test/icon_export_test.dart`, where it IS one of the goldens. It therefore
/// cannot go stale without CI saying so.
///
/// `web/favicon.svg` is a byte-for-byte copy of that same source SVG (the tab
/// icon has to exist before Flutter boots, so it cannot come from here);
/// `test/mica_logo_test.dart` refuses to let the two drift.
class MicaLogo extends StatelessWidget {
  const MicaLogo({this.size = 24, super.key});

  final double size;

  @override
  Widget build(BuildContext context) {
    return Image.asset(
      'assets/logo_mark.png',
      width: size,
      height: size,
      // The mark is square and already has its own breathing room; letting it
      // letterbox would shrink it against the sizes callers picked.
      fit: BoxFit.contain,
      filterQuality: FilterQuality.medium,
    );
  }
}
