import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/widgets/mica_logo.dart';

/// `Image.asset` with a path that is not declared in `pubspec.yaml` fails at
/// RUNTIME, not at build — so a typo ships. That is the whole first test.
///
/// The second guards the one duplicate this mark cannot design away: the
/// browser tab icon has to exist before Flutter boots, so `web/favicon.svg` is
/// a COPY of the source SVG rather than something generated from it. A copy
/// with nothing checking it is precisely how the tray icon stayed the stock
/// Flutter logo for over a month.
void main() {
  testWidgets('draws the bundled mark, at the path pubspec declares', (tester) async {
    await tester.pumpWidget(
      const MaterialApp(home: Scaffold(body: Center(child: MicaLogo(size: 40)))),
    );
    final image = tester.widget<Image>(find.byType(Image));
    expect(
      (image.image as AssetImage).assetName,
      'assets/logo_mark.png',
      reason: 'must match the path declared in pubspec.yaml',
    );
  });

  testWidgets('and that asset actually resolves out of the bundle', (tester) async {
    // Asserting the path STRING is not the same as asserting the asset exists:
    // the string test passes just as happily against a name that pubspec no
    // longer declares. Decoding it is what catches a rename that only got done
    // in half the places (this widget was renamed logo_crystal -> logo_mark on
    // 2026-09-02, which is exactly when that gap would have shipped).
    await tester.pumpWidget(
      const MaterialApp(home: Scaffold(body: Center(child: MicaLogo(size: 40)))),
    );
    await tester.runAsync(
      () => precacheImage(
        const AssetImage('assets/logo_mark.png'),
        tester.element(find.byType(MicaLogo)),
      ),
    );
    await tester.pumpAndSettle();
    expect(tester.takeException(), isNull);
  });

  test('web/favicon.svg is a verbatim copy of the source SVG', () {
    String read(String p) =>
        File(p).readAsStringSync().replaceAll('\r\n', '\n');
    expect(
      read('web/favicon.svg'),
      read('../../assets/mica-logo.svg'),
      reason: 'the tab icon is a copy of assets/mica-logo.svg — re-copy it, '
          'do not hand-edit it',
    );
  });
}
