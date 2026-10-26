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
openWebDocMirror(
  String origin,
  String docId, {
  String dbName = WebIdbDocStore.defaultDbName,
}) async {
  // A prior holder may have JUST released its Web Lock (dispose waits for the
  // write tail, so the release is asynchronous); a single open could race that
  // window and read "no mirror" for a doc that is actually mirrored. Retry
  // briefly, mirroring the `reopen` helper in the store tests.
  WebIdbDocStore? store;
  for (var attempt = 0; attempt < 100; attempt++) {
    store = await WebIdbDocStore.open(origin, docId, dbName: dbName);
    if (store != null) break;
    await Future<void>.delayed(const Duration(milliseconds: 10));
  }
  if (store == null) return null;
  // The store holds the doc's single-writer Web Lock for as long as it lives.
  // This helper only READS the mirror, so every exit path (never mirrored,
  // empty root, undecodable, success) must hand the lock back — otherwise the
  // NEXT same-doc open in this very tab is refused and sync degrades to
  // no-IndexedDB persistence (issue #149). dispose() waits for the write tail
  // (nothing is queued here) before releasing.
  try {
    final loaded = store.load();
    if (loaded == null) return null;
    try {
      final doc = MicaYDoc.fromState(loaded.state);
      final root = doc.rootBlockId();
      if (root.isEmpty) return null;
      return (rootBlockId: root, blocks: doc.toBlocks());
    } catch (_) {
      return null; // corrupt mirror → behave as never-mirrored
    }
  } finally {
    store.dispose();
  }
}
