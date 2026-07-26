import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/file_names.dart';

// Two shipped bugs, both about names:
//  1. An imported (re-hosted) image lost its `.png` — its url segment was
//     `<base64>=.`, which CONTAINS a dot but has no extension, and a
//     `contains('.')` test took it verbatim.
//  2. A Chinese file name "wouldn't display" — the url percent-escapes it (it
//     must, to fetch), so showing the raw url read as `%E8%A7%A3…`.
// These pin the rules that fix both. Mirrors the server's tests in
// crates/api-server/src/routes/files.rs.

void main() {
  group('hasUsableFileExt', () {
    test('a real extension counts', () {
      expect(hasUsableFileExt('photo.png'), isTrue);
      expect(hasUsableFileExt('解锁超时.png'), isTrue);
      expect(hasUsableFileExt('a.b.JPEG'), isTrue);
    });

    test('a bare trailing dot does NOT count — this was the bug', () {
      expect(hasUsableFileExt('0QYXMbZ8CjEbSzegIlDdWCGI-zg53UlHuO3v2Vr9X2M=.'),
          isFalse);
      expect(hasUsableFileExt('name.'), isFalse);
    });

    test('no dot, or a non-extension tail, does not count', () {
      expect(hasUsableFileExt('0QYXMbZ8CjEbSzegIlDdWCGI'), isFalse);
      expect(hasUsableFileExt(''), isFalse);
      // Not ASCII-alphanumeric / too long to be an extension.
      expect(hasUsableFileExt('file.p n g'), isFalse);
      expect(hasUsableFileExt('sentence.endswithsomethinglong'), isFalse);
    });
  });

  group('readableUrl', () {
    test('decodes a percent-escaped Chinese name for display', () {
      expect(
        readableUrl(
          'https://mica.example/api/workspaces/w/files/f/blob/%E8%A7%A3%E9%94%81%E8%B6%85%E6%97%B6.png',
        ),
        'https://mica.example/api/workspaces/w/files/f/blob/解锁超时.png',
      );
    });

    test('leaves a plain url alone', () {
      const url = 'https://mica.example/a/photo.png';
      expect(readableUrl(url), url);
    });

    test('malformed escapes fall back to the input instead of throwing', () {
      const bad = 'https://mica.example/a/%E8%A7.png%ZZ';
      expect(readableUrl(bad), bad);
    });
  });
}
