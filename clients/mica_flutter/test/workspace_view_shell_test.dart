// Issue #86, gap 2: `main_workspace_view.dart` had no test reference at all.
//
// [WorkspaceView] is the shell's 160-prop composite (sidebar + tab strip +
// editor pane); everything below pins its contract with the host at the
// widget level, without the full app: the tab strip's `+` / tabs / close /
// context menus route to the right callbacks with the right arguments, the
// flush-before-sidebar-select ordering hook survives (the sidebar's tap is
// `_navigateToView` — `await _commandHook.flush()` BEFORE `onSelectView`,
// asserted indirectly below through the callback sequence test), and the
// component builds at phone + desktop sizes without layout errors.
//
// Ordering the flush itself carries no observable side effect at this level
// (the hook's `flush()` is a no-op with no editor attached); the shell-level
// ordering is pinned end-to-end in shell_flush_before_switch_test.dart.

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';
import 'package:mica_flutter/doc_tab.dart';
import 'package:mica_flutter/ui/doc_tab_strip.dart';
import 'package:mica_flutter/ui/narrow_tab_switcher.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'package:mica_flutter/main.dart' as app;
import 'package:mica_flutter/ui/update_controls.dart';
import 'package:mica_flutter/updater.dart' show updateSupported;
import 'package:flutter_localizations/flutter_localizations.dart';

Workspace _ws(String id, String name) =>
    Workspace(id: id, name: name, ownerId: 'u', role: 'owner');

DocumentView _view(String id, String name) => DocumentView(
  id: id,
  parentViewId: null,
  objectId: 'doc-$id',
  objectType: 'document',
  name: name,
  position: '0000000010',
);

/// A scripted host: every prop the component requires, with the callbacks the
/// test is watching collected into [calls].
class _Host {
  final calls = <String>[];
  final workspaces = [_ws('w1', 'One')];
  late final views = [_view('va', 'Alpha'), _view('vb', 'Beta')];

  Widget build({
    List<DocTab>? tabs,
    int activeTabIndex = 0,
    DocumentView? selectedView,
    Size? size,
  }) {
    return _SizeBox(
      size ?? const Size(1280, 800),
      child: app.WorkspaceView(
        apiBase: Uri.parse('https://mica.test'),
        onChangeAvatar: null,
        onRemoveAvatar: null,
        onReplyComment: (_, _) async {},
        onSetCommentResolved: (_, _) async {},
        onDeleteCommentThread: (_) async {},
        onFocusCommentThread: (_) {},
        session: null,
        entries: const [],
        activeOrigin: 'local',
        selectedRef: null,
        onSelectEntry: (_) async {},
        onRenameEntry: (_, _) async {},
        onDeleteEntry: (_) async {},
        onExportEntryZip: (_) async => Uint8List(0),
        onImportTreeIntoEntry:
            (_, _, {String? sourceName, String? container}) async {},
        onImportTreeIntoFolder:
            (_, _, {String? sourceName, String? container}) async {},
        onReorderWorkspaces: (_) async {},
        cloudOriginLabel: 'Cloud',
        onSignIn: () {},
        localAvailable: true,
        isBusy: false,
        onRefresh: () {},
        onSignOut: () {},
        workspaces: workspaces,
        selectedWorkspace: workspaces.first,
        members: const [],
        views: views,
        selectedView: selectedView,
        selectedBootstrap: null,
        tabs: tabs ?? [DocTab()],
        activeTabIndex: activeTabIndex,
        onSelectTab: (i) => calls.add('selectTab:$i'),
        onCloseTab: (i) => calls.add('closeTab:$i'),
        onOpenInNewTab: (v) => calls.add('openInNewTab:${v.id}'),
        onOpenInNewTabById: (id, ws) => calls.add('openInNewTabById:$id:$ws'),
        onNewTabPage: () => calls.add('newTabPage'),
        presence: const [],
        message: null,
        onSelectWorkspace: (_) async => calls.add('selectWorkspace'),
        onCreateWorkspace: (_) async {},
        onRenameWorkspace: (_, _) async {},
        onDeleteWorkspace: (_) async {},
        onCreateDocument: (_) async => null,
        onCreateChildDocument: (_, _) async => null,
        onCreateFolder: (_) async => null,
        onCreateChildFolder: (_, _) async => null,
        onReorderViews: (_, _) async {},
        onLoadTrash: () async => const [],
        onRestoreView: (_) async {},
        onPurgeView: (_) async {},
        onSelectView: (v) async => calls.add('selectView:${v.id}'),
        onRenameView: (_, _) async => true,
        onDeleteView: (_) async {},
        onDeleteViews: (_) async {},
        onCloneView: (_) async {},
        onApplyOperations: (_) async {},
        onUploadImage: (_, _, _) async => null,
        onImportImageUrl: (_) async => null,
        onLoadImageBytes: (_) async => null,
        onResolveImageUrls: (_) async => const {},
        onAiStream: (_, {String? system}) => const Stream.empty(),
        onAiNewPage: (_) async {},
        onAiCurrentPage: null,
        onAiNewWorkspace: (_) async {},
        onLoadAiSettings: null,
        onLoadImportHistory: null,
        onListAiModels: null,
        onSaveAiSettings: null,
        onDeleteAiProvider: null,
        userName: 'User',
        userEmail: 'u@mica.test',
        onUpdateProfile: null,
        onChangePassword: null,
        onDeleteAccount: null,
        onSwitchWorld: () {},
        onRemoveServer: (_) async {},
        appearance: const EditorAppearance(),
        pageWidth: 800,
        reHostImages: false,
        onReHostImagesChanged: (_) {},
        showFormatBar: true,
        onShowFormatBarChanged: (_) {},
        showPageTitle: true,
        onShowPageTitleChanged: (_) {},
        showAi: false,
        aiEnabled: false,
        onAiEnabledChanged: (_) {},
        onAppearanceChanged: (_, _) {},
        onOpenSearchResult: (_) async {},
        onExportPage: (_) async => (bytes: Uint8List(0), name: 'n', mime: 'm'),
        onCopyPageMarkdown: () async => '',
        onExportPageHtml: (_) async => '',
        onExportPagePdf: (_) async => null,
        onImportMarkdown: (_, _) async {},
        onExportWorkspaceZip: (_) async => Uint8List(0),
        onImportWorkspaceZip: null,
        onImportWorkspaceTreeInto: (_, _) async {},
      ),
    );
  }
}

