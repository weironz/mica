// P2-M4 (web→yjs): the web counterpart of the desktop `MicaDocument` (FFI).
//
// Backed by a JS `Y.Doc` (driven over `yjs_interop.dart`) instead of the Rust
// `yrs` core, but reading/writing the SAME shared-type layout the Rust model
// uses (`blocks` map of block maps with `ty`/`text`/`props`/`children`, `meta`
// with `root`) so the two are wire-compatible. W1 implements the read side +
// sync primitives; marks reconstruction and the write side (from_blocks / edit
// ops) follow in W2.
//
// Web-only (imports `dart:js_interop`); reached solely via the conditional in
// `yjs_probe.dart`.
import 'dart:convert';
import 'dart:js_interop';
import 'dart:typed_data';

import 'yjs_interop.dart';

class MicaYDoc {
  MicaYDoc._(this._doc);
  final JSObject _doc;

  /// Rebuild a doc from an encoded yrs/yjs v1 update (e.g. the server's base).
  static MicaYDoc fromState(Uint8List bytes) =>
      MicaYDoc._(micaYjs.docFromUpdate(bytes.toJS));

  static MicaYDoc empty() => MicaYDoc._(micaYjs.newDoc());

  // ── sync primitives (mirror MicaDocument) ──
  Uint8List stateVector() => micaYjs.encodeStateVector(_doc).toDart;
  Uint8List encodeState() => micaYjs.encodeState(_doc).toDart;
  Uint8List encodeDiffSince(Uint8List sv) =>
      micaYjs.encodeDiff(_doc, sv.toJS).toDart;
  bool applyUpdate(Uint8List bytes) => micaYjs.applyUpdate(_doc, bytes.toJS);

  String rootBlockId() {
    final meta = micaYjs.getMap(_doc, 'meta');
    final r = micaYjs.mapGet(meta, 'root');
    return (r != null && r.isA<JSString>()) ? (r as JSString).toDart : '';
  }

  /// The document as a flat block list in tree order (mirrors `to_blocks`).
  List<Map<String, dynamic>> toBlocks() {
    final blocks = micaYjs.getMap(_doc, 'blocks');
    final byId = <String, Map<String, dynamic>>{};
    for (final k in micaYjs.mapKeys(blocks).toDart) {
      final id = k.toDart;
      final bm = micaYjs.mapGet(blocks, id);
      if (bm == null || !micaYjs.isMap(bm)) continue;
      final block = bm as JSObject;

      final tyAny = micaYjs.mapGet(block, 'ty');
      final ty =
          (tyAny != null && tyAny.isA<JSString>()) ? (tyAny as JSString).toDart : 'paragraph';

      final textAny = micaYjs.mapGet(block, 'text');
      final text = (textAny != null && micaYjs.isText(textAny))
          ? micaYjs.textToString(textAny)
          : '';

      final propsAny = micaYjs.mapGet(block, 'props');
      final propsStr =
          (propsAny != null && propsAny.isA<JSString>()) ? (propsAny as JSString).toDart : null;
      var data = <String, dynamic>{};
      if (propsStr != null && propsStr.isNotEmpty && propsStr != 'null') {
        final decoded = jsonDecode(propsStr);
        if (decoded is Map<String, dynamic>) data = decoded;
      }

      final childrenAny = micaYjs.mapGet(block, 'children');
      final children = <String>[];
      if (childrenAny != null && micaYjs.isArray(childrenAny)) {
        for (final c in micaYjs.arrayToList(childrenAny).toDart) {
          if (c != null && c.isA<JSString>()) children.add((c as JSString).toDart);
        }
      }

      byId[id] = {
        'id': id,
        'type': ty,
        'text': text,
        'data': data,
        'children': children,
      };
    }

    final out = <Map<String, dynamic>>[];
    final seen = <String>{};
    void dfs(String id) {
      if (!seen.add(id)) return;
      final b = byId[id];
      if (b == null) return;
      out.add(b);
      for (final c in (b['children'] as List).cast<String>()) {
        dfs(c);
      }
    }

    final root = rootBlockId();
    if (byId.containsKey(root)) {
      dfs(root);
    } else {
      out.addAll(byId.values);
    }
    return out;
  }

  String toBlocksJson() => jsonEncode(toBlocks());
}
