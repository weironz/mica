// Regression for M-R (red line #1 / #2). Runs the real FFI CloudSyncSession
// against a fake in-process WS server, so no backend is needed.
//
//   flutter test integration_test/cloud_sync_integrity_test.dart -d windows
//
// Covers:
//  - B1: a remote update the replica can't apply must fault + self-heal, NOT
//    advance the cursor past it (which would strip updates on the next pull).
//  - C1: unacked local diffs restored at startup are re-pushed on connect, and
//    an ack (matched by the id the push carried) drains them + clears the
//    persisted queue. This also proves the id-tagged push/ack/drain path works.
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:mica_flutter/cloud/cloud_sync.dart';
import 'package:mica_flutter/cloud/store_cloud_doc_store.dart';
import 'package:mica_flutter/src/rust/api/document.dart';
import 'package:mica_flutter/src/rust/api/store.dart';
import 'package:mica_flutter/src/rust/frb_generated.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(() async => RustLib.init());
  tearDownAll(() async => RustLib.dispose());

  test('B1: a bad remote update faults + self-heals instead of silently skipping',
      () async {
    final server = await _FakeSyncServer.start(_buildBase());

    final faults = <String>[];
    var ready = false;
    final session = CloudSyncSession(
      uri: server.uri,
      clientId: BigInt.from(7),
      onReady: (_, _) => ready = true,
      onRemoteBlocks: (_) {},
      onFault: (reason, _) => faults.add(reason),
    );
    session.connect();

    await _until(() => ready, reason: 'cold bootstrap');
    expect(server.bootstrapCount, 1);

    server.pushGarbageUpdate(rid: 999);

    await _until(() => faults.isNotEmpty, reason: 'onFault fires');
    expect(faults.first, 'bad_remote_update');
    await _until(() => server.bootstrapCount >= 2,
        reason: 'session re-bootstraps to self-heal');

    session.dispose();
    await server.stop();
  });

  test('C1: recovered unacked diffs are re-pushed on connect, acked, and cleared',
      () async {
    final base = _buildBase();
    final diff = _buildDiff(base); // a valid yrs diff, as if it never got acked
    final server = await _FakeSyncServer.start(base);

    final persisted = <List<Uint8List>>[];
    var ready = false;
    final session = CloudSyncSession(
      uri: server.uri,
      clientId: BigInt.from(9),
      onReady: (_, _) => ready = true,
      onRemoteBlocks: (_) {},
      restoreUnacked: [diff],
      onPersistUnacked: persisted.add,
    );
    session.connect();

    await _until(() => ready, reason: 'cold bootstrap');
    // The recovered diff must reach the server (re-pushed after the base).
    await _until(() => server.pushed.isNotEmpty, reason: 'recovered diff pushed');
    expect(base64.decode(server.pushed.first.update), diff,
        reason: 'the exact recovered diff was sent');
    expect(server.pushed.first.id, isNotEmpty, reason: 'push carries an ack id');

    // Once acked (by the id it carried), the queue drains and persistence is
    // rewritten empty.
    final drained =
        await session.drainOutbox(timeout: const Duration(seconds: 5));
    expect(drained, isTrue);
    await _until(() => persisted.isNotEmpty && persisted.last.isEmpty,
        reason: 'persisted queue cleared after ack');

    session.dispose();
    await server.stop();
  });

  test('C1: a live edit is pushed and persisted while unacked (recoverable)',
      () async {
    // A server that records pushes but never acks — so the pending diff stays in
    // the queue and the debounced persist fires with it, exactly the state a
    // crash-then-restart must recover.
    final server = await _FakeSyncServer.start(_buildBase(), ackPushes: false);

    final persisted = <List<Uint8List>>[];
    var ready = false;
    final session = CloudSyncSession(
      uri: server.uri,
      clientId: BigInt.from(11),
      onReady: (_, _) => ready = true,
      onRemoteBlocks: (_) {},
      onPersistUnacked: persisted.add,
    );
    session.connect();
    await _until(() => ready, reason: 'cold bootstrap');

    // The real user path: an editor op → local yrs diff → enqueued + pushed.
    session.applyLocalOps([
      {'type': 'update_block', 'block_id': 'a', 'text': 'hi there'},
    ]);

    await _until(() => server.pushed.isNotEmpty, reason: 'live edit pushed');
    expect(server.pushed.first.id, isNotEmpty, reason: 'push carries an ack id');
    // Never acked → the pending diff is persisted, so a crash now recovers it.
    await _until(() => persisted.isNotEmpty && persisted.last.isNotEmpty,
        reason: 'unacked diff persisted for crash recovery');
    expect(persisted.last.length, 1);
    expect(base64.encode(persisted.last.first), server.pushed.first.update,
        reason: 'the persisted diff is exactly the one pushed');

    session.dispose();
    await server.stop();
  });

  test('B2: catch-up keeps pulling until the update stream drains', () async {
    final base = _buildBase();
    final diff = _buildDiff(base); // inserts "!!" into block "a" → "hi!!"
    final server = await _FakeSyncServer.start(base);
    server.queuePullUpdate(rid: 2, update: diff);

    var applied = false;
    var ready = false;
    final session = CloudSyncSession(
      uri: server.uri,
      clientId: BigInt.from(13),
      onReady: (_, _) => ready = true,
      onRemoteBlocks: (blocks) {
        final a = blocks.firstWhere((b) => b['id'] == 'a',
            orElse: () => const <String, dynamic>{});
        if (a['text'] == 'hi!!') applied = true;
      },
    );
    session.connect();

    await _until(() => ready, reason: 'cold bootstrap');
    await _until(() => applied, reason: 'queued update applied');
    // The non-empty batch may have been truncated, so the client must pull again
    // (one pull after the base + at least one follow-up) rather than assuming it
    // caught up in a single round.
    await _until(() => server.pullCount >= 2, reason: 're-pulls until drained');

    session.dispose();
    await server.stop();
  });

  test('reconnect: a dropped socket auto-reconnects on its own', () async {
    final server = await _FakeSyncServer.start(_buildBase());

    var ready = false;
    final session = CloudSyncSession(
      uri: server.uri,
      clientId: BigInt.from(21),
      onReady: (_, _) => ready = true,
      onRemoteBlocks: (_) {},
    );
    session.connect();

    await _until(() => ready, reason: 'first bootstrap');
    expect(server.connectionCount, 1);

    // The server drops the socket — the client must reconnect by itself (backoff),
    // not stay dead until the doc is reopened.
    await server.dropCurrentSocket();
    await _until(() => server.connectionCount >= 2,
        reason: 'client auto-reconnects after a drop',
        timeout: const Duration(seconds: 8));

    session.dispose();
    await server.stop();
  });

  // ── P2b: cloud outbox on the on-device append-log (desktop persistence) ──────

  test('P2b: a cloud edit lands in the append-log outbox; ack advances pushed_clock',
      () async {
    final dir = Directory.systemTemp.createTempSync('mica_p2b');
    final store = MicaStore.open(path: '${dir.path}/s.db')!;
    final adapter = StoreCloudDocStore(store, 'doc-p2b');
    final server = await _FakeSyncServer.start(_buildBase());

    var ready = false;
    final session = CloudSyncSession(
      uri: server.uri,
      clientId: store.clientId(),
      onReady: (_, _) => ready = true,
      onRemoteBlocks: (_) {},
      persistence: adapter, // desktop → append-log outbox, not the prefs queue
    );
    session.connect();
    await _until(() => ready, reason: 'cold bootstrap');

    // Real user path: editor op → local yrs diff → appended to the durable outbox.
    session.applyLocalOps([
      {'type': 'update_block', 'block_id': 'a', 'text': 'hi there'},
    ]);

    await _until(() => server.pushed.isNotEmpty, reason: 'edit pushed');
    expect(server.pushed.first.id, '1', reason: 'push id is the outbox clock');
    // Durably in the append-log (survives a crash before ack).
    expect(store.updatesAfter(docId: 'doc-p2b', after: 0).length, 1);

    // Acked (server acks by id) → pushed_clock advances → outbox drains.
    final drained = await session.drainOutbox(timeout: const Duration(seconds: 5));
    expect(drained, isTrue);
    expect(store.syncCursor(docId: 'doc-p2b').pushedClock, 1,
        reason: 'ack advanced pushed_clock');
    expect(store.updatesAfter(docId: 'doc-p2b', after: 1), isEmpty,
        reason: 'nothing left in the outbox');
    // The acked entry stays in the log until P2e trims it — the outbox query
    // (updates_after(pushed_clock)) already excludes it.
    expect(store.updatesAfter(docId: 'doc-p2b', after: 0).length, 1);

    session.dispose();
    await server.stop();
    _bestEffortDelete(dir);
  });

  test('P2b: an unacked cloud edit survives a session restart and re-pushes',
      () async {
    final dir = Directory.systemTemp.createTempSync('mica_p2b2');
    final store = MicaStore.open(path: '${dir.path}/s.db')!;

    // Session 1 against a server that records but never acks — the edit stays
    // un-pushed in the durable outbox, exactly the crash-then-restart state.
    final server1 = await _FakeSyncServer.start(_buildBase(), ackPushes: false);
    var ready1 = false;
    final s1 = CloudSyncSession(
      uri: server1.uri,
      clientId: store.clientId(),
      onReady: (_, _) => ready1 = true,
      onRemoteBlocks: (_) {},
      persistence: StoreCloudDocStore(store, 'doc-r'),
    );
    s1.connect();
    await _until(() => ready1, reason: 'bootstrap 1');
    s1.applyLocalOps([
      {'type': 'update_block', 'block_id': 'a', 'text': 'edited offline'},
    ]);
    await _until(() => server1.pushed.isNotEmpty, reason: 'edit pushed (unacked)');
    expect(store.updatesAfter(docId: 'doc-r', after: 0).length, 1,
        reason: 'edit durable in the outbox despite no ack');
    s1.dispose();
    await server1.stop();

    // "Restart": a fresh session + adapter over the SAME store; a fresh server
    // that acks. The recovered outbox entry must be re-pushed and then drain.
    final server2 = await _FakeSyncServer.start(_buildBase());
    var ready2 = false;
    final s2 = CloudSyncSession(
      uri: server2.uri,
      clientId: store.clientId(),
      onReady: (_, _) => ready2 = true,
      onRemoteBlocks: (_) {},
      persistence: StoreCloudDocStore(store, 'doc-r'),
    );
    s2.connect();
    await _until(() => ready2, reason: 'bootstrap 2 (seeds from the local mirror)');
    await _until(() => server2.pushed.isNotEmpty, reason: 're-pushed after restart');
    expect(server2.pushed.first.id, '1', reason: 'same outbox clock re-sent');

    final drained = await s2.drainOutbox(timeout: const Duration(seconds: 5));
    expect(drained, isTrue);
    expect(store.syncCursor(docId: 'doc-r').pushedClock, 1);

    s2.dispose();
    await server2.stop();
    _bestEffortDelete(dir);
  });
}

