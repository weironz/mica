// A new page carries a default name (the server rejects empty view names); the
// title field renders that default as an empty placeholder, so the page shows a
// grey hint + caret rather than solid text.
//
// There are TWO spellings and neither is legacy: the default is written by
// whichever client created the page (`l10n.untitledPage`), so a Chinese client
// saves 未命名页面 and an English one saves Untitled. Both have to keep reading
// as untouched — including across clients, since one workspace can be edited
// from both.
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/main.dart';

void main() {
  test('isUntitledPageName treats both language defaults as untitled', () {
    expect(isUntitledPageName('未命名页面'), isTrue); // zh client's default
    expect(isUntitledPageName('Untitled'), isTrue); // en client's default
    expect(isUntitledPageName('  Untitled  '), isTrue); // trimmed
  });

  test('a real title is not treated as untitled', () {
    expect(isUntitledPageName('My Notes'), isFalse);
    expect(isUntitledPageName('未命名的心事'), isFalse); // superstring, not the default
    expect(isUntitledPageName(''), isFalse);
  });
}
