import 'dart:async';

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';
import 'package:mica_flutter/doc_tab.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'package:mica_flutter/ui/application_menu.dart';
import 'package:mica_flutter/ui/update_controls.dart';
import 'package:mica_flutter/updater_common.dart';

DocumentView _page(String id) => DocumentView(
  id: id,
  parentViewId: null,
  objectId: id,
  objectType: 'document',
  name: id,
  position: '0',
);

Widget _host(
  DesktopUpdateController updates,
  List<String> calls, {
  List<DocTab> tabs = const [],
  int active = 0,
  bool local = false,
  bool readOnly = false,
  bool sidebar = true,
  bool sidePanel = false,
  Locale locale = const Locale('zh'),
}) => MaterialApp(
  locale: locale,
  localizationsDelegates: AppLocalizations.localizationsDelegates,
  supportedLocales: AppLocalizations.supportedLocales,
  home: Scaffold(
    body: Align(
      alignment: Alignment.topLeft,
      child: ApplicationMenu(
        updates: updates,
        onNewPage: readOnly ? null : () => calls.add('newPage'),
        onNewTab: local ? null : () => calls.add('newTab'),
        onCloseTab: local ? null : () => calls.add('close:$active'),
        tabs: tabs,
        activeTabIndex: active,
        onHistory: local ? null : (forward) => calls.add('history:$forward'),
        onSettings: () => calls.add('settings'),
        onToggleSidebar: () => calls.add('sidebar'),
        onToggleSidePanel: () => calls.add('sidePanel'),
        sidebarVisible: sidebar,
        sidePanelVisible: sidePanel,
      ),
    ),
  ),
);

MenuItemButton _item(WidgetTester tester, String label) =>
    tester.widget<MenuItemButton>(find.widgetWithText(MenuItemButton, label));

Future<void> _open(WidgetTester tester, String group) async {
  await tester.tap(find.byTooltip('应用菜单'));
  await tester.pumpAndSettle();
  await tester.tap(find.text(group));
  await tester.pumpAndSettle();
}

