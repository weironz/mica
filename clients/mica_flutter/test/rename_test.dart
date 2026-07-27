import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/ui/rename.dart';

/// Three call sites commit renames (sidebar row, page title, breadcrumb) and all
/// three commit on blur — so "nothing to save" is the common case, not the edge
/// case. Getting it wrong means either a rejected empty name or a write on every
/// visit to the field.
void main() {
  test('a real change is saved, trimmed', () {
    expect(renamedTo('新名字', '旧名字'), '新名字');
    expect(renamedTo('  新名字  ', '旧名字'), '新名字');
  });

  test('blank means "I changed my mind", not "call it nothing"', () {
    // The server rejects an empty name, and a whitespace name renders as a dash.
    expect(renamedTo('', '旧名字'), isNull);
    expect(renamedTo('   ', '旧名字'), isNull);
    expect(renamedTo('\t\n ', '旧名字'), isNull);
  });

  test('unchanged saves nothing', () {
    // Commit-on-blur fires on every visit; a write here would bump updated_at and
    // reshuffle "recently edited" for nothing.
    expect(renamedTo('旧名字', '旧名字'), isNull);
  });

  test('a whitespace-only difference is not a change', () {
    // A trailing space would otherwise "rename" a page to an identical-looking
    // name — a write whose result the user cannot see.
    expect(renamedTo('旧名字 ', '旧名字'), isNull);
    expect(renamedTo(' 旧名字', '旧名字'), isNull);
  });

  test('inner whitespace is preserved — only the ends are trimmed', () {
    expect(renamedTo('  两 个 词  ', 'x'), '两 个 词');
  });

  test('renaming away from a blank current name works', () {
    // A freshly created page can legitimately be sitting on an empty name.
    expect(renamedTo('第一个名字', ''), '第一个名字');
  });

  test('emoji and CJK survive untouched', () {
    expect(renamedTo('📗 读书笔记', '旧'), '📗 读书笔记');
  });
}
