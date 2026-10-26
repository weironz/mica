// Healthy-cache seed path (issue #162 follow-up): a persisted base must make
// the session seed its REAL replica — `onReady` fires with the doc's actual
// blocks, and the seeded replica is usable for editing. A corrupt-cache fix
// that forgets `_doc = doc` would mark the session ready while `childBlocks()`
// returns nothing and `applyLocalOps` throws (found by code review).
//
// Uses the REAL FFI engine (desktop `MicaDocument`) so `replicaFromState`
// runs its true decode — no debug-attach bypass of the production path.
@TestOn('vm')
library;

import 'dart:convert';
import 'dart:typed_data';
import 'dart:async';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/cloud/cloud_doc_store.dart';
import 'package:mica_flutter/cloud/cloud_sync_session.dart';
import 'package:mica_flutter/src/rust/api/document.dart';
import 'package:mica_flutter/src/rust/frb_generated.dart';

class _LoadedStore implements CloudDocStore {
  _LoadedStore(this._state, this._cursor);
  final Uint8List _state;
  final int _cursor;

  @override
  ({Uint8List state, int cursor})? load() => (state: _state, cursor: _cursor);

  @override
  void save(Uint8List state, int cursor) {}
  @override
  int appendOutbox(Uint8List diff) => 1;
  @override
  List<({int clock, Uint8List bytes})> outboxAfter(int pushedClock) => const [];
  @override
  ({int lastSyncedRid, int pushedClock}) cursor() =>
      (lastSyncedRid: _cursor, pushedClock: 0);
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

class _CorruptStore extends _LoadedStore {
  _CorruptStore() : super(Uint8List(0), 0);
  @override
  ({Uint8List state, int cursor})? load() =>
      throw 'corrupt local CRDT state for doc-1 (checksum or yrs integration failed)';
}

void main() {
  setUpAll(() async => RustLib.init());
  tearDownAll(() async => RustLib.dispose());

  test(
    'a persisted base seeds the replica: ready with real blocks, editable',
    () async {
      final doc = MicaDocument.fromBlocksJson(
        rootId: 'r',
        blocksJson: jsonEncode([
          {
            'id': 'r',
            'type': 'page',
            'children': ['a'],
          },
          {'id': 'a', 'type': 'paragraph', 'text': 'Seed text'},
        ]),
      );
      final state = doc.encodeState();

      String? readyRoot;
      List<Map<String, dynamic>> readyBlocks = const [];
      var uriCalls = 0;
      final s = CloudSyncSession(
        uri: () async {
          uriCalls++;
          return Uri.parse(
            'ws://127.0.0.1:1/ws',
          ); // offline is a state, not a crash
        },
        clientId: BigInt.from(7),
        onReady: (root, blocks) {
          readyRoot = root;
          readyBlocks = blocks;
        },
        onRemoteBlocks: (_) {},
        persistence: _LoadedStore(state, 3),
        documentId: 'doc-1',
      );
      addTearDown(s.dispose);

      s.connect();

      expect(readyRoot, 'r', reason: 'the seeded replica reports its root');
      expect(
        readyBlocks.where((b) => b['text'] == 'Seed text'),
        isNotEmpty,
        reason: 'onReady must carry the persisted blocks, not an empty shell',
      );
      // The seeded replica is live: a local edit must apply (and enqueue durably).
      expect(
        () => s.applyLocalOps([
          {
            'type': 'insert_block',
            'parent_id': 'r',
            'index': 1,
            'block': {
              'id': 'b2',
              'type': 'paragraph',
              'text': 'offline edit',
              'data': <String, dynamic>{},
              'children': <String>[],
            },
          },
        ]),
        returnsNormally,
      );
      expect(s.childBlocks().any((b) => b['text'] == 'offline edit'), isTrue);

      await Future<void>.delayed(const Duration(milliseconds: 60));
      expect(uriCalls, 1, reason: 'seeding must not block the socket attempt');
    },
  );

  test(
    'corrupt cache opens the socket and becomes ready from the server base',
    () async {
      final base = MicaDocument.fromBlocksJson(
        rootId: 'r',
        blocksJson: jsonEncode([
          {
            'id': 'r',
            'type': 'page',
            'children': ['a'],
          },
          {'id': 'a', 'type': 'paragraph', 'text': 'Cloud recovery'},
        ]),
      ).encodeState();
      final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
      final connected = Completer<void>();
      WebSocket? socket;
      server.listen((request) async {
        socket = await WebSocketTransformer.upgrade(request);
        connected.complete();
        socket!.listen((raw) {
          final frame = jsonDecode(raw as String) as Map;
          if (frame['type'] == 'sync.bootstrap') {
            socket!.add(
              jsonEncode({
                'type': 'sync.base',
                'base': base64.encode(base),
                'base_rid': 7,
              }),
            );
          }
        });
      });
      addTearDown(() async {
        await socket?.close();
        await server.close(force: true);
      });
      var uriCalls = 0;
      final session = CloudSyncSession(
        uri: () async {
          uriCalls++;
          return Uri.parse('ws://127.0.0.1:${server.port}/ws');
        },
        clientId: BigInt.from(9),
        persistence: _CorruptStore(),
        onReady: (_, __) {},
        onRemoteBlocks: (_) {},
      );
      addTearDown(session.dispose);
      session.connect();
      await connected.future.timeout(const Duration(seconds: 5));
      await session.ready.timeout(const Duration(seconds: 5));
      expect(uriCalls, 1);
      expect(session.childBlocks().single['text'], 'Cloud recovery');
      expect(session.isReady, isTrue);
    },
  );
}
