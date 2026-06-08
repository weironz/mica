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
}
