import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/mermaid_svg_inline.dart';

void main() {
  group('inlineMermaidCss background materialization', () {
    test('emits an opaque bg rect from the root CSS background-color', () {
      // merman puts the canvas colour only in the root <svg> CSS; flutter_svg
      // drops it, so without materializing it the raster is transparent and the
      // diagram shows the (possibly dark) surface behind it.
      const svg = '<svg id="merman" width="100" height="200" '
          'style="max-width:100px;background-color:white" viewBox="0 0 100 200">'
          '<g><rect class="node" x="10" y="10" width="20" height="20"/></g></svg>';
      final out = inlineMermaidCss(svg);
      // A white background rect covering the whole viewBox is injected (the node
      // rect in the fixture carries no fill, so fill="white" is uniquely the bg).
      expect(out, contains('fill="white"'));
      expect(out, contains('width="100.000"'));
      expect(out, contains('height="200.000"'));
    });

    test('handles a non-zero viewBox origin', () {
      const svg = '<svg style="background-color:#ffffff" viewBox="-5 -8 30 40">'
          '<circle r="3"/></svg>';
      final out = inlineMermaidCss(svg);
      expect(out, contains('x="-5.000"'));
      expect(out, contains('y="-8.000"'));
      expect(out, contains('width="30.000"'));
    });

    test('no-op when the canvas is transparent', () {
      const svg = '<svg viewBox="0 0 10 10" style="background-color:transparent">'
          '<rect x="0" y="0" width="5" height="5"/></svg>';
      final out = inlineMermaidCss(svg);
      expect(out, isNot(contains('fill="transparent"')));
    });

    test('no-op when there is no background-color', () {
      const svg = '<svg viewBox="0 0 10 10"><circle r="3"/></svg>';
      final out = inlineMermaidCss(svg);
      // Only the original circle — no injected background rect.
      expect(out, isNot(contains('<rect')));
    });
  });
}
