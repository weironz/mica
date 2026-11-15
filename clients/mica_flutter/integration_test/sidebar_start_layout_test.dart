// Real Windows window + production shell widgets, synthetic content only.
//
// The sidebar's top half after the home page was dropped (2026-11-15): one
// workspace row, one search row, nothing else above the page tree; the start
// pane that replaced home; and the search dialog's one-click workspace row.
// Set MICA_LAYOUT_SCREENSHOT_DIR to keep a 100% screenshot of each state.
import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:mica_flutter/main.dart';
import 'package:mica_flutter/markdown/markdown_codec.dart';
import 'package:mica_flutter/prefs.dart';
import 'package:mica_flutter/ui/font_warmup.dart';
import 'package:mica_flutter/ui/home_pane.dart';
import 'package:mica_flutter/ui/theme_tokens.dart';
import 'package:mica_flutter/window_setup.dart';
import 'package:window_manager/window_manager.dart';

import '../test/support/isolated_profile_io.dart';
import '../test/support/title_workspace.dart';

DocumentView _view(
  String id,
  String name, {
  String? parent,
  bool folder = false,
  int pos = 0,
}) => DocumentView.fromJson({
  'id': id,
  'parent_view_id': parent,
  'object_id': id,
  'object_type': folder ? 'folder' : 'document',
  'name': name,
  'position': '$pos',
  'updated_at': '2026-11-15T08:00:00Z',
});

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  isolateProfileForTests();

  final shot = GlobalKey();
  final outDir = Platform.environment['MICA_LAYOUT_SCREENSHOT_DIR'];

  Future<void> capture(String name) async {
    if (outDir == null) return;
    final boundary =
        shot.currentContext!.findRenderObject() as RenderRepaintBoundary;
    final image = await boundary.toImage(pixelRatio: 1);
    final bytes = await image.toByteData(format: ui.ImageByteFormat.png);
    await File('$outDir/$name.png').writeAsBytes(bytes!.buffer.asUint8List());
    image.dispose();
  }

  final doc = titleBootstrap('周会记录 11-14', id: 'w1114');
  final views = [
    _view('p', '项目', folder: true, pos: 0),
    _view('p1', 'Mica 发布清单', parent: 'p', pos: 0),
    _view('p2', '桌面端冒烟测试', parent: 'p', pos: 1),
    _view('w', '周会', folder: true, pos: 1),
    doc.view,
    _view('w1107', '周会记录 11-07', parent: 'w', pos: 1),
    _view('w1031', '周会记录 10-31', parent: 'w', pos: 2),
    _view('r', '读书笔记', folder: true, pos: 2),
    _view('i', '收件箱', pos: 3),
    _view('n', '灵感', pos: 4),
    _view('d', '部署备忘', pos: 5),
  ];
  const workspaces = [
    Workspace(id: 'workspace', name: '知识库', ownerId: 'user', role: 'owner'),
    Workspace(id: 'work', name: '工作记录', ownerId: 'user', role: 'owner'),
    Workspace(id: 'books', name: '读书笔记', ownerId: 'user', role: 'owner'),
  ];

  Future<void> pumpHost(WidgetTester tester, Widget host) async {
    final app = host as MaterialApp;
    await tester.pumpWidget(
      RepaintBoundary(
        key: shot,
        child: MaterialApp(
          locale: app.locale,
          localizationsDelegates: app.localizationsDelegates,
          supportedLocales: app.supportedLocales,
          theme: MicaTokens.light.toMaterialTheme(),
          builder: micaAppBuilder,
          home: app.home,
        ),
      ),
    );
    await tester.pumpAndSettle();
  }

  testWidgets('sidebar top, start pane and search workspace row', (
    tester,
  ) async {
    savePref('windowWidth', '1280');
    savePref('windowHeight', '800');
    await initDesktopWindow();
    await initMarkdownCodec();
    await warmUpFonts();
    await windowManager.unmaximize();
    await windowManager.setSize(const Size(1280, 800));
    await tester.pumpAndSettle();

    final entries = [
      for (final w in workspaces)
        WorkspaceEntry(origin: 'local', workspace: w, role: 'owner'),
    ];
    final switched = <String>[];
    // 1. A page is open: the sidebar's top half is two rows.
    await pumpHost(
      tester,
      titleWorkspace(
        doc,
        (_, _) async => true,
        views: views,
        workspaces: workspaces,
        entries: entries,
        selectedRef: entries.first.ref,
        onSelectWorkspaceById: switched.add,
        onSearch: (_) async => const [],
      ),
    );
    expect(find.text('首页'), findsNothing);
    expect(find.textContaining('个页面'), findsNothing);
    final search = find.text('Ctrl+Shift+F');
    expect(search, findsOneWidget);
    // Nothing but the workspace row sits above the search row, and the page
    // tree follows right after: well under the ~200 px the old header took.
    expect(tester.getTopLeft(search).dy, lessThan(110));
    expect(tester.getTopLeft(find.text('收件箱')).dy, lessThan(330));
    await capture('sidebar');

    // 2. The search dialog, before typing, offers the other workspaces.
    // Opened from the sidebar box: the shortcut needs a focused descendant,
    // which this synthetic host does not have.
    expect(find.text('知识库'), findsWidgets);
    await tester.tap(search);
    await tester.pumpAndSettle();
    expect(find.widgetWithText(OutlinedButton, '工作记录'), findsOneWidget);
    await capture('search');
    await tester.tap(find.widgetWithText(OutlinedButton, '读书笔记'));
    await tester.pumpAndSettle();
    expect(switched, ['books']);

    // 3. Nothing open: the start pane.
    await pumpHost(
      tester,
      titleWorkspace(
        null,
        (_, _) async => true,
        views: views,
        workspaces: workspaces,
        entries: entries,
        selectedRef: entries.first.ref,
        onSearch: (_) async => const [],
        homePane: ({required onSearch, required onCreatePage}) => Builder(
          builder: (context) => buildHomePane(
            context,
            viewsByWorkspace: {'workspace': views},
            workspaceNames: {'workspace': '知识库'},
            onCreatePage: onCreatePage,
            onSearch: onSearch,
            onOpenView: (_) {},
            now: DateTime.utc(2026, 11, 15, 10),
          ),
        ),
      ),
    );
    expect(find.text('新建页面'), findsWidgets);
    expect(find.text('搜索页面'), findsOneWidget);
    expect(find.textContaining('最近编辑：'), findsOneWidget);
    await capture('start');
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox());
  });
}
