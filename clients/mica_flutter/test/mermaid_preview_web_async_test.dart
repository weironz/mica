@TestOn('browser')
library;

import 'dart:async';
import 'dart:js_interop';
import 'dart:js_interop_unsafe';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/mermaid_preview_web.dart';
import 'package:mica_flutter/ui/theme_tokens.dart';

void main() {
  test(
    'theme initialization waits for render and failures release queue',
    () async {
      final original = globalContext.getProperty<JSAny?>('mermaid'.toJS);
      addTearDown(() => globalContext.setProperty('mermaid'.toJS, original));
      final engine = JSObject();
      final themes = <String>[];
      final jobs = <Completer<JSObject>>[];
      engine.setProperty(
        'initialize'.toJS,
        ((JSObject config) {
          themes.add((config.getProperty('theme'.toJS) as JSString).toDart);
        }).toJS,
      );
      engine.setProperty(
        'render'.toJS,
        ((JSString id, JSString source) {
          final job = Completer<JSObject>();
          jobs.add(job);
          return job.future.toJS;
        }).toJS,
      );
      globalContext.setProperty('mermaid'.toJS, engine);

      final light = renderMermaid('light source', 100, MicaTokens.light);
      await Future<void>.delayed(Duration.zero);
      expect(jobs, hasLength(1));
      expect(themes, ['neutral']);

      final dark = renderMermaid('dark source', 100, MicaTokens.dark_);
      await Future<void>.delayed(Duration.zero);
      expect(jobs, hasLength(1));
      expect(themes, ['neutral']);

      // A failed old render still owns the global light theme until it settles.
      // Releasing it must let the queued dark render initialize and start.
      jobs.first.completeError(StateError('invalid light diagram'));
      expect(await light, isNull);
      await Future<void>.delayed(Duration.zero);
      expect(jobs, hasLength(2));
      expect(themes, ['neutral', 'dark']);
      jobs.last.completeError(StateError('invalid dark diagram'));
      expect(await dark, isNull);
    },
  );
}
