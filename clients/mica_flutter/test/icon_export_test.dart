import 'dart:io';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/widgets/mica_logo.dart';

// The icon pipeline's first half: rasterise the SHIPPING `MicaLogoPainter` at
// every size a Windows .ico carries, into test/icon_src/. `scripts/gen-icons.py`
// then packs those into the app and tray icons.
//
// It is a GOLDEN test on purpose, which makes it the drift gate: change the
// painter without regenerating and these fail, in CI, with a diff image. The
// alternative — a script nobody remembers to run — is exactly how the tray icon
// stayed the stock Flutter logo from 2026-07-20 to 2026-08-28.
//
// To regenerate after an intentional change:
//   flutter test --update-goldens test/icon_export_test.dart
//   python scripts/gen-icons.py
//
// Each size is separate artwork, not a downscale: the painter drops the corner
// nodes below 22px so the mark survives a 16px tray slot.
void main() {
  const sizes = [16, 24, 32, 48, 64, 128, 256];
  for (final s in sizes) {
    testWidgets('export $s', (tester) async {
      final key = GlobalKey();
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(
        MaterialApp(
          home: Align(
            alignment: Alignment.topLeft,
            child: RepaintBoundary(
              key: key,
              child: SizedBox.square(
                dimension: s.toDouble(),
                child: MicaLogo(size: s.toDouble()),
              ),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      await expectLater(
        find.byKey(key),
        matchesGoldenFile('icon_src/$s.png'),
      );
      expect(File('test/icon_src/$s.png').existsSync(), isTrue);
    });
  }
}
