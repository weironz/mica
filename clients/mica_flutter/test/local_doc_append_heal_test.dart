// Review 2026-10-21 C6: a local edit whose log append FAILED was dropped.
//
// `MicaStore.appendUpdate` returns 0 when the store errored (busy / disk full /
// io) — the FFI swallows the error. `LocalDocBackend.applyOps` ignored the
// return, so the edit lived only in memory; the next `compactLocal` rebuilds
// the base FROM DISK, and the edit was gone without a word. The cloud store
// checks the same 0 (`store_cloud_doc_store.dart`); the local one must too.
//
// Fakes, not the native library: what is under test is the backend's reaction
// to a failed write, and a real SQLite store cannot be made to fail on cue.
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/local/local_doc.dart';
import 'package:mica_flutter/src/rust/api/document.dart';
import 'package:mica_flutter/src/rust/api/store.dart';
import 'package:mica_flutter/swallowed.dart';

class _FakeDoc implements MicaDocument {
  @override
  String toBlocksJson() =>
      '[{"id":"root","type":"page","text":"","data":{},"children":[]}]';

  @override
  String rootBlockId() => 'root';

  @override
  Uint8List stateVector() => Uint8List(0);

  /// Every batch produces a non-empty diff — the edit is real.
  @override
  Uint8List encodeDiffSince({required List<int> stateVector}) =>
      Uint8List.fromList([1, 2, 3]);

  @override
  dynamic noSuchMethod(Invocation invocation) => null;
}

class _FakeStore implements MicaStore {
  _FakeStore(this.doc);

  final MicaDocument doc;
  int appendResult = 1;
  final List<MicaDocument> saved = [];

  @override
  MicaDocument? loadDoc({required String origin, required String docId}) => doc;

  @override
  int appendUpdate({
    required String origin,
    required String docId,
    required List<int> update,
  }) => appendResult;

  @override
  void saveDoc({
    required String origin,
    required String docId,
    required MicaDocument doc,
  }) => saved.add(doc);

  @override
  bool compactLocal({required String origin, required String docId}) => true;

  @override
  void checkpointDoc({required String origin, required String docId}) {}

  @override
  (int, int) logSizes({required String origin, required String docId}) => (0, 0);

  @override
  dynamic noSuchMethod(Invocation invocation) => null;
}

void main() {
  setUp(resetSwallowed);

  test('a failed append writes the live doc as the base instead', () async {
    final doc = _FakeDoc();
    final store = _FakeStore(doc);
    final backend = LocalDocBackend.open(store, 'local', 'd');

    store.appendResult = 0; // the store errored
    await backend.applyOps(const []);

    expect(store.saved, [same(doc)], reason: 'the edit must reach disk');
    expect(swallowedCounts()['local_append_failed'], 1);
  });

  test('a successful append does not pay for a full save', () async {
    final store = _FakeStore(_FakeDoc());
    final backend = LocalDocBackend.open(store, 'local', 'd');

    await backend.applyOps(const []);

    expect(store.saved, isEmpty);
    expect(swallowedCounts(), isEmpty);
  });
}
