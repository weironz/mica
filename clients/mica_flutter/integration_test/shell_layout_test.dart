// Real Windows plugin + production shell widgets, synthetic content only.
import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:mica_flutter/doc_tab.dart';
import 'package:mica_flutter/main.dart';
import 'package:mica_flutter/markdown/markdown_codec.dart';
import 'package:mica_flutter/prefs.dart';
import 'package:mica_flutter/ui/doc_tab_strip.dart';
import 'package:mica_flutter/ui/font_warmup.dart';
import 'package:mica_flutter/ui/theme_tokens.dart';
import 'package:mica_flutter/ui/window_chrome.dart';
import 'package:mica_flutter/window_setup.dart';
import 'package:window_manager/window_manager.dart';

import '../test/support/title_workspace.dart';
import '../test/support/isolated_profile_io.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  isolateProfileForTests();

  Future<void> waitForWindow(
    WidgetTester tester,
    Future<bool> Function() condition,
  ) async {
    // Window commands post native messages; Flutter having no pending frames
    // does not mean Windows has applied them.
    final reached = await tester.runAsync(() async {
      final deadline = DateTime.now().add(const Duration(seconds: 5));
      while (!await condition()) {
        if (DateTime.now().isAfter(deadline)) return false;
        await Future<void>.delayed(const Duration(milliseconds: 50));
      }
      return true;
    });
    expect(reached, isTrue, reason: 'native window state did not change');
    await tester.pumpAndSettle();
  }

  testWidgets('merged title bar, account menu and native window controls', (
    tester,
  ) async {
    // A saved floating rect keeps startup from queueing its first-run maximize
    // behind this test's own window commands.
    savePref('windowWidth', '1280');
    savePref('windowHeight', '800');
    await initDesktopWindow();
    await initMarkdownCodec();
    await warmUpFonts();
    await windowManager.unmaximize();
    await waitForWindow(tester, () async => !await windowManager.isMaximized());
    await windowManager.setSize(const Size(1280, 800));
    final doc = titleBootstrap('公有云环境');
    final tabs = [DocTab(view: doc.view), DocTab(view: doc.view)];
    var selected = -1;
    final host =
        titleWorkspace(
              doc,
              (_, _) async => true,
              tabs: tabs,
              onSelectTab: (i) => selected = i,
              onCloseTab: (_) {},
              session: const AuthSession(
                accessToken: 'synthetic-only',
                user: User(
                  id: 'layout-test',
                  email: 'layout@example.test',
                  displayName: '界面示例',
                ),
              ),
            )
            as MaterialApp;
    final shot = GlobalKey();
    await tester.pumpWidget(
      RepaintBoundary(
        key: shot,
        child: MaterialApp(
          locale: host.locale,
          localizationsDelegates: host.localizationsDelegates,
          supportedLocales: host.supportedLocales,
          theme: MicaTokens.light.toMaterialTheme(),
          builder: micaAppBuilder,
          home: host.home,
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(hasCustomWindowChrome, isTrue);
    final strip = find.byType(DocTabStrip);
    expect(tester.getTopLeft(strip).dy, 0);
    await tester.tap(
      find.descendant(of: strip, matching: find.text('公有云环境')).last,
    );
    await tester.pumpAndSettle();
    expect(selected, 1);
    await tester.tap(find.byTooltip('收起左侧栏'));
    await tester.pumpAndSettle();
    expect(find.byTooltip('展开左侧栏'), findsOneWidget);
    await tester.tap(find.byTooltip('展开左侧栏'));
    await tester.pumpAndSettle();
    expect(find.text('layout@example.test'), findsNothing);
    await tester.tap(find.text('界面示例'));
    await tester.pumpAndSettle();
    expect(find.text('layout@example.test'), findsOneWidget);
    expect(find.text('检查更新'), findsWidgets);
    final output = Platform.environment['MICA_LAYOUT_SCREENSHOT'];
    if (output != null) {
      final boundary =
          shot.currentContext!.findRenderObject() as RenderRepaintBoundary;
      final image = await boundary.toImage(pixelRatio: 1);
      final bytes = await image.toByteData(format: ui.ImageByteFormat.png);
      await File(output).writeAsBytes(bytes!.buffer.asUint8List());
      image.dispose();
    }
    await tester.tapAt(const Offset(600, 500));
    await tester.pumpAndSettle();
    // The actual caption callbacks, not direct plugin calls, trigger each action.
    final buttons = find.byType(WindowCaptionButton);
    expect(buttons, findsNWidgets(3));
    await tester.tap(buttons.at(1));
    await waitForWindow(tester, windowManager.isMaximized);
    expect(await windowManager.isMaximized(), isTrue);
    await tester.tap(buttons.at(1));
    await waitForWindow(tester, () async => !await windowManager.isMaximized());
    expect(await windowManager.isMaximized(), isFalse);
    await tester.tap(buttons.first);
    await waitForWindow(tester, windowManager.isMinimized);
    expect(await windowManager.isMinimized(), isTrue);
    await windowManager.restore();
    await windowManager.show();
    await tester.pumpAndSettle();
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox());
  });
}
