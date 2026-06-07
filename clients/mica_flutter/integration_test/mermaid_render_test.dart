// Real-device render verification for the non-web (merman FFI) mermaid path.
//
// Runs on the Windows runner where merman_ffi.dll is bundled, so it exercises
// the actual pure-Rust engine + css-inline + flutter_svg rasterization
// end-to-end — the only honest way to prove the FFI path works (a plain
// `flutter test` VM has no native lib). Rendering is offline by construction:
// merman touches no network.
//
//   flutter test integration_test/mermaid_render_test.dart -d windows
import 'dart:ui' as ui;

import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:mica_flutter/editor/mermaid_preview.dart';

/// Fraction of pixels that carry ink (non-transparent). A blank/failed render
/// is fully transparent, so any healthy diagram clears this easily; this is the
/// guard that a too-weak "image is non-null" assertion would miss.
Future<double> _inkFraction(ui.Image img) async {
  final data = await img.toByteData(format: ui.ImageByteFormat.rawRgba);
  final bytes = data!.buffer.asUint8List();
  var ink = 0;
  for (var i = 3; i < bytes.length; i += 4) {
    if (bytes[i] > 16) ink++;
  }
  return ink / (img.width * img.height);
}

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  test('mermaid is available on this (non-web) platform', () {
    expect(mermaidAvailable, isTrue);
  });

  // One representative diagram per major mermaid family, in valid (newline)
  // syntax — each must render to a non-empty, inked raster.
  const cases = <String, String>{
    'flowchart': 'graph TD\n  A[Start] --> B{Choice}\n  B -->|yes| C[OK]\n  B -->|no| D[Stop]',
    'sequence': 'sequenceDiagram\n  Alice->>Bob: Hello\n  Bob-->>Alice: Hi',
    'class': 'classDiagram\n  Animal <|-- Dog\n  Animal : +int age\n  Dog : +bark()',
    'state': 'stateDiagram-v2\n  [*] --> Idle\n  Idle --> Running\n  Running --> [*]',
    'pie': 'pie title Pets\n  "Dogs" : 386\n  "Cats" : 85',
    'gantt': 'gantt\n  title Plan\n  section A\n  Task1 : a1, 2026-01-01, 7d\n  Task2 : after a1, 5d',
  };

  const targetWidth = 800.0;

  cases.forEach((name, source) {
    testWidgets('renders $name to an inked raster', (tester) async {
      ui.Image? image;
      double ink = 0;
      await tester.runAsync(() async {
        image = await renderMermaid(source, targetWidth);
        if (image != null) ink = await _inkFraction(image!);
      });
      expect(image, isNotNull, reason: '$name should render to an image');
      expect(image!.width, greaterThan(0));
      expect(image!.height, greaterThan(0));
      expect(image!.width, lessThanOrEqualTo((targetWidth * 8).round()));
      expect(ink, greaterThan(0.003),
          reason: '$name rendered blank (ink=$ink) — theme/CSS likely lost');
      image!.dispose();
    });
  });

  testWidgets('bad syntax degrades to null (never throws)', (tester) async {
    ui.Image? image;
    await tester.runAsync(() async {
      image = await renderMermaid('graph TD; <<<not mermaid>>>', targetWidth);
    });
    expect(image, isNull);
  });
}
