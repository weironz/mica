import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/local/cache_stats.dart';

/// One directory holds both re-downloadable mirrors and the only copy of a
/// local-only image. Getting this predicate wrong in one direction costs a
/// re-download; in the other it loses an original — so "not sure" must fall to
/// local.
void main() {
  test('a server file id (UUID) is a mirror', () {
    expect(isCloudMirrorBlobId('3f2504e0-4f89-11d3-9a0c-0305e82c3301'), isTrue);
    // Upper-case hex is still a UUID.
    expect(isCloudMirrorBlobId('3F2504E0-4F89-11D3-9A0C-0305E82C3301'), isTrue);
  });

  test('a local sha256 id is NOT a mirror', () {
    // 64 unbroken hex characters — the content-addressed local form.
    const sha =
        'e3b0c44298fc1c149afbf4c8996fb924'
        '27ae41e4649b934ca495991b7852b855';
    expect(sha.length, 64);
    expect(isCloudMirrorBlobId(sha), isFalse);
  });

  test('anything unrecognised falls to local — the safe direction', () {
    // Losing an original is worse than a needless re-download, so only a shape
    // positively recognised as a server id may be treated as disposable.
    for (final id in [
      '',
      'not-an-id',
      '3f2504e0-4f89-11d3-9a0c',
      '3f2504e04f8911d39a0c0305e82c3301',
      '3f2504e0-4f89-11d3-9a0c-0305e82c3301-extra',
      'zzzzzzzz-4f89-11d3-9a0c-0305e82c3301',
    ]) {
      expect(isCloudMirrorBlobId(id), isFalse, reason: 'id=$id');
    }
  });

  test('dashes must sit in the UUID positions, not merely be present', () {
    // Right length, right characters, wrong layout.
    expect(isCloudMirrorBlobId('3f2504e04-f89-11d3-9a0c-0305e82c330'), isFalse);
  });

  test('length alone is not enough', () {
    // 36 characters, no dashes at all.
    expect(isCloudMirrorBlobId('a' * 36), isFalse);
  });
}
