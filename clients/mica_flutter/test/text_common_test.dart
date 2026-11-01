import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/text_common.dart';

void main() {
  test('prefix and suffix are UTF-16 units and never overlap', () {
    expect(commonPrefix('hello', 'help'), 3);
    expect(commonPrefix('', 'x'), 0);
    expect(commonSuffix('abcabc', 'abc', 3), 0, reason: 'capped by the prefix');
    expect(commonSuffix('xyz-end', 'abc-end', 0), 4);
    // A CJK character is one unit, an emoji two.
    expect(commonPrefix('中文', '中国'), 1);
    expect(commonPrefix('😀a', '😀b'), 2);
  });
}
