// The local page-tree operations, through the REAL FFI.
//
// `rust/src/api/store.rs` has thorough unit tests, but those call the Rust
// functions directly. They cannot catch the layer this file covers: whether
// the generated bindings actually carry the arguments and results across —
// an `Option<String>` that arrives as the wrong variant, a `Vec<String>` that
// comes back empty, a null store degrading to the wrong default. Everything
// below runs against a real SQLite file in a temp dir via a loaded native lib.
//
// Every operation here is on the daily path: new page, new folder, drag to
// reorder, duplicate, delete, restore, delete forever.
//
//   flutter test integration_test/frb_view_tree_test.dart -d windows
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:mica_flutter/src/rust/api/document.dart';
import 'package:mica_flutter/src/rust/api/store.dart';
import 'package:mica_flutter/src/rust/frb_generated.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(() async => RustLib.init());
  tearDownAll(() async => RustLib.dispose());

  MicaStore freshStore() {
    final dir = Directory.systemTemp.createTempSync('mica_tree');
    return MicaStore.open(path: '${dir.path}/store.db')!;
  }

  String? parentOf(MicaStore s, String id) =>
      s.listViews(origin: 'local').firstWhere((v) => v.id == id).parentId;

  String posOf(MicaStore s, String id) =>
      s.listViews(origin: 'local').firstWhere((v) => v.id == id).position;

  test('create assigns ids and positions across the bridge', () {
    final s = freshStore();
    final a = s.createView(
        workspaceId: 'ws', objectId: 'doc_a', name: 'A', objectType: 'document');
    final b = s.createView(
        workspaceId: 'ws', objectId: 'doc_b', name: 'B', objectType: 'document');

    expect(a, startsWith('view_'), reason: 'Rust minted the id, Dart got it');
    expect(a, isNot(b));
    expect(posOf(s, a), '0000000010');
    expect(posOf(s, b), '0000000020');

    // A null parent must arrive as None, not as the string "null" — the
    // Option<String> crossing is exactly what a unit test cannot check.
    expect(parentOf(s, a), isNull);

    final child = s.createView(
        workspaceId: 'ws',
        parentId: a,
        objectId: 'doc_c',
        name: 'C',
        objectType: 'folder');
    expect(parentOf(s, child), a, reason: 'a present parent crossed intact');
    expect(posOf(s, child), '0000000010', reason: 'positions are per-parent');
  });

  test('reorder renumbers and reparents through the bridge', () {
    final s = freshStore();
    final x = s.createView(
        workspaceId: 'ws', objectId: 'dx', name: 'X', objectType: 'document');
    final y = s.createView(
        workspaceId: 'ws', objectId: 'dy', name: 'Y', objectType: 'document');
    final f = s.createView(
        workspaceId: 'ws', objectId: 'df', name: 'F', objectType: 'folder');

    s.reorderViews(parentId: f, orderedIds: [y, x]);
    expect(posOf(s, y), '0000000010');
    expect(posOf(s, x), '0000000020');
    expect(parentOf(s, x), f);
  });

  test('clone copies the subtree and its content', () {
    final s = freshStore();
    final root = s.createView(
        workspaceId: 'ws',
        objectId: 'd_root',
        name: 'Notes',
        objectType: 'document');
    final kid = s.createView(
        workspaceId: 'ws',
        parentId: root,
        objectId: 'd_kid',
        name: 'Kid',
        objectType: 'document');
    s.saveDoc(
        docId: 'd_root', doc: MicaDocument.fromMarkdown(markdown: '# hello'));
    s.saveDoc(docId: 'd_kid', doc: MicaDocument.fromMarkdown(markdown: '# kid'));

    final out = s.cloneView(viewId: root, rootName: 'Notes')!;
    expect(out.newName, 'Notes 2', reason: 'deduped against the original');
    expect(out.docs, 2);

    final all = s.listViews(origin: 'local');
    expect(all.length, 4);
    final copiedKid = all.firstWhere((v) => v.parentId == out.rootViewId);
    expect(copiedKid.id, isNot(kid), reason: 'the copy is a new row');
    expect(
      s.loadDoc(docId: copiedKid.objectId)!.exportMarkdown(),
      contains('kid'),
      reason: 'content came along, not just the row',
    );
  });

  test('trash → restore → purge round-trips a real subtree', () {
    final s = freshStore();
    final folder = s.createView(
        workspaceId: 'ws', objectId: 'd_f', name: 'F', objectType: 'folder');
    final page = s.createView(
        workspaceId: 'ws',
        parentId: folder,
        objectId: 'd_p',
        name: 'P',
        objectType: 'document');
    s.saveDoc(docId: 'd_p', doc: MicaDocument.fromMarkdown(markdown: 'x'));

    final trashed = s.trashViewSubtree(viewId: folder);
    expect(trashed.toSet(), {folder, page},
        reason: 'the Vec<String> crossed with both ids');
    expect(
      s.listViews(origin: 'local').every((v) => v.trashed),
      isTrue,
      reason: 'a folder takes its children to the bin',
    );

    s.restoreViewSubtree(viewId: folder);
    expect(s.listViews(origin: 'local').any((v) => v.trashed), isFalse);
    expect(parentOf(s, page), folder,
        reason: 'a live parent is kept — no silent reparenting');

    final purged = s.purgeViewSubtree(viewId: folder);
    expect(purged.toSet(), {folder, page});
    expect(s.listViews(origin: 'local'), isEmpty);
    expect(s.loadDoc(docId: 'd_p'), isNull, reason: 'the document went too');
  });

  test('restoring under a still-trashed parent lifts to the top level', () {
    final s = freshStore();
    final parent = s.createView(
        workspaceId: 'ws', objectId: 'd_a', name: 'A', objectType: 'folder');
    final child = s.createView(
        workspaceId: 'ws',
        parentId: parent,
        objectId: 'd_b',
        name: 'B',
        objectType: 'document');

    s.trashViewSubtree(viewId: parent);
    s.restoreViewSubtree(viewId: child);

    expect(parentOf(s, child), isNull,
        reason: 'restoring into a trashed parent would leave it unreachable');
    expect(
      s.listViews(origin: 'local').firstWhere((v) => v.id == parent).trashed,
      isTrue,
    );
  });
}
