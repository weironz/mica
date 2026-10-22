import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/editor/model.dart';

/// A controller that captures the ops emitted to the "backend" so tests can
/// assert the undo/redo diff is correct. Ops flow through an internal Future
/// chain, so call [drain] before inspecting [batches].
class _Harness {
  final List<List<Map<String, dynamic>>> batches = [];
  late final EditorController c;

  _Harness(List<EditorNode> initial) {
    c = EditorController(
      rootBlockId: 'root',
      onOps: (ops) async {
        batches.add(ops);
      },
    );
    c.load(initial);
  }

  Future<void> drain() => pumpEventQueue();

  List<Map<String, dynamic>> get lastBatch => batches.last;
}

EditorNode _p(String id, String text) =>
    EditorNode(id: id, kind: 'paragraph', text: text);

void main() {
  test('undo/redo a text edit restores text and caret', () async {
    final h = _Harness([_p('a', 'hello')]);
    h.c.selection = DocSelection.collapsed(const DocPosition(0, 5));

    h.c.setFocusedText('hello world', 11, 11);
    h.c.flushPending(); // commit the burst (records history)
    await h.drain();
    expect(h.c.nodes[0].text, 'hello world');
    expect(h.c.canUndo, isTrue);

    h.c.undo();
    await h.drain();
    expect(h.c.nodes[0].text, 'hello');
    expect(h.c.canRedo, isTrue);
    // The diff to walk back is a single update_block on the same node.
    expect(h.lastBatch.length, 1);
    expect(h.lastBatch.first['type'], 'update_block');
    expect(h.lastBatch.first['block_id'], 'a');
    expect(h.lastBatch.first['text'], 'hello');

    h.c.redo();
    await h.drain();
    expect(h.c.nodes[0].text, 'hello world');
  });

  test('undo a split deletes the created block; redo re-inserts it', () async {
    final h = _Harness([_p('a', 'abcd')]);
    h.c.selection = DocSelection.collapsed(const DocPosition(0, 2));

    h.c.splitAtCaret();
    await h.drain();
    expect(h.c.nodes.length, 2);
    expect(h.c.nodes[0].text, 'ab');
    expect(h.c.nodes[1].text, 'cd');
    final createdId = h.c.nodes[1].id;

    h.c.undo();
    await h.drain();
    expect(h.c.nodes.length, 1);
    expect(h.c.nodes[0].text, 'abcd');
    // Diff back: delete the created block + restore the first block's text.
    expect(
      h.lastBatch.any(
        (o) => o['type'] == 'delete_block' && o['block_id'] == createdId,
      ),
      isTrue,
    );

    h.c.redo();
    await h.drain();
    expect(h.c.nodes.length, 2);
    expect(h.c.nodes[1].text, 'cd');
    // Redo re-inserts the block at its original index, keeping the same id.
    expect(h.c.nodes[1].id, createdId);
    expect(
      h.lastBatch.any(
        (o) => o['type'] == 'insert_block' && o['block']['id'] == createdId,
      ),
      isTrue,
    );
  });

  test('undo a merge re-creates the removed block', () async {
    final h = _Harness([_p('a', 'foo'), _p('b', 'bar')]);
    h.c.selection = DocSelection.collapsed(const DocPosition(1, 0));

    expect(h.c.mergeBackward(), isTrue);
    await h.drain();
    expect(h.c.nodes.length, 1);
    expect(h.c.nodes[0].text, 'foobar');

    h.c.undo();
    await h.drain();
    expect(h.c.nodes.length, 2);
    expect(h.c.nodes[0].text, 'foo');
    expect(h.c.nodes[1].text, 'bar');
    expect(h.c.nodes[1].id, 'b');
  });

  test('undo a mark toggle clears the mark', () async {
    final h = _Harness([_p('a', 'hello')]);
    h.c.selection = DocSelection(
      anchor: const DocPosition(0, 0),
      focus: const DocPosition(0, 5),
    );

    h.c.toggleMark('bold');
    await h.drain();
    expect(marksFromData(h.c.nodes[0].data).single.type, 'bold');

    h.c.undo();
    await h.drain();
    expect(marksFromData(h.c.nodes[0].data), isEmpty);

    h.c.redo();
    await h.drain();
    expect(marksFromData(h.c.nodes[0].data).single.type, 'bold');
  });

  test('a new edit after undo clears the redo stack', () async {
    final h = _Harness([_p('a', 'x')]);
    h.c.selection = DocSelection.collapsed(const DocPosition(0, 1));

    h.c.setFocusedText('xy', 2, 2);
    h.c.flushPending();
    await h.drain();
    h.c.undo();
    await h.drain();
    expect(h.c.canRedo, isTrue);

    h.c.setFocusedText('xz', 2, 2);
    h.c.flushPending();
    await h.drain();
    expect(h.c.canRedo, isFalse);
  });

  test('undo coalesces an uncommitted typing burst', () async {
    final h = _Harness([_p('a', '')]);
    h.c.selection = DocSelection.collapsed(const DocPosition(0, 0));

    // Type several chars without an intervening flush (debounce still pending).
    h.c.setFocusedText('h', 1, 1);
    h.c.setFocusedText('hi', 2, 2);
    expect(h.c.canUndo, isTrue); // dirty counts as undoable

    h.c.undo(); // flushes the burst, then steps back over it
    await h.drain();
    expect(h.c.nodes[0].text, '');
  });
  // ── 2026-10-21 review P1-4: undo while collaborating ──────────────────────
  //
  // History holds whole-document snapshots. Restoring one verbatim used to roll
  // back what a collaborator wrote since — and DELETE blocks they added, sent
  // to everyone. Undo now leaves every block someone else touched as it is.

  test('undo leaves a collaborator\'s edits and new blocks alone', () async {
    final h = _Harness([_p('a', 'mine'), _p('b', 'theirs')]);
    h.c.selection = const DocSelection.collapsed(DocPosition(0, 4));
    h.c.setFocusedText('mine!', 5, 5);
    h.c.flushPending();
    await h.drain();

    // A collaborator rewrites b and adds c; our own edit echoes back unchanged.
    h.c.reconcile([
      _p('a', 'mine!'),
      _p('b', 'theirs, edited'),
      _p('c', 'new'),
    ]);
    h.batches.clear();

    h.c.undo();
    await h.drain();
    expect(
      [for (final n in h.c.nodes) n.text],
      ['mine', 'theirs, edited', 'new'],
    );
    final sent = [for (final b in h.batches) ...b];
    expect(
      sent.where((o) => o['type'] == 'delete_block'),
      isEmpty,
      reason: 'undo deleted a block a collaborator added',
    );
    expect(
      sent.where((o) => o['block_id'] == 'b'),
      isEmpty,
      reason: 'undo rolled back a collaborator\'s edit',
    );
    expect(sent.single['block_id'], 'a', reason: 'only our own edit is undone');

    // And redo brings ours back without touching theirs.
    h.batches.clear();
    h.c.redo();
    await h.drain();
    expect(
      [for (final n in h.c.nodes) n.text],
      ['mine!', 'theirs, edited', 'new'],
    );
  });

  test('a block a collaborator deleted stays deleted on undo', () async {
    final h = _Harness([_p('a', 'x'), _p('b', 'gone soon')]);
    h.c.selection = const DocSelection.collapsed(DocPosition(0, 1));
    h.c.setFocusedText('xy', 2, 2);
    h.c.flushPending();
    await h.drain();
    h.c.reconcile([_p('a', 'xy')]); // b deleted remotely
    h.c.undo();
    await h.drain();
    expect([for (final n in h.c.nodes) n.id], ['a']);
    expect(h.c.nodes.single.text, 'x');
  });

  test('alone, undo behaves exactly as before', () async {
    final h = _Harness([_p('a', 'one')]);
    h.c.selection = const DocSelection.collapsed(DocPosition(0, 3));
    h.c.setFocusedText('one two', 7, 7);
    h.c.flushPending();
    await h.drain();
    h.c.reconcile([_p('a', 'one two')]); // our own echo: nothing foreign
    h.c.undo();
    await h.drain();
    expect(h.c.nodes.single.text, 'one');
  });

  test('typing between undo and redo is kept, not overwritten (C8)', () async {
    final h = _Harness([_p('a', 'one')]);
    h.c.selection = const DocSelection.collapsed(DocPosition(0, 3));
    h.c.setFocusedText('one two', 7, 7);
    h.c.flushPending();
    await h.drain();
    h.c.undo();
    await h.drain();
    expect(h.c.nodes.single.text, 'one');

    // Type within the debounce window, then redo before it fires.
    h.c.selection = const DocSelection.collapsed(DocPosition(0, 3));
    h.c.setFocusedText('one three', 9, 9);
    h.c.redo();
    await h.drain();
    expect(h.c.nodes.single.text, 'one three',
        reason: 'redo overwrote text typed after the undo');
    // And the new typing is in history: undo takes it back.
    h.c.undo();
    await h.drain();
    expect(h.c.nodes.single.text, 'one');
  });
}
