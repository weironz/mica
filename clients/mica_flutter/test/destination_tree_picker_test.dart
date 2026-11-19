import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'package:mica_flutter/ui/destination_tree_picker.dart';
import 'package:mica_flutter/ui/transfer_dialog.dart';

// gh#183 + gh#186: sidebar-ordered destination picker with auto-expand on
// folder tap and exact sibling positioning (inside / before / after / root top).

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
  testWidgets('gh#186: tapping folder name auto-expands and selects it; chevron toggles', (
    t,
  ) async {
    DestinationPick current = DestinationPick.root;
    final picked = <DestinationPick>[];
    await t.pumpWidget(
      _host(
        StatefulBuilder(
          builder: (context, setState) => DestinationTreePicker(
            views: _tree,
            selection: current,
            onSelected: (p) {
              picked.add(p);
              setState(() => current = p);
            },
            rootLabel: '根目录',
            filterHint: '筛选',
          ),
        ),
      ),
    );
    // Top level by position: 项目 (1), 归档 (2), then the root-level page.
    expect(_labels(t), ['根目录', '项目', '归档', '收件箱']);

    // Clicking the folder name auto-expands it AND selects it.
    await t.tap(find.text('项目'));
    await t.pump();
    expect(_labels(t), ['根目录', '项目', '设计', '发布清单', '归档', '收件箱']);
    expect(picked.last, const DestinationPick(parentViewId: 'a'));

    // Clicking the chevron collapses it back without changing selection.
    await t.tap(
      find.descendant(
        of: find.byKey(const ValueKey('destination-a')),
        matching: find.byIcon(Icons.expand_more),
      ),
    );
    await t.pump();
    expect(_labels(t), ['根目录', '项目', '归档', '收件箱']);

    // Re-tapping folder name expands it again, then tapping nested folder name
    // expands nested children too.
    await t.tap(find.text('项目'));
    await t.pump();
    await t.tap(find.text('设计'));
    await t.pump();
    expect(_labels(t), ['根目录', '项目', '设计', '首页草图', '发布清单', '归档', '收件箱']);
    expect(picked.last, const DestinationPick(parentViewId: 'a1'));
  });

  testWidgets('gh#186: supports exact sibling placement (before / after / root top)', (
    t,
  ) async {
    DestinationPick current = DestinationPick.root;
    await t.pumpWidget(
      _host(
        StatefulBuilder(
          builder: (context, setState) => DestinationTreePicker(
            views: _tree,
            selection: current,
            onSelected: (p) => setState(() => current = p),
            rootLabel: '根目录',
            filterHint: '筛选',
          ),
        ),
      ),
    );

    // Root top chip targets before the first root view ('a').
    await t.tap(find.byKey(const ValueKey('placement-root-top')));
    await t.pump();
    expect(current, const DestinationPick(parentViewId: null, beforeViewId: 'a'));

    // Expand 项目 and tap page 发布清单 -> inserts AFTER 发布清单 inside 项目.
    await t.tap(find.text('项目'));
    await t.pump();
    await t.tap(find.text('发布清单'));
    await t.pump();
    expect(current, const DestinationPick(parentViewId: 'a', afterViewId: 'ap'));

    // Switch to BEFORE 发布清单.
    await t.tap(find.byKey(const ValueKey('placement-before')));
    await t.pump();
    expect(current, const DestinationPick(parentViewId: 'a', beforeViewId: 'ap'));

    // Select folder 归档 -> inside by default, then switch to AFTER 归档 at root.
    await t.tap(find.text('归档'));
    await t.pump();
    expect(current, const DestinationPick(parentViewId: 'b'));
    await t.tap(find.byKey(const ValueKey('placement-after')));
    await t.pump();
    expect(current, const DestinationPick(parentViewId: null, afterViewId: 'b'));
  });

  testWidgets('filtering keeps each hit with its ancestors, expanded', (
    t,
  ) async {
    await t.pumpWidget(
      _host(
        DestinationTreePicker(
          views: _tree,
          selection: DestinationPick.root,
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
    'the transfer dialog previews into the folder or sibling position picked in the tree',
    (t) async {
      final calls = <({String? parent, String? before, String? after, bool dryRun})>[];
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
                  beforeViewId,
                  afterViewId,
                  required dryRun,
                }) async {
                  calls.add((
                    parent: parentViewId,
                    before: beforeViewId,
                    after: afterViewId,
                    dryRun: dryRun,
                  ));
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

      // Tap folder 项目 (auto-expands!) then tap page 发布清单 inside it.
      await t.tap(find.text('项目'));
      await t.pumpAndSettle();
      await t.tap(find.text('发布清单'));
      await t.pumpAndSettle();

      expect(calls, [
        (parent: null, before: null, after: null, dryRun: true),
        (parent: 'a', before: null, after: null, dryRun: true),
        (parent: 'a', before: null, after: 'ap', dryRun: true),
      ]);
    },
  );
}
