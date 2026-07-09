// S-tier vault import (Obsidian-style "open my folder of .md"): a pre-walked
// tree of files lands in the local store as documents, mirroring the directory
// layout as a page tree — parsing done by the authoritative Rust engine
// (MicaDocument.fromMarkdown). Read-only w.r.t. the source; drives the real FFI
// through the LocalOffline facade, no backend needed.
//
//   flutter test integration_test/vault_import_test.dart -d windows
import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:mica_flutter/local/local_offline.dart';
import 'package:mica_flutter/src/rust/frb_generated.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(() async {
    try {
      await RustLib.init();
    } catch (_) {}
  });

  List<int> md(String s) => utf8.encode(s);

  test('vault import: md files become docs + folder pages, content round-trips',
      () async {
    final dir = Directory.systemTemp.createTempSync('mica_vault');
    final local = LocalOffline(rootDirOverride: dir.path);
    await local.open();

    final result = await local.importVaultTree([
      (path: 'welcome.md', bytes: md('# Welcome\n\nHello **world**')),
      (path: 'work/todo.md', bytes: md('a nested note')),
      (path: 'assets/pic.png', bytes: const [1, 2, 3]), // not .md → ignored
      (path: '.obsidian/app.json', bytes: md('{}')), // dot-dir → ignored
    ], 'local');

    expect(result.docs, 2, reason: 'two .md files imported');
    expect(result.folders, 1, reason: 'one folder-page created (work)');
    expect(result.errors, isEmpty);

    final views = local.listViews();
    final work = views.firstWhere((v) => v.name == 'work');
    final welcome = views.firstWhere((v) => v.name == 'welcome');
    final todo = views.firstWhere((v) => v.name == 'todo');
    expect(welcome.parentId, isNull, reason: 'top-level file');
    expect(todo.parentId, work.id, reason: 'nested file under its folder page');
    // The asset dir and dot-dir produced no views.
    expect(views.any((v) => v.name == 'assets'), isFalse);
    expect(views.any((v) => v.name == 'pic'), isFalse);

    // Content parsed by the authoritative engine and readable back.
    final doc = local.openDoc(welcome.objectId)!;
    expect(
      doc.blocks.any((b) => b['type'] == 'heading' && b['text'] == 'Welcome'),
      isTrue,
      reason: 'heading imported',
    );
    expect(
      // "**world**" is a mark inside data; the plain text stays clean.
      doc.blocks.any((b) => b['text'] == 'Hello world'),
      isTrue,
      reason: 'paragraph text imported clean',
    );

    try {
      dir.deleteSync(recursive: true);
    } catch (_) {
      // The native store holds the SQLite file open (no close() on the facade);
      // on Windows the temp dir can't be removed until the process exits.
      // Best-effort cleanup — the assertions above are what matter.
    }
  });
}
