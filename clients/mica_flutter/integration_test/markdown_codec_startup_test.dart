import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:mica_flutter/main.dart' as app;
import 'package:mica_flutter/markdown/markdown_codec.dart';
import '../test/support/native_build.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  testWidgets('desktop main initializes codec before the application frame', (
    tester,
  ) async {
    await initCheckedNativeRust();
    app.main();
    for (
      var i = 0;
      i < 40 && find.byType(app.MicaApp).evaluate().isEmpty;
      i++
    ) {
      await tester.pump(const Duration(milliseconds: 100));
    }
    expect(find.byType(app.MicaApp), findsOneWidget);
    expect(markdownCodec, isNot(isA<UnavailableMarkdownCodec>()));
    expect(markdownCodec.engineInfo.schemaVersion, 1);
    expect(markdownCodec.parseInline('**desktop**').text, 'desktop');
  });
}
