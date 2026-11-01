import 'dart:async';
import 'dart:js_interop';
import 'package:web/web.dart' as web;
import 'package:mica_flutter/web/mica_doc_wasm.dart';

@JS('micaDocReady')
external JSPromise<JSAny?>? get _micaDocReady;

/// Load the generated document engine (scripts/build-doc-wasm.sh copies it to
/// test/support/mica_doc) the way web/index.html does, then run the app's own
/// [initDocEngine]. Fails the test if the engine does not come up — a browser
/// suite that silently ran without it would test nothing.
Future<void> loadBrowserDocEngine() async {
  if (_micaDocReady == null) {
    final script = web.HTMLScriptElement()..src = 'support/mica_doc/mica_doc.js';
    final loaded = Completer<void>();
    script.onload = ((web.Event _) => loaded.complete()).toJS;
    script.onerror = ((web.Event _) => loaded.completeError(
      StateError('Cannot load generated mica_doc.js — run scripts/build-doc-wasm.sh'),
    )).toJS;
    web.document.body!.append(script);
    await loaded.future.timeout(const Duration(seconds: 10));
    // The no-modules glue declares a top-level `let`, not a globalThis
    // property; bridge it from a classic script, exactly like index.html.
    final initialize = web.HTMLScriptElement()
      ..text = '''
        globalThis.micaDocReady = mica_doc_bindgen({module_or_path: 'support/mica_doc/mica_doc_bg.wasm'})
          .then(() => { globalThis.micaDoc = mica_doc_bindgen; })
          .catch((error) => { globalThis.micaDocError = String(error); });
      ''';
    web.document.body!.append(initialize);
  }
  await initDocEngine();
  final error = docEngineError;
  if (error != null) throw StateError('document engine did not load: $error');
}
