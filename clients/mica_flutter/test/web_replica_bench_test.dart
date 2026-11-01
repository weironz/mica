// Timing of the web CRDT replica through the sync seam, as the session uses
// it. Browser-only, and not a gate: it asserts nothing about time, the numbers
// go to the log (`BENCH …`). Run:
//   flutter test --platform chrome test/web_replica_bench_test.dart
//
// 2026-11-01, Chromium, flutter test (debug Dart), 2000-block document — the
// same benchmark on the last yjs build (ff1c3c14) vs this engine, in ms:
//   open 115.9 -> 29.4 · full read 24.4 -> 12.4 · keystroke (op + diff)
//   0.20 -> 0.20 · remote update (merge + full read) 21.2 -> 11.4 ·
//   undo of a batch 23.3 -> 10.5
// Same day, the session switched to patching its block list from the change
// log: remote update (merge + changed blocks) 0.10 vs (merge + full read) 9.0.
@TestOn('browser')
library;

import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/cloud/sync_doc_replica.dart';
import 'package:mica_flutter/cloud/sync_replica_web.dart';
import 'package:mica_flutter/web/mica_doc_wasm.dart';

import 'support/doc_engine_web.dart';

double _ms(Stopwatch s) => s.elapsedMicroseconds / 1000;

double _median(List<double> xs) {
  xs.sort();
  return xs[xs.length ~/ 2];
}

/// 2000 paragraphs, each with a bold and a link mark — about 800 KB of state.
Uint8List _document() {
  final ids = [for (var i = 0; i < 2000; i++) 'b$i'];
  final doc = MicaDocWasm.fromBlocks('r', [
    {'id': 'r', 'type': 'page', 'text': '', 'data': <String, dynamic>{}, 'children': ids},
    for (var i = 0; i < ids.length; i++)
      {
        'id': ids[i],
        'type': 'paragraph',
        'text': 'Paragraph $i: the quick brown fox jumps over the lazy dog, 中文段落内容测试。',
        'data': {
          'marks': [
            {'start': 0, 'end': 9, 'type': 'bold'},
            {'start': 14, 'end': 19, 'type': 'link', 'href': 'https://example.com'},
          ],
        },
        'children': <String>[],
      },
  ])!;
  try {
    return doc.encodeState();
  } finally {
    doc.free();
  }
}

void main() {
  late Uint8List base;
  const label = 'wasm';
  setUpAll(() async {
    await loadBrowserDocEngine();
    base = _document();
  });

  test('$label: 2000-block document', () {
    final out = <String, String>{};
    final open = <double>[];
    late SyncDocReplica a;
    for (var i = 0; i < 3; i++) {
      final s = Stopwatch()..start();
      a = replicaFromState(base, BigInt.zero)!;
      open.add(_ms(s));
    }
    out['open (decode + seed)'] = _median(open).toStringAsFixed(1);

    final read = <double>[];
    for (var i = 0; i < 5; i++) {
      final s = Stopwatch()..start();
      a.toBlocks();
      read.add(_ms(s));
    }
    out['toBlocks (full read)'] = _median(read).toStringAsFixed(1);

    // A keystroke as the session handles it: replay the op, encode the push.
    var text = 'Paragraph 1000: ';
    final keys = <double>[];
    for (var i = 0; i < 200; i++) {
      text += 'x';
      final sv = a.stateVector();
      final s = Stopwatch()..start();
      a.applyEditorOp({
        'type': 'update_block',
        'block_id': 'b1000',
        'text': text,
        'data': {
          'marks': [
            {'start': 0, 'end': 9, 'type': 'bold'},
          ],
        },
      });
      a.encodeDiffSince(sv);
      keys.add(_ms(s));
    }
    out['keystroke (op + diff)'] = _median(keys).toStringAsFixed(2);

    // A remote update as the session handles it: merge, then full read.
    final b = replicaFromState(base, BigInt.zero)!;
    final remote = <double>[];
    for (var i = 0; i < 30; i++) {
      final sv = b.stateVector();
      b.applyEditorOp({'type': 'update_block', 'block_id': 'b5', 'text': 'remote $i'});
      final diff = b.encodeDiffSince(sv);
      final s = Stopwatch()..start();
      a.applyUpdate(diff);
      a.toBlocks();
      remote.add(_ms(s));
    }
    out['remote update (merge + read)'] = _median(remote).toStringAsFixed(1);

    // The same update as the session reads it since 2026-11-01: merge, then
    // only the blocks the change log names (a full read only on a shape change).
    a.takeChangedBlockIds();
    final patched = <double>[];
    var fullReads = 0;
    for (var i = 0; i < 30; i++) {
      final sv = b.stateVector();
      b.applyEditorOp({'type': 'update_block', 'block_id': 'b5', 'text': 'patched $i'});
      final diff = b.encodeDiffSince(sv);
      final s = Stopwatch()..start();
      a.applyUpdate(diff);
      final ids = a.takeChangedBlockIds();
      if (ids == null) {
        fullReads++;
        a.toBlocks();
      } else {
        a.blocksById(ids);
      }
      patched.add(_ms(s));
    }
    out['remote update (merge + changed blocks)'] =
        '${_median(patched).toStringAsFixed(2)} (full reads: $fullReads)';

    // Undo of a 10-keystroke batch (adapter reseeds its marks cache).
    final undo = <double>[];
    for (var r = 0; r < 5; r++) {
      a.beginHistoryBatch();
      for (var i = 0; i < 10; i++) {
        a.applyEditorOp({'type': 'update_block', 'block_id': 'b10', 'text': 'u$r$i'});
      }
      a.endHistoryBatch();
      final s = Stopwatch()..start();
      a.undo();
      undo.add(_ms(s));
    }
    out['undo (batch)'] = _median(undo).toStringAsFixed(1);
    out['state bytes'] = '${a.encodeState().length}';
    // ignore: avoid_print
    print('BENCH $label ${jsonEncode(out)}');
  });
}
