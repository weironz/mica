// P2-M5: the on-device image CAS — content-addressed (sha256), offline, dedup.
// Pure file I/O (no FFI), so it runs as a plain VM test against a temp dir.
@TestOn('vm')
library;

import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/local/local_offline.dart';

void main() {
  late Directory tmp;
  late LocalOffline local;

  setUp(() {
    tmp = Directory.systemTemp.createTempSync('mica_cas');
    local = LocalOffline(rootDirOverride: tmp.path);
  });
  tearDown(() => tmp.deleteSync(recursive: true));

  test('put returns sha256 and round-trips bytes', () {
    final bytes = Uint8List.fromList(List.generate(64, (i) => i % 256));
    final id = local.putBlob(bytes);

    // Content id is a 64-hex sha256.
    expect(id, matches(RegExp(r'^[0-9a-f]{64}$')));
    expect(local.hasBlob(id), isTrue);
    expect(local.loadBlob(id), equals(bytes));
    // Stored under {root}/local/blobs/{id}.
    expect(File('${tmp.path}/local/blobs/$id').existsSync(), isTrue);
  });

  test('same bytes dedup to the same id (idempotent)', () {
    final a = Uint8List.fromList([1, 2, 3, 4, 5]);
    final id1 = local.putBlob(a);
    final id2 = local.putBlob(Uint8List.fromList([1, 2, 3, 4, 5]));
    expect(id1, equals(id2), reason: 'content-addressed → identical bytes share a blob');

    // Different bytes → different id.
    final id3 = local.putBlob(Uint8List.fromList([9, 9, 9]));
    expect(id3, isNot(equals(id1)));
  });

  test('missing blob loads as null', () {
    expect(local.hasBlob('deadbeef'), isFalse);
    expect(local.loadBlob('deadbeef'), isNull);
    expect(local.blobFileUri('deadbeef'), isNull);
  });

  test('blobFileUri points at the stored file', () {
    final id = local.putBlob(Uint8List.fromList([7, 7, 7]));
    final uri = local.blobFileUri(id);
    expect(uri, isNotNull);
    expect(uri, startsWith('file://'));
    expect(File.fromUri(Uri.parse(uri!)).existsSync(), isTrue);
  });

  // §7 cloud-mode mirror: cache a downloaded cloud image under its server file
  // id (a UUID, not the content hash) so it re-reads offline from the same CAS.
  test('putBlobAs caches under an explicit (cloud) id and round-trips', () {
    final bytes = Uint8List.fromList([10, 20, 30, 40]);
    const cloudId = 'a1b2c3d4-1111-2222-3333-444455556666'; // UUID-shaped
    expect(local.hasBlob(cloudId), isFalse);

    local.putBlobAs(cloudId, bytes);
    expect(local.hasBlob(cloudId), isTrue);
    expect(local.loadBlob(cloudId), equals(bytes));
    expect(File('${tmp.path}/local/blobs/$cloudId').existsSync(), isTrue);

    // Idempotent: re-caching the same id keeps the original bytes (first wins).
    local.putBlobAs(cloudId, Uint8List.fromList([99]));
    expect(local.loadBlob(cloudId), equals(bytes));
  });

  test('putBlobAs ignores an empty id', () {
    local.putBlobAs('', Uint8List.fromList([1]));
    expect(local.hasBlob(''), isFalse);
  });

  test('UUID-keyed and sha256-keyed blobs coexist without colliding', () {
    final shaId = local.putBlob(Uint8List.fromList([1, 2, 3]));
    const uuid = 'ffffffff-0000-1111-2222-333344445555';
    local.putBlobAs(uuid, Uint8List.fromList([4, 5, 6]));
    expect(local.loadBlob(shaId), equals(Uint8List.fromList([1, 2, 3])));
    expect(local.loadBlob(uuid), equals(Uint8List.fromList([4, 5, 6])));
  });
}
