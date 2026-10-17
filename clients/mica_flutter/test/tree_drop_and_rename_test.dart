import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';
import 'package:mica_flutter/ui/inline_rename_controller.dart';
import 'package:mica_flutter/ui/tree_drop_planner.dart';

/// Characterization tests for the god-class wrap-up extractions
/// (docs/code-optimization/OPTIMIZATION_PLAN_ROUND3.md §7): the inline-rename
/// flow state and the sidebar drag-drop decision logic, pinned against the
/// behavior they had as `_WorkspaceViewState` methods.
void main() {
  DocumentView view(
    String id, {
    String? parent,
    String type = 'document',
    String name = 'N',
  }) {
    return DocumentView(
      id: id,
      parentViewId: parent,
      objectId: 'obj-$id',
      objectType: type,
      name: name,
      position: '0',
    );
  }

  group('InlineRenameController', () {
    test('begin marks exactly one row as renaming', () {
      final c = InlineRenameController();
      c.begin('v1');
      expect(c.isRenaming('v1'), isTrue);
      expect(c.isRenaming('v2'), isFalse);
    });

    test('cancel is a no-op when nothing is renaming', () {
      final c = InlineRenameController();
      var n = 0;
      c.addListener(() => n++);
      c.cancel();
      expect(n, 0);
      c.begin('v1');
      c.cancel();
      expect(c.renamingId, isNull);
      expect(n, 2);
    });

    test('commit always ends editing and delegates the save decision', () {
      final c = InlineRenameController();
      final v = view('v1', name: 'Old');
      c.begin('v1');
      // Blank → "changed my mind", nothing to save, but editing still ends.
      expect(c.commit(v, '   '), isNull);
      expect(c.renamingId, isNull);
      // Unchanged → nothing to send (a visited field is not an edit).
      c.begin('v1');
      expect(c.commit(v, ' Old '), isNull);
      // Changed → the trimmed name.
      c.begin('v1');
      expect(c.commit(v, ' New Name '), 'New Name');
      expect(c.renamingId, isNull);
    });

    test('commit without a rename in flight still validates', () {
      final c = InlineRenameController();
      final v = view('v1', name: 'Old');
      expect(c.commit(v, 'New'), 'New',
          reason: 'the dialog path commits without ever beginning inline edit');
    });
  });

  group('TreeDropPlanner', () {
    // root(0) → f1(1) → f2(2) → page(3)
    final tree = [
      view('root', type: 'folder'),
      view('f1', parent: 'root', type: 'folder'),
      view('f2', parent: 'f1', type: 'folder'),
      view('page', parent: 'f2'),
    ];
    late final TreeDropPlanner p;
    p = TreeDropPlanner(tree);

    test('a drop target inside a dragged row is refused (cycle)', () {
      expect(p.isSelfOrDescendant('f1', 'root'), isTrue);
      expect(p.isSelfOrDescendant('page', 'root'), isTrue);
      expect(p.isSelfOrDescendant('root', 'page'), isFalse);
      // Dragging a folder onto anything inside its own subtree is refused...
      expect(p.canDrop([tree[0]], tree[2], into: true), isFalse);
      expect(p.canDrop([tree[0]], tree[3], into: true), isFalse);
      // ...and one poisoned row refuses the whole batch.
      expect(
        p.canDrop([view('other'), tree[0]], tree[2], into: true),
        isFalse,
      );
      // Dropping it into its own PARENT (a sibling move) is fine.
      expect(p.canDrop([tree[2]], tree[0], into: true), isTrue);
    });

    test('into nests under the target; before/after under its parent', () {
      final f2 = tree[2];
      expect(p.dropParentId(f2, into: true), 'f2');
      expect(p.dropParentId(f2, into: false), 'f1');
      expect(p.dropParentId(tree[0], into: false), isNull,
          reason: 'root-level siblings land at the workspace root');
    });

    test('only folders (and the root) accept children', () {
      expect(p.parentAllowsChildren('f1'), isTrue);
      expect(p.parentAllowsChildren('page'), isFalse);
      expect(p.parentAllowsChildren(null), isTrue);
      expect(p.parentAllowsChildren('missing'), isFalse);
      // A before/after drop beside a PAGE therefore resolves to the page's
      // parent, which is a folder → allowed.
      expect(p.canDrop([view('drag')], tree[3], into: false), isTrue);
      // ...while dropping INTO a page is refused.
      expect(p.canDrop([view('drag')], tree[3], into: true), isFalse);
    });

    test('ancestorAtLevel resolves a sideways level to the sibling anchor', () {
      final page = tree[3];
      expect(p.ancestorAtLevel(page, 3, 2), tree[2],
          reason: 'level 2 = next sibling of f2');
      expect(p.ancestorAtLevel(page, 3, 1), tree[1]);
      expect(p.ancestorAtLevel(page, 3, 0), tree[0]);
      // Level ≥ own depth = the row itself (the plain after drop).
      expect(p.ancestorAtLevel(page, 3, 3), page);
    });

    test('a broken parent chain stops at the nearest resolvable ancestor', () {
      final orphan = view('orphan', parent: 'gone');
      final planner = TreeDropPlanner([...tree, orphan]);
      expect(planner.ancestorAtLevel(orphan, 3, 0), orphan);
    });

    test('resolveAfterTarget mirrors the accept-time resolution', () {
      final page = tree[3];
      expect(p.resolveAfterTarget(page, 3, null), page);
      expect(p.resolveAfterTarget(page, 3, 3), page);
      expect(p.resolveAfterTarget(page, 3, 1), tree[1]);
    });

    test('isSelfOrDescendant survives a parent-chain cycle', () {
      final cyclic = [
        view('a', parent: 'b', type: 'folder'),
        view('b', parent: 'a', type: 'folder'),
      ];
      final planner = TreeDropPlanner(cyclic);
      expect(planner.isSelfOrDescendant('a', 'b'), isTrue);
      expect(planner.isSelfOrDescendant('a', 'zzz'), isFalse);
    });
  });
}
