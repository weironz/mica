import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'package:mica_flutter/ui/shell_widgets.dart';

// The sidebar's workspace switcher, driven the way the redesign promises: open
// it, type a few letters into the filter, press Enter, land in the first match.
// And the trigger is one line now — no "N pages · world" subtitle under the name.

WorkspaceEntry _entry(String id, String name) => WorkspaceEntry(
  origin: 'https://example.invalid',
  workspace: Workspace(id: id, name: name, ownerId: 'u', role: 'owner'),
  role: 'owner',
);

void main() {
  // Above the filter threshold (8), so the menu grows its filter field.
  final entries = [
    for (var i = 0; i < 9; i++) _entry('w$i', 'Alpha $i'),
    _entry('notes', 'Reading notes'),
  ];

  Future<List<String>> pumpSwitcher(WidgetTester tester) async {
    final selected = <String>[];
    await tester.pumpWidget(
      MaterialApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        locale: const Locale('zh'),
        home: Scaffold(
          body: Align(
            alignment: Alignment.topLeft,
            child: SizedBox(
              width: 240,
              child: WorkspaceSelector(
                entries: entries,
                activeIsLocal: false,
                selectedRef: entries.first.ref,
                cloudEmail: null,
                onSignIn: null,
                onSelect: (e) async => selected.add(e.workspace.id),
                onRename: (_) {},
                onDelete: (_) {},
                onExport: (_, _) {},
                onCreate: () {},
                onImport: null,
                onImportFilesInto: (_) {},
                onImportFolderInto: (_) {},
                onMigrate: null,
                onDetach: null,
                onReorder: (_) {},
              ),
            ),
          ),
        ),
      ),
    );
    return selected;
  }

  testWidgets('Enter in the filter switches to the first match', (
    tester,
  ) async {
    final selected = await pumpSwitcher(tester);
    await tester.tap(find.text('Alpha 0'));
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField), 'read');
    await tester.pump();
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();
    expect(selected, ['notes']);
    // The menu closed behind the choice.
    expect(find.byType(TextField), findsNothing);
  });

  testWidgets('Enter with no match does nothing', (tester) async {
    final selected = await pumpSwitcher(tester);
    await tester.tap(find.text('Alpha 0'));
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField), 'zzz');
    await tester.pump();
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();
    expect(selected, isEmpty);
  });

  testWidgets('the trigger is a single line: no page-count subtitle', (
    tester,
  ) async {
    await pumpSwitcher(tester);
    expect(find.text('Alpha 0'), findsOneWidget);
    expect(find.textContaining('个页面'), findsNothing);
  });
}