void _bestEffortDelete(Directory dir) {
  try {
    dir.deleteSync(recursive: true);
  } catch (_) {
    // ignore: locked db file on Windows
  }
}

/// A minimal valid yrs base (folded document state) built through the FFI.
Uint8List _buildBase() {
  final doc = MicaDocument.fromBlocksJson(
    rootId: 'r',
    blocksJson: jsonEncode([
      {
        'id': 'r',
        'type': 'page',
        'children': ['a'],
      },
      {'id': 'a', 'type': 'paragraph', 'text': 'hi'},
    ]),
  );
  return doc.encodeState();
}

/// A valid yrs diff over [base] — the kind of unacked edit C1 must recover.
Uint8List _buildDiff(Uint8List base) {
  final doc =
      MicaDocument.fromStateWithClientId(bytes: base, clientId: BigInt.from(9))!;
  final sv = doc.stateVector();
  doc.textInsert(id: 'a', at: 2, text: '!!');
  return doc.encodeDiffSince(stateVector: sv);
}

/// Waits (polling) until [cond] holds, failing the test on [timeout].
Future<void> _until(
  bool Function() cond, {
  String? reason,
  Duration timeout = const Duration(seconds: 10),
}) async {
  final deadline = DateTime.now().add(timeout);
  while (!cond()) {
    if (DateTime.now().isAfter(deadline)) {
      fail('timed out waiting${reason == null ? '' : ': $reason'}');
    }
    await Future<void>.delayed(const Duration(milliseconds: 20));
  }
}

