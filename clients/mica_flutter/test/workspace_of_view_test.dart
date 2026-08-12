// Locking down the field that a flatten used to discard.
//
// Home lists pages from every workspace, but opening one is workspace-scoped.
// The old code found the view with `viewsByWorkspace.values.expand(...)`, which
// answers "does this view exist" and NOT "where does it live" — so every recent
// belonging to another workspace was opened against the current one and 404'd.

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';

DocumentView _view(String id, {String type = 'document'}) => DocumentView(
  id: id,
  parentViewId: null,
  objectId: 'obj-$id',
  objectType: type,
  name: 'Page $id',
  position: '10',
);

void main() {
  group('workspaceIdOfView', () {
    final tree = {
      'ws-a': [_view('a1'), _view('a2')],
      'ws-b': [_view('b1'), _view('b2', type: 'folder')],
    };

    test('finds a page in the first workspace', () {
      expect(workspaceIdOfView(viewsByWorkspace: tree, viewId: 'a2'), 'ws-a');
    });

    test('finds a page in a LATER workspace, not just the current one', () {
      // The regression: this is the case that used to be opened against
      // whichever workspace happened to be selected, and 404.
      expect(workspaceIdOfView(viewsByWorkspace: tree, viewId: 'b1'), 'ws-b');
    });

    test('answers for folders too — reveal is workspace-scoped as well', () {
      expect(workspaceIdOfView(viewsByWorkspace: tree, viewId: 'b2'), 'ws-b');
    });

    test('null for an unknown id, rather than guessing a workspace', () {
      // The caller opens nothing on null. Returning a plausible workspace here
      // would turn "this page is gone" into "the wrong page opened".
      expect(workspaceIdOfView(viewsByWorkspace: tree, viewId: 'missing'), isNull);
    });

    test('null on an empty tree', () {
      expect(workspaceIdOfView(viewsByWorkspace: const {}, viewId: 'a1'), isNull);
    });

    test('a workspace with no views does not shadow a later match', () {
      final withEmpty = {
        'ws-empty': <DocumentView>[],
        'ws-a': [_view('a1')],
      };
      expect(workspaceIdOfView(viewsByWorkspace: withEmpty, viewId: 'a1'), 'ws-a');
    });
  });
}
