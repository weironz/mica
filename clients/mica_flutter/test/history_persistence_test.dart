import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/cloud/cloud_sync_session.dart';
import 'package:mica_flutter/editor/operation_history.dart';
import 'package:mica_flutter/sync/pending_doc_update.dart';

/// The session and outbox buffer are real. Engine semantics are independently
/// exercised with yrs/yjs; this deterministic replica injects append failures
/// exactly after the engine has advanced its history.
class _Replica implements SyncDocReplica {
  String text = 'ab';
  final _undo = ['', 'a'];
  final _redo = <String>[];
  int serial = 0;
  int undoCalls = 0;
  int redoCalls = 0;
  int beginCalls = 0;
  bool tracking = false;
  @override
  String rootBlockId() => 'root';
  @override
  Uint8List stateVector() => Uint8List.fromList([serial]);
  @override
  Uint8List encodeState() => Uint8List.fromList([serial]);
  @override
  Uint8List encodeDiffSince(Uint8List sv) =>
      Uint8List.fromList(sv[0] == serial ? [0, 0] : [1, serial]);
  @override
  bool applyUpdate(Uint8List update) => true;
  @override
  Uint8List mergeUpdates(List<Uint8List> updates) => updates.last;
  @override
  List<Map<String, dynamic>> toBlocks() => [
    {
      'id': 'root',
      'type': 'page',
      'text': '',
      'children': ['a'],
      'data': <String, dynamic>{},
    },
    {
      'id': 'a',
      'type': 'paragraph',
      'text': text,
      'children': <String>[],
      'data': <String, dynamic>{},
    },
  ];
  @override
  void beginHistoryBatch() {
    tracking = true;
    beginCalls++;
  }

  @override
  void endHistoryBatch() {
    tracking = false;
  }

  @override
  void resetHistory() {
    _undo.clear();
    _redo.clear();
  }

  @override
  void applyEditorOp(DocOp op) {
    final next = op['text'] as String;
    if (next == text) return;
    if (tracking) {
      _undo.add(text);
      _redo.clear();
    }
    text = next;
    serial++;
  }

  @override
  bool undo() {
    undoCalls++;
    if (_undo.isEmpty) return false;
    _redo.add(text);
    text = _undo.removeLast();
    serial++;
    return true;
  }

  @override
  bool redo() {
    redoCalls++;
    if (_redo.isEmpty) return false;
    _undo.add(text);
    text = _redo.removeLast();
    serial++;
    return true;
  }

  @override
  bool canUndo() => _undo.isNotEmpty;
  @override
  bool canRedo() => _redo.isNotEmpty;
  @override
  (int, int) historyDepths() => (_undo.length, _redo.length);
}

class _Store implements CloudDocStore {
  bool fail = true;
  final updates = <Uint8List>[];
  @override
  int appendOutbox(Uint8List diff) {
    if (fail) throw StateError('disk full');
    updates.add(Uint8List.fromList(diff));
    return updates.length;
  }

  @override
  List<({int clock, Uint8List bytes})> outboxAfter(int clock) => [
    for (var i = clock; i < updates.length; i++)
      (clock: i + 1, bytes: updates[i]),
  ];
  @override
  ({int lastSyncedRid, int pushedClock}) cursor() =>
      (lastSyncedRid: 0, pushedClock: 0);
  @override
  ({Uint8List state, int cursor})? load() => null;
  @override
  void save(Uint8List state, int cursor) {}
  @override
  void advance({int? lastSyncedRid, int? pushedClock}) {}
  @override
  void trimOutboxThrough(int clock) {}
  @override
  bool appendRemote(int rid, Uint8List update) => true;
  @override
  bool appendRemoteBatch(List<({int rid, Uint8List update})> items) => true;
  @override
  ({int local, int remote}) logSizes() => (local: updates.length, remote: 0);
  @override
  void compact() {}
  @override
  void dispose() {}
}

