// Web variant of the P4-2 platform doc-store factory: an IndexedDB-backed
// local-first mirror for one cloud doc. Returns null when IndexedDB is
// unavailable (e.g. some private-browsing modes) — online-only, as before.
import '../web/mica_ydoc.dart';
import 'cloud_doc_store.dart';
import 'web_idb_doc_store.dart';

Future<CloudDocStore?> openWebDocStore(String origin, String docId) =>
    WebIdbDocStore.open(origin, docId);

/// Decode a doc's IndexedDB mirror into editor blocks (offline doc-open on
/// web — the async counterpart of the desktop's synchronous FFI mirror read).
/// Null when never mirrored / IndexedDB unavailable / undecodable.
Future<({String rootBlockId, List<Map<String, dynamic>> blocks})?>
    openWebDocMirror(String origin, String docId) async {
  final store = await WebIdbDocStore.open(origin, docId);
  final loaded = store?.load();
  if (loaded == null) return null;
  try {
    final doc = MicaYDoc.fromState(loaded.state);
    final root = doc.rootBlockId();
    if (root.isEmpty) return null;
    return (rootBlockId: root, blocks: doc.toBlocks());
  } catch (_) {
    return null; // corrupt mirror → behave as never-mirrored
  }
}
