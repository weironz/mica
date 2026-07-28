// Reclaiming the cache, against a real directory.
//
// The blob half needs no FFI (files under `{root}/local/blobs/`), so this runs as
// a plain VM test on a temp root — which matters, because this is the one code
// path in the app whose bug is "the user's only copy of something is gone".
@TestOn('vm')
library;

import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/local/local_offline.dart';

void main() {
  late Directory tmp;
  late LocalOffline local;

  /// A server file id: `8-4-4-4-12` hex, the shape a mirrored image keeps.
  const mirrorId = '3f2504e0-4f89-11d3-9a0c-0305e82c3301';

  setUp(() {
    tmp = Directory.systemTemp.createTempSync('mica_clear');
    local = LocalOffline(rootDirOverride: tmp.path);
  });
  tearDown(() => tmp.deleteSync(recursive: true));

  Uint8List bytes(int n, int fill) =>
      Uint8List.fromList(List.filled(n, fill & 0xff));

  test('the mirror goes, the local original stays — bytes and all', () async {
    // A local image is content-addressed, so putBlob decides its own id.
    final localId = local.putBlob(bytes(120, 7));
    local.putBlobAs(mirrorId, bytes(300, 9));

    final before = await local.cacheStats(mirroredOrigins: const ['http://s']);
    expect(before.mirroredBytes, 300);
    expect(before.localOnlyBytes, 120);

    final after = await local.clearMirrorCache(
      mirroredOrigins: const ['http://s'],
    );

    expect(after.mirroredBytes, 0, reason: 'the mirror is re-downloadable');
    expect(
      after.localOnlyBytes,
      120,
      reason: 'the local original has nowhere to come back from',
    );
    expect(File('${tmp.path}/local/blobs/$mirrorId').existsSync(), isFalse);
    // Present is not enough — the bytes must still be readable.
    expect(local.loadBlob(localId), equals(bytes(120, 7)));
  });

  test('an id of an unrecognised shape is kept, not swept', () async {
    // "Not sure" must fall on the side of keeping: a needless re-download and a
    // lost original are not the same class of mistake. A future id scheme, or a
    // stray file, must therefore survive a clear.
    local.putBlobAs('legacy-image-id', bytes(64, 3));
    local.putBlobAs(mirrorId, bytes(64, 4));

    final after = await local.clearMirrorCache(
      mirroredOrigins: const ['http://s'],
    );

    expect(
      File('${tmp.path}/local/blobs/legacy-image-id').existsSync(),
      isTrue,
    );
    expect(after.localOnlyBytes, 64);
    expect(after.mirroredBytes, 0);
  });

  test('clearing an empty store is a no-op, not a failure', () async {
    // Reachable: the button is only drawn when there is something to clear, but
    // a second press after the first one finished must not throw.
    final after = await local.clearMirrorCache(
      mirroredOrigins: const ['http://s'],
    );
    expect(after.mirroredBytes, 0);
    expect(after.localOnlyBytes, 0);
    expect(after.mirroredPages, 0);
    expect(after.localOnlyPages, 0);
  });

  test('the numbers it returns are measured, not predicted', () async {
    // The panel reports "freed X" from these numbers, so they have to come from
    // the disk after the fact — a file that could not be unlinked must still be
    // counted, or the app would claim space it did not reclaim.
    local.putBlobAs(mirrorId, bytes(500, 1));
    local.putBlob(bytes(50, 2));

    final after = await local.clearMirrorCache(
      mirroredOrigins: const ['http://s'],
    );
    final recounted = await local.cacheStats(
      mirroredOrigins: const ['http://s'],
    );

    expect(after, equals(recounted));
  });
}
