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
    // Preserve pushed_clock (the outbox high-water); only advance the synced rid.
    final cur = _store.syncCursor(docId: _docId);
    _store.setSyncCursor(
      docId: _docId,
      cursor: SyncCursor(lastSyncedRid: cursor, pushedClock: cur.pushedClock),
    );
  }

  @override
  int appendOutbox(Uint8List diff) {
    final clock = _store.appendUpdate(docId: _docId, update: diff);
    // A valid clock is ≥ 1; the FFI returns 0 only when the underlying store
    // errored (which it otherwise swallows). Surface it so a failed append can't
    // silently drop the edit from the outbox — the caller must not treat it as
    // pushed. (P2b wires the caller; it decides how to recover.)
    if (clock == 0) {
      throw StateError('outbox append failed for doc $_docId');
    }
    return clock;
  }

  @override
  List<({int clock, Uint8List bytes})> outboxAfter(int pushedClock) => [
    for (final u in _store.updatesAfter(docId: _docId, after: pushedClock))
      (clock: u.clock, bytes: u.payload),
  ];

  @override
  ({int lastSyncedRid, int pushedClock}) cursor() {
    final c = _store.syncCursor(docId: _docId);
    return (lastSyncedRid: c.lastSyncedRid, pushedClock: c.pushedClock);
  }

  @override
  void advance({int? lastSyncedRid, int? pushedClock}) {
    final c = _store.syncCursor(docId: _docId);
    _store.setSyncCursor(
      docId: _docId,
      cursor: SyncCursor(
        lastSyncedRid: lastSyncedRid ?? c.lastSyncedRid,
        pushedClock: pushedClock ?? c.pushedClock,
      ),
    );
  }

  @override
  void trimOutboxThrough(int pushedClock) =>
      _store.trimUpdatesThrough(docId: _docId, upToClock: pushedClock);

  @override
  void appendRemote(int rid, Uint8List update) =>
      _store.appendRemoteUpdate(docId: _docId, rid: rid, update: update);

  @override
  ({int local, int remote}) logSizes() {
    final s = _store.logSizes(docId: _docId);
    return (local: s.$1, remote: s.$2);
  }

  @override
  void compact() => _store.squash(docId: _docId);
}