typedef _Push = ({String id, String update});

/// Speaks just enough of the WS sync protocol to bootstrap a client, record its
/// pushes, ack them by id, and shove a crafted update at it.
class _FakeSyncServer {
  _FakeSyncServer(this._server, this._base, this.ackPushes);

  final HttpServer _server;
  final Uint8List _base;
  final bool ackPushes;
  WebSocket? _socket;
  int connectionCount = 0;
  int bootstrapCount = 0;
  int pullCount = 0;
  final List<_Push> pushed = [];
  final List<Map<String, dynamic>> pullQueue = [];
  int _rid = 1;

  /// Hand a single update out on the next pull, then nothing — models a capped
  /// batch the client must keep pulling past (B2).
  void queuePullUpdate({required int rid, required Uint8List update}) {
    pullQueue.add({
      'rid': rid,
      'actor_id': '00000000-0000-0000-0000-000000000000',
      'update': base64.encode(update),
    });
  }

  static Future<_FakeSyncServer> start(Uint8List base,
      {bool ackPushes = true}) async {
    final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    final self = _FakeSyncServer(server, base, ackPushes);
    server.listen((req) async {
      if (WebSocketTransformer.isUpgradeRequest(req)) {
        final ws = await WebSocketTransformer.upgrade(req);
        self._socket = ws;
        self.connectionCount++;
        ws.listen(self._onMessage);
      } else {
        req.response.statusCode = HttpStatus.badRequest;
        await req.response.close();
      }
    });
    return self;
  }

