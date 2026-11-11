@TestOn('browser')
library;

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/mermaid_theme.dart';
import 'package:mica_flutter/editor/preview_engine.dart'
    show PreviewUnavailable;
import 'package:mica_flutter/editor/preview_engine_web.dart';
import 'package:mica_flutter/ui/theme_tokens.dart';

/// The host side of the preview worker's reply protocol: a crash costs only
/// the source that caused it, and everything else is retried, not failed.
void main() {
  final theme = mermaidThemeFor(MicaTokens.light);

  test(
    'a crash fails only its own source; the rest come back retryable',
    () async {
      previewWorkerUrl = 'support/preview_worker_scripted.js';
      // Listen before the crash: it fails this request while it is in flight.
      final held = expectLater(
        mermaidSvg('hold', theme),
        throwsA(isA<PreviewUnavailable>()),
        reason: 'an innocent request in flight must be retried, not failed',
      );
      final crashed = mermaidSvg('crash', theme);

      expect(
        await crashed,
        isNull,
        reason: 'the crashing source has no preview',
      );
      await held;

      // A fresh worker takes the next request.
      expect(await mermaidSvg('next', theme), 'ok:next');
    },
  );

  test(
    'a renderer that cannot load is retryable; a bad source is final',
    () async {
      previewWorkerUrl = 'support/preview_worker_scripted.js';
      final held = expectLater(
        mermaidSvg('hold', theme),
        throwsA(isA<PreviewUnavailable>()),
        reason: 'the worker is gone, so its other requests retry too',
      );
      await expectLater(
        mermaidSvg('away', theme),
        throwsA(isA<PreviewUnavailable>()),
      );
      await held;
      expect(await mermaidSvg('bad', theme), isNull);
      expect(await mermaidSvg('fine', theme), 'ok:fine');
    },
  );
}
