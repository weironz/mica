// Gold-file contract tests (#85): decode /contracts/*.json — produced by the
// SERVER's DTO types — into the Flutter client's own models, and assert every
// field the app reads.
//
// There is deliberately no shared DTO code between server / cli / mcp / Dart
// (the web build needs a Dart implementation, so a Rust crate could only cover
// three of the four). These golds are the drift guard instead: a server-side
// rename fails the server's gold test AND this one in the same CI run, instead
// of breaking the app against a deployed server.
//
// Regenerating after an intentional shape change:
//   MICA_UPDATE_GOLDEN=1 cargo test -p mica-api-server contract_gold

import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';

Map<String, dynamic> gold(String name) {
  final file = File('../../contracts/$name.json');
  expect(file.existsSync(), isTrue, reason: 'gold file missing: ${file.path}');
  return jsonDecode(file.readAsStringSync()) as Map<String, dynamic>;
}

void main() {
  test('user decodes every field', () {
    final user = User.fromJson(gold('user'));
    expect(user.id, '11111111-1111-4111-8111-111111111111');
    expect(user.email, 'owner@mica.test');
    expect(user.displayName, 'Gold Owner');
    expect(user.avatarVersion, 'v3-20261024');
  });

  test('workspace list decodes every field', () {
    final rows = gold('workspace_list')['workspaces'] as List<dynamic>;
    expect(rows, hasLength(1));
    final ws = Workspace.fromJson(rows.single as Map<String, dynamic>);
    expect(ws.id, '22222222-2222-4222-8222-222222222222');
    expect(ws.name, 'Gold Workspace');
    expect(ws.ownerId, '11111111-1111-4111-8111-111111111111');
    expect(ws.role, 'owner');
    expect(ws.pageCount, 12);
  });

  test('workspace members decode every field', () {
    final rows = (gold('workspace_members')['members'] as List<dynamic>)
        .map((m) => WorkspaceMember.fromJson(m as Map<String, dynamic>))
        .toList();
    expect(rows, hasLength(2));

    final owner = rows[0];
    expect(owner.userId, '11111111-1111-4111-8111-111111111111');
    expect(owner.email, 'owner@mica.test');
    expect(owner.displayName, 'Gold Owner');
    expect(owner.role, 'owner');
    expect(owner.avatarVersion, isNull);

    final editor = rows[1];
    expect(editor.role, 'editor');
    // Serialized as the derived `avatar_version`, never the raw storage key.
    expect(editor.avatarVersion, 'v3-20261024');
  });

  test('view list decodes every field', () {
    final rows = (gold('view_list')['views'] as List<dynamic>)
        .map((v) => DocumentView.fromJson(v as Map<String, dynamic>))
        .toList();
    expect(rows, hasLength(3));

    final folder = rows[0];
    expect(folder.id, '33333333-3333-4333-8333-333333333333');
    expect(folder.objectType, 'folder');
    expect(folder.isFolder, isTrue);
    expect(folder.name, 'Gold Folder');
    expect(folder.icon, '📗');
    expect(folder.parentViewId, isNull);
    expect(folder.position, '0000000001');
    expect(folder.updatedAt, isNotNull);

    final stats = rows[1];
    expect(stats.objectId, '66666666-6666-4666-8666-666666666666');
    expect(stats.name, 'With Stats');
    expect(stats.parentViewId, '33333333-3333-4333-8333-333333333333');
    expect(stats.icon, isNull, reason: 'absent stays absent, not null-in-map');

    final plain = rows[2];
    expect(plain.name, 'Plain Page');
  });

  test('document create decodes document + view', () {
    final result = DocumentCreateResult.fromJson(gold('document_create'));
    expect(result.document.id, '66666666-6666-4666-8666-666666666666');
    expect(result.document.rootBlockId, 'block_root');
    expect(result.view.id, '44444444-4444-4444-8444-444444444444');
    expect(result.view.name, 'Created Page');
  });

  test('document update decodes document + snapshot', () {
    final result = DocumentUpdateResult.fromJson(gold('document_update'));
    expect(result.document.id, '66666666-6666-4666-8666-666666666666');
    expect(result.document.currentSeq, 4);
    expect(result.snapshot.versionSeq, 4);
    final blocks = result.snapshot.payload['blocks'];
    expect(blocks, isA<List<dynamic>>());
    expect((blocks as List<dynamic>), hasLength(3));
  });

  test('document bootstrap decodes document + view + snapshot', () {
    final boot = DocumentBootstrap.fromJson(gold('document_bootstrap'));
    expect(boot.document.id, '66666666-6666-4666-8666-666666666666');
    expect(boot.view.name, 'Bootstrapped Page');
    expect(boot.snapshot.versionSeq, 3);
    // The block projection the editor walks.
    final children = boot.childBlocks;
    expect(children, hasLength(2));
    expect(children.first.id, 'block_a');
    expect(children.last.id, 'block_b');
    expect(children.first.text, 'First paragraph');
  });

  test('history decodes the version rows', () {
    final rows = (gold('history')['versions'] as List<dynamic>)
        .map((v) => DocVersion.fromJson(v as Map<String, dynamic>))
        .toList();
    expect(rows, hasLength(2));
    // Newest first — the endpoint's documented order.
    expect(rows[0].label, 'before the rewrite');
    expect(rows[0].createdBy, '11111111-1111-4111-8111-111111111111');
    expect(rows[0].isAuto, isFalse);
    expect(rows[1].label, isNull);
    expect(rows[1].createdBy, isNull);
    expect(rows[1].isAuto, isTrue);
  });

  test('version decodes the checkpoint row', () {
    final version = DocVersion.fromJson(
      gold('version')['version'] as Map<String, dynamic>,
    );
    expect(version.id, 'aaaaaaaa-0000-4000-8000-000000000001');
    expect(version.label, 'before the rewrite');
  });

  // batchTrashViews and the offline page-tree cache navigate the raw JSON by
  // key, not through a struct — those string keys are as much the contract as
  // any type.
  test('raw navigation keys exist where the client walks them', () {
    final batch = gold('batch_views');
    expect(batch['affected'], 3);
    expect(batch['skipped'], [
      '55555555-5555-4555-8555-555555555555',
    ]);

    final read = gold('batch_read');
    final docs = read['documents'] as List<dynamic>;
    final first = docs[0] as Map<String, dynamic>;
    expect(first['document_id'], '66666666-6666-4666-8666-666666666666');
    expect(first['markdown'], '# First\n\nbody');
    final failed = docs[1] as Map<String, dynamic>;
    expect(failed['error'], 'not found');
    expect(
      failed['markdown'],
      isNull,
      reason: 'never both markdown and error on one row',
    );
  });
}
