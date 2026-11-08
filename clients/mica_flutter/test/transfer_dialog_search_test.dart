import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'package:mica_flutter/ui/transfer_dialog.dart';

// gh#165: the move/copy destination list could only be scrolled. Typing in
// the field now narrows it by name, and picking an entry runs the dry run
// against THAT workspace.
void main() {
  testWidgets('typing filters destinations; picking previews that one', (
    tester,
  ) async {
    final previewed = <String>[];
    await tester.pumpWidget(
      MaterialApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        locale: const Locale('zh'),
        home: Scaffold(
          body: TransferDialog(
            copy: false,
            destinations: [
              for (final n in ['研发周报', '产品规划', '研发知识库', '个人笔记'])
                (id: 'id-$n', name: n),
            ],
            onLoadFolders: (_) async => const [],
            onTransfer: ({
              required destWorkspaceId,
              required parentViewId,
              required dryRun,
            }) async {
              previewed.add(destWorkspaceId);
              return const TransferReport(
                newRootViewId: '',
                newRootViewIds: [],
                documents: 1,
                folders: 0,
                images: 0,
                danglingLinks: [],
                removedSource: false,
                dryRun: true,
              );
            },
          ),
        ),
      ),
    );
    final field = find.descendant(
      of: find.byKey(const ValueKey('transfer-destination')),
      matching: find.byType(TextField),
    );
    await tester.tap(field);
    await tester.pumpAndSettle();
    await tester.enterText(field, '研发');
    await tester.pumpAndSettle();

    final shown = find
        .descendant(of: find.byType(MenuItemButton), matching: find.byType(Text))
        .evaluate()
        .map((e) => (e.widget as Text).data)
        .toList();
    expect(shown, ['研发周报', '研发知识库'], reason: 'only names containing 研发');

    await tester.tap(find.widgetWithText(MenuItemButton, '研发知识库'));
    await tester.pumpAndSettle();
    expect(previewed, ['id-研发知识库']);
  });
}
