import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/ime_geometry.dart';

// gh#171: on the web the platform's hidden input must cover only the caret.
// iOS Safari lets it take pointer events; sized to the whole editor it took
// every touch drag, and the selection jumped to wherever the browser put its
// own caret in the invisible text.
void main() {
  const editor = Size(600, 900);
  // The editor sits 30 px right / 80 px down in the window.
  final at = Matrix4.translationValues(30, 80, 0);
  const caret = Rect.fromLTWH(120, 340, 2, 24);

  test('web: a caret-sized box placed at the caret', () {
    final g = imeGeometry(editor, at, caret, web: true);
    expect(g.size, const Size(2, 24));
    expect(
      MatrixUtils.transformPoint(g.transform, Offset.zero),
      caret.topLeft + const Offset(30, 80),
      reason: 'the box starts at the caret, in window coordinates',
    );
    expect(at, Matrix4.translationValues(30, 80, 0), reason: 'input untouched');
    expect(g.caret, const Rect.fromLTWH(0, 0, 2, 24));
  });

  test('a zero-width caret still gets a box', () {
    final g = imeGeometry(
      editor,
      at,
      const Rect.fromLTWH(5, 6, 0, 20),
      web: true,
    );
    expect(g.size.width, greaterThan(0));
  });

  test('desktop: the editor-sized box its native IMEs always had', () {
    final g = imeGeometry(editor, at, caret, web: false);
    expect(g.size, const Size(600, 900));
    expect(
      MatrixUtils.transformPoint(g.transform, Offset.zero),
      const Offset(30, 80),
    );
    expect(g.caret, caret);
  });
}
