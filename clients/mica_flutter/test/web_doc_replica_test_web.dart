// The web replica is the same Rust document core desktop drives over FFI, so
// these check the WIRING (wasm loads, ops cross, history and merges behave),
// not the edit rules themselves — those are mica-core's tests, and run once.
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/cloud/sync_doc_replica.dart';
import 'package:mica_flutter/cloud/sync_replica_web.dart';
import 'package:mica_flutter/web/mica_doc_wasm.dart';

import 'support/doc_engine_web.dart';

Uint8List _base() {
  final doc = MicaDocWasm.fromBlocks('r', [
    {'id': 'r', 'type': 'page', 'text': '', 'data': <String, dynamic>{}, 'children': ['p']},
    {'id': 'p', 'type': 'paragraph', 'text': 'hello', 'data': <String, dynamic>{}, 'children': <String>[]},
  ])!;
  try {
    return doc.encodeState();
  } finally {
    doc.free();
  }
}

SyncDocReplica _replica([Uint8List? state]) =>
    replicaFromState(state ?? _base(), BigInt.zero)!;

Map<String, dynamic> _block(SyncDocReplica r, String id) =>
    r.toBlocks().firstWhere((b) => b['id'] == id);

void main_() {
  setUpAll(loadBrowserDocEngine);

  test('the engine reports a source fingerprint', () {
    expect(docEngineInfo()!['source_fingerprint'], matches(RegExp(r'^[0-9a-f]{16}$')));
  });

  test('editor ops replay, and a text-only straggler keeps its marks', () {
    final r = _replica();
    r.applyEditorOp({
      'type': 'update_block',
      'block_id': 'p',
      'text': 'hello world',
      'data': {
        'marks': [
          {'start': 0, 'end': 5, 'type': 'bold'},
        ],
      },
    });
    // The editor's debounced straggler carries text only; the shared mirror
    // supplies the marks from its cache.
    r.applyEditorOp({'type': 'update_block', 'block_id': 'p', 'text': 'hello world!'});
    final p = _block(r, 'p');
    expect(p['text'], 'hello world!');
    expect((p['data'] as Map)['marks'], [
      {'start': 0, 'end': 5, 'type': 'bold'},
    ]);

    r.applyEditorOp({
      'type': 'insert_block',
      'parent_id': 'r',
      'index': 1,
      'block': {'id': 'q', 'type': 'heading', 'text': 'T', 'data': {'level': 2}, 'children': <String>[]},
    });
    r.applyEditorOp({'type': 'move_block', 'block_id': 'q', 'parent_id': 'r', 'index': 0});
    expect(_block(r, 'r')['children'], ['q', 'p']);
    expect((_block(r, 'q')['data'] as Map)['level'], 2);
    r.applyEditorOp({'type': 'delete_block', 'block_id': 'q'});
    expect(_block(r, 'r')['children'], ['p']);
  });

  test('a history batch undoes and redoes as one step', () {
    final r = _replica();
    r.beginHistoryBatch();
    r.applyEditorOp({'type': 'update_block', 'block_id': 'p', 'text': 'hello1'});
    r.applyEditorOp({'type': 'update_block', 'block_id': 'p', 'text': 'hello12'});
    r.endHistoryBatch();
    expect(r.canUndo(), isTrue);
    expect(r.historyDepths(), (1, 0));
    expect(r.undo(), isTrue);
    expect(_block(r, 'p')['text'], 'hello');
    expect(r.redo(), isTrue);
    expect(_block(r, 'p')['text'], 'hello12');
  });

  test('two replicas converge through state-vector diffs', () {
    final base = _base();
    final a = _replica(base);
    final b = _replica(base);
    a.applyEditorOp({'type': 'update_block', 'block_id': 'p', 'text': 'A hello'});
    b.applyEditorOp({'type': 'update_block', 'block_id': 'p', 'text': 'hello B'});
    final toB = a.encodeDiffSince(b.stateVector());
    final toA = b.encodeDiffSince(a.stateVector());
    expect(a.applyUpdate(toA), isTrue);
    expect(b.applyUpdate(toB), isTrue);
    // Two actors (random per replica, never the 0 the web passes in) editing
    // disjoint ends of one paragraph keep both edits.
    expect(_block(a, 'p')['text'], 'A hello B');
    expect(_block(b, 'p')['text'], 'A hello B');
  });

  test('merged updates apply like the originals, and carry no meta.root', () {
    final base = _base();
    final a = _replica(base);
    final updates = <Uint8List>[];
    for (final t in ['h1', 'h12', 'h123']) {
      final sv = a.stateVector();
      a.applyEditorOp({'type': 'update_block', 'block_id': 'p', 'text': t});
      updates.add(a.encodeDiffSince(sv));
    }
    final merged = a.mergeUpdates(updates);
    final b = _replica(base);
    expect(b.applyUpdate(merged), isTrue);
    expect(_block(b, 'p')['text'], 'h123');
    expect(a.mergeUpdates(const []), [0, 0], reason: 'an empty merge is the empty update');
  });

  test('undecodable bytes are no replica, and a bad update is refused', () {
    expect(replicaFromState(Uint8List.fromList([9, 9, 9, 9]), BigInt.zero), isNull);
    final r = _replica();
    expect(r.applyUpdate(Uint8List.fromList([9, 9, 9, 9])), isFalse);
    expect(_block(r, 'p')['text'], 'hello', reason: 'the replica is still usable');
  });

  // Wasm memory never shrinks, so what matters is that a closed document's
  // memory is reused by the next one. Sessions free their replica on dispose
  // (SyncDocReplica.dispose) instead of waiting for the GC's finalizer.
  test('opening and disposing replicas stops growing wasm memory', () {
    final ids = [for (var i = 0; i < 2000; i++) 'b$i'];
    final seed = MicaDocWasm.fromBlocks('r', [
      {'id': 'r', 'type': 'page', 'children': ids},
      for (final id in ids) {'id': id, 'type': 'paragraph', 'text': 'Paragraph $id'},
    ])!;
    final state = seed.encodeState();
    seed.free();
    double afterRounds(int rounds) {
      for (var i = 0; i < rounds; i++) {
        final r = replicaFromState(state, BigInt.zero)!;
        r.toBlocks();
        r.dispose();
      }
      return MicaDocWasm.memoryBytes();
    }

    final warm = afterRounds(5);
    expect(warm, greaterThan(0), reason: 'the probe reads real memory');
    expect(afterRounds(30), warm, reason: '30 more open/close cycles reuse the same memory');
  });
}