/// Desktop-sized surface + the app's localization, the two things the
/// component reads from context (`MediaQuery` + `context.l10n`).
class _SizeBox extends StatelessWidget {
  const _SizeBox(this.size, {required this.child});

  final Size size;
  final Widget child;

  @override
  Widget build(BuildContext context) => MediaQuery(
    data: MediaQueryData(size: size),
    child: MaterialApp(
      localizationsDelegates: const [
        AppLocalizations.delegate,
        GlobalMaterialLocalizations.delegate,
        GlobalWidgetsLocalizations.delegate,
        GlobalCupertinoLocalizations.delegate,
      ],
      supportedLocales: AppLocalizations.supportedLocales,
      home: Scaffold(body: child),
    ),
  );
}

/// Put keyboard focus on something under the shell's CallbackShortcuts. In
/// the app that is the editor; this harness has no document open, so any
/// focusable button in the shell stands in.
void _focusInsideShell(WidgetTester tester) {
  final icon = find.descendant(
    of: find.byType(IconButton).first,
    matching: find.byType(Icon),
  );
  Focus.of(tester.element(icon)).requestFocus();
}

void _viewSize(WidgetTester tester, Size size) {
  tester.view.physicalSize = size;
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
}

void main() {
  // Chinese locale: the popup menus size to their content and the wider
  // test-font metrics push the EN labels a few pixels past the route's cap.
  void useZh(WidgetTester tester) {
    tester.platformDispatcher.localesTestValue = [const Locale('zh', 'CN')];
    addTearDown(tester.platformDispatcher.clearLocalesTestValue);
  }

  for (final size in [const Size(1280, 800), const Size(390, 844)]) {
    testWidgets('the composite builds clean at $size', (tester) async {
      useZh(tester);
      final host = _Host();
      await tester.pumpWidget(host.build(size: size));
      await tester.pump();
      expect(tester.takeException(), isNull);
    });
  }

  testWidgets('desktop update entry shares the recycle-bin account row', (
    tester,
  ) async {
    useZh(tester);
    await tester.pumpWidget(_Host().build());
    await tester.pumpAndSettle();
    final update = find.byType(UpdateButton);
    if (!updateSupported) {
      expect(update, findsNothing);
      return;
    }
    expect(update, findsOneWidget);
    final trash = find.byTooltip('回收站');
    expect(trash, findsOneWidget);
    expect(tester.getCenter(update).dy, tester.getCenter(trash).dy);
    expect(tester.getCenter(update).dx, lessThan(tester.getCenter(trash).dx));
    expect(tester.getSize(update).height, lessThanOrEqualTo(40));
    expect(tester.takeException(), isNull);
  });

  testWidgets('the tab strip routes select/close/+/menu callbacks', (
    tester,
  ) async {
    useZh(tester);
    final host = _Host();
    final tabs = [
      DocTab(view: _view('va', 'Alpha')),
      DocTab(view: _view('vb', 'Beta')),
    ];
    await tester.pumpWidget(
      host.build(tabs: tabs, activeTabIndex: 1, selectedView: null),
    );
    await tester.pump();
    expect(tester.takeException(), isNull);

    // A tab in the strip selects.
    await tester.tap(find.text('Alpha').last);
    await tester.pump();
    expect(host.calls, contains('selectTab:0'));

    // The strip's `+` opens the new-page / open-existing menu (children of the
    // host's onNewTabPage world only — the local world hides the strip).
    await tester.tap(
      find.descendant(
        of: find.byType(DocTabStrip),
        matching: find.byIcon(Icons.add),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('新建页面'));
    await tester.pump();
    expect(host.calls, contains('newTabPage'));
    expect(tester.takeException(), isNull);
  });

  testWidgets('Ctrl+Tab, Ctrl+Shift+Tab and Ctrl+W drive the tabs', (
    tester,
  ) async {
    useZh(tester);
    _viewSize(tester, const Size(1280, 800));
    final host = _Host();
    final tabs = [
      DocTab(view: _view('va', 'Alpha')),
      DocTab(view: _view('vb', 'Beta')),
      DocTab(view: _view('vc', 'Gamma')),
    ];
    await tester.pumpWidget(
      host.build(tabs: tabs, activeTabIndex: 1, selectedView: null),
    );
    await tester.pump();
    _focusInsideShell(tester);
    await tester.pump();

    Future<void> chord(LogicalKeyboardKey key, {bool shift = false}) async {
      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      if (shift) await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      await tester.sendKeyEvent(key);
      if (shift) await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await tester.pump();
    }

    await chord(LogicalKeyboardKey.tab);
    expect(host.calls.last, 'selectTab:2');
    await chord(LogicalKeyboardKey.tab, shift: true);
    expect(host.calls.last, 'selectTab:0');
    await chord(LogicalKeyboardKey.keyW);
    expect(host.calls.last, 'closeTab:1');
    expect(tester.takeException(), isNull);
  });

  testWidgets('Ctrl+W does nothing on the only tab', (tester) async {
    useZh(tester);
    _viewSize(tester, const Size(1280, 800));
    final host = _Host();
    await tester.pumpWidget(
      host.build(tabs: [DocTab(view: _view('va', 'Alpha'))], selectedView: null),
    );
    await tester.pump();
    _focusInsideShell(tester);
    await tester.pump();
    await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyW);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    await tester.pump();
    expect(host.calls.where((c) => c.startsWith('closeTab')), isEmpty);
  });

  testWidgets('the narrow shell swaps the strip for a tab switcher', (
    tester,
  ) async {
    useZh(tester);
    _viewSize(tester, const Size(390, 844));
    final host = _Host();
    final tabs = [
      DocTab(view: _view('va', 'Alpha')),
      DocTab(view: _view('vb', 'Beta')),
    ];
    await tester.pumpWidget(
      host.build(
        tabs: tabs,
        activeTabIndex: 1,
        selectedView: null,
        size: const Size(390, 844),
      ),
    );
    await tester.pump();
    expect(find.byType(DocTabStrip), findsNothing);
    final switcher = find.byType(NarrowTabSwitcher);
    expect(switcher, findsOneWidget);
    expect(
      find.descendant(of: switcher, matching: find.text('2')),
      findsOneWidget,
    );

    // Open it: both tabs listed; picking one selects it.
    await tester.tap(switcher);
    await tester.pumpAndSettle();
    expect(find.text('Alpha'), findsWidgets);
    await tester.tap(find.text('Alpha').last);
    await tester.pumpAndSettle();
    expect(host.calls.last, 'selectTab:0');

    // A row's X closes that tab.
    await tester.tap(switcher);
    await tester.pumpAndSettle();
    await tester.tap(find.byIcon(Icons.close).first);
    await tester.pumpAndSettle();
    expect(host.calls.last, 'closeTab:0');

    // The trailing item opens the same new-tab menu as the strip's `+`.
    await tester.tap(switcher);
    await tester.pumpAndSettle();
    await tester.tap(find.text('新建标签页'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('新建页面'));
    await tester.pumpAndSettle();
    expect(host.calls.last, 'newTabPage');
    expect(tester.takeException(), isNull);
  });

  testWidgets('a sidebar row tap reports the view to the host', (tester) async {
    useZh(tester);
    final host = _Host();
    await tester.pumpWidget(host.build());
    await tester.pump();
    await tester.tap(find.text('Alpha').first);
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 100));
    expect(
      host.calls.where((c) => c == 'selectView:va'),
      isNotEmpty,
      reason:
          '_navigateToView flushes the editor, then tells the host which '
          'view was tapped: ${host.calls}',
    );
    expect(tester.takeException(), isNull);
  });

  testWidgets('the sidebar row context menu offers open-in-new-tab', (
    tester,
  ) async {
    useZh(tester);
    final host = _Host();
    await tester.pumpWidget(host.build());
    await tester.pump();
    final alpha = tester.getCenter(find.text('Alpha').first);
    await tester.tapAt(alpha, buttons: kSecondaryMouseButton);
    await tester.pumpAndSettle();
    await tester.tap(find.text('在新标签页打开'));
    await tester.pump();
    expect(host.calls, contains('openInNewTab:va'));
    expect(tester.takeException(), isNull);
  });
}
