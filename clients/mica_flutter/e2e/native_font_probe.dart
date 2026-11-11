// Native font acceptance probe. Uses the production fallback chain and SVG
// rasterizer; writes a rendered screenshot and glyph-distinction results.
// flutter build windows|linux --release --target=e2e/native_font_probe.dart
// MICA_FONT_PROBE_OUTPUT names an isolated output directory at runtime.
import 'dart:convert';
import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:mica_flutter/cjk_fonts.dart';
import 'package:mica_flutter/editor/mermaid_preview.dart';
import 'package:mica_flutter/markdown/markdown_codec.dart';
import 'package:mica_flutter/ui/font_warmup.dart';
import 'package:mica_flutter/ui/theme_tokens.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await initMarkdownCodec();
  const bundled = bool.fromEnvironment(
    'MICA_FONT_PROBE_BUNDLED',
    defaultValue: true,
  );
  if (bundled) await warmUpFonts();
  final loadedBundle = bundled && usesBundledCjkFonts;
  final images = <ui.Image>[];
  final checks = <String, bool>{};
  for (final (name, left, right) in const [
    ('Han', '一', '三'),
    ('Japanese', 'あ', 'ぬ'),
    ('Korean', '한', '글'),
    ('Extension B', '𠮷', '𠀀'),
  ]) {
    final pair = <ui.Image>[];
    for (final text in [left, right]) {
      final image = await renderMermaid(
        'flowchart LR\n A[$text]',
        150,
        MicaTokens.light,
      );
      if (image == null) throw StateError('Mermaid raster failed: $name');
      images.add(image);
      pair.add(image);
    }
    final a = await pair[0].toByteData(format: ui.ImageByteFormat.rawRgba);
    final b = await pair[1].toByteData(format: ui.ImageByteFormat.rawRgba);
    checks[name] =
        a != null &&
        b != null &&
        !listEquals(a.buffer.asUint8List(), b.buffer.asUint8List());
  }
  final boundary = GlobalKey();
  runApp(
    MaterialApp(
      home: Scaffold(
        body: RepaintBoundary(
          key: boundary,
          child: Container(
            color: Colors.white,
            padding: const EdgeInsets.all(24),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
              Text(
                  'Mica native fonts — bundled=$loadedBundle',
                style: const TextStyle(fontSize: 24),
                ),
                const SizedBox(height: 16),
                for (final text in const [
                  '中文简体：开始、确认、完成；繁體：臺灣、香港、龘',
                  '日本語：あいうえお、ぬ、髙橋、東京都',
                  '한국어：한글 글꼴 확인',
                  '生僻字 / Extension B：𠮷 𠀀',
                  'Mixed text 123：表格 / table；公式 x² + y² = z²',
                ])
                  Padding(
                    padding: const EdgeInsets.only(bottom: 12),
                    child: Text(
                      text,
                      style: TextStyle(
                        fontSize: 24,
                        fontFamilyFallback: cjkFontFallback,
                      ),
                    ),
                  ),
                Text(
                  'Production Mermaid rasters: ${checks.entries.map((e) => '${e.key}=${e.value}').join(', ')}',
                ),
                const SizedBox(height: 12),
                Wrap(
                  children: [
                    for (final image in images)
                      RawImage(image: image, width: 120, height: 80),
                  ],
                ),
              ],
            ),
          ),
        ),
      ),
    ),
  );
  await WidgetsBinding.instance.endOfFrame;
  final image =
      await (boundary.currentContext!.findRenderObject()!
              as RenderRepaintBoundary)
          .toImage(pixelRatio: 1);
  try {
    final output = Directory(
      Platform.environment['MICA_FONT_PROBE_OUTPUT'] ?? 'font-probe-output',
    );
    output.createSync(recursive: true);
    final png = await image.toByteData(format: ui.ImageByteFormat.png);
    File(
      '${output.path}/native-fonts.png',
    ).writeAsBytesSync(png!.buffer.asUint8List());
    File('${output.path}/native-fonts.json').writeAsStringSync(
      jsonEncode({
        'platform': Platform.operatingSystem,
        'bundled': loadedBundle,
        'distinctGlyphs': checks,
      }),
    );
  } finally {
    image.dispose();
    for (final image in images) {
      image.dispose();
    }
  }
  exit(0);
}
