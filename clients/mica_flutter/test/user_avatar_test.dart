import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/ui/theme_tokens.dart';
import 'package:mica_flutter/ui/user_avatar.dart';

/// The one circle that stands for a person, and it had no tests.
///
/// Written after mistaking a correct render for a bug: the account tile showed a
/// solid red circle with no letter, which looks exactly like "the fallback is
/// broken". It was not — that dev account's stored avatar really is an 8x8 solid
/// #DC2626 PNG (a fixture from the avatar-upload smoke test), and
/// `foregroundImage` paints over the child by design. So the interesting thing to
/// pin is not only that the initial appears, but WHEN it is supposed to be
/// covered.
void main() {
  Widget host(Widget child, {MicaTokens tokens = MicaTokens.light}) =>
      MaterialApp(
        home: MicaTheme(
          tokens: tokens,
          child: Scaffold(body: Center(child: child)),
        ),
      );

  testWidgets('no url renders the fallback initial', (tester) async {
    // Null url is not a failed load — it means the user never set a picture, and
    // the letter is the real, intended rendering.
    await tester.pumpWidget(host(const UserAvatar(url: null, fallback: 'W')));

    expect(find.text('W'), findsOneWidget);
  });

  testWidgets('a picture covers the initial rather than sitting beside it', (
    tester,
  ) async {
    // `foregroundImage`, not `backgroundImage`. The letter stays mounted
    // underneath so a slow load never shows an empty circle — which is exactly
    // why a fully opaque avatar legitimately hides it.
    await tester.pumpWidget(
      host(
        const UserAvatar(url: 'https://example.invalid/a.png', fallback: 'W'),
      ),
    );

    final avatar = tester.widget<CircleAvatar>(find.byType(CircleAvatar));
    expect(avatar.foregroundImage, isNotNull);
    expect(find.text('W'), findsOneWidget, reason: 'it stays underneath');
  });

  testWidgets('a load that fails still shows the initial, never a broken glyph', (
    tester,
  ) async {
    // An avatar is decoration. A broken-image icon in the corner of the sidebar
    // reads as "the app is broken" when the only thing wrong is one missing file.
    await tester.pumpWidget(
      host(
        const UserAvatar(url: 'https://example.invalid/nope.png', fallback: 'W'),
      ),
    );
    await tester.pump();

    expect(tester.takeException(), isNull);
    expect(find.text('W'), findsOneWidget);
    expect(find.byIcon(Icons.broken_image), findsNothing);
  });

  group('the initial is legible against the circle it sits on', () {
    double luminance(Color c) {
      double channel(double v) => v <= 0.03928
          ? v / 12.92
          : math.pow((v + 0.055) / 1.055, 2.4).toDouble();
      return 0.2126 * channel(c.r) +
          0.7152 * channel(c.g) +
          0.0722 * channel(c.b);
    }

    double contrast(Color fg, Color bg) {
      final a = luminance(fg), b = luminance(bg);
      final hi = math.max(a, b), lo = math.min(a, b);
      return (hi + 0.05) / (lo + 0.05);
    }

    for (final (name, tokens) in [
      ('light', MicaTokens.light),
      ('dark', MicaTokens.dark_),
    ]) {
      testWidgets('in $name mode', (tester) async {
        // The defaults are nullable so they can name token ROLES; a default
        // parameter has to be a constant, and a token is read from the tree. The
        // risk that introduces is the two roles drifting until the letter
        // disappears into its own circle — so assert the pair, per palette.
        await tester.pumpWidget(
          host(const UserAvatar(url: null, fallback: 'W'), tokens: tokens),
        );

        final avatar = tester.widget<CircleAvatar>(find.byType(CircleAvatar));
        final text = tester.widget<Text>(find.text('W'));
        final bg = avatar.backgroundColor!;
        final fg = text.style!.color!;

        expect(
          contrast(fg, bg),
          greaterThanOrEqualTo(4.5),
          reason: '$name: an initial you cannot read is the same as no initial',
        );
      });
    }
  });
}
