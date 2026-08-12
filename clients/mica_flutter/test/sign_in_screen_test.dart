import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/ui/auth_form.dart';
import 'package:mica_flutter/ui/sign_in_screen.dart';

/// Design 01 is brand-left / form-right, and the shipped screen had them
/// mirrored — with the brand half's own doc comment saying "left" while the
/// wiring put it on the right. These pin the placement so the two cannot drift
/// apart again, and pin the two platform differences (escape hatch, narrow
/// window) that are the whole reason one widget serves both.
void main() {
  /// The surface size is what LayoutBuilder actually sees — wrapping a SizedBox
  /// in the default 800x600 test window just gets clamped, which is how the
  /// first version of this test "proved" the brand half was missing.
  Future<void> pump(
    WidgetTester tester, {
    VoidCallback? onClose,
    String? closeLabel,
    Size size = const Size(1280, 800),
  }) async {
    await tester.binding.setSurfaceSize(size);
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: SignInScreen(
            hero: const ColoredBox(
              color: Color(0xFF0B1220),
              child: Center(child: Text('BRAND')),
            ),
            pane: const Text('FORM'),
            onClose: onClose,
            closeLabel: closeLabel,
          ),
        ),
      ),
    );
  }

  testWidgets('wide: the brand is on the LEFT, the form on the RIGHT', (
    tester,
  ) async {
    await pump(tester);
    final brand = tester.getCenter(find.text('BRAND'));
    final form = tester.getCenter(find.text('FORM'));
    expect(
      brand.dx,
      lessThan(form.dx),
      reason: 'design 01 puts the brand half first',
    );
  });

  testWidgets('the form column keeps a usable width, not half the window', (
    tester,
  ) async {
    await pump(tester);
    // The brand half flexes; the form does not — a 640px-wide form on a wide
    // monitor is a stretched text field, not a form.
    final formWidth = tester
        .getSize(
          find
              .ancestor(
                of: find.text('FORM'),
                matching: find.byType(ColoredBox),
              )
              .first,
        )
        .width;
    expect(formWidth, kSignInFormWidth);
  });

  testWidgets('narrow: the brand is dropped, not squeezed', (tester) async {
    // Two compromised halves are worse than one usable form.
    await pump(tester, size: const Size(700, 800));
    expect(find.text('FORM'), findsOneWidget);
    expect(find.text('BRAND'), findsNothing);
  });

  testWidgets(
    'no close button without onClose — web has nowhere to go back to',
    (tester) async {
      await pump(tester);
      expect(find.byIcon(Icons.close), findsNothing);
    },
  );

  testWidgets('onClose makes it leaveable — desktop sign-in is optional', (
    tester,
  ) async {
    var closed = 0;
    await pump(tester, onClose: () => closed++);
    await tester.tap(find.byIcon(Icons.close));
    expect(closed, 1);
  });

  // Reported 2026-08-12: opening this screen while signed in looked like being
  // logged out. The whole app was behind it — the only way back was an unlabelled
  // × in the corner, which the user did not find until told it was there.
  testWidgets('a label turns the corner × into a button that says what it does',
      (tester) async {
    var closed = 0;
    await pump(tester, onClose: () => closed++, closeLabel: '返回');
    expect(find.text('返回'), findsOneWidget);
    await tester.tap(find.text('返回'));
    expect(closed, 1);
  });

  testWidgets('without a label it stays the bare × (web has nothing to name)',
      (tester) async {
    await pump(tester, onClose: () {});
    expect(find.byIcon(Icons.close), findsOneWidget);
    expect(find.byType(TextButton), findsNothing);
  });

  group('SignedInCard', () {
    const strings = SignedInCardStrings(
      title: '你已经登录了',
      body: 'willmica · mica.example.com',
      action: '返回',
      switchHint: '要去别处的话,在上面选另一个服务器。',
    );

    testWidgets('states WHO and WHERE, and offers the way back', (
      tester,
    ) async {
      // The empty password form it replaces said neither, which is why an
      // intact session read as a logged-out app.
      var back = 0;
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: SignedInCard(strings: strings, onContinue: () => back++),
          ),
        ),
      );
      expect(find.text('你已经登录了'), findsOneWidget);
      expect(find.text('willmica · mica.example.com'), findsOneWidget);
      expect(find.text('要去别处的话,在上面选另一个服务器。'), findsOneWidget);

      await tester.tap(find.widgetWithText(FilledButton, '返回'));
      expect(back, 1);
    });

    testWidgets('asks for nothing — no fields to fill', (tester) async {
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: SignedInCard(strings: strings, onContinue: () {}),
          ),
        ),
      );
      expect(find.byType(TextField), findsNothing);
    });
  });
}
