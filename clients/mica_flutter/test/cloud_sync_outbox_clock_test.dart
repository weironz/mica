// Issue #159: `_sentThroughClock` must reflect only the CONTIGUOUS sent prefix
// of the durable outbox. A capped merge window sends a run through clock T and
// returns with T+1..N still unsent; a live edit arriving before the run's ack
// used to jump `_sentThroughClock` straight to N+1, so the ack handler's
// `floor = max(pushed, _sentThroughClock)` skipped T+1..N until reconnect.
// Red probe: fake WS server WITHHOLDS the merged run's ack, a live edit is
// enqueued, then the run is acked — clocks 9 and 10 must still go out and the
// outbox must drain.
import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/cloud/cloud_doc_store.dart';
import 'package:mica_flutter/cloud/cloud_sync_session.dart';
import 'package:mica_flutter/cloud/sync_doc_replica.dart';

/// A deterministic replica (same shape as history_persistence_test): it never
/// touches the real yrs engine, so `mergeUpdates` "succeeds" and the send-side
/// pacing/ack bookkeeping is what's under test.
class _Replica implements SyncDocReplica {
  @override
  String rootBlockId() => 'root';
  @override
  Uint8List stateVector() => Uint8List.fromList([0]);
  @override
  Uint8List encodeState() => Uint8List.fromList([0]);
  @override
  Uint8List encodeDiffSince(Uint8List sv) => Uint8List.fromList([1]);
  @override
  bool applyUpdate(Uint8List update) => true;
  @override
  Uint8List mergeUpdates(List<Uint8List> updates) => Uint8List.fromList([2]);
  @override
  List<Map<String, dynamic>> toBlocks() => [
    {
      'id': 'root',
      'type': 'page',
      'text': '',
      'children': <String>[],
      'data': <String, dynamic>{},
    },
  ];
  @override
  void beginHistoryBatch() {}
  @override
  void endHistoryBatch() {}
  @override
  void resetHistory() {}
  @override
  void applyEditorOp(DocOp op) {}
  @override
  bool undo() => true;
  @override
  bool redo() => true;
  @override
  bool canUndo() => false;
  @override
  bool canRedo() => false;
  @override
  (int, int) historyDepths() => (0, 0);
}

/// Durable outbox fake: pre-seeded rows + monotonic append, ackable cursor.
class _Store implements CloudDocStore {
  _Store(List<({int clock, Uint8List bytes})> outbox)
    : _outbox = List.of(outbox);
  final List<({int clock, Uint8List bytes})> _outbox;
  int _pushedClock = 0;
  int _lastSyncedRid = 0;

  @override
  ({Uint8List state, int cursor})? load() => null;
  @override
  void save(Uint8List state, int cursor) {}
  @override
  int appendOutbox(Uint8List diff) {
    final maxClock = _outbox.isEmpty ? 0 : _outbox.last.clock;
    final clock = (maxClock > _pushedClock ? maxClock : _pushedClock) + 1;
    _outbox.add((clock: clock, bytes: diff));
    return clock;
  }

  @override
  List<({int clock, Uint8List bytes})> outboxAfter(int pushedClock) => [
    for (final e in _outbox)
      if (e.clock > pushedClock) e,
  ];
  @override
  ({int lastSyncedRid, int pushedClock}) cursor() =>
      (lastSyncedRid: _lastSyncedRid, pushedClock: _pushedClock);
  @override
  void advance({int? lastSyncedRid, int? pushedClock}) {
    if (lastSyncedRid != null) _lastSyncedRid = lastSyncedRid;
    if (pushedClock != null) _pushedClock = pushedClock;
  }

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

/// Loopback WS server speaking just enough sync protocol: bootstrap reply, and
/// ack-on-sight of every push EXCEPT ids in [withhold] (which are only acked
/// when the test calls [ack]).
class _Server {
  _Server(this._server);
  final HttpServer _server;
  WebSocket? _socket;
  int bootstraps = 0;
  final List<String> pushed = [];
  final Set<String> withhold = {};
  final Set<String> acked = {};
  int _rid = 0;

  static Future<_Server> start() async {
    final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    final self = _Server(server);
    server.listen((req) async {
      if (WebSocketTransformer.isUpgradeRequest(req)) {
        final ws = await WebSocketTransformer.upgrade(req);
        self._socket = ws;
        ws.listen(self._onMessage);
      } else {
        req.response.statusCode = HttpStatus.badRequest;
        await req.response.close();
      }
    });
    return self;
  }

  Uri get uri => Uri.parse('ws://127.0.0.1:${_server.port}/ws');

  void _onMessage(dynamic data) {
    if (data is! String) return;
    final m = jsonDecode(data) as Map<String, dynamic>;
    switch (m['type']) {
      case 'sync.bootstrap':
        bootstraps++;
        _send({'type': 'sync.base', 'base': '', 'base_rid': 0});
      case 'sync.push':
        final id = m['id'];
        if (id is String) {
          pushed.add(id);
          if (!withhold.contains(id)) ack(id);
        }
    }
  }

