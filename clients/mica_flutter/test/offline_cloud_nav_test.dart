import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/main.dart';
import 'package:mica_flutter/local/local_offline.dart';

// P1c offline read: the pure reconstruction that turns an on-device page-tree
// mirror back into the cloud nav model when the server is unreachable.
void main() {
  test('rebuildCloudNavFromCache reconstructs workspaces + grouped views', () {
    final CloudPageTreeCache cache = (
      workspaces: <WorkspaceData>[
        (id: 'w1', name: 'Work', position: '0000000010'),
        (id: 'w2', name: '文档', position: '0000000020'),
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
        ),
        (
          id: 'v2',
          workspaceId: 'w1',
          parentId: 'v1',
          objectId: 'd2',
          name: '子页',
          position: 'b',
          trashed: false,
        ),
        (
          id: 'v3',
          workspaceId: 'w2',
          parentId: null,
          objectId: 'd3',
          name: 'Other',
          position: 'a',
          trashed: false,
        ),
      ],
    );

    final rebuilt = rebuildCloudNavFromCache(cache, 'user-1');

    // Workspace list preserved in order; provenance defaulted.
    expect(rebuilt.workspaces.map((w) => w.id).toList(), ['w1', 'w2']);
    expect(rebuilt.workspaces[1].name, '文档');
    expect(rebuilt.workspaces[0].ownerId, 'user-1');
    // Offline is read-only: role='viewer' drives matchesEditRole to block edits.
    expect(rebuilt.workspaces.every((w) => w.role == 'viewer'), isTrue);
    expect(matchesEditRole(rebuilt.workspaces[0].role), isFalse);

    // Views grouped by workspace, order + tree fields preserved.
    expect(rebuilt.views['w1']!.map((v) => v.objectId).toList(), ['d1', 'd2']);
    expect(rebuilt.views['w1']![1].parentViewId, 'v1');
    expect(rebuilt.views['w1']![1].name, '子页');
    expect(rebuilt.views['w1']![1].objectType, 'document');
    expect(rebuilt.views['w2']!.single.objectId, 'd3');
  });

  test('rebuildCloudNavFromCache handles a workspace with no views', () {
    final CloudPageTreeCache cache = (
      workspaces: <WorkspaceData>[(id: 'w1', name: 'W', position: '0000000010')],
      views: <ViewData>[],
    );
    final rebuilt = rebuildCloudNavFromCache(cache, 'u');
    expect(rebuilt.workspaces.single.id, 'w1');
    expect(rebuilt.views, isEmpty);
  });
}
