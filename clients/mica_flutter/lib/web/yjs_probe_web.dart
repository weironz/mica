// P2-M4 (web→yjs) W1 verification hook: expose `window.micaYjsSelfTest(b64)` so a
// browser harness can confirm Dart-in-browser drives yjs and reads our
// wire-compatible doc layout (apply a real yrs-produced update → read blocks).
import 'dart:convert';
import 'dart:js_interop';

import 'mica_ydoc.dart';
import 'yjs_interop.dart';

@JS('micaYjsSelfTest')
external set _selfTest(JSFunction f);

void registerYjsSelfTest() {
  if (!yjsAvailable) return;
  _selfTest = ((JSString b64) {
    try {
      final doc = MicaYDoc.fromState(base64.decode(b64.toDart));
      return jsonEncode({
        'ok': true,
        'root': doc.rootBlockId(),
        'blocks': doc.toBlocks(),
      }).toJS;
    } catch (e) {
      return jsonEncode({'ok': false, 'error': '$e'}).toJS;
    }
  }).toJS;
}
