// A corrupt on-device cloud cache must not kill `connect()` before the socket
// even opens (issue #162). The desktop FFI store reports a present-but-
// unreadable snapshot by THROWING (StoreError::CorruptDoc, message template
// "corrupt local CRDT state for …"), distinct from absent (null) and from a
// genuine storage fault (I/O, lock contention). Only the corrupt case is
// recoverable by cold-bootstrap — the session must fall through to a normal
// server bootstrap (socket opens, uri is built) rather than leaving the doc
// permanently unready. A genuine storage fault must still propagate, not be
// silently swallowed as "no cache".
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/cloud/cloud_doc_store.dart';
import 'package:mica_flutter/cloud/cloud_sync_session.dart';

/// The FFI throw shape the session must recognise as "corrupt" — the exact
/// Rust `#[error]` template (crates/mica-core/src/store.rs, StoreError::CorruptDoc).
String corruptMessage(String docId) =>
    'corrupt local CRDT state for $docId (checksum or yrs integration failed)';

class _Store implements CloudDocStore {
  _Store(this.error);
  final Object error;

  @override
  ({Uint8List state, int cursor})? load() => throw error;

  @override
  void save(Uint8List state, int cursor) {}

  @override
  int appendOutbox(Uint8List diff) => 0;

  @override
  List<({int clock, Uint8List bytes})> outboxAfter(int pushedClock) => const [];

  @override
  ({int lastSyncedRid, int pushedClock}) cursor() =>
      (lastSyncedRid: 0, pushedClock: 0);

  @override
  void advance({int? lastSyncedRid, int? pushedClock}) {}

  @override
  void trimOutboxThrough(int pushedClock) {}

  @override
  bool appendRemote(int rid, Uint8List update) => true;

  @override
  bool appendRemoteBatch(List<({int rid, Uint8List update})> items) => true;

  @override
  ({int local, int remote}) logSizes() => (local: 0, remote: 0);

  @override
  void compact() {}

  @override
  void dispose() {}
}

class _NullStore extends _Store {
  _NullStore() : super(StateError('unused'));
  @override
  ({Uint8List state, int cursor})? load() => null;
}

CloudSyncSession _session(CloudDocStore store, void Function() onUri) {
  return CloudSyncSession(
    uri: () async {
      onUri();
      // A port nothing is listening on — the connect failure is handled as
      // offline (state, not a crash); the uri callback count is the observable.
      return Uri.parse('ws://127.0.0.1:1/ws');
    },
    clientId: BigInt.from(7),
    onReady: (_, __) {},
    onRemoteBlocks: (_) {},
    persistence: store,
    documentId: 'doc-1',
  );
}

void main() {
  test(
    'a corrupt local cache falls through to cold bootstrap (socket opens)',
    () async {
      final store = _Store(corruptMessage('doc-1'));
      var uriCalls = 0;
      final s = _session(store, () => uriCalls++);
      addTearDown(s.dispose);

      // Must NOT throw: corrupt local copy → treat as never-mirrored, keep going.
      s.connect();

      await Future<void>.delayed(const Duration(milliseconds: 60));
      expect(
        uriCalls,
        1,
        reason: 'connect() must reach the network after swallowing corruption',
      );
      expect(s.isReady, isFalse, reason: 'cold bootstrap is still pending');
    },
  );

  test('a never-mirrored cache still connects (no regression)', () async {
    final store = _NullStore();
    var uriCalls = 0;
    final s = _session(store, () => uriCalls++);
    addTearDown(s.dispose);

    s.connect();
    await Future<void>.delayed(const Duration(milliseconds: 60));
    expect(uriCalls, 1);
  });

  test('a genuine storage fault is NOT swallowed as corruption', () async {
    final store = _Store(StateError('database is locked'));
    var uriCalls = 0;
    final s = _session(store, () => uriCalls++);
    addTearDown(s.dispose);

    expect(() => s.connect(), throwsA(isA<StateError>()));
    await Future<void>.delayed(const Duration(milliseconds: 20));
    expect(
      uriCalls,
      0,
      reason:
          'only the corrupt-cache shape is recoverable; other FFI errors '
          'must surface, not be laundered into a silent cold-bootstrap',
    );
  });
}