  void ack(String id) {
    if (!acked.add(id)) return;
    _send({'type': 'sync.ack', 'ack_id': id, 'rid': ++_rid});
  }

  void _send(Map<String, dynamic> m) {
    try {
      _socket?.add(jsonEncode(m));
    } catch (_) {}
  }

  Future<void> stop() async {
    await _socket?.close();
    await _server.close(force: true);
  }
}

Future<void> _until(bool Function() cond, {String? reason}) async {
  final deadline = DateTime.now().add(const Duration(seconds: 10));
  while (!cond()) {
    if (DateTime.now().isAfter(deadline)) {
      fail('timed out waiting${reason == null ? '' : ': $reason'}');
    }
    await Future<void>.delayed(const Duration(milliseconds: 20));
  }
}

/// 10 offline edits of ~512 KiB each: ≥8 entries and >4 MiB total, so the
/// reconnect flush merges the first capped run (clocks 1..8).
List<({int clock, Uint8List bytes})> bigBacklog() => [
  for (var i = 0; i < 10; i++) (clock: i + 1, bytes: Uint8List(512 * 1024)),
];

void main() {
  test(
    'live edit during a capped merge window does not skip the unsent run',
    () async {
      final server = await _Server.start();
      addTearDown(server.stop);
      server.withhold.add('8'); // merged run ack withheld until the edit lands

      final store = _Store(bigBacklog());
      final s = CloudSyncSession(
        uri: () async => server.uri,
        clientId: BigInt.from(1),
        onReady: (_, __) {},
        onRemoteBlocks: (_) {},
        persistence: store,
        documentId: 'doc-1',
      );
      s.debugAttachReplica(_Replica());
      addTearDown(s.dispose);

      s.connect();

      // Reconnect flush merges clocks 1..8 into one push and sends it.
      await _until(
        () => server.pushed.contains('8'),
        reason: 'merged run sent',
      );

      // A live edit before the run is acked: enqueued + sent directly.
      s.applyLocalOps(const [
        {'text': 'x'},
      ]);
      await _until(
        () => server.pushed.contains('11'),
        reason: 'live edit sent',
      );

      // The run is acked; the unsent 9/10 must NOT be skipped over.
      server.ack('8');
      await _until(
        () => server.pushed.contains('9') && server.pushed.contains('10'),
        reason: 'clocks 9 and 10 resend after the run ack',
      );

      expect(
        await s.drainOutbox(timeout: const Duration(seconds: 3)),
        isTrue,
        reason: 'the whole outbox drains',
      );
    },
  );

  test('a REJECTED merged run (de-merge path) still drains 9/10', () async {
    final server = await _Server.start();
    addTearDown(server.stop);
    server.withhold.add('8');

    final store = _Store(bigBacklog());
    final s = CloudSyncSession(
      uri: () async => server.uri,
      clientId: BigInt.from(1),
      onReady: (_, __) {},
      onRemoteBlocks: (_) {},
      persistence: store,
      documentId: 'doc-1',
    );
    s.debugAttachReplica(_Replica());
    addTearDown(s.dispose);

    s.connect();
    await _until(() => server.pushed.contains('8'));
    s.applyLocalOps(const [
      {'text': 'x'},
    ]);
    await _until(() => server.pushed.contains('11'));

    // Reject the merged run instead of acking it → de-merge rewind + retry.
    // From here on the retry's individual resends must be acked.
    server.withhold.remove('8');
    server._send({'type': 'error', 'ack_id': '8', 'code': 'internal'});
    await _until(
      () => server.pushed.contains('9') && server.pushed.contains('10'),
      reason: 'de-merge retry re-sends the run one entry at a time',
    );
    expect(await s.drainOutbox(timeout: const Duration(seconds: 3)), isTrue);
  });

  test('a small backlog still pushes one-at-a-time and drains', () async {
    final server = await _Server.start();
    addTearDown(server.stop);

    final store = _Store([
      for (var i = 0; i < 3; i++) (clock: i + 1, bytes: Uint8List(100)),
    ]);
    final s = CloudSyncSession(
      uri: () async => server.uri,
      clientId: BigInt.from(1),
      onReady: (_, __) {},
      onRemoteBlocks: (_) {},
      persistence: store,
      documentId: 'doc-1',
    );
    s.debugAttachReplica(_Replica());
    addTearDown(s.dispose);

    s.connect();
    await _until(() => server.pushed.contains('3'));
    expect(await s.drainOutbox(timeout: const Duration(seconds: 3)), isTrue);
    expect(
      server.pushed.where((id) => id == '8'),
      isEmpty,
      reason: 'a 3-entry backlog is never merged',
    );
  });
}