void main() {
  test('retrying the same mirrored batch only confirms its retained diff', () {
    final store = _Store();
    final engine = _Replica();
    final session = CloudSyncSession(
      uri: () async => Uri.parse('ws://unused'),
      clientId: BigInt.one,
      persistence: store,
      onReady: (_, _) {},
      onRemoteBlocks: (_) {},
    );
    session.debugAttachReplica(engine);
    addTearDown(() {
      store.fail = false;
      session.dispose();
    });
    final batch = <DocOp>[
      {'type': 'update_block', 'block_id': 'a', 'text': 'abc'},
    ];
    expect(
      () => session.applyLocalOps(batch, trackHistory: true),
      throwsStateError,
    );
    expect(engine.beginCalls, 1);
    store.fail = false;
    session.applyLocalOps(batch, trackHistory: true);
    expect(engine.beginCalls, 1, reason: 'the engine must not replay a batch');
    expect(store.updates, hasLength(1));
    expect(engine.text, 'abc');
  });

  test(
    'failed undo append returns canonical state and preserves ordered updates',
    () async {
      final store = _Store();
      final engine = _Replica();
      final session = CloudSyncSession(
        uri: () async => Uri.parse('ws://unused'),
        clientId: BigInt.one,
        persistence: store,
        onReady: (_, _) {},
        onRemoteBlocks: (_) {},
      );
      session.debugAttachReplica(engine);
      addTearDown(() {
        store.fail = false;
        session.dispose();
      });

      final undo = session.applyHistory(HistoryAction.undo);
      expect(undo.persistenceError, isA<StateError>());
      expect(undo.nodes.single.text, 'a');
      expect(undo.undoDepth, 1);
      expect(undo.redoDepth, 1);
      expect(store.updates, isEmpty);
      expect(await session.drainOutbox(timeout: Duration.zero), isFalse);

      // A second command must not mutate the engine until its predecessor saves.
      final blocked = session.applyHistory(HistoryAction.redo);
      expect(blocked.persistenceError, isA<StateError>());
      expect(engine.redoCalls, 0);
      expect(blocked.nodes.single.text, 'a');
      store.fail = false;
      final redo = session.applyHistory(HistoryAction.redo);
      expect(redo.persistenceError, isNull);
      expect(redo.nodes.single.text, 'ab');
      expect(engine.undoCalls, 1);
      expect(engine.redoCalls, 1);
      expect(store.updates.map((u) => u[1]), [1, 2]);
    },
  );

  test(
    'normal op retries publish their predecessor before any new mutation',
    () {
      final store = _Store();
      final engine = _Replica();
      final session = CloudSyncSession(
        uri: () async => Uri.parse('ws://unused'),
        clientId: BigInt.one,
        persistence: store,
        onReady: (_, _) {},
        onRemoteBlocks: (_) {},
      );
      session.debugAttachReplica(engine);
      addTearDown(() {
        store.fail = false;
        session.dispose();
      });
      final op = <String, dynamic>{
        'type': 'update_block',
        'block_id': 'a',
        'text': 'abc',
      };
      expect(
        () => session.applyLocalOps([op], trackHistory: true),
        throwsStateError,
      );
      expect(engine.historyDepths(), (3, 0));
      expect(
        () => session.applyLocalOps([op], trackHistory: true),
        throwsStateError,
      );
      expect(
        engine.beginCalls,
        1,
        reason: 'failed predecessor blocks a second engine batch',
      );
      store.fail = false;
      session.applyLocalOps([op], trackHistory: true);
      expect(engine.historyDepths(), (3, 0));
      expect(store.updates.map((u) => u[1]), [1]);
      session.applyLocalOps([
        {'type': 'update_block', 'block_id': 'a', 'text': 'maintenance'},
      ]);
      expect(engine.historyDepths(), (
        3,
        0,
      ), reason: 'maintenance is untracked');
    },
  );

  testWidgets(
    'retained updates retry with a bounded budget and keep owned bytes',
    (tester) async {
      var attempts = 0;
      var fail = true;
      final written = <List<int>>[];
      final buffer = PendingDocUpdate(
        persist: (u) {
          attempts++;
          if (fail) throw StateError('disk');
          written.add(u.toList());
        },
      );
      addTearDown(buffer.dispose);
      final source = Uint8List.fromList([1, 7]);
      expect(() => buffer.publish(source), throwsStateError);
      source[1] = 99;
      await tester.pump(const Duration(seconds: 30));
      expect(attempts, 6, reason: 'initial append plus five retries');
      expect(buffer.hasPending, isTrue);
      fail = false;
      buffer.flush();
      expect(written, [
        [1, 7],
      ]);
      expect(buffer.hasPending, isFalse);
    },
  );
}
