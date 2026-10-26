import 'dart:typed_data' hide Int64List;

import 'package:flutter_rust_bridge/flutter_rust_bridge.dart' show Int64List;

import '../src/rust/api/document.dart';
import '../src/rust/api/store.dart';
import 'cloud_doc_store.dart';

/// [CloudDocStore] backed by the on-device SQLite [MicaStore], keyed by an
/// account-scoped cloud mirror [origin] plus the document's UUID — the desktop
/// local-first mirror for cloud docs (P2 Phase 1).
///
/// The replica persists as a full base snapshot (`saveDoc`) plus the synced
/// `rid` in the doc's `sync_cursor`. Desktop-only: web has no on-device store, so
/// the web cloud session is constructed with `persistence: null`.
///
/// [origin] is REQUIRED and comes from `cloudCacheOrigin(server, accountId)`
/// (issue #111): the store also holds other accounts' mirrors and local pages,
/// and a bare doc id would let two accounts on one server share a body, an
/// outbox and a cursor.
class StoreCloudDocStore implements CloudDocStore {
  StoreCloudDocStore(this._store, this._origin, this._docId);

  final MicaStore _store;

  /// Account+server scope of this replica (see `cloudCacheOrigin`).
  final String _origin;

  /// The cloud document's UUID — the key under which its replica is mirrored.
  final String _docId;

  @override
  ({Uint8List state, int cursor})? load() {
    final doc = _store.loadDoc(origin: _origin, docId: _docId);
    if (doc == null) return null;
    final cursor = _store.syncCursor(origin: _origin, docId: _docId).lastSyncedRid;
    return (state: doc.encodeState(), cursor: cursor);
  }

  @override
  void save(Uint8List state, int cursor) {
    final doc = MicaDocument.fromState(bytes: state);
    if (doc == null) return;
    _store.saveDoc(origin: _origin, docId: _docId, doc: doc);
    // Preserve pushed_clock (the outbox high-water); only advance the synced rid.
    final cur = _store.syncCursor(origin: _origin, docId: _docId);
    _store.setSyncCursor(
      origin: _origin,
      docId: _docId,
      cursor: SyncCursor(lastSyncedRid: cursor, pushedClock: cur.pushedClock),
    );
  }

  @override
  int appendOutbox(Uint8List diff) {
    final clock = _store.appendUpdate(
      origin: _origin,
      docId: _docId,
      update: diff,
    );
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
    for (final u in _store.updatesAfter(
      origin: _origin,
      docId: _docId,
      after: pushedClock,
    ))
      (clock: u.clock, bytes: u.payload),
  ];

  @override
  ({int lastSyncedRid, int pushedClock}) cursor() {
    final c = _store.syncCursor(origin: _origin, docId: _docId);
    return (lastSyncedRid: c.lastSyncedRid, pushedClock: c.pushedClock);
  }

  @override
  void advance({int? lastSyncedRid, int? pushedClock}) {
    final c = _store.syncCursor(origin: _origin, docId: _docId);
    _store.setSyncCursor(
      origin: _origin,
      docId: _docId,
      cursor: SyncCursor(
        lastSyncedRid: lastSyncedRid ?? c.lastSyncedRid,
        pushedClock: pushedClock ?? c.pushedClock,
      ),
    );
  }

  @override
  void trimOutboxThrough(int pushedClock) => _store.trimUpdatesThrough(
    origin: _origin,
    docId: _docId,
    upToClock: pushedClock,
  );

  @override
  bool appendRemote(int rid, Uint8List update) => _store.appendRemoteUpdate(
    origin: _origin,
    docId: _docId,
    rid: rid,
    update: update,
  );

  @override
  bool appendRemoteBatch(List<({int rid, Uint8List update})> items) =>
      items.isEmpty ||
      _store.appendRemoteUpdates(
        origin: _origin,
        docId: _docId,
        rids: Int64List.fromList([for (final i in items) i.rid]),
        updates: [for (final i in items) i.update],
      );

  @override
  ({int local, int remote}) logSizes() {
    final s = _store.logSizes(origin: _origin, docId: _docId);
    return (local: s.$1, remote: s.$2);
  }

  @override
  void compact() => _store.squash(origin: _origin, docId: _docId);

  /// No-op: the wrapped [MicaStore] is shared and outlives any one session
  /// (opened once for the device, not per cloud doc). Only the web store holds
  /// per-session resources (a Web Lock + IDB handle) worth releasing.
  @override
  void dispose() {}
}
