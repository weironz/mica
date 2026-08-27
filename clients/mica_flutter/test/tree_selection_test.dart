import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/ui/tree_selection.dart';

/// The one rule in sidebar multi-select that can be wrong rather than ugly.
///
/// A Shift range in a TREE has no obvious meaning across parents — which is why
/// AppFlowy has no sidebar multi-select at all, AFFiNE does it in a flat list,
/// and Windows Explorer allows it only in its flat right pane. The rule tested
/// here does not answer the ambiguous question, it refuses it: within one parent
/// a range is exact; across parents the gesture degrades to a toggle.
void main() {
  // A workspace root holding: a, F (expanded, with f1/f2 inside), b, c.
  //
  //   a          parent=null
  //   F          parent=null
  //     f1       parent=F
  //     f2       parent=F
  //   b          parent=null
  //   c          parent=null
  const tree = <SelectableRow>[
    (id: 'a', parentId: null),
    (id: 'F', parentId: null),
    (id: 'f1', parentId: 'F'),
    (id: 'f2', parentId: 'F'),
    (id: 'b', parentId: null),
    (id: 'c', parentId: null),
  ];

  group('shiftRangeSelection', () {
    test('a run of siblings is taken whole', () {
      expect(
        shiftRangeSelection(visibleRows: tree, anchorId: 'b', targetId: 'c'),
        ['b', 'c'],
      );
    });

    test('dragging the range upward gives the same set', () {
      // The anchor can sit below the target; a range is not directional.
      expect(
        shiftRangeSelection(visibleRows: tree, anchorId: 'c', targetId: 'b'),
        ['b', 'c'],
      );
    });

    test('an expanded folder inside the range is not a reason to refuse', () {
      // a → b spans F, which is open, so f1/f2 sit between them on screen.
      // "Select these siblings" must still work; the children are NOT added,
      // because selecting F already carries its subtree wherever this selection
      // gets used.
      expect(
        shiftRangeSelection(visibleRows: tree, anchorId: 'a', targetId: 'b'),
        ['a', 'F', 'b'],
      );
    });

    test('across parents it refuses, so the caller can toggle instead', () {
      // The ambiguity the whole rule exists to avoid: from a root row to a row
      // inside a folder, nobody can say whether the folders in between — or
      // their collapsed children — belong to the range.
      expect(
        shiftRangeSelection(visibleRows: tree, anchorId: 'a', targetId: 'f1'),
        isNull,
      );
      expect(
        shiftRangeSelection(visibleRows: tree, anchorId: 'f2', targetId: 'c'),
        isNull,
      );
    });

    test('a range inside one folder works like any other', () {
      expect(
        shiftRangeSelection(visibleRows: tree, anchorId: 'f1', targetId: 'f2'),
        ['f1', 'f2'],
      );
    });

    test('anchor and target on the same row selects that row', () {
      expect(
        shiftRangeSelection(visibleRows: tree, anchorId: 'b', targetId: 'b'),
        ['b'],
      );
    });

    test('an off-screen end refuses rather than guessing', () {
      // The anchor's folder was collapsed, or the row deleted, since the first
      // click. Selecting rows nobody can see is how a bulk delete surprises
      // someone.
      expect(
        shiftRangeSelection(visibleRows: tree, anchorId: 'gone', targetId: 'b'),
        isNull,
      );
      expect(
        shiftRangeSelection(visibleRows: tree, anchorId: 'b', targetId: 'gone'),
        isNull,
      );
    });
  });

  group('selectionAfterToggle', () {
    test('adds when absent and removes when present', () {
      expect(selectionAfterToggle({'a'}, 'b'), {'a', 'b'});
      expect(selectionAfterToggle({'a', 'b'}, 'b'), {'a'});
    });

    test('does not mutate the set it was given', () {
      // The caller holds this in State; mutating in place would change the
      // value setState is about to compare against.
      final before = {'a'};
      selectionAfterToggle(before, 'b');
      expect(before, {'a'});
    });

    test('toggling the last one out leaves an empty selection', () {
      expect(selectionAfterToggle({'a'}, 'a'), isEmpty);
    });
  });

  group('rightClickActsOnSelection', () {
    test('a right-click inside a multi-selection acts on all of it', () {
      expect(rightClickActsOnSelection({'a', 'b'}, 'a'), isTrue);
    });

    test('a right-click OUTSIDE the selection acts on that row alone', () {
      // Every file manager behaves this way, and the alternative is a context
      // menu that deletes rows the user scrolled away from and forgot about.
      expect(rightClickActsOnSelection({'a', 'b'}, 'c'), isFalse);
    });

    test('a single selected row is not a batch', () {
      // One row is the ordinary case; a batch menu there would only rename what
      // already works, and its wording ("delete 1 page?") reads as a bug.
      expect(rightClickActsOnSelection({'a'}, 'a'), isFalse);
      expect(rightClickActsOnSelection(<String>{}, 'a'), isFalse);
    });
  });
}
