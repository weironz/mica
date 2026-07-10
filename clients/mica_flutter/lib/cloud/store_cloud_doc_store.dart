import 'dart:typed_data';

import '../src/rust/api/document.dart';
import '../src/rust/api/store.dart';
import 'cloud_doc_store.dart';

/// [CloudDocStore] backed by the on-device SQLite [MicaStore], keyed by a cloud
/// document's UUID — the desktop local-first mirror for cloud docs (P2 Phase 1).
///
/// The replica persists as a full base snapshot (`saveDoc`) plus the synced
/// `rid` in the doc's `sync_cursor`. Desktop-only: web has no on-device store, so
/// the web cloud session is constructed with `persistence: null`.
class StoreCloudDocStore implements CloudDocStore {
  StoreCloudDocStore(this._store, this._docId);

  final MicaStore _store;

  /// The cloud document's UUID — the key under which its replica is mirrored.
  final String _docId;

  @override
  ({Uint8List state, int cursor})? load() {
    final doc = _store.loadDoc(docId: _docId);
    if (doc == null) return null;
    final cursor = _store.syncCursor(docId: _docId).lastSyncedRid;
    return (state: doc.encodeState(), cursor: cursor);
  }

  @override
  void save(Uint8List state, int cursor) {
    final doc = MicaDocument.fromState(bytes: state);
    if (doc == null) return;
    _store.saveDoc(docId: _docId, doc: doc);
    _store.setSyncCursor(
      docId: _docId,
      cursor: SyncCursor(lastSyncedRid: cursor, pushedClock: 0),
    );
  }
}
