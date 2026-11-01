// A single-page export is a `.zip` exactly when the page has an image the
// `.md` alone cannot carry (lib/ui/export_shape.dart). Decided from blocks,
// not by scanning Markdown text — an image example inside a code block is
// code, not an image.
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/ui/export_shape.dart';

Map<String, Object?> image(Map<String, Object?> data) =>
    {'type': 'image', 'text': '', 'data': data};

void main() {
  test('a stored or relative image needs the bundle', () {
    expect(pageNeedsAssetBundle([image({'file_id': 'abc'})]), isTrue);
    expect(pageNeedsAssetBundle([image({'url': 'assets/a.png'})]), isTrue);
  });

  test('remote images and plain pages export as .md', () {
    expect(pageNeedsAssetBundle([image({'url': 'https://x.y/a.png'})]), isFalse);
    expect(pageNeedsAssetBundle([image({'url': 'data:image/png;base64,AA'})]), isFalse);
    expect(pageNeedsAssetBundle([image({'file_id': ''})]), isFalse);
    expect(pageNeedsAssetBundle(const []), isFalse);
  });

  test('image syntax inside a code block is not an image', () {
    expect(
      pageNeedsAssetBundle([
        {'type': 'code_block', 'text': '![x](local.png)', 'data': {}},
        {'type': 'paragraph', 'text': '![y](z.png)', 'data': {}},
      ]),
      isFalse,
    );
  });
}