void main() {
  late DesktopUpdateController updates;
  late List<String> calls;
  setUp(() {
    calls = [];
    updates = DesktopUpdateController(
      currentVersion: '1.2.3',
      supported: false,
    );
  });
  tearDown(() => updates.dispose());

  testWidgets('logo opens groups; hover opens a submenu; outside tap closes', (
    tester,
  ) async {
    await tester.pumpWidget(_host(updates, calls, tabs: [DocTab()]));
    expect(find.text('文件'), findsNothing);
    await tester.tap(find.byTooltip('应用菜单'));
    await tester.pumpAndSettle();
    for (final group in ['文件', '视图', '历史', '帮助']) {
      expect(find.text(group), findsOneWidget);
    }
    final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
    await mouse.addPointer(location: const Offset(500, 500));
    await mouse.moveTo(tester.getCenter(find.text('文件')));
    await tester.pumpAndSettle();
    expect(find.text('新建页面'), findsOneWidget);
    await tester.tapAt(const Offset(600, 500));
    await tester.pumpAndSettle();
    expect(find.text('文件'), findsNothing);
    await mouse.removePointer();
  });

  for (final (label, result) in [
    ('新建页面', 'newPage'),
    ('新建标签页', 'newTab'),
    ('关闭当前标签页', 'close:1'),
    ('设置', 'settings'),
  ]) {
    testWidgets('file/$label dispatches once and closes all menus', (
      tester,
    ) async {
      await tester.pumpWidget(
        _host(updates, calls, tabs: [DocTab(), DocTab()], active: 1),
      );
      await _open(tester, '文件');
      await tester.tap(find.text(label));
      await tester.pumpAndSettle();
      expect(calls, [result]);
      expect(find.text('文件'), findsNothing);
    });
  }

  testWidgets('read-only and single-tab actions are disabled', (tester) async {
    await tester.pumpWidget(
      _host(updates, calls, tabs: [DocTab()], readOnly: true),
    );
    await _open(tester, '文件');
    expect(_item(tester, '新建页面').onPressed, isNull);
    expect(_item(tester, '关闭当前标签页').onPressed, isNull);
    expect(_item(tester, '设置').onPressed, isNotNull);
    expect(calls, isEmpty);
  });

  testWidgets('local mode hides tab and history commands, not settings', (
    tester,
  ) async {
    await tester.pumpWidget(_host(updates, calls, local: true));
    await _open(tester, '文件');
    expect(find.text('新建标签页'), findsNothing);
    expect(find.text('关闭当前标签页'), findsNothing);
    expect(find.text('历史'), findsNothing);
    expect(find.text('设置'), findsOneWidget);
  });

  testWidgets(
    'view commands reflect state and reach the same panel callbacks',
    (tester) async {
      await tester.pumpWidget(_host(updates, calls));
      await _open(tester, '视图');
      expect(find.text('收起左侧栏'), findsOneWidget);
      expect(find.text('展开右侧栏'), findsOneWidget);
      await tester.tap(find.text('收起左侧栏'));
      await tester.pumpAndSettle();
      expect(calls, ['sidebar']);
      await tester.pumpWidget(
        _host(updates, calls, sidebar: false, sidePanel: true),
      );
      await _open(tester, '视图');
      expect(find.text('展开左侧栏'), findsOneWidget);
      await tester.tap(find.text('收起右侧栏'));
      await tester.pumpAndSettle();
      expect(calls, ['sidebar', 'sidePanel']);
    },
  );

  testWidgets('history follows the active tab, not another tab', (
    tester,
  ) async {
    final a = DocTab(view: _page('a'))..show(_page('b'));
    final b = DocTab(view: _page('c'));
    await tester.pumpWidget(_host(updates, calls, tabs: [a, b], active: 1));
    await _open(tester, '历史');
    expect(_item(tester, '后退').onPressed, isNull);
    expect(_item(tester, '前进').onPressed, isNull);
    await tester.tapAt(const Offset(600, 500));
    await tester.pumpAndSettle();
    await tester.pumpWidget(_host(updates, calls, tabs: [a, b]));
    await _open(tester, '历史');
    expect(_item(tester, '后退').onPressed, isNotNull);
    await tester.tap(find.text('后退'));
    await tester.pumpAndSettle();
    expect(calls, ['history:false']);
    final id = a.history.back(a.view?.id)!;
    a.show(_page(id));
    await tester.pumpWidget(_host(updates, calls, tabs: [a, b]));
    await _open(tester, '历史');
    expect(_item(tester, '后退').onPressed, isNull);
    expect(_item(tester, '前进').onPressed, isNotNull);
    await tester.tap(find.text('前进'));
    await tester.pumpAndSettle();
    expect(calls, ['history:false', 'history:true']);
  });

  testWidgets(
    'about shows the shared version details; unsupported updates absent',
    (tester) async {
      await tester.pumpWidget(_host(updates, calls));
      await _open(tester, '帮助');
      expect(find.text('检查更新'), findsNothing);
      await tester.tap(find.text('关于 Mica'));
      await tester.pumpAndSettle();
      expect(find.byType(AboutDialog), findsOneWidget);
      expect(find.text('v1.2.3'), findsOneWidget);
      expect(find.text('文件'), findsNothing);
    },
  );

  testWidgets('update action shares controller busy state and checks once', (
    tester,
  ) async {
    final done = Completer<UpdateInfo?>();
    final shared = DesktopUpdateController(
      currentVersion: '1.2.3',
      supported: true,
      check: (_) {
        calls.add('check');
        return done.future;
      },
    );
    addTearDown(shared.dispose);
    await tester.pumpWidget(_host(shared, calls));
    await _open(tester, '帮助');
    await tester.tap(find.text('检查更新'));
    await tester.pumpAndSettle();
    await _open(tester, '帮助');
    expect(_item(tester, '检查中…').onPressed, isNull);
    expect(calls, ['check']);
    done.complete(null);
    await tester.pumpAndSettle();
    expect(_item(tester, '检查更新').onPressed, isNotNull);
    expect(tester.takeException(), isNull);
  });

  testWidgets('English labels and existing shortcut hints are localized', (
    tester,
  ) async {
    await tester.pumpWidget(
      _host(updates, calls, tabs: [DocTab()], locale: const Locale('en')),
    );
    await tester.tap(find.byTooltip('Application menu'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('File'));
    await tester.pumpAndSettle();
    final button = _item(tester, 'New page');
    final shortcut = button.shortcut! as SingleActivator;
    expect(shortcut.trigger, LogicalKeyboardKey.keyN);
    expect(shortcut.control, isTrue);
    expect(shortcut.meta, isFalse);
    expect(find.text('Help'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });
}
