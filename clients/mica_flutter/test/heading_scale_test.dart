import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/render.dart';

// H4/H5/H6 all used to render at 17px — indistinguishable. The ramp is now
// strictly monotonic so size gives a rough hierarchy (the gutter H1..H6 badge
// carries the exact level).
void main() {
  double sizeOf(int level) => EditorTheme.styleFor(
    EditorNode(id: 'h', kind: 'heading', text: 'x', data: {'level': level}),
  ).fontSize!;

  test('heading font sizes are strictly decreasing across all 6 levels', () {
    final sizes = [for (var l = 1; l <= 6; l++) sizeOf(l)];
    expect(sizes, [30, 24, 20, 18, 16, 15]);
    for (var i = 0; i < sizes.length - 1; i++) {
      expect(
        sizes[i] > sizes[i + 1],
        isTrue,
        reason: 'H${i + 1} (${sizes[i]}) must be larger than H${i + 2} (${sizes[i + 1]})',
      );
    }
  });
}
