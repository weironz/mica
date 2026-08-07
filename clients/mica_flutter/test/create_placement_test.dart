import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';

// Where a new page/folder lands, given what the sidebar has located.
//
// `createParentForLocated` (nesting_rule_test.dart) answers WHICH GROUP. This
// answers WHERE IN IT: a located page means the next line, not the end of the
// folder. Creation carries no position at any layer, so the row is appended and
// then slid up — and the interesting cases are the ones where it must NOT be
// slid, because a pointless reorder is a round trip that can fail.

DocumentView view(String id, {String? parent, required String position}) =>
    DocumentView(
      id: id,
      parentViewId: parent,
      objectId: 'o_$id',
      objectType: 'document',
      name: id,
      position: position,
    );

void main() {
  test('a new row slides to just below the anchor', () {
    final a = view('a', parent: 'f', position: '10');
    final b = view('b', parent: 'f', position: '20');
    final c = view('c', parent: 'f', position: '30');
    final created = view('new', parent: 'f', position: '40');

    final ordered = orderedSiblingsPlacingAfter(
      views: [a, b, c, created],
      anchor: a,
      created: created,
    );
    expect(ordered?.map((v) => v.id).toList(), ['a', 'new', 'b', 'c']);
  });

  test('anchor already last means no reorder at all', () {
    // Not "reorder into the same order" — null, so the caller skips the round
    // trip. The create appends, which for a last anchor is already correct.
    final a = view('a', parent: 'f', position: '10');
    final created = view('new', parent: 'f', position: '20');
    expect(
      orderedSiblingsPlacingAfter(
        views: [a, created],
        anchor: a,
        created: created,
      ),
      isNull,
    );
  });

  test('an anchor that vanished mid-flight leaves the row where it is', () {
    // Deleted or moved away while the create was in flight. Reordering around a
    // stale anchor would put the row somewhere nobody asked for; ending up last
    // is the honest fallback.
    final gone = view('gone', parent: 'f', position: '10');
    final created = view('new', parent: 'f', position: '20');
    expect(
      orderedSiblingsPlacingAfter(
        views: [created],
        anchor: gone,
        created: created,
      ),
      isNull,
    );
  });

  test("only the anchor's own group is reordered", () {
    // Rows under other parents must not be pulled into the list — handing them
    // to onReorderViews would reparent them.
    final a = view('a', parent: 'f', position: '10');
    final b = view('b', parent: 'f', position: '20');
    final elsewhere = view('x', parent: 'other', position: '10');
    final rootRow = view('r', position: '10');
    final created = view('new', parent: 'f', position: '30');

    final ordered = orderedSiblingsPlacingAfter(
      views: [a, b, elsewhere, rootRow, created],
      anchor: a,
      created: created,
    );
    expect(ordered?.map((v) => v.id).toList(), ['a', 'new', 'b']);
  });

  test('root-level rows work the same way', () {
    // parentViewId null is a real group, not "no group".
    final a = view('a', position: '10');
    final b = view('b', position: '20');
    final created = view('new', position: '30');
    final ordered = orderedSiblingsPlacingAfter(
      views: [a, b, created],
      anchor: a,
      created: created,
    );
    expect(ordered?.map((v) => v.id).toList(), ['a', 'new', 'b']);
  });

  test('position order wins over list order', () {
    // The tree sorts by `position`; a caller handing views in arrival order
    // must still get the anchor's real neighbours.
    final a = view('a', parent: 'f', position: '30');
    final b = view('b', parent: 'f', position: '10');
    final created = view('new', parent: 'f', position: '40');
    final ordered = orderedSiblingsPlacingAfter(
      views: [a, b, created],
      anchor: b,
      created: created,
    );
    expect(ordered?.map((v) => v.id).toList(), ['b', 'new', 'a']);
  });
}
