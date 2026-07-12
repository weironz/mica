// A new page carries a default name (the server rejects empty view names); the
// title field renders that default — and the legacy English 'Untitled' — as an
// empty placeholder so the page shows a grey hint + caret, not solid text.
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/main.dart';

void main() {
  test('isUntitledPageName treats the default names as untitled', () {
    expect(isUntitledPageName(kUntitledPage), isTrue);
    expect(isUntitledPageName('未命名页面'), isTrue);
    expect(isUntitledPageName('Untitled'), isTrue); // legacy English default
    expect(isUntitledPageName('  Untitled  '), isTrue); // trimmed
  });

  test('a real title is not treated as untitled', () {
    expect(isUntitledPageName('My Notes'), isFalse);
    expect(isUntitledPageName('未命名的心事'), isFalse); // superstring, not the default
    expect(isUntitledPageName(''), isFalse);
  });
}
