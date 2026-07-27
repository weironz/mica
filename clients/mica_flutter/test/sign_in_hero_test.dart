import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/ui/sign_in_hero.dart';

/// This is the first screen a new user reads, and the easiest place in the app to
/// promise something the product does not do. What is worth locking in is that
/// every line the caller passes actually reaches the screen — a silently dropped
/// feature line would be invisible in review.
void main() {
  const strings = SignInHeroStrings(
    tagline: '本地优先的协作知识库',
    pitch: '块编辑器 + 实时协作。',
    features: ['第一条', '第二条', '第三条'],
    badge: 'CRDT SYNC · v0.13.0',
  );

  Future<void> pump(
    WidgetTester tester, {
    Size size = const Size(900, 800),
  }) async {
    tester.view.physicalSize = size;
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(
      const MaterialApp(
        home: Scaffold(body: SignInHero(strings: strings)),
      ),
    );
    await tester.pumpAndSettle();
  }

  testWidgets('shows the brand name and every piece of copy', (tester) async {
    await pump(tester);

    expect(find.text('Mica'), findsOneWidget);
    expect(find.text('本地优先的协作知识库'), findsOneWidget);
    expect(find.text('块编辑器 + 实时协作。'), findsOneWidget);
    expect(find.text('CRDT SYNC · v0.13.0'), findsOneWidget);
  });

  testWidgets('renders one row per feature — none silently dropped', (
    tester,
  ) async {
    await pump(tester);

    for (final f in strings.features) {
      expect(find.text(f), findsOneWidget, reason: 'missing feature: $f');
    }
  });

  testWidgets('an empty feature list is fine, not a crash or a stray bullet', (
    tester,
  ) async {
    await tester.pumpWidget(
      const MaterialApp(
        home: Scaffold(
          body: SignInHero(
            strings: SignInHeroStrings(
              tagline: 't',
              pitch: 'p',
              features: [],
              badge: 'b',
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('t'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });

  testWidgets('a short viewport scrolls instead of overflowing', (
    tester,
  ) async {
    // The sign-in screen is a fixed two-pane row; on a laptop the brand half can
    // easily be shorter than its content, and an overflow here would paint a
    // yellow-and-black stripe across the first screen anyone sees.
    await pump(tester, size: const Size(900, 320));

    expect(tester.takeException(), isNull);
    expect(find.byType(SingleChildScrollView), findsOneWidget);
  });
}
