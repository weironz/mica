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

  test('naturalCompare orders digit runs numerically', () {
    expect(naturalCompare('v2', 'v10'), lessThan(0));
    expect(naturalCompare('v10', 'v10'), 0);
    expect(naturalCompare('v10a', 'v10'), greaterThan(0));
    expect(naturalCompare('第2章', '第10章'), lessThan(0));
  });
}
