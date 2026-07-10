// P2c: reconnect reconciliation converges two replicas without loss.
//
// Two devices each edit the SAME cloud doc while offline (different blocks), then
// reconnect one after the other through a fake server that actually FOLDS pushes
// (via the real yrs FFI) and relays them to the other replica. Proves: pull-then-
// push converges both sides, each side's offline edit survives, and — the P2c
// invariant — a remote update merged on reconnect does NOT enter the local outbox
// (doc_update stays purely the device's own un-pushed edits).
//
//   flutter test integration_test/cloud_sync_converge_test.dart -d windows
import 'dart:convert';
import 'dart:io';

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

  test('P2c: two replicas edit offline then converge on reconnect; log stays pure-local',
      () async {
    final dirA = Directory.systemTemp.createTempSync('mica_cvA');
    final dirB = Directory.systemTemp.createTempSync('mica_cvB');
    final storeA = MicaStore.open(path: '${dirA.path}/s.db')!;
    final storeB = MicaStore.open(path: '${dirB.path}/s.db')!;
    // Two DISTINCT device actors — a real two-device scenario. (Sharing a
    // client_id makes A's and B's concurrent edits collide at the same
    // (client_id, clock), so yrs skips each other's update as a duplicate.)
    final cidA = BigInt.from(0xA11CE);
    final cidB = BigInt.from(0xB0B);

    // ONE shared base — the SAME bytes for every replica. (Re-encoding the doc
    // per replica would mint different yrs item ids for the identical text, so a
    // diff from one replica would reference items the other doesn't have and
    // never merge — the replicas must descend from byte-identical state.)
    final base = MicaDocument.fromBlocksJson(
      rootId: 'r',
      blocksJson: jsonEncode([
        {'id': 'r', 'type': 'page', 'children': ['a', 'b']},
        {'id': 'a', 'type': 'paragraph', 'text': 'hi'},
        {'id': 'b', 'type': 'paragraph', 'text': 'yo'},
      ]),
    ).encodeState();

    // Both devices start from the same base, already synced to rid 1 (as after a
    // prior online session).
    StoreCloudDocStore(storeA, 'doc').save(base, 1);
    StoreCloudDocStore(storeB, 'doc').save(base, 1);

    // ── Offline phase: each device edits a DIFFERENT block, no network ──────────
    Future<void> editOffline(
        MicaStore store, BigInt cid, Map<String, dynamic> op) async {
      final s = CloudSyncSession(
        uri: Uri.parse('ws://127.0.0.1:1/nope'), // dead — seed comes from local
        clientId: cid,
        onReady: (_, _) {},
        onRemoteBlocks: (_) {},
        persistence: StoreCloudDocStore(store, 'doc'),
      );
      s.connect();
      await _until(() => s.isReady, reason: 'offline seed from local base');
      s.applyLocalOps([op]);
      s.dispose(); // flushes the edited base to the store (outbox already durable)
    }

    await editOffline(
        storeA, cidA, {'type': 'update_block', 'block_id': 'a', 'text': 'hi A'});
    await editOffline(
        storeB, cidB, {'type': 'update_block', 'block_id': 'b', 'text': 'yo B'});
    // Each device's outbox holds exactly its own one offline edit.
    expect(storeA.updatesAfter(docId: 'doc', after: 0).length, 1);
    expect(storeB.updatesAfter(docId: 'doc', after: 0).length, 1);

    // ── Reconnect phase: a server that folds pushes and relays to the other ─────
    final server =
        await _FakeRelayServer.start(MicaDocument.fromState(bytes: base)!);

    List<Map<String, dynamic>>? blocksA, blocksB;
    void capA(List<Map<String, dynamic>> b) => blocksA = b;
    void capB(List<Map<String, dynamic>> b) => blocksB = b;
    final sessionA = CloudSyncSession(
      uri: server.uri,
      clientId: cidA,
      onReady: (_, b) => capA(b),
      onRemoteBlocks: capA,
      persistence: StoreCloudDocStore(storeA, 'doc'),
    );
    final sessionB = CloudSyncSession(
      uri: server.uri,
      clientId: cidB,
      onReady: (_, b) => capB(b),
      onRemoteBlocks: capB,
      persistence: StoreCloudDocStore(storeB, 'doc'),
    );

    // A reconnects first and folds its edit into the server; then B reconnects,
    // pulls A's edit, merges, and pushes its own.
    sessionA.connect();
    await _until(() => server.folded >= 1, reason: 'A pushed + folded server-side');
    sessionB.connect();

    bool converged(List<Map<String, dynamic>>? blocks) {
      if (blocks == null) return false;
      String? textOf(String id) => blocks
          .cast<Map<String, dynamic>?>()
          .firstWhere((x) => x?['id'] == id, orElse: () => null)?['text'] as String?;
      return textOf('a') == 'hi A' && textOf('b') == 'yo B';
    }

    await _until(() => converged(blocksA),
        reason: 'A converges to both edits (its own + B via broadcast)');
    await _until(() => converged(blocksB),
        reason: 'B converges to both edits (its own + A via pull)');

    // Drain both outboxes (all acked) and assert the P2c invariant: a device
    // only ever appended (and pushed) its OWN single edit — the remote edit it
    // merged arrived as a sync.update and must NOT have entered the outbox. If
    // it had, it would have taken clock 2, been pushed, and acked: pushed_clock
    // would read 2. (Asserted via the clock rather than log length because P2e
    // compacts acked entries out of the log after the debounced write-through.)
    expect(await sessionA.drainOutbox(timeout: const Duration(seconds: 6)), isTrue);
    expect(await sessionB.drainOutbox(timeout: const Duration(seconds: 6)), isTrue);
    expect(storeA.syncCursor(docId: 'doc').pushedClock, 1,
        reason: "A pushed exactly its own edit — remote update never entered A's outbox");
    expect(storeB.syncCursor(docId: 'doc').pushedClock, 1,
        reason: "B pushed exactly its own edit — remote update never entered B's outbox");
    expect(storeA.updatesAfter(docId: 'doc', after: 1), isEmpty);
    expect(storeB.updatesAfter(docId: 'doc', after: 1), isEmpty);

    sessionA.dispose();
    sessionB.dispose();
    await server.stop();
    _bestEffortDelete(dirA);
    _bestEffortDelete(dirB);
  });
}

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

