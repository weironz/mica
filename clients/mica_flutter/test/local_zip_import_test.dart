// Local-mode workspace ZIP import, through the REAL FFI engine: Rust reads
// and normalizes the archive the way the server import does (wrapper peeled,
// nested ZIP expanded), and the entries land through the same vault import
// the folder picker uses — pages, folders and the image in the blob store.
//
// Before this the local "import workspace ZIP" entry was hidden: the client
// could pack a ZIP but not read one.
@TestOn('vm')
library;

import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/import/local_zip_import.dart';
import 'package:mica_flutter/local/local_offline.dart';
import 'package:mica_flutter/src/rust/api/zip.dart' as zip_ffi;

import 'support/native_build.dart';

Uint8List _zip(Map<String, List<int>> files) => zip_ffi.buildStoreZip(
  entries: [
    for (final e in files.entries)
      zip_ffi.StoreZipEntry(name: e.key, bytes: Uint8List.fromList(e.value)),
  ],
);

void main() {
  late Directory root;
  late LocalOffline local;

  setUpAll(() async {
    await initCheckedNativeRust();
  });

  setUp(() async {
    root = Directory.systemTemp.createTempSync('local_zip_import');
    local = LocalOffline(rootDirOverride: root.path);
    await local.open();
  });

  tearDown(() {
    try {
      root.deleteSync(recursive: true);
    } catch (_) {}
  });

  test('a vault ZIP becomes pages, a folder and a stored image', () async {
    final png = [0x89, 0x50, 0x4e, 0x47, 1, 2, 3];
    final bytes = _zip({
      'Vault/A.md': utf8.encode('# A\n\n![pic](img.png)\n'),
      'Vault/sub/B.md': utf8.encode('# B\n\nsee [A](../A.md)\n'),
      'Vault/img.png': png,
      'Vault/.obsidian/app.json': utf8.encode('{}'),
    });

    expect(local.canUnpackZip, isTrue);
    final unpacked = await local.unpackImportZip(bytes);
    expect(unpacked.wrapperName, 'Vault');
    expect(unpacked.markdownCount, 2);
    expect(
      unpacked.entries.map((e) => e.path).toSet(),
      {'A.md', 'sub/B.md', 'img.png'},
    );

    final ws = local.createLocalWorkspace('Vault');
    final result = await local.importVaultTree(unpacked.entries, ws);
    expect(result.errors, isEmpty);
    expect(result.docs, 2);
    expect(result.folders, 1);

    final views = local
        .listViews()
        .where((v) => v.workspaceId == ws && !v.trashed)
        .toList();
    final names = {for (final v in views) v.name: v};
    expect(names.keys.toSet(), containsAll(['A', 'B', 'sub']));
    expect(names['B']!.parentId, names['sub']!.id);

    // The image is in the on-device blob store, and page A points at it
    // rather than at the archive path.
    final blobs = Directory(
      '${root.path}/local/blobs',
    ).listSync().whereType<File>().toList();
    expect(blobs, hasLength(1));
    final doc = local.openDoc(names['A']!.objectId)!;
    final image = doc.blocks.firstWhere((b) => b['type'] == 'image');
    final data = image['data'] as Map;
    expect(data['file_id'], blobs.single.uri.pathSegments.last);
  });

  test('a nested ZIP is expanded, not stored as an attachment', () async {
    final inner = _zip({'Inner.md': utf8.encode('# Inner\n')});
    final unpacked = await local.unpackImportZip(
      _zip({'Top.md': utf8.encode('# Top\n'), 'part.zip': inner}),
    );
    expect(unpacked.markdownCount, 2);
    expect(unpacked.entries.any((e) => e.path.endsWith('.zip')), isFalse);
  });

  test('an archive with no pages reports zero instead of importing', () async {
    final unpacked = await local.unpackImportZip(
      _zip({'notes.txt.png': const [1, 2, 3]}),
    );
    expect(unpacked.markdownCount, 0);
  });

  test('a file that is not a ZIP is an error', () async {
    await expectLater(
      local.unpackImportZip(Uint8List.fromList(utf8.encode('not a zip'))),
      throwsA(anything),
    );
  });

  test('the workspace is named after the file, then the wrapper', () {
    expect(localZipWorkspaceName('Notes.zip', 'Vault'), 'Notes');
    expect(
      localZipWorkspaceName(
        'Export-0123456789abcdef0123456789abcdef.zip',
        null,
      ),
      'Export',
    );
    expect(localZipWorkspaceName('.zip', 'Vault'), 'Vault');
    expect(localZipWorkspaceName('.zip', null), 'Imported');
  });
}
