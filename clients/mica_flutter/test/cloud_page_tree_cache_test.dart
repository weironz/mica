import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/main.dart';

void main() {
  test('cloud page tree round-trips through JSON (offline nav cache)', () {
    const ws = [
      Workspace(id: 'w1', name: 'Work', ownerId: 'u1', role: 'owner'),
      Workspace(id: 'w2', name: '文档', ownerId: 'u1', role: 'editor'),
    ];
    const views = {
      'w1': [
        DocumentView(
          id: 'v1',
          parentViewId: null,
          objectId: 'd1',
          objectType: 'document',
          name: 'Page',
          position: 'a',
        ),
        DocumentView(
          id: 'v2',
          parentViewId: 'v1',
          objectId: 'd2',
          objectType: 'document',
          name: '子页',
          position: 'b',
        ),
      ],
      'w2': <DocumentView>[],
    };
    final restored = cloudPageTreeFromJson(cloudPageTreeToJson(ws, views));
    expect(restored, isNotNull);
    expect(restored!.workspaces.map((w) => w.id).toList(), ['w1', 'w2']);
    expect(restored.workspaces[1].name, '文档');
    expect(restored.workspaces[0].role, 'owner');
    expect(restored.views['w1']!.map((v) => v.objectId).toList(), ['d1', 'd2']);
    expect(restored.views['w1']![1].parentViewId, 'v1');
    expect(restored.views['w1']![1].name, '子页');
    expect(restored.views['w2'], isEmpty);
  });

  test('absent / corrupt cache deserializes to null (no crash)', () {
    expect(cloudPageTreeFromJson(null), isNull);
    expect(cloudPageTreeFromJson(''), isNull);
    expect(cloudPageTreeFromJson('not json'), isNull);
    expect(cloudPageTreeFromJson('{"bad":1}'), isNull);
  });
}
