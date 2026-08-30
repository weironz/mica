import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/widgets/mica_logo.dart';

/// The mark is one design at two optical sizes and the switch is silent: pick
/// the wrong branch and you get a legible-but-wrong logo, or a crystal that has
/// smeared into a blue dot. Neither throws.
///
/// The asset path is the other half: `Image.asset` with a path that is not in
/// `pubspec.yaml` fails at RUNTIME, not at build — so a typo ships.
void main() {
  Future<void> show(WidgetTester tester, Widget child) =>
      tester.pumpWidget(MaterialApp(home: Scaffold(body: Center(child: child))));

  testWidgets('at or above the threshold it draws the crystal artwork', (tester) async {
    await show(tester, const MicaLogo(size: MicaLogo.crystalFrom));
    final image = tester.widget<Image>(find.byType(Image));
    expect(
      (image.image as AssetImage).assetName,
      'assets/logo_crystal.png',
      reason: 'must match the path declared in pubspec.yaml',
    );
  });

  testWidgets('below the threshold it draws the wireframe painter', (tester) async {
    await show(tester, const MicaLogo(size: MicaLogo.crystalFrom - 1));
    expect(find.byType(Image), findsNothing);
    final painted = tester
        .widgetList<CustomPaint>(find.byType(CustomPaint))
        .where((c) => c.painter is MicaLogoPainter);
    expect(painted, isNotEmpty, reason: 'the small tier is the painter');
  });

  testWidgets('a flat colour always takes the painter, at any size', (tester) async {
    // A monochrome crystal is mud — the whole reason `color` exists is contexts
    // that cannot show a gradient, and those must not get the artwork instead.
    await show(tester, const MicaLogo(size: 128, color: Colors.white));
    expect(find.byType(Image), findsNothing);
    final painter = tester
        .widgetList<CustomPaint>(find.byType(CustomPaint))
        .map((c) => c.painter)
        .whereType<MicaLogoPainter>()
        .single;
    expect(painter.flat, Colors.white);
  });

  testWidgets('the default size stays on the small tier', (tester) async {
    // Callers that pass nothing get the sidebar-sized mark; if the default ever
    // crossed the threshold they would silently start loading a 512px raster.
    await show(tester, const MicaLogo());
    expect(find.byType(Image), findsNothing);
  });
}
