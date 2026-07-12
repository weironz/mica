// F5 Fix D: the local delete/restore/purge subtree cascade. The bug was that
// local delete only trashed the clicked node, orphaning its children (deep
// descendants vanished from the sidebar) — unlike the server, whose recursive
// CTE cascades the whole subtree. collectSubtreeIds is the pure core that must
// walk EVERY descendant.
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/main.dart';

void main() {
  ({String id, String? parentId}) n(String id, String? parent) =>
      (id: id, parentId: parent);

  test('walks the whole subtree, not just direct children', () {
    // F ─ D ─ E  (folder → child folder → document): the exact 3-level shape
    // that used to lose E when delete only trashed the root.
    final nodes = [n('F', null), n('D', 'F'), n('E', 'D'), n('X', null)];
    expect(collectSubtreeIds(nodes, 'F'), {'F', 'D', 'E'});
    // Sibling subtrees are untouched; deeper roots collect only their own.
    expect(collectSubtreeIds(nodes, 'D'), {'D', 'E'});
    expect(collectSubtreeIds(nodes, 'E'), {'E'});
    expect(collectSubtreeIds(nodes, 'X'), {'X'});
  });

  test('a folder with several children at one level collects them all', () {
    final nodes = [
      n('root', null),
      n('a', 'root'),
      n('b', 'root'),
      n('c', 'a'),
    ];
    expect(collectSubtreeIds(nodes, 'root'), {'root', 'a', 'b', 'c'});
  });

  test('returns empty for a missing root', () {
    expect(collectSubtreeIds([n('A', null)], 'nope'), isEmpty);
  });

  test('is cycle-safe (corrupt parent links must not hang)', () {
    final nodes = [n('A', 'B'), n('B', 'A')];
    expect(collectSubtreeIds(nodes, 'A'), {'A', 'B'});
  });
}
