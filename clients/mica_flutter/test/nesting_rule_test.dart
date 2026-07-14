// Pages are leaves — nothing nests under a document; only folders (or the
// workspace root) accept children. canNestUnder is the shared gate the
// drag-drop uses, kept consistent with the menu (child-create on folders only).
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/main.dart';

void main() {
  DocumentView v(String id, String type) => DocumentView(
        id: id,
        parentViewId: null,
        objectId: 'o$id',
        objectType: type,
        name: id,
        position: '0000000010',
      );

  final views = [v('doc', 'document'), v('fold', 'folder')];

  test('workspace root (null parent) always accepts children', () {
    expect(canNestUnder(views, null), isTrue);
  });

  test('a folder accepts children', () {
    expect(canNestUnder(views, 'fold'), isTrue);
  });

  test('a document is a leaf — it rejects children', () {
    expect(canNestUnder(views, 'doc'), isFalse);
  });

  test('an unknown parent rejects children (strict/safe)', () {
    expect(canNestUnder(views, 'ghost'), isFalse);
  });

  // The top-of-sidebar New buttons create relative to the located node.
  group('createParentForLocated — where a new node lands', () {
    DocumentView node(String id, String type, {String? parent}) => DocumentView(
          id: id,
          parentViewId: parent,
          objectId: 'o$id',
          objectType: type,
          name: id,
          position: '0000000010',
        );

    test('nothing located → workspace root (null)', () {
      expect(createParentForLocated(views, null), isNull);
    });

    test('a located folder → create INSIDE it', () {
      final fold = node('fold', 'folder');
      expect(createParentForLocated([fold], fold)?.id, 'fold');
    });

    test('a located page under a folder → create BESIDE it (its parent)', () {
      final fold = node('fold', 'folder');
      final page = node('page', 'document', parent: 'fold');
      expect(createParentForLocated([fold, page], page)?.id, 'fold');
    });

    test('a located root-level page → create at root (null)', () {
      final page = node('page', 'document');
      expect(createParentForLocated([page], page), isNull);
    });

    test('a located page whose parent is missing → root (safe)', () {
      final page = node('page', 'document', parent: 'ghost');
      expect(createParentForLocated([page], page), isNull);
    });
  });
}
