// A new page opens on an EMPTY document. The editor seeds one paragraph to
// type in, but that insert waits for the sync session; meanwhile the session's
// first snapshot (still empty) reconciles the editor. Dropping the seed there
// made the empty-document check seed a second paragraph, and both inserts
// landed: an empty first line, and the caret on line two.
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/model.dart';

void main() {
  test('an empty snapshot before the seed lands keeps the one seed', () async {
    final sent = <Map<String, dynamic>>[];
    final c = EditorController(
      rootBlockId: 'root',
      onOps: (ops) async => sent.addAll(ops),
    );
    c.load(const []);

    final seed = c.ensureNotEmpty();
    c.reconcile(const []); // the session's first snapshot: the insert isn't in yet
    expect(c.nodes.map((n) => n.id), [seed.id]);
    expect(identical(c.ensureNotEmpty(), c.nodes.first), isTrue);

    await Future<void>.delayed(Duration.zero);
    final inserts = sent.where((op) => op['type'] == 'insert_block').toList();
    expect(inserts, hasLength(1), reason: 'exactly one seed paragraph is sent');
  });

  test('text typed into the seed before sync is ready survives', () {
    final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
    c.load(const []);
    c.ensureNotEmpty();
    c.setFocusedText('hello', 5, 5);
    c.reconcile(const []);
    expect(c.nodes.single.text, 'hello');
  });

  test('once the server has the seed, an empty snapshot is a real deletion', () {
    final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
    c.load(const []);
    final seed = c.ensureNotEmpty();
    c.reconcile([EditorNode(id: seed.id, kind: 'paragraph', text: '')]);
    c.reconcile(const []); // e.g. another client cleared the page
    expect(c.nodes, isEmpty);
  });
}
