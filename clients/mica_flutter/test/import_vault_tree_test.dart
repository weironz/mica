// Issue #86, gap 3: the vault import used to be reachable ONLY by the manual
// Windows integration test (integration_test/vault_import_test.dart, `-d
// windows`), because `LocalOffline.open()` goes through the native
// FlutterRustBridge — which plain `flutter test` cannot load. The Dart half of
// the import is real logic though, so it now runs HERE against a fake
// `MicaStore` (the same fake-the-FFI-interfaces pattern as
// local_doc_append_heal_test.dart), with the blob CAS writing a real temp dir:
//
//   - dot-directories are dropped before anything is stored,
//   - non-Markdown entries become content-addressed blobs and the
//     path → blob-id map crosses to the store for reference rewiring,
//   - a blob that fails to store is NAMED in the result, not swallowed,
//   - the store's outcome (docs/folders/errors) is merged with the local
//     blob errors.
// File selection itself needs no mocking: entries arrive pre-walked (that is
// the production contract — the picker's walk lives outside this function).
// The tree planning itself (Rust, `MicaStore.importVaultTree`) stays covered
// by the integration test.
import 'dart:async';
import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/local/local_offline.dart';
import 'package:mica_flutter/src/rust/api/store.dart';
import 'package:mica_flutter/upload/sha256.dart';

/// Records the one FFI call the Dart half makes and answers it on script.
class _FakeStore implements MicaStore {
  List<VaultTreeEntry>? gotEntries;
  String? gotWorkspaceId;
  String? gotParentViewId;
  Map<String, String>? gotAssetIds;
  var answer = const VaultImportOutcome(docs: 0, folders: 0, errors: []);

  @override
  Future<VaultImportOutcome> importVaultTree({
    required List<VaultTreeEntry> entries,
    required String workspaceId,
    String? parentViewId,
    required Map<String, String> assetIds,
  }) async {
    gotEntries = entries;
    gotWorkspaceId = workspaceId;
    gotParentViewId = parentViewId;
    gotAssetIds = assetIds;
    return answer;
  }

  @override
  dynamic noSuchMethod(Invocation invocation) => null;
}

({String path, List<int> bytes}) file(String path, List<int> bytes) =>
    (path: path, bytes: bytes);

void main() {
  late Directory root;
  late LocalOffline local;
  late _FakeStore store;

  setUp(() {
    root = Directory.systemTemp.createTempSync('vault_import_unit');
    local = LocalOffline(rootDirOverride: root.path);
    store = _FakeStore();
    local.debugSetStoreForTesting(store);
  });

  tearDown(() {
    try {
      root.deleteSync(recursive: true);
    } catch (_) {}
  });

  final blobsDir = () => Directory('${root.path}/local/blobs');
  int blobCount() =>
      blobsDir().existsSync() ? blobsDir().listSync().length : 0;

  test('an unopened store fails loudly instead of inventing success', () async {
    final unopened = LocalOffline(rootDirOverride: root.path);
    final result = await unopened
        .importVaultTree([file('a.md', 'x'.codeUnits)], 'local');
    expect(result.docs, 0);
    expect(result.folders, 0);
    expect(result.errors, ['local store not open']);
  });

  test('markdown crosses to the store; dot-dirs never leave the client',
      () async {
    store.answer =
        const VaultImportOutcome(docs: 2, folders: 1, errors: ['store said']);
    final result = await local.importVaultTree([
      file('welcome.md', '# Welcome'.codeUnits),
      file('work/todo.MD', 'uppercase extension counts'.codeUnits),
      file('assets/pic.png', const [1, 2]),
      file('.obsidian/app.json', '{}'.codeUnits),
      file('.git/config', 'x'.codeUnits),
    ], 'local', parentViewId: 'folder-1');

    expect(result.docs, 2);
    expect(result.folders, 1);
    expect(result.errors, ['store said']);

    // Exactly the two Markdown entries, in order, with the container folder
    // forwarded.
    expect(store.gotEntries!.map((e) => e.path).toList(),
        ['welcome.md', 'work/todo.MD']);
    expect(store.gotWorkspaceId, 'local');
    expect(store.gotParentViewId, 'folder-1');

    // The asset went to the CAS and its path → id pair crossed for reference
    // rewiring; the dot-dir files never became blobs and never crossed.
    expect(store.gotAssetIds!.keys, ['assets/pic.png']);
    expect(blobCount(), 1);
    expect(local.hasBlob(store.gotAssetIds!['assets/pic.png']!), isTrue);
  });

  test('an asset that fails to store is named, and the import continues',
      () async {
    // Poison exactly this blob's slot: its content-addressed path pre-exists
    // as a DIRECTORY, so the CAS write throws (and is caught per entry).
    const doomedBytes = [9, 9, 9];
    Directory('${root.path}/local/blobs/${sha256Hex(doomedBytes)}')
        .createSync(recursive: true);

    store.answer = const VaultImportOutcome(docs: 1, folders: 0, errors: []);
    final result = await local.importVaultTree([
      file('ok.md', '# ok'.codeUnits),
      file('assets/broken.png', doomedBytes),
      file('assets/fine.png', const [1]),
    ], 'local');

    expect(result.docs, 1,
        reason: 'the store outcome is merged with the local half');
    expect(
      result.errors.single,
      startsWith('assets/broken.png: '),
      reason: 'a failed blob is named so references are not silently dead',
    );
    expect(
      store.gotAssetIds!.keys,
      ['assets/fine.png'],
      reason: 'the failed blob has no id to rewire against; the good one does',
    );
    // The Markdown still crossed — the import does not abort on a bad asset.
    expect(store.gotEntries!.single.path, 'ok.md');
  });

  test('only dot-dir input imports nothing and stores nothing', () async {
    final result = await local
        .importVaultTree([file('.hidden/only.md', 'x'.codeUnits)], 'local');
    expect(result.docs, 0);
    expect(result.folders, 0);
    expect(result.errors, isEmpty);
    expect(store.gotEntries, isEmpty);
    expect(blobCount(), 0);
  });
}
