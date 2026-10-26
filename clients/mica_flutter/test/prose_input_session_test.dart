import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/prose_input.dart';

EditorController _doc() {
  final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
  c.load([
    EditorNode(id: 'a', kind: 'paragraph', text: 'hello'),
    EditorNode(id: 'b', kind: 'paragraph', text: 'ab'),
  ]);
  return c;
}

void main() {
  test('backward cross-block IME commit does not duplicate its preedit', () {
    final c = _doc();
    // Backward: focus (earlier block) is the START of the selection.
    c.selection = const DocSelection(
      anchor: DocPosition(1, 2),
      focus: DocPosition(0, 2),
    );
    final s = ProseInputSession();
    expect(
      s.apply(c, const TextEditingValue(
        text: 'he你llo',
        selection: TextSelection.collapsed(offset: 3),
        composing: TextRange(start: 2, end: 3),
      )),
      isTrue,
    );
    expect(
      s.apply(c, const TextEditingValue(
        text: 'he你llo',
        selection: TextSelection.collapsed(offset: 3),
      )),
      isTrue,
    );
    expect(c.nodes.length, 1);
    expect(c.nodes.single.text, 'he你');
  });

  test('reset drops a capture so a later session never diffs stale text', () {
    final c = _doc();
    c.selection = const DocSelection(
      anchor: DocPosition(0, 0),
      focus: DocPosition(1, 2),
    );
    final s = ProseInputSession();
    s.apply(c, const TextEditingValue(
      text: '你hello',
      selection: TextSelection.collapsed(offset: 1),
      composing: TextRange(start: 0, end: 1),
    ));
    s.reset();
    expect(
      s.apply(c, const TextEditingValue(
        text: 'hello',
        selection: TextSelection.collapsed(offset: 5),
      )),
      isFalse,
    );
  });
}