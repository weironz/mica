import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/layout_caret.dart';
import 'package:mica_flutter/editor/model.dart';

void main() {
  test('an editor without focus has no layout caret', () {
    expect(layoutCaret(null), (node: -1, offset: 0));
  });

  test('a collapsed selection preserves the caret node and offset', () {
    expect(layoutCaret(const DocSelection.collapsed(DocPosition(4, 120))), (
      node: 4,
      offset: 120,
    ));
  });

  test('forward and backward ranges do not auto-scroll a code caret', () {
    for (final positions in [
      (const DocPosition(1, 3), const DocPosition(2, 8)),
      (const DocPosition(2, 8), const DocPosition(1, 3)),
    ]) {
      expect(
        layoutCaret(DocSelection(anchor: positions.$1, focus: positions.$2)),
        (node: -1, offset: 0),
      );
    }
  });
}
