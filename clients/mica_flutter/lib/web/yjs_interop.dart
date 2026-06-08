// P2-M4 (web→yjs): Dart bindings to the browser yjs engine exposed by
// `web/yjs_bundle.js` as `globalThis.micaYjs`. Web-only — this file imports
// `dart:js_interop` and must never be compiled for desktop (it's reached solely
// through the `dart.library.html` conditional in `yjs_probe.dart`).
import 'dart:js_interop';

/// The flat helper API from `tool/yjs/entry.js`.
@JS('micaYjs')
external MicaYjs get micaYjs;

/// Whether the yjs bundle loaded (the `<script>` ran before Flutter).
@JS('micaYjs')
external JSAny? get _micaYjsRaw;
bool get yjsAvailable => _micaYjsRaw != null;

extension type MicaYjs._(JSObject _) implements JSObject {
  external JSObject newDoc();
  external JSObject docFromUpdate(JSUint8Array u8);
  external bool applyUpdate(JSObject doc, JSUint8Array u8);
  external JSUint8Array encodeState(JSObject doc);
  external JSUint8Array encodeStateVector(JSObject doc);
  external JSUint8Array encodeDiff(JSObject doc, JSUint8Array sv);

  external JSObject getMap(JSObject doc, String name);
  external JSAny? mapGet(JSObject m, String k);
  external void mapSet(JSObject m, String k, JSAny v);
  external bool mapHas(JSObject m, String k);
  external void mapDelete(JSObject m, String k);
  external JSArray<JSString> mapKeys(JSObject m);
  external void mapSetJson(JSObject m, String k, String jsonStr);
  external String mapEntriesJson(JSObject m);

  external JSObject newMap();
  external JSObject newText(String s);
  external JSObject newArray();

  external String textToString(JSAny? t);
  external int textLength(JSAny? t);
  external void textInsert(JSObject t, int i, String s);
  external void textDelete(JSObject t, int i, int len);
  external void textFormat(JSObject t, int i, int len, JSAny attrs);
  external JSArray<JSAny?> textDelta(JSObject t);
  external String textDeltaJson(JSObject t);
  external void textFormatJson(JSObject t, int i, int len, String attrsJson);

  external JSArray<JSAny?> arrayToList(JSAny? a);
  external int arrayLength(JSAny? a);
  external void arrayInsert(JSObject a, int i, JSArray<JSAny?> items);
  external void arrayInsertJson(JSObject a, int i, String itemsJson);
  external void arrayDelete(JSObject a, int i, int len);

  external bool isText(JSAny? x);
  external bool isMap(JSAny? x);
  external bool isArray(JSAny? x);
}
