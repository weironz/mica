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

  // The stateful half — and the one that actually shipped broken.
  //
  // Every rule below was already correct and tested as a pure function. The code
  // that folded clicks INTO the selection lived inline in the shell's State,
  // written as `_ids..clear()..addAll(selectionAfterToggle(_ids, id))`, where
  // Dart evaluates the argument after `clear()` has run — so it read an empty
  // set and every Ctrl-click reset the selection to one row. Nothing caught it
  // until the assembled app was clicked. It is a class now so these can exist.
  group('TreeSelection', () {
    List<SelectableRow> rows() => const [
          (id: 'a', parentId: null),
          (id: 'b', parentId: null),
          (id: 'c', parentId: null),
          (id: 'F', parentId: null),
          (id: 'f1', parentId: 'F'),
        ];

    TreeSelection ctrlClick(TreeSelection s, List<String> ids) {
      for (final id in ids) {
        s.click(id, extendRange: false, visibleRows: rows());
      }
      return s;
    }

    test('two Ctrl-clicks leave two rows selected', () {
      // THE regression. Under the cascade bug this ended at {'b'}.
      expect(ctrlClick(TreeSelection(), ['a', 'b']).ids, {'a', 'b'});
    });

    test('five Ctrl-clicks leave five', () {
      expect(
        ctrlClick(TreeSelection(), ['a', 'b', 'c', 'F', 'f1']).ids.length,
        5,
      );
    });

    test('Ctrl-clicking a selected row takes it back out', () {
      expect(ctrlClick(TreeSelection(), ['a', 'b', 'a']).ids, {'b'});
    });

    test('a Shift range adds to what Ctrl already picked', () {
      // Explorer's behaviour: Shift after a few Ctrl-clicks is "and also these".
      final s = ctrlClick(TreeSelection(), ['f1']);
      s.click('a', extendRange: false, visibleRows: rows()); // anchor at 'a'
      s.click('c', extendRange: true, visibleRows: rows());
      expect(s.ids, {'f1', 'a', 'b', 'c'});
    });

    test('a Shift range with no meaning degrades to a toggle', () {
      // 'a' (root) to 'f1' (inside F) crosses parents — the case the whole rule
      // exists to refuse. It must still do SOMETHING sensible with the click.
      final s = ctrlClick(TreeSelection(), ['a']);
      s.click('f1', extendRange: true, visibleRows: rows());
      expect(s.ids, {'a', 'f1'});
    });

    // The ordinary gesture, and the one that shipped broken: click a row, then
    // Shift-click another. Every test above starts with a Ctrl-click, which is
    // the UNUSUAL way to begin a range — so all of them passed while the common
    // path selected exactly one row.
    test('a plain click sets the anchor, so Shift after it makes a range', () {
      final s = TreeSelection();
      s.anchorOn('a');
      s.click('c', extendRange: true, visibleRows: rows());
      expect(s.ids, {'a', 'b', 'c'});
    });

    test('a plain click drops whatever was selected', () {
      final s = ctrlClick(TreeSelection(), ['a', 'b']);
      expect(s.anchorOn('F'), isTrue);
      expect(s.ids, isEmpty);
    });

    test('re-clicking the row that is already the anchor changes nothing', () {
      // Every row tap goes through this, so it must not force a rebuild per
      // click on a tree that can be thousands of rows.
      final s = TreeSelection();
      expect(s.anchorOn('a'), isTrue);
      expect(s.anchorOn('a'), isFalse);
    });

    test('clicking the blank area forgets the anchor too', () {
      // clear() is for a click that hit NO row; a following Shift-click then
      // has nothing to measure from, which is correct — there is no "from".
      final s = TreeSelection();
      s.anchorOn('a');
      s.clear();
      s.click('c', extendRange: true, visibleRows: rows());
      expect(s.ids, {'c'});
    });

    test('Shift with no anchor yet is just a toggle', () {
      final s = TreeSelection();
      s.click('b', extendRange: true, visibleRows: rows());
      expect(s.ids, {'b'});
    });

    test('one row is not a batch; two are', () {
      final s = ctrlClick(TreeSelection(), ['a']);
      expect(s.actsOnWholeSelection('a'), isFalse);
      ctrlClick(s, ['b']);
      expect(s.actsOnWholeSelection('a'), isTrue);
      expect(
        s.actsOnWholeSelection('c'),
        isFalse,
        reason: 'a gesture on a row OUTSIDE the selection acts on that row',
      );
    });

    test('clear reports whether it changed anything', () {
      final s = TreeSelection();
      expect(s.clear(), isFalse, reason: 'already empty — no rebuild needed');
      ctrlClick(s, ['a']);
      expect(s.clear(), isTrue);
      expect(s.ids, isEmpty);
    });

    test('clearing also forgets the anchor', () {
      // Otherwise the next Shift-click measures from a row nobody remembers
      // selecting, and silently takes everything in between.
      final s = ctrlClick(TreeSelection(), ['a']);
      s.clear();
      s.click('c', extendRange: true, visibleRows: rows());
      expect(s.ids, {'c'});
    });

    test('rows that scrolled or folded out of view are dropped', () {
      final s = ctrlClick(TreeSelection(), ['a', 'f1']);
      expect(s.retainVisible({'a', 'b', 'c', 'F'}), isTrue);
      expect(s.ids, {'a'}, reason: 'F was collapsed; f1 is no longer visible');
    });

    test('pruning nothing reports no change', () {
      final s = ctrlClick(TreeSelection(), ['a']);
      expect(s.retainVisible({'a', 'b', 'c', 'F', 'f1'}), isFalse);
    });

    test('pruning away the anchor forgets it', () {
      final s = ctrlClick(TreeSelection(), ['f1']);
      s.retainVisible({'a', 'b', 'c', 'F'});
      s.click('c', extendRange: true, visibleRows: rows());
      expect(s.ids, {'c'}, reason: 'no anchor left, so this is a toggle');
    });

    test('the exposed set cannot be mutated from outside', () {
      // It is the thing the whole feature is about; handing out a live handle
      // to it is how a selection changes without a setState to redraw it.
      final s = ctrlClick(TreeSelection(), ['a']);
      expect(() => s.ids.add('b'), throwsUnsupportedError);
    });
  });

  group('flattenedFolderOptions', () {
    FolderCandidate f(
      String id, {
      String? parent,
      String position = '01',
      bool folder = true,
    }) => (
      id: id,
      parentId: parent,
      name: id,
      position: position,
      isFolder: folder,
    );

    test('folders come out depth-first, with their depth', () {
      final options = flattenedFolderOptions([
        f('B', position: '02'),
        f('A', position: '01'),
        f('A2', parent: 'A', position: '01'),
        f('A2a', parent: 'A2', position: '01'),
      ]);
      expect(options.map((o) => '${o.id}@${o.depth}'), [
        'A@0',
        'A2@1',
        'A2a@2',
        'B@0',
      ]);
    });

    test('pages are not offered as destinations', () {
      // Only a folder may hold children — the tree's central invariant, and the
      // server enforces it. Listing a page here builds a dropdown whose entries
      // are guaranteed to 400.
      final options = flattenedFolderOptions([
        f('F'),
        f('page', parent: 'F', folder: false),
      ]);
      expect(options.map((o) => o.id), ['F']);
    });

    test('children follow position, not the order the server listed them', () {
      final options = flattenedFolderOptions([
        f('second', parent: null, position: '02'),
        f('first', parent: null, position: '01'),
      ]);
      expect(options.map((o) => o.id), ['first', 'second']);
    });

    test('a folder whose parent is missing is dropped, not promoted', () {
      // A partially-loaded tree. Promoting it would show a nested folder at
      // depth 0, and the indentation is the only thing distinguishing two
      // folders that share a name.
      final options = flattenedFolderOptions([f('orphan', parent: 'gone')]);
      expect(options, isEmpty);
    });

    test('a folder listed twice is offered once', () {
      // The walk descends from the root, so a parent CYCLE is simply
      // unreachable and needs no guard. What the guard is actually for is a
      // duplicated row in the response — which would otherwise put the same
      // folder in the dropdown twice, and its whole subtree with it.
      final options = flattenedFolderOptions([
        f('dup'),
        f('dup'),
        f('inside', parent: 'dup'),
      ]);
      expect(options.map((o) => o.id), ['dup', 'inside']);
    });
  });

  group('actsOnSelection', () {
    test('a gesture inside a multi-selection acts on all of it', () {
      expect(actsOnSelection({'a', 'b'}, 'a'), isTrue);
    });

    test('a gesture OUTSIDE the selection acts on that row alone', () {
      // Every file manager behaves this way, and the alternative is a context
      // menu that deletes rows the user scrolled away from and forgot about.
      expect(actsOnSelection({'a', 'b'}, 'c'), isFalse);
    });

    test('a single selected row is not a batch', () {
      // One row is the ordinary case; a batch menu there would only rename what
      // already works, and its wording ("delete 1 page?") reads as a bug.
      expect(actsOnSelection({'a'}, 'a'), isFalse);
      expect(actsOnSelection(<String>{}, 'a'), isFalse);
    });
  });

  group('independentSelection', () {
    // a, F(f1, f2 — f2 holds deep), b
    const parents = <String, String?>{
      'a': null,
      'F': null,
      'f1': 'F',
      'f2': 'F',
      'deep': 'f2',
      'b': null,
    };
    String? parentOf(String id) => parents[id];

    test('a child of a selected folder is dropped', () {
      // Otherwise a drag re-parents the child alongside its own folder, i.e.
      // pulls it OUT of the folder it was just moved with.
      expect(independentSelection(['F', 'f1'], parentOf), ['F']);
    });

    test('a grandchild is dropped too', () {
      expect(independentSelection(['F', 'deep'], parentOf), ['F']);
    });

    test('siblings under an UNSELECTED folder are all kept', () {
      // The common case: Ctrl-clicking two pages inside the same folder. The
      // folder is not selected, so neither of them is riding along inside
      // anything, and dropping one would silently move less than was asked.
      expect(independentSelection(['f1', 'f2'], parentOf), ['f1', 'f2']);
    });

    test('order is preserved, and repeats collapse', () {
      expect(independentSelection(['b', 'a', 'b'], parentOf), ['b', 'a']);
    });

    test('an id the tree does not know is treated as a root', () {
      expect(independentSelection(['ghost'], parentOf), ['ghost']);
    });

    test('a parent cycle terminates instead of hanging the drag', () {
      // Not reachable through a legal tree, but reachable through a stale
      // client snapshot mid-move — and a spin here freezes the whole app.
      String? cyclic(String id) => id == 'x' ? 'y' : 'x';
      expect(independentSelection(['x'], cyclic), ['x']);
    });
  });

  group('siblingOrderAfterDrop', () {
    List<String>? drop(List<String> dragged, String target, {required bool before}) =>
        siblingOrderAfterDrop(
          siblings: ['p', 'q', 'r', 's'],
          dragged: dragged,
          targetId: target,
          before: before,
        );

    test('a block lands together, in its own order', () {
      expect(drop(['p', 'r'], 'q', before: false), ['q', 'p', 'r', 's']);
    });

    test('before and after differ by exactly one slot', () {
      expect(drop(['s'], 'q', before: true), ['p', 's', 'q', 'r']);
      expect(drop(['s'], 'q', before: false), ['p', 'q', 's', 'r']);
    });

    test('rows are removed BEFORE the target index is read', () {
      // The bug this exists to prevent: with 'p' dragged from above 'r', taking
      // r's index first (2) and removing after leaves the block one slot late.
      expect(drop(['p'], 'r', before: true), ['q', 'p', 'r', 's']);
    });

    test('dropping onto a row that is itself being dragged does nothing', () {
      expect(drop(['p', 'q'], 'q', before: true), isNull);
    });

    test('a target that is no longer a sibling does nothing', () {
      // The tree changed under the drag — another device moved the row, or a
      // refresh landed. Guessing an index here silently reorders the wrong pair.
      expect(drop(['p'], 'gone', before: true), isNull);
    });

    test('a single-row drag behaves exactly as it did before', () {
      expect(drop(['r'], 'p', before: true), ['r', 'p', 'q', 's']);
    });
  });

  // The guards for the GLOBAL Esc handler. The handler itself cannot be reached
  // by a widget test — its whole reason to exist is that key dispatch never
  // enters the shell (see the function's doc) — so what is pinned here is the
  // decision: when Esc may clear, and when something else owns the key.
  group('escClearsTreeSelection', () {
    test('clears when a selection exists and nothing else owns Esc', () {
      expect(
        escClearsTreeSelection(
          hasSelection: true,
          routeIsCurrent: true,
          renamingInline: false,
        ),
        isTrue,
      );
    });

    test('no selection: nothing to clear, the key is not ours', () {
      expect(
        escClearsTreeSelection(
          hasSelection: false,
          routeIsCurrent: true,
          renamingInline: false,
        ),
        isFalse,
      );
    });

    // A dialog or menu is on top: Esc there means "close this", and a selection
    // silently dropping behind it is a change the user cannot see happening.
    test('a dialog on top owns Esc', () {
      expect(
        escClearsTreeSelection(
          hasSelection: true,
          routeIsCurrent: false,
          renamingInline: false,
        ),
        isFalse,
      );
    });

    // The inline-rename field cancels on Esc; one keypress must not both cancel
    // the rename and empty the selection.
    test('an inline rename owns Esc', () {
      expect(
        escClearsTreeSelection(
          hasSelection: true,
          routeIsCurrent: true,
          renamingInline: true,
        ),
        isFalse,
      );
    });
  });
}
