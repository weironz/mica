import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/rich_paste_stub.dart';

/// A well-formed Windows CF_HTML payload whose header offsets are real UTF-8
/// byte offsets into the full buffer. [pre]/[post] are the context around the
/// selection that a real app emits; only the fragment must be imported.
String cfHtml({
  required String pre,
  required String fragment,
  required String post,
}) {
  const zeros = '0000000000';
  const template = 'Version:0.9\r\n'
      'StartHTML:$zeros\r\n'
      'EndHTML:$zeros\r\n'
      'StartFragment:$zeros\r\n'
      'EndFragment:$zeros\r\n';
  final startHtml = utf8.encode(template).length;
  final payload = '$pre$fragment$post';
  final total = startHtml + utf8.encode(payload).length;
  final startFragment = startHtml + utf8.encode(pre).length;
  final endFragment = startFragment + utf8.encode(fragment).length;
  String pad(int v) => v.toString().padLeft(10, '0');
  return 'Version:0.9\r\n'
      'StartHTML:${pad(startHtml)}\r\n'
      'EndHTML:${pad(total)}\r\n'
      'StartFragment:${pad(startFragment)}\r\n'
      'EndFragment:${pad(endFragment)}\r\n'
      '$payload';
}

void main() {
  group('stripCfHtmlHeader', () {
    test('strips the Windows CF_HTML descriptor header', () {
      final cf = cfHtml(
        pre: '<!DOCTYPE html><html><body><!--StartFragment-->',
        fragment: '<p>你好 hi</p>',
        post: '<!--EndFragment--></body></html>',
      );
      final out = stripCfHtmlHeader(cf);
      expect(out, '<p>你好 hi</p>');
    });

    test('leaves bare HTML (non-Windows clipboards) untouched', () {
      const bare = '<html><body><p>hello</p></body></html>';
      expect(stripCfHtmlHeader(bare), bare);
    });

    test('does not misfire on content that merely starts with "Version:"', () {
      const text = 'Version: 2 of the spec, no StartHTML here';
      expect(stripCfHtmlHeader(text), text);
    });
  });

  // Regression for issue #132: the header was stripped but the fragment bounds
  // were ignored, so selecting `selected` on a page pasted the surrounding page
  // context (`before`, `after`) as extra paragraphs.
  group('CF_HTML fragment selection', () {
    test('imports only the selected fragment, not its page context', () {
      final cf = cfHtml(
        pre: '<html><body>before <!--StartFragment-->',
        fragment: 'selected',
        post: '<!--EndFragment--> after</body></html>',
      );
      final out = stripCfHtmlHeader(cf);
      expect(out, 'selected');
    });

    test('byte offsets survive non-ASCII context before the fragment', () {
      final cf = cfHtml(
        pre: '<html><body>日本語のコンテキスト <!--StartFragment-->',
        fragment: '<b>選択</b>',
        post: '<!--EndFragment--> 後</body></html>',
      );
      expect(stripCfHtmlHeader(cf), '<b>選択</b>');
    });

    test('out-of-range fragment offsets never fall back to the whole HTML', () {
      const zeros = '0000000000';
      const cf = 'Version:0.9\r\n'
          'StartHTML:0000000000\r\n'
          'EndHTML:9999999999\r\n'
          'StartFragment:9999999900\r\n'
          'EndFragment:9999999950\r\n'
          '<html><body>before <!--StartFragment-->selected'
          '<!--EndFragment--> after</body></html>';
      expect(cf, contains(zeros));
      expect(stripCfHtmlHeader(cf), isNull);
    });

    test('a CF_HTML header without fragment bounds is rejected', () {
      const cf = 'Version:0.9\r\n'
          'StartHTML:0000000010\r\n'
          'EndHTML:0000000090\r\n'
          '<html><body>context only</body></html>';
      expect(stripCfHtmlHeader(cf), isNull);
    });

    test('long SourceURL does not hide fragment bounds', () {
      final source = 'SourceURL:https://example.com/${'x' * 900}\r\n';
      const zeros = '0000000000';
      // Some producers emit SourceURL before the fragment fields; a 512-byte
      // header scan then misses StartFragment entirely.
      final template = 'Version:1.0\r\nStartHTML:$zeros\r\nEndHTML:$zeros\r\n'
          '${source}StartFragment:$zeros\r\nEndFragment:$zeros\r\n';
      final start = utf8.encode(template).length;
      const pre = '<html><body>before <!--StartFragment-->';
      const fragment = 'selected';
      const post = '<!--EndFragment--> after</body></html>';
      String pad(int v) => v.toString().padLeft(10, '0');
      final cf = 'Version:1.0\r\nStartHTML:${pad(start)}\r\n'
          'EndHTML:${pad(start + utf8.encode('$pre$fragment$post').length)}\r\n'
          '${source}StartFragment:${pad(start + utf8.encode(pre).length)}\r\n'
          'EndFragment:${pad(start + utf8.encode('$pre$fragment').length)}\r\n'
          '$pre$fragment$post';
      expect(stripCfHtmlHeader(cf), fragment);
    });
  });
}