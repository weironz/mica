import 'dart:async';
import 'dart:js_interop';
import 'package:web/web.dart' as web;
import 'package:mica_flutter/markdown/markdown_codec.dart';
import 'package:mica_flutter/markdown/markdown_codec_web.dart' as platform;

@JS('micaMdReady')
external JSPromise<JSAny?> get _micaMdReady;
@JS('micaMdError')
external set _micaMdError(JSString value);

Future<MarkdownCodec> loadBrowserMarkdownCodec() async {
  final script = web.HTMLScriptElement()..src = 'support/mica_md/mica_md.js';
  final loaded = Completer<void>();
  script.onload = ((web.Event _) => loaded.complete()).toJS;
  script.onerror = ((web.Event _) => loaded.completeError(
    StateError('Cannot load generated mica_md.js'),
  )).toJS;
  web.document.body!.append(script);
  try {
    await loaded.future.timeout(const Duration(seconds: 10));
    // wasm-bindgen no-modules declares a top-level `let`, which is a global
    // lexical binding rather than a globalThis property. JS interop getters
    // read globalThis, so bridge it from an actual classic script, like index.
    final initialize = web.HTMLScriptElement()
      ..text = '''
        globalThis.micaMdReady = wasm_bindgen({module_or_path: 'support/mica_md/mica_md_bg.wasm'})
          .then(() => { globalThis.micaMd = wasm_bindgen; });
      ''';
    web.document.body!.append(initialize);
    await _micaMdReady.toDart.timeout(const Duration(seconds: 10));
    return await platform.createMarkdownCodec();
  } catch (error) {
    _micaMdError = '$error'.toJS;
    rethrow;
  } finally {
    script.onload = null;
    script.onerror = null;
  }
}
