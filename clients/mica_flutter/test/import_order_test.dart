import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/upload/import_order.dart';

void main() {
  test('manifest order wins over alphabetical order', () {
    final manifest = jsonEncode({
      'version': 1,
      'pages': [
        {'path': 'Zebra.md', 'title': 'Zebra'},
        {'path': 'Zebra/Inner.md', 'title': 'Inner'},
        {'path': 'Apple.md', 'title': 'Apple'},
      ],
    });
    expect(
      orderPagePaths(['Apple.md', 'Zebra/Inner.md', 'Zebra.md'], manifest),
      ['Zebra.md', 'Zebra/Inner.md', 'Apple.md'],
    );
  });

  test('files unknown to the manifest follow, parents-first', () {
    final manifest = jsonEncode({
      'version': 1,
      'pages': [
        {'path': 'B.md', 'title': 'B'},
        {'path': 'A.md', 'title': 'A'},
      ],
    });
    expect(
      orderPagePaths(
          ['New/Deep.md', 'A.md', 'New.md', 'B.md', 'Extra.md'], manifest),
      ['B.md', 'A.md', 'Extra.md', 'New.md', 'New/Deep.md'],
    );
  });

  test('no manifest: depth then natural sort', () {
    expect(
      orderPagePaths(
          ['b/10.md', 'b/2.md', 'b.md', '10 篇.md', '2 篇.md'], null),
      ['2 篇.md', '10 篇.md', 'b.md', 'b/2.md', 'b/10.md'],
    );
  });

  test('malformed manifest falls back gracefully', () {
    expect(
      orderPagePaths(['b.md', 'a.md'], '{not json'),
      ['a.md', 'b.md'],
    );
    expect(
      orderPagePaths(['b.md', 'a.md'], '{"pages": "nope"}'),
      ['a.md', 'b.md'],
    );
  });

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

  test('folderPageIndex associates Notion ID-suffixed pages with folders', () {
    final idx = folderPageIndex([
      'apple 31f57556969b56ade626c2502854fc6d.md',
      'apple/iphone 31f57556969b81b5973cf30d40c5b6f1.md',
      '公司项目环境 31f57556969b8052b86dd24762a25b14.md',
      'Guide.md', // plain layout still works
      'Guide/Setup.md',
    ]);
    expect(idx['apple'], 'apple 31f57556969b56ade626c2502854fc6d.md');
    expect(idx['apple/iphone'],
        'apple/iphone 31f57556969b81b5973cf30d40c5b6f1.md');
    expect(idx['公司项目环境'], '公司项目环境 31f57556969b8052b86dd24762a25b14.md');
    expect(idx['Guide'], 'Guide.md');
    expect(idx['Guide/Setup'], 'Guide/Setup.md');
  });

  test('normalizeNotionPath strips IDs per segment', () {
    expect(
      normalizeNotionPath(
          'apple 31f57556969b56ade626c2502854fc6d/sub 0123456789abcdef0123456789abcdef'),
      'apple/sub',
    );
    expect(normalizeNotionPath('a/b'), 'a/b');
  });

  test('naturalCompare orders digit runs numerically', () {
    expect(naturalCompare('v2', 'v10'), lessThan(0));
    expect(naturalCompare('v10', 'v10'), 0);
    expect(naturalCompare('v10a', 'v10'), greaterThan(0));
    expect(naturalCompare('第2章', '第10章'), lessThan(0));
  });
}
