import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'package:mica_flutter/ui/account_menu.dart';
import 'package:mica_flutter/ui/update_controls.dart';

void main() {
  testWidgets(
    'only the nickname is resident; the menu holds identity and actions',
    (tester) async {
      final calls = <String>[];
      final updates = DesktopUpdateController(
        currentVersion: '1.0.0',
        supported: true,
        check: (_) async {
          calls.add('update');
          return null;
        },
      );
      addTearDown(updates.dispose);
      await tester.pumpWidget(
        MaterialApp(
          locale: const Locale('zh'),
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          home: Scaffold(
            body: Align(
              alignment: Alignment.bottomLeft,
              child: SizedBox(
                width: 220,
                child: AccountMenu(
                  name: '昵称',
                  initial: '昵',
                  email: 'demo@example.test',
                  updates: updates,
                  onSettings: () => calls.add('settings'),
                  onSwitchWorld: () => calls.add('world'),
                  onSignOut: () => calls.add('signOut'),
                ),
              ),
            ),
          ),
        ),
      );
      expect(find.text('昵称'), findsOneWidget);
      expect(find.text('demo@example.test'), findsNothing);
      expect(find.byIcon(Icons.more_vert), findsNothing);
      await tester.tap(find.text('昵称'));
      await tester.pumpAndSettle();
      expect(find.text('demo@example.test'), findsOneWidget);
      expect(find.text('检查更新'), findsOneWidget);
      expect(find.byIcon(Icons.settings_outlined), findsOneWidget);
      await tester.tap(find.text('检查更新'));
      await tester.pumpAndSettle();
      expect(calls, ['update']);
      expect(find.text('demo@example.test'), findsNothing);
      expect(find.textContaining('1.0.0'), findsOneWidget);
      expect(tester.takeException(), isNull);
    },
  );
}
