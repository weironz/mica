import 'package:flutter/material.dart';
import 'package:flutter_localizations/flutter_localizations.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'package:mica_flutter/l10n/locale_controller.dart';
import 'package:mica_flutter/ui/home_pane.dart';

/// The greeting has to cope with having nobody to name. The local world has no
/// account, and the pane was handed the world's own label in place of a name —
/// so home greeted the user as 「下午好，本地模式」, addressing them by the name of a
/// mode. Caught on a real machine rather than by a test, which is why this one
/// exists.
void main() {
  Future<void> pumpPane(
    WidgetTester tester, {
    required String userName,
    required DateTime now,
    Locale locale = const Locale('zh'),
  }) async {
    await tester.pumpWidget(
      MaterialApp(
        locale: locale,
        localizationsDelegates: const [
          AppLocalizations.delegate,
          GlobalMaterialLocalizations.delegate,
          GlobalWidgetsLocalizations.delegate,
          GlobalCupertinoLocalizations.delegate,
        ],
        supportedLocales: kSupportedLocales,
        home: Scaffold(
          body: Builder(
            builder: (context) => buildHomePane(
              context,
              userName: userName,
              viewsByWorkspace: const {},
              workspaceNames: const {},
              onCreatePage: () {},
              onOpenView: (_) {},
              now: now,
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
  }

  final afternoon = DateTime(2026, 7, 27, 14, 30);

  testWidgets('a named user is greeted by name', (tester) async {
    await pumpPane(tester, userName: '林深', now: afternoon);

    expect(find.text('下午好，林深'), findsOneWidget);
  });

  testWidgets('no name greets without one — never a trailing comma', (
    tester,
  ) async {
    await pumpPane(tester, userName: '', now: afternoon);

    expect(find.text('下午好'), findsOneWidget);
    // The failure mode being locked out: an empty name interpolated into the
    // named form, leaving 「下午好，」 hanging.
    expect(find.text('下午好，'), findsNothing);
  });

  testWidgets('whitespace is not a name', (tester) async {
    await pumpPane(tester, userName: '   ', now: afternoon);

    expect(find.text('下午好'), findsOneWidget);
  });

  testWidgets('the time-of-day boundaries pick the right greeting', (
    tester,
  ) async {
    await pumpPane(tester, userName: '', now: DateTime(2026, 7, 27, 5, 0));
    expect(find.text('早上好'), findsOneWidget);

    await pumpPane(tester, userName: '', now: DateTime(2026, 7, 27, 11, 59));
    expect(find.text('早上好'), findsOneWidget);

    await pumpPane(tester, userName: '', now: DateTime(2026, 7, 27, 18, 0));
    expect(find.text('晚上好'), findsOneWidget);

    // Before 05:00 reads as evening rather than morning — someone up at 3am is
    // still in their night, not their morning.
    await pumpPane(tester, userName: '', now: DateTime(2026, 7, 27, 3, 0));
    expect(find.text('晚上好'), findsOneWidget);
  });

  testWidgets('English falls back to the nameless form too', (tester) async {
    await pumpPane(
      tester,
      userName: '',
      now: afternoon,
      locale: const Locale('en'),
    );

    expect(find.text('Good afternoon'), findsOneWidget);
  });
}
