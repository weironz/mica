import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'package:mica_flutter/ui/search_dialog.dart';

// Before anything is typed, the search dialog offers the OTHER workspaces as
// one-click buttons: switching workspace is as frequent as searching, and the
// dialog already switches when you type a name.

void main() {
  Future<List<String>> pumpDialog(
    WidgetTester tester, {
    bool canSwitch = true,
  }) async {
    final switched = <String>[];
    await tester.pumpWidget(
      MaterialApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        locale: const Locale('zh'),
        home: Scaffold(
          body: SearchDialog(
            onSearch: (_) async => const [],
            onOpen: (_, _, {inNewTab = false}) {},
            onReveal: (_, _) {},
            workspaceId: 'here',
            workspaceName: 'Here',
            workspaces: const [
              (id: 'here', name: 'Here'),
              (id: 'work', name: 'Work log'),
              (id: 'books', name: 'Reading notes'),
            ],
            onOpenWorkspace: canSwitch ? switched.add : null,
          ),
        ),
      ),
    );
    await tester.pump();
    return switched;
  }

  testWidgets('other workspaces are one click away before typing', (
    tester,
  ) async {
    final switched = await pumpDialog(tester);
    // The current workspace is not offered: you are already there.
    expect(find.widgetWithText(OutlinedButton, 'Here'), findsNothing);
    expect(find.widgetWithText(OutlinedButton, 'Work log'), findsOneWidget);
    await tester.tap(find.widgetWithText(OutlinedButton, 'Reading notes'));
    expect(switched, ['books']);
  });

  testWidgets('no switching (local world), no buttons', (tester) async {
    await pumpDialog(tester, canSwitch: false);
    expect(find.byType(OutlinedButton), findsNothing);
  });

  testWidgets('typing hides the buttons', (tester) async {
    await pumpDialog(tester);
    await tester.enterText(find.byType(TextField), 'zz');
    await tester.pump(const Duration(seconds: 1));
    expect(find.widgetWithText(OutlinedButton, 'Work log'), findsNothing);
  });
}
