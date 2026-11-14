import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_localizations/flutter_localizations.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'package:mica_flutter/ui/account_menu.dart';
import 'package:mica_flutter/ui/update_controls.dart';
import 'package:mica_flutter/updater_common.dart';

const _release = UpdateInfo(
  version: '0.13.69',
  downloadUrl: 'https://example.test/Mica-Setup-0.13.69.exe',
);

Widget _host(DesktopUpdateController controller, {bool about = false}) =>
    MaterialApp(
      locale: const Locale('zh'),
      localizationsDelegates: const [
        AppLocalizations.delegate,
        GlobalMaterialLocalizations.delegate,
        GlobalWidgetsLocalizations.delegate,
        GlobalCupertinoLocalizations.delegate,
      ],
      supportedLocales: AppLocalizations.supportedLocales,
      home: Scaffold(
        body: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            SizedBox(
              width: 240,
              child: AccountMenu(
                name: 'Account',
                initial: 'A',
                onSettings: () {},
                onSwitchWorld: () {},
                updates: controller,
              ),
            ),
            if (about) UpdateChecker(controller: controller),
          ],
        ),
      ),
    );

const _dot = ValueKey('account-update-dot');

/// The update entry lives in the account menu; open it and press [label].
Future<void> _pressInMenu(WidgetTester tester, String label) async {
  await tester.tap(find.text('Account'));
  await tester.pumpAndSettle();
  await tester.tap(
    find.descendant(
      of: find.byType(MenuItemButton),
      matching: find.text(label),
    ),
  );
  await tester.pumpAndSettle();
}