void _bestEffortDelete(Directory dir) {
  try {
    dir.deleteSync(recursive: true);
  } catch (_) {
    // ignore: locked db file on Windows
  }
}

/// A fake sync server that actually folds pushed yrs updates into an authoritative
/// document (real FFI) and relays each accepted update to the OTHER connected
/// replicas — enough to prove multi-device convergence without a real backend.
class _FakeRelayServer {
  _FakeRelayServer(this._server, this._doc);

  final HttpServer _server;
  final MicaDocument _doc; // authoritative folded state
  int _rid = 1; // base is rid 1; folded pushes get 2, 3, …
  final List<Map<String, dynamic>> _log = []; // {rid, actor_id, update}
  final Map<int, WebSocket> _sockets = {};
  int _nextConn = 0;

  /// Number of pushes folded into the authoritative doc.
  int get folded => _log.length;

  static Future<_FakeRelayServer> start(MicaDocument base) async {
    final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    final self = _FakeRelayServer(server, base);
    server.listen((req) async {
      if (WebSocketTransformer.isUpgradeRequest(req)) {
        final ws = await WebSocketTransformer.upgrade(req);
        final conn = self._nextConn++;
        self._sockets[conn] = ws;
        ws.listen(
          (data) => self._onMessage(conn, data),
          onDone: () => self._sockets.remove(conn),
          cancelOnError: false,
        );
      } else {
        req.response.statusCode = HttpStatus.badRequest;
        await req.response.close();
      }
    });
    return self;
  }

  Uri get uri => Uri.parse('ws://${_server.address.address}:${_server.port}/ws');

  void _send(WebSocket? s, Map<String, dynamic> m) {
    try {
      s?.add(jsonEncode(m));
    } catch (_) {
      // socket closing during teardown — drop the frame
    }
  }

  void _onMessage(int conn, dynamic data) {
    if (data is! String) return;
    final m = jsonDecode(data) as Map<String, dynamic>;
    final sock = _sockets[conn];
    switch (m['type']) {
      case 'sync.bootstrap':
        _send(sock, {
          'type': 'sync.base',
          'base': base64.encode(_doc.encodeState()),
          'base_rid': _rid,
        });
      case 'sync.pull':
        final since = (m['payload']?['since_rid'] as num?)?.toInt() ?? 0;
        final ups = [
          for (final e in _log)
            if ((e['rid'] as int) > since) e,
        ];
        _send(sock, {
          'type': 'sync.updates',
          'updates': ups,
          'head': ups.isEmpty ? _rid : ups.last['rid'],
        });
      case 'sync.push':
        final id = m['id'];
        final b64 = m['payload']?['update'];
        if (id is! String || b64 is! String) return;
        final ok = _doc.applyUpdate(update: base64.decode(b64));
        if (!ok) {
          _send(sock, {'type': 'error', 'ack_id': id, 'code': 'invalid'});
          return;
        }
        final rid = ++_rid;
        final entry = <String, dynamic>{
          'rid': rid,
          'actor_id': '00000000-0000-0000-0000-000000000000',
          'update': b64,
        };
        _log.add(entry);
        _send(sock, {'type': 'sync.ack', 'ack_id': id, 'rid': rid});
        // Fan out to the other replicas (not the sender — it gets only the ack).
        for (final e in _sockets.entries.toList()) {
          if (e.key != conn) _send(e.value, {'type': 'sync.update', ...entry});
        }
    }
  }

  Future<void> stop() async {
    // Copy — closing a socket fires its onDone, which mutates _sockets.
    for (final s in _sockets.values.toList()) {
      try {
        await s.close();
      } catch (_) {
        // ignore
      }
    }
    await _server.close(force: true);
  }
}
