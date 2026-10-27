@TestOn('browser')
library;

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/markdown/markdown_codec.dart';

void main() {
  testWidgets('missing wasm installs unavailable codec and permits first frame', (
    tester,
  ) async {
    // This browser test deliberately does not load mica_md.js. It exercises the
    // real initialization failure rather than substituting a Dart parser.
    debugSetMarkdownCodec(null);
    await initMarkdownCodec();
    expect(markdownCodec, isA<UnavailableMarkdownCodec>());
    await tester.pumpWidget(
      const Directionality(
        textDirection: TextDirection.ltr,
        child: Text('Application remains available'),
      ),
    );
    expect(find.text('Application remains available'), findsOneWidget);
    expect(
      () => markdownCodec.parseBlocks('text'),
      throwsA(isA<MarkdownCodecException>()),
    );
    debugSetMarkdownCodec(null);
  });
}