  Uri get uri =>
      Uri.parse('ws://${_server.address.address}:${_server.port}/ws');

  void _onMessage(dynamic data) {
    if (data is! String) return;
    final m = jsonDecode(data) as Map<String, dynamic>;
    switch (m['type']) {
      case 'sync.bootstrap':
        bootstrapCount++;
        _send({
          'type': 'sync.base',
          'base': base64.encode(_base),
          'base_rid': 1,
        });
      case 'sync.pull':
        pullCount++;
        final ups = List<Map<String, dynamic>>.from(pullQueue);
        pullQueue.clear();
        _send({
          'type': 'sync.updates',
          'updates': ups,
          'head': ups.isEmpty ? 1 : ups.last['rid'],
        });
      case 'sync.push':
        final id = m['id'];
        final update = m['payload']?['update'];
        if (id is String && update is String) {
          pushed.add((id: id, update: update));
          if (ackPushes) {
            _send({'type': 'sync.ack', 'ack_id': id, 'rid': ++_rid});
          }
        }
    }
  }

  void pushGarbageUpdate({required int rid}) {
    _send({
      'type': 'sync.update',
      'rid': rid,
      'update': base64.encode(const [9, 9, 9, 9]),
    });
  }

  void _send(Map<String, dynamic> m) => _socket?.add(jsonEncode(m));

  /// Close the current socket without stopping the server — models a transient
  /// drop the client must reconnect through.
  Future<void> dropCurrentSocket() async {
    await _socket?.close();
    _socket = null;
  }

  Future<void> stop() async {
    await _socket?.close();
    await _server.close(force: true);
  }
}
