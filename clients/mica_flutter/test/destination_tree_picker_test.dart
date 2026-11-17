import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'package:mica_flutter/ui/destination_tree_picker.dart';
import 'package:mica_flutter/ui/transfer_dialog.dart';

// gh#183: the move/copy folder picker was a dropdown of folder names indented
// with spaces. It is now the sidebar's tree — same order, folders and pages,
// pages not pickable — with a filter that keeps each hit's ancestors.

DocumentView _v(
  String id,
  String name, {
  String? parent,
  bool folder = false,
  String pos = '1',
}) => DocumentView.fromJson({
  'id': id,
  'parent_view_id': parent,
  'object_id': id,
  'object_type': folder ? 'folder' : 'document',
  'name': name,
  'position': pos,
});

final _tree = [
  _v('b', '归档', folder: true, pos: '2'),
  _v('a', '项目', folder: true, pos: '1'),
  _v('a1', '设计', parent: 'a', folder: true, pos: '1'),
  _v('a1p', '首页草图', parent: 'a1', pos: '1'),
  _v('ap', '发布清单', parent: 'a', pos: '2'),
  _v('root-page', '收件箱', pos: '3'),
];

Widget _host(Widget child) => MaterialApp(
  localizationsDelegates: AppLocalizations.localizationsDelegates,
  supportedLocales: AppLocalizations.supportedLocales,
  locale: const Locale('zh'),
  home: Scaffold(body: SizedBox(width: 440, child: child)),
);

List<String> _labels(WidgetTester t) => [
  for (final e
      in find
          .descendant(of: find.byType(ListView), matching: find.byType(Text))
          .evaluate())
    (e.widget as Text).data!,
];

void main() {
  testWidgets('sidebar order, pages shown; expanding reveals children', (
    t,
  ) async {
    final picked = <String?>[];
    await t.pumpWidget(
      _host(
        DestinationTreePicker(
          views: _tree,
          selectedId: null,
          onSelected: picked.add,
          rootLabel: '根目录',
          filterHint: '筛选',
        ),
      ),
    );
    // Top level by position: 项目 (1), 归档 (2), then the root-level page.
    expect(_labels(t), ['根目录', '项目', '归档', '收件箱']);
    await t.tap(
      find.descendant(
        of: find.byKey(const ValueKey('destination-a')),
        matching: find.byIcon(Icons.chevron_right),
      ),
    );
    await t.pump();
    expect(_labels(t), ['根目录', '项目', '设计', '发布清单', '归档', '收件箱']);

    // A page is not a destination; a folder is.
    await t.tap(find.text('发布清单'));
    await t.tap(find.text('设计'));
    await t.tap(find.text('根目录'));
    expect(picked, ['a1', null]);
  });

  testWidgets('filtering keeps each hit with its ancestors, expanded', (
    t,
  ) async {
    await t.pumpWidget(
      _host(
        DestinationTreePicker(
          views: _tree,
          selectedId: null,
          onSelected: (_) {},
          rootLabel: '根目录',
          filterHint: '筛选',
        ),
      ),
    );
    await t.enterText(
      find.byKey(const ValueKey('destination-tree-filter')),
      '草图',
    );
    await t.pump();
    expect(_labels(t), ['项目', '设计', '首页草图']);
  });

  testWidgets(
    'the transfer dialog previews into the folder picked in the tree',
    (t) async {
      final previews = <String?>[];
      await t.pumpWidget(
        _host(
          TransferDialog(
            copy: false,
            destinations: const [(id: 'w2', name: '工作记录')],
            onLoadTree: (_) async => _tree,
            onTransfer:
                ({
                  required destWorkspaceId,
                  required parentViewId,
                  required dryRun,
                }) async {
                  previews.add(parentViewId);
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
      );
      await t.tap(find.byType(TextField).first);
      await t.pumpAndSettle();
      await t.tap(find.widgetWithText(MenuItemButton, '工作记录'));
      await t.pumpAndSettle();
      expect(find.byType(DestinationTreePicker), findsOneWidget);
      await t.tap(find.text('归档'));
      await t.pumpAndSettle();
      expect(previews, [null, 'b']);
    },
  );
}
