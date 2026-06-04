import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/upload/notion.dart';

void main() {
  test('stripNotionId removes Notion export ID suffixes', () {
    expect(
      stripNotionId('My Page 1f2e3d4c5b6a7890abcdef1234567890'),
      'My Page',
    );
    expect(
      stripNotionId('读书笔记 0123456789abcdef0123456789ABCDEF'),
      '读书笔记',
    );
    expect(
      stripNotionId('Export-1f2e3d4c-5b6a-7890-abcd-ef1234567890'),
      'Export',
    );
    // Ordinary names are untouched.
    expect(stripNotionId('Guide'), 'Guide');
    expect(stripNotionId('2024 总结'), '2024 总结');
    expect(stripNotionId('deadbeef'), 'deadbeef');
  });

  test('looksLikeNotionExport detects ID-suffixed archives', () {
    expect(
      looksLikeNotionExport([
        'apple 31f57556969b56ade626c2502854fc6d.md',
        'apple/iphone 31f57556969b81b5973cf30d40c5b6f1.md',
        'notes.md', // one plain file doesn't flip the verdict
      ]),
      isTrue,
    );
    // A standard archive with one hash-like name is NOT Notion.
    expect(
      looksLikeNotionExport([
        'Guide.md',
        'Notes.md',
        'hashes/build 0123456789abcdef0123456789abcdef.md',
      ]),
      isFalse,
    );
    expect(looksLikeNotionExport(['Guide.md']), isFalse);
    expect(looksLikeNotionExport([]), isFalse);
  });

  group('folderPageIndex', () {
    test('notion mode matches folders to ID-suffixed pages', () {
      final idx = folderPageIndex([
        'apple 31f57556969b56ade626c2502854fc6d.md',
        'apple/iphone 31f57556969b81b5973cf30d40c5b6f1.md',
        '公司项目环境 31f57556969b8052b86dd24762a25b14.md',
        'Guide.md', // plain layout still works in notion mode
        'Guide/Setup.md',
      ], notion: true);
      expect(idx['apple'], 'apple 31f57556969b56ade626c2502854fc6d.md');
      expect(idx['apple/iphone'],
          'apple/iphone 31f57556969b81b5973cf30d40c5b6f1.md');
      expect(
          idx['公司项目环境'], '公司项目环境 31f57556969b8052b86dd24762a25b14.md');
      expect(idx['Guide'], 'Guide.md');
      expect(idx['Guide/Setup'], 'Guide/Setup.md');
    });

    test('standard mode is exact — hash-like names stay intact', () {
      final idx = folderPageIndex([
        'build 0123456789abcdef0123456789abcdef.md',
        'Guide.md',
      ], notion: false);
      expect(idx['build 0123456789abcdef0123456789abcdef'],
          'build 0123456789abcdef0123456789abcdef.md');
      expect(idx['build'], isNull);
      expect(idx['Guide'], 'Guide.md');
    });
  });
}
