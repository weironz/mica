import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/main.dart';
import 'package:mica_flutter/local/local_offline.dart';

// P1c offline read: the pure reconstruction that turns an on-device page-tree
// mirror back into the cloud nav model when the server is unreachable.
void main() {
  test('rebuildCloudNavFromCache reconstructs workspaces + grouped views', () {
    final CloudPageTreeCache cache = (
      workspaces: <WorkspaceData>[
        (id: 'w1', name: 'Work', position: '0000000010', role: 'editor'),
        (id: 'w2', name: '文档', position: '0000000020', role: 'viewer'),
      ],
      views: <ViewData>[
        (
          id: 'v1',
          workspaceId: 'w1',
          parentId: null,
          objectId: 'd1',
          name: 'Page',
          position: 'a',
          trashed: false,
          objectType: 'document',
        ),
        (
          id: 'v2',
          workspaceId: 'w1',
          parentId: 'v1',
          objectId: 'd2',
          name: '子页',
          position: 'b',
          trashed: false,
          objectType: 'document',
        ),
        (
          id: 'v3',
          workspaceId: 'w2',
          parentId: null,
          objectId: 'd3',
          name: 'Other',
          position: 'a',
          trashed: false,
          objectType: 'document',
        ),
      ],
    );

    final rebuilt = rebuildCloudNavFromCache(cache, 'user-1');

    // Workspace list preserved in order; ownerId defaulted to the current user.
    expect(rebuilt.workspaces.map((w) => w.id).toList(), ['w1', 'w2']);
    expect(rebuilt.workspaces[1].name, '文档');
    expect(rebuilt.workspaces[0].ownerId, 'user-1');
    // P2d: the MIRRORED role is used — an editor can edit its cached docs offline,
    // a viewer stays read-only (via matchesEditRole).
    expect(rebuilt.workspaces[0].role, 'editor');
    expect(matchesEditRole(rebuilt.workspaces[0].role), isTrue);
    expect(rebuilt.workspaces[1].role, 'viewer');
    expect(matchesEditRole(rebuilt.workspaces[1].role), isFalse);

    // Views grouped by workspace, order + tree fields preserved.
    expect(rebuilt.views['w1']!.map((v) => v.objectId).toList(), ['d1', 'd2']);
    expect(rebuilt.views['w1']![1].parentViewId, 'v1');
    expect(rebuilt.views['w1']![1].name, '子页');
    expect(rebuilt.views['w1']![1].objectType, 'document');
    expect(rebuilt.views['w2']!.single.objectId, 'd3');
  });

  // F5 Fix C: every auto-open path funnels through firstOpenableView so a folder
  // is never opened as a document (which would 404 / show a blank editor).
  test('firstOpenableView skips folders and returns the first document', () {
    DocumentView v(String id, String type) => DocumentView(
          id: id,
          parentViewId: null,
          objectId: 'o$id',
          objectType: type,
          name: id,
          position: '0000000010',
        );
    // A folder sorted first is skipped; the first document wins.
    expect(
      firstOpenableView([v('f', 'folder'), v('d1', 'document'), v('d2', 'document')])?.id,
      'd1',
    );
    // Only folders → nothing openable.
    expect(firstOpenableView([v('f1', 'folder'), v('f2', 'folder')]), isNull);
    // Empty → null.
    expect(firstOpenableView(const <DocumentView>[]), isNull);
  });

  test('rebuildCloudNavFromCache handles a workspace with no views', () {
    final CloudPageTreeCache cache = (
      workspaces: <WorkspaceData>[
        (id: 'w1', name: 'W', position: '0000000010', role: 'owner'),
      ],
      views: <ViewData>[],
    );
    final rebuilt = rebuildCloudNavFromCache(cache, 'u');
    expect(rebuilt.workspaces.single.id, 'w1');
    expect(rebuilt.views, isEmpty);
  });
}
