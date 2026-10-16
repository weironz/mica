import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';
import 'package:mica_flutter/ui/sidebar_tree_controller.dart';

/// Characterization tests for [SidebarTreeController], pinned against the
/// behavior the state had when it was still `_WorkspaceViewState` fields
/// (docs/code-optimization/OPTIMIZATION_PLAN_ROUND3.md §7).
void main() {
  DocumentView view(
    String id, {
    String? parent,
    String type = 'document',
  }) {
    return DocumentView(
      id: id,
      parentViewId: parent,
      objectId: 'obj-$id',
      objectType: type,
      name: id,
      position: '0',
    );
  }

  // root → f1 → f2 → page
  final tree = [
    view('root', type: 'folder'),
    view('f1', parent: 'root', type: 'folder'),
    view('f2', parent: 'f1', type: 'folder'),
    view('page', parent: 'f2'),
  ];

  group('toggle (folder toggle = locate + expand)', () {
    test('expands a collapsed folder and locates it', () {
      final c = SidebarTreeController();
      c.toggle('f1');
      expect(c.isExpanded('f1'), isTrue);
      expect(c.focusedNavId, 'f1');
      expect(c.rootFocused, isFalse);
    });

    test('toggling again collapses but STILL relocates', () {
      final c = SidebarTreeController();
      c.toggle('f1');
      c.locateRow('page');
      c.toggle('f1');
      expect(c.isExpanded('f1'), isFalse);
      expect(c.focusedNavId, 'f1', reason: 'folders locate on every toggle');
    });
  });

  group('focus', () {
    test('focusRoot clears the row focus once, then is a no-op', () {
      final c = SidebarTreeController();
      var notifications = 0;
      c.addListener(() => notifications++);
      c.focusRoot();
      expect(c.rootFocused, isTrue);
      expect(c.focusedNavId, isNull);
      expect(notifications, 1);
      c.focusRoot();
      expect(notifications, 1, reason: 'already at root: no rebuild fires');
    });

    test('locateRow is a no-op for an identical re-tap', () {
      final c = SidebarTreeController();
      c.locateRow('page');
      var notifications = 0;
      c.addListener(() => notifications++);
      c.locateRow('page');
      expect(notifications, 0);
    });

    test('locateRow from a root-focused state moves the focus', () {
      final c = SidebarTreeController();
      c.focusRoot();
      c.locateRow('page');
      expect(c.rootFocused, isFalse);
      expect(c.focusedNavId, 'page');
    });
  });

  group('reveal', () {
    test('revealAncestors expands the ancestors, never the node itself', () {
      final c = SidebarTreeController();
      c.revealAncestors(tree, 'page');
      expect(c.isExpanded('root'), isTrue);
      expect(c.isExpanded('f1'), isTrue);
      expect(c.isExpanded('f2'), isTrue);
      expect(c.isExpanded('page'), isFalse,
          reason: 'expanding the node itself would show ITS children');
    });

    test('expandForChildOf expands the node AND its ancestors', () {
      final c = SidebarTreeController();
      c.expandForChildOf(tree, 'f2');
      expect(c.isExpanded('f2'), isTrue);
      expect(c.isExpanded('f1'), isTrue);
    });

    test('revealFolder locates and expands through the whole chain', () {
      final c = SidebarTreeController();
      c.revealFolder(tree, 'page');
      expect(c.focusedNavId, 'page');
      expect(c.rootFocused, isFalse);
      expect(c.isExpanded('page'), isTrue);
      expect(c.isExpanded('f2'), isTrue);
      expect(c.isExpanded('f1'), isTrue);
    });

    test('revealFolder ignores an id that renders nowhere', () {
      final c = SidebarTreeController();
      c.revealFolder(tree, 'not-in-tree');
      expect(c.focusedNavId, isNull);
      expect(c.expandedIds, isEmpty);
    });

    test('revealFolder survives a cycle in the parent chain', () {
      final cyclic = [
        view('a', parent: 'b', type: 'folder'),
        view('b', parent: 'a', type: 'folder'),
      ];
      final c = SidebarTreeController();
      c.revealFolder(cyclic, 'a');
      expect(c.isExpanded('a'), isTrue);
      expect(c.isExpanded('b'), isTrue);
    });

    test('revealInTree stashes a pending id when the row is not there yet', () {
      final c = SidebarTreeController();
      c.revealInTree(tree, 'from-another-workspace');
      expect(c.pendingRevealId, 'from-another-workspace');
      expect(c.revealAttempts, 0);
      expect(c.focusedNavId, isNull);
    });

    test('revealInTree reveals AND stashes when the row is present', () {
      final c = SidebarTreeController();
      c.revealInTree(tree, 'page');
      expect(c.focusedNavId, 'page');
      expect(c.pendingRevealId, 'page');
      expect(c.revealAttempts, 0);
    });
  });

  group('pending reveal timeout', () {
    test('gives up after 120 frames and clears itself', () {
      final c = SidebarTreeController();
      c.revealInTree(tree, 'late');
      var alive = true;
      for (var i = 0; i < 200 && alive; i++) {
        alive = c.bumpRevealAttempt();
      }
      expect(alive, isFalse);
      expect(c.pendingRevealId, isNull);
      expect(c.revealAttempts, 0);
    });

    test('no pending reveal means nothing to wait for', () {
      final c = SidebarTreeController();
      expect(c.bumpRevealAttempt(), isFalse);
    });
  });

  group('workspace switch', () {
    test('clearExpansion drops every expansion', () {
      final c = SidebarTreeController();
      c.expandForChildOf(tree, 'page');
      c.clearExpansion();
      expect(c.expandedIds, isEmpty);
    });
  });
}