void main() {
  testWidgets('a found update marks the account row and the menu entry', (
    tester,
  ) async {
    var checks = 0;
    final controller = DesktopUpdateController(
      currentVersion: '0.13.68',
      supported: true,
      check: (_) async {
        checks++;
        return _release;
      },
    );
    addTearDown(controller.dispose);
    await tester.pumpWidget(_host(controller));
    await tester.pumpAndSettle();

    expect(checks, 1);
    expect(find.byKey(_dot), findsOneWidget);
    expect(find.text('立即更新'), findsNothing, reason: 'no resident button');
    await tester.tap(find.text('Account'));
    await tester.pumpAndSettle();
    expect(find.text('立即更新'), findsOneWidget);
    expect(find.text('v0.13.69'), findsOneWidget);
    expect(find.byType(AlertDialog), findsNothing);
  });

  testWidgets('cancel preserves the available update without applying it', (
    tester,
  ) async {
    var applies = 0;
    final controller = DesktopUpdateController(
      currentVersion: '0.13.68',
      supported: true,
      check: (_) async => _release,
      apply: (_, {onProgress}) async => applies++,
    );
    addTearDown(controller.dispose);
    await tester.pumpWidget(_host(controller));
    await tester.pumpAndSettle();
    await _pressInMenu(tester, '立即更新');

    expect(find.text('更新到 v0.13.69'), findsOneWidget);
    expect(find.textContaining('自动关闭并重启 Mica'), findsOneWidget);
    expect(applies, 0);
    await tester.tap(find.text('取消'));
    await tester.pumpAndSettle();
    expect(applies, 0);
    expect(controller.stage, DesktopUpdateStage.available);
    expect(controller.isBusy, isFalse);
  });

  testWidgets('one confirmation starts automatic update and shares progress', (
    tester,
  ) async {
    var applies = 0;
    final finished = Completer<void>();
    final controller = DesktopUpdateController(
      currentVersion: '0.13.68',
      supported: true,
      check: (_) async => _release,
      apply: (info, {onProgress}) async {
        expect(info, same(_release));
        applies++;
        onProgress?.call(0.5);
        await finished.future;
      },
    );
    addTearDown(controller.dispose);
    await tester.pumpWidget(_host(controller, about: true));
    await tester.pumpAndSettle();
    await _pressInMenu(tester, '立即更新');
    expect(find.byType(AlertDialog), findsOneWidget);
    await tester.tap(find.text('更新并重启'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 300));

    expect(applies, 1);
    expect(find.byType(AlertDialog), findsNothing);
    expect(find.text('下载中 50%…'), findsOneWidget);
    expect(controller.isBusy, isTrue);
    final progress = tester.widget<LinearProgressIndicator>(
      find.byType(LinearProgressIndicator),
    );
    expect(progress.value, 0.5);
    await tester.pumpWidget(const SizedBox());
    finished.complete();
    await tester.pump();
  });

  testWidgets('a failed automatic check is quiet and can be retried', (
    tester,
  ) async {
    var checks = 0;
    final controller = DesktopUpdateController(
      currentVersion: '0.13.68',
      supported: true,
      check: (_) async {
        if (++checks == 1) throw Exception('offline');
        return _release;
      },
    );
    addTearDown(controller.dispose);
    await tester.pumpWidget(_host(controller));
    await tester.pumpAndSettle();
    expect(find.byType(AlertDialog), findsNothing);
    expect(find.byType(SnackBar), findsNothing);
    expect(controller.stage, DesktopUpdateStage.error);
    expect(find.byKey(_dot), findsNothing);
    await _pressInMenu(tester, '检查更新');
    expect(checks, 2);
    expect(find.text('更新并重启'), findsOneWidget);
  });

  testWidgets('an integrity failure keeps the app usable and offers a retry', (
    tester,
  ) async {
    var applies = 0;
    final controller = DesktopUpdateController(
      currentVersion: '0.13.68',
      supported: true,
      check: (_) async => _release,
      apply: (_, {onProgress}) async {
        applies++;
        throw const UpdateIntegrityException('invalid digest');
      },
    );
    addTearDown(controller.dispose);
    await tester.pumpWidget(_host(controller, about: true));
    await tester.pumpAndSettle();
    await _pressInMenu(tester, '立即更新');
    await tester.tap(find.text('更新并重启'));
    await tester.pumpAndSettle();
    expect(applies, 1);
    expect(controller.integrityFailed, isTrue);
    expect(controller.isBusy, isFalse);
    expect(find.text('更新已中止'), findsOneWidget);
    expect(find.textContaining('当前版本 v0.13.68 仍可正常使用'), findsOneWidget);
  });

  testWidgets('unsupported platforms have no updater or background checks', (
    tester,
  ) async {
    var checks = 0;
    final controller = DesktopUpdateController(
      currentVersion: '0.13.68',
      supported: false,
      check: (_) async {
        checks++;
        return _release;
      },
    );
    addTearDown(controller.dispose);
    await tester.pumpWidget(_host(controller, about: true));
    await tester.pumpAndSettle();
    expect(checks, 0);
    expect(find.byIcon(Icons.system_update_alt), findsNothing);
    await tester.tap(find.text('Account'));
    await tester.pumpAndSettle();
    expect(find.text('检查更新'), findsNothing);
  });

  test(
    'shared controller coalesces checks and locks confirmation/install',
    () async {
      final checked = Completer<UpdateInfo?>();
      final confirmed = Completer<bool>();
      var checks = 0;
      var applies = 0;
      var confirmations = 0;
      final controller = DesktopUpdateController(
        currentVersion: '0.13.68',
        supported: true,
        check: (_) {
          checks++;
          return checked.future;
        },
        apply: (_, {onProgress}) async => applies++,
      );
      addTearDown(controller.dispose);
      final checking = controller.check();
      await controller.check(force: true);
      expect(checks, 1);
      checked.complete(_release);
      await checking;
      await controller.check();
      expect(
        checks,
        1,
        reason: 'workspace rebuilds must not poll GitHub again',
      );
      final updating = controller.confirmAndApply((_) {
        confirmations++;
        return confirmed.future;
      });
      await controller.confirmAndApply((_) async {
        confirmations++;
        return true;
      });
      await controller.check(force: true);
      expect(confirmations, 1);
      expect(checks, 1);
      confirmed.complete(true);
      await updating;
      expect(applies, 1);
    },
  );

  testWidgets('periodically rechecks and cancels its timer when removed', (
    tester,
  ) async {
    var checks = 0;
    final controller = DesktopUpdateController(
      currentVersion: '0.13.68',
      supported: true,
      check: (_) async {
        checks++;
        return checks == 1 ? null : _release;
      },
    );
    addTearDown(controller.dispose);
    await tester.pumpWidget(_host(controller));
    await tester.pumpAndSettle();
    expect(controller.stage, DesktopUpdateStage.upToDate);
    expect(find.byKey(_dot), findsNothing);
    await tester.pump(DesktopUpdateController.checkInterval);
    await tester.pumpAndSettle();
    expect(checks, 2);
    expect(controller.stage, DesktopUpdateStage.available);
    expect(find.byKey(_dot), findsOneWidget);
    await tester.pumpWidget(const SizedBox());
    await tester.pump(DesktopUpdateController.checkInterval);
    expect(checks, 2);
  });
}
