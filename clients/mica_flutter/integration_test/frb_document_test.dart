// P2-M1: the yrs document model is callable from Dart through frb.
//
// Drives MicaDocument (the opaque handle over crates/mica-core) on the Windows
// runner: build from blocks JSON, apply edit ops, read back, and persist via
// encode/decode. This is the bridge the editor (P2-M3) will sit on.
//
//   flutter test integration_test/frb_document_test.dart -d windows
import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:mica_flutter/src/rust/api/document.dart';
import 'package:mica_flutter/src/rust/frb_generated.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(() async => RustLib.init());
  tearDownAll(() async => RustLib.dispose());

  List<Map<String, dynamic>> blocksOf(MicaDocument d) =>
      (jsonDecode(d.toBlocksJson()) as List).cast<Map<String, dynamic>>();
  Map<String, dynamic> block(List<Map<String, dynamic>> bs, String id) =>
      bs.firstWhere((b) => b['id'] == id);

  String docJson(List<Map<String, dynamic>> b) => jsonEncode(b);

  test('build, edit, and read back through FFI', () {
    final doc = MicaDocument.fromBlocksJson(
      rootId: 'r',
      blocksJson: docJson([
        {'id': 'r', 'type': 'page', 'children': ['a']},
        {'id': 'a', 'type': 'paragraph', 'text': 'Hello'},
      ]),
    );
    expect(doc.rootBlockId(), 'r');

    doc.textInsert(id: 'a', at: 5, text: ' world');
    doc.insertBlockJson(
      parentId: 'r',
      index: 1,
      blockJson: jsonEncode({'id': 'b', 'type': 'paragraph', 'text': 'second'}),
    );

    final bs = blocksOf(doc);
    expect(block(bs, 'a')['text'], 'Hello world');
    expect((block(bs, 'r')['children'] as List), ['a', 'b']);
    expect(block(bs, 'b')['text'], 'second');
  });

  test('split + format through FFI', () {
    final doc = MicaDocument.fromBlocksJson(
      rootId: 'r',
      blocksJson: docJson([
        {'id': 'r', 'type': 'page', 'children': ['a']},
        {'id': 'a', 'type': 'paragraph', 'text': 'HelloWorld'},
      ]),
    );
    doc.splitBlock(id: 'a', at: 5, newId: 'n', newKind: 'paragraph');
    doc.textFormat(id: 'a', start: 0, end: 5, ty: 'bold', href: null, title: null);

    final bs = blocksOf(doc);
    expect((block(bs, 'r')['children'] as List), ['a', 'n']);
    expect(block(bs, 'a')['text'], 'Hello');
    expect(block(bs, 'n')['text'], 'World');
    expect((block(bs, 'a')['data']['marks'] as List).first['type'], 'bold');
  });

  test('encode_state -> from_state persists the doc', () {
    final doc = MicaDocument.fromBlocksJson(
      rootId: 'r',
      blocksJson: docJson([
        {'id': 'r', 'type': 'page', 'children': ['a']},
        {'id': 'a', 'type': 'paragraph', 'text': 'Persisted'},
      ]),
    );
    final bytes = doc.encodeState();
    final restored = MicaDocument.fromState(bytes: bytes);
    expect(restored, isNotNull);
    final bs = (jsonDecode(restored!.toBlocksJson()) as List);
    expect(bs.any((b) => (b as Map)['text'] == 'Persisted'), isTrue);
  });
}
