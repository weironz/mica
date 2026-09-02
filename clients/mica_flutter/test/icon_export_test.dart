import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter_svg/flutter_svg.dart';
import 'package:flutter_test/flutter_test.dart';
import 'dart:io';

// The icon pipeline: rasterise `assets/mica-logo.svg` — the one source for the
// mark — at every size that ships, as GOLDEN files. `scripts/gen-icons.py` then
// packs `test/icon_src/` into the app icon and the tray icon.
//
// Golden and not a script, on purpose: change the SVG without regenerating and
// these fail, in CI, with a diff image. A script nobody remembers to run is
// exactly how the tray icon stayed the stock Flutter logo from 2026-07-20 to
// 2026-08-28.
//
// THE 512 FRAME IS THE SHIPPED ASSET (2026-09-02). `assets/logo_mark.png` —
// what `MicaLogo` draws — is itself one of these goldens, so the generator and
// the gate are a single act and the bundled raster cannot silently lag its
// source.
//
// That deliberately REVERSES a warning that used to live in
// `logo_asset_export_test.dart` (now deleted) against combining the two. The
// warning was about the other direction: folding the always-running GATE into
// an env-gated GENERATOR turned it into a skipped test, and the suite stayed
// green with the drift check gone. Folding a generator into a gate that runs on
// every push has the opposite effect — nothing here is skipped.
//
// EVERY FRAME IS ITS OWN RASTERISATION, not a downscale of the biggest.
// Measured 2026-09-02: 512 -> 16 through FilterQuality.medium softens the M's
// inner notch to about half its depth, while rasterising the vector straight at
// 16 keeps it. An .ico is a container; it is meant to hold per-size artwork.
//
// To regenerate after an intentional change to the SVG:
//   flutter test --update-goldens test/icon_export_test.dart
//   python scripts/gen-icons.py
void main() {
  // What a Windows .ico carries. 16/32/48/256 are what the shell actually asks
  // for; the rest stop it from downscaling 256 for mid-DPI taskbars.
  const icoFrames = {
    16: 'icon_src/16.png',
    24: 'icon_src/24.png',
    32: 'icon_src/32.png',
    48: 'icon_src/48.png',
    64: 'icon_src/64.png',
    128: 'icon_src/128.png',
    256: 'icon_src/256.png',
    // Not an .ico frame: the asset the app bundles. Listed here so it is
    // produced by the same command and gated by the same run.
    512: '../assets/logo_mark.png',
  };

  icoFrames.forEach((size, golden) {
    testWidgets('export $size', (tester) async {
      late ui.Image image;
      // Inside runAsync: `Picture.toImage` is engine-side async work and the
      // widget tester's fake clock never advances for it — outside, the call
      // hangs until the test times out with no error to read.
      await tester.runAsync(() async {
        final svg = File('../../assets/mica-logo.svg').readAsStringSync();
        final picture = await vg.loadPicture(SvgStringLoader(svg), null);

        final recorder = ui.PictureRecorder();
        final canvas = Canvas(recorder);
        // drawPicture, not a widget: no layout, no device pixel ratio, exactly
        // size x size out of the 1024-unit artwork.
        canvas.scale(size / picture.size.width);
        canvas.drawPicture(picture.picture);
        image = await recorder.endRecording().toImage(size, size);
        picture.picture.dispose();
      });
      // A ui.Image and NOT the encoded PNG bytes, which would be the obvious
      // thing to hand a golden. `MatchesGoldenFile.matchAsync` is asymmetric:
      // the Finder/Image branches run the comparator inside
      // `binding.runAsync`, the `List<int>` branch awaits the comparator's file
      // IO directly — and a real-IO future never completes inside the fake-async
      // zone `testWidgets` runs in. The bytes form does not fail, it HANGS, with
      // no output at all until the timeout. (Cost an hour on 2026-09-02.)
      await expectLater(image, matchesGoldenFile(golden));
      // The Image branch sets disposeImage = false: the matcher does not own
      // what it was handed.
      image.dispose();
    });
  });
}
