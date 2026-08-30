@Tags(['icon-export'])
library;

import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter_svg/flutter_svg.dart';
import 'package:flutter_test/flutter_test.dart';

/// Rasterise `assets/mica-logo.svg` into the bundled `assets/logo_crystal.png`
/// that `MicaLogo` shows at >= 32px.
///
/// SKIPPED unless `MICA_EXPORT_ICONS=1`: it writes into the source tree, and a
/// test that dirties the tree on every run is one people learn to ignore.
///
/// THIS IS A GENERATOR, NOT A GATE — and the distinction matters, because the
/// gate already exists downstream. `icon_export_test.dart` is a GOLDEN test over
/// what `MicaLogo` actually renders, so a stale or wrong crystal here surfaces
/// there as a failing golden with a diff image. Keeping the generator separate
/// is what lets that gate keep running on every CI push while this one stays
/// off. (Folding the two together was tried on 2026-08-30 and silently replaced
/// the gate with a skipped test — the suite stayed green with the drift check
/// gone, which is the exact failure this repo keeps writing down.)
///
/// Why a raster at all, when the SVG is right there: `flutter_svg` is a NON-WEB
/// dependency (CLAUDE.md's exemption table) and `MicaLogo` renders on web too.
/// 512 covers every size the mark is shown at.
///
/// EXPECTED WARNING: `unhandled element <filter/>`. flutter_svg does not
/// implement SVG filters, so the two Gaussian-blurred circles at the core come
/// out hard-edged. Checked against a browser rendering of the same file before
/// accepting it: the halo is a `radialGradient` (which does render) and what is
/// lost is only the softness of the inner core — it reads as a crisp glowing
/// centre, not as a bug. Do NOT "fix" it by flattening the source SVG: that file
/// is the design, and a second flattened copy is the drift this repo keeps
/// paying for.
///
/// After running this, regenerate what depends on it:
///   flutter test --update-goldens test/icon_export_test.dart
///   python scripts/gen-icons.py
void main() {
  final enabled = Platform.environment['MICA_EXPORT_ICONS'] == '1';

  testWidgets('rasterise the crystal artwork into the bundled asset', (tester) async {
    // Inside runAsync: `Picture.toImage` is engine-side async work and the
    // widget tester's fake clock never advances for it — outside, the call
    // simply hangs until the 10-minute test timeout with no error to read.
    await tester.runAsync(() async {
      const size = 512;
      final svg = File('../../assets/mica-logo.svg').readAsStringSync();
      final picture = await vg.loadPicture(SvgStringLoader(svg), null);

      final recorder = ui.PictureRecorder();
      final canvas = Canvas(recorder);
      // drawPicture, not a widget: no layout, no device pixel ratio, exactly
      // size x size out of the 1024-unit artwork.
      canvas.scale(size / picture.size.width);
      canvas.drawPicture(picture.picture);
      final image = await recorder.endRecording().toImage(size, size);
      final png = await image.toByteData(format: ui.ImageByteFormat.png);
      File('assets/logo_crystal.png').writeAsBytesSync(png!.buffer.asUint8List());
      picture.picture.dispose();
    });
  }, skip: !enabled);
}
