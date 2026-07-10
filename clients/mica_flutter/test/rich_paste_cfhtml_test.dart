import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/rich_paste_stub.dart';

void main() {
  group('stripCfHtmlHeader', () {
    test('strips the Windows CF_HTML descriptor header', () {
      const cf = 'Version:0.9\r\n'
          'StartHTML:00000097\r\n'
          'EndHTML:00012796\r\n'
          'StartFragment:00000132\r\n'
          'EndFragment:00012760\r\n'
          'SourceURL:https://example.com\r\n'
          '<!DOCTYPE html><html><body><!--StartFragment-->'
          '<p>你好 hi</p><!--EndFragment--></body></html>';
      final out = stripCfHtmlHeader(cf);
      expect(out, startsWith('<!DOCTYPE html>'));
      expect(out, isNot(contains('Version:')));
      expect(out, isNot(contains('StartFragment:')));
      // Non-ASCII survives intact — proof that cutting at the first `<` is right,
      // where a char-index slice by the (byte-offset) StartFragment would not be.
      expect(out, contains('<p>你好 hi</p>'));
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
}
