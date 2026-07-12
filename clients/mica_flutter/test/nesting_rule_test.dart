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
}
