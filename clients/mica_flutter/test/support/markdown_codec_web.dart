import 'dart:async';
import 'dart:js_interop';
import 'package:web/web.dart' as web;
import 'package:mica_flutter/markdown/markdown_codec.dart';
import 'package:mica_flutter/markdown/markdown_codec_web.dart' as platform;

@JS('wasm_bindgen')
external JSPromise<JSAny?> _initWasm(JSObject options);
@JS('wasm_bindgen')
external JSObject get _wasmBindings;
@JS('micaMd')
external set _micaMd(JSObject value);
@JS('micaMdReady')
external set _micaMdReady(JSPromise<JSAny?> value);
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
    final ready = _initWasm(
      ({'module_or_path': 'support/mica_md/mica_md_bg.wasm'}).jsify()
          as JSObject,
    );
    _micaMdReady = ready;
    await ready.toDart.timeout(const Duration(seconds: 10));
    _micaMd = _wasmBindings;
    return await platform.createMarkdownCodec();
  } catch (error) {
    _micaMdError = '$error'.toJS;
    rethrow;
  } finally {
    script.onload = null;
    script.onerror = null;
  }
}
