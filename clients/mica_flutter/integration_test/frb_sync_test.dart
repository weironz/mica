// P2-M4.5: the yrs sync primitives work through the FFI — two MicaDocuments
// diverge then converge by exchanging state-vector diffs, and apply rejects
// garbage. This is the CRDT engine the cloud sync session (push/pull) rides on,
// proven from Dart end of the bridge.
//
//   flutter test integration_test/frb_sync_test.dart -d windows
import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:mica_flutter/src/rust/api/document.dart';
import 'package:mica_flutter/src/rust/frb_generated.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(() async => RustLib.init());
  tearDownAll(() async => RustLib.dispose());

  List<Map<String, dynamic>> blocks(MicaDocument d) =>
      (jsonDecode(d.toBlocksJson()) as List).cast<Map<String, dynamic>>();
  String textOf(MicaDocument d, String id) =>
      blocks(d).firstWhere((b) => b['id'] == id)['text'] as String;

  test('two FFI docs converge via state-vector diff exchange', () {
    final base = MicaDocument.fromBlocksJson(
      rootId: 'r',
      blocksJson: jsonEncode([
        {'id': 'r', 'type': 'page', 'children': ['a']},
        {'id': 'a', 'type': 'paragraph', 'text': 'Hello'},
      ]),
    );
    final state = base.encodeState();

    // Two replicas of the same base, each pinned to a distinct device actor.
    final a = MicaDocument.fromStateWithClientId(bytes: state, clientId: BigInt.from(10))!;
    final b = MicaDocument.fromStateWithClientId(bytes: state, clientId: BigInt.from(20))!;

    // Capture each replica's state vector, make concurrent edits, encode the diffs.
    final svA = a.stateVector();
    final svB = b.stateVector();
    a.textInsert(id: 'a', at: 5, text: ' from A');
    b.insertBlockJson(
      parentId: 'r',
      index: 1,
      blockJson: jsonEncode({'id': 'b', 'type': 'paragraph', 'text': 'from B'}),
    );
    final diffA = a.encodeDiffSince(stateVector: svA);
    final diffB = b.encodeDiffSince(stateVector: svB);

    // Exchange + apply the peer's diff.
    expect(a.applyUpdate(update: diffB), isTrue);
    expect(b.applyUpdate(update: diffA), isTrue);

    // Both replicas converge to the same document with both edits.
    expect(blocks(a), blocks(b), reason: 'replicas converge');
    expect(textOf(a, 'a'), 'Hello from A');
    expect(
      blocks(a).any((x) => x['id'] == 'b' && x['text'] == 'from B'),
      isTrue,
    );
    expect((blocks(a).firstWhere((x) => x['id'] == 'r')['children'] as List), ['a', 'b']);
  });

  test('applyUpdate rejects garbage bytes', () {
    final d = MicaDocument.fromBlocksJson(
      rootId: 'r',
      blocksJson: jsonEncode([
        {'id': 'r', 'type': 'page', 'children': <String>[]},
      ]),
    );
    expect(d.applyUpdate(update: const [9, 9, 9, 9]), isFalse);
  });
}
