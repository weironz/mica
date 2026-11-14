import 'package:flutter/material.dart';
import 'package:flutter_localizations/flutter_localizations.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'package:mica_flutter/ui/interface_text_scale.dart';
import 'package:mica_flutter/ui/settings_dialog.dart';
import 'package:mica_flutter/updater_controller.dart';

void main() {
  testWidgets('appearance scale and recycle bin are available in Settings', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(1440, 900);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);

    var trashLoads = 0;
    double? chosenScale;
    final updates = DesktopUpdateController(
      currentVersion: 'test',
      supported: false,
    );
    addTearDown(updates.dispose);

    await tester.pumpWidget(
      MaterialApp(
        locale: const Locale('en'),
        localizationsDelegates: const [
          AppLocalizations.delegate,
          GlobalMaterialLocalizations.delegate,
          GlobalWidgetsLocalizations.delegate,
          GlobalCupertinoLocalizations.delegate,
        ],
        supportedLocales: const [Locale('en')],
        home: Scaffold(
          body: SettingsDialog(
            updates: updates,
            onLoadAiSettings: null,
            onListAiModels: null,
            onSaveAiSettings: null,
            userName: '',
            userEmail: '',
            onUpdateProfile: null,
            currentAvatarUrl: () => null,
            onChangeAvatar: null,
            onRemoveAvatar: null,
            onChangePassword: null,
            onDeleteAccount: null,
            appearance: const EditorAppearance(),
            pageWidth: 800,
            uiFontScale: kUiFontScaleStandard,
            onUiFontScaleChanged: (scale) => chosenScale = scale,
            reHostImages: true,
            onReHostImagesChanged: (_) {},
            showFormatBar: false,
            onShowFormatBarChanged: (_) {},
            showPageTitle: true,
            onShowPageTitleChanged: (_) {},
            aiEnabled: false,
            onAiEnabledChanged: (_) {},
            onAppearanceChanged: (_, _) {},
            onLoadTrash: () async {
              trashLoads++;
              return [];
            },
            onRestoreView: (_) async {},
            onPurgeView: (_) async {},
            canEditTrash: true,
            liveViews: const [],
            onImportWorkspace: null,
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Interface text size'), findsOneWidget);
    await tester.tap(find.text('Large'));
    await tester.pumpAndSettle();
    expect(chosenScale, kUiFontScaleLarge);

    await tester.tap(find.text('Recycle bin'));
    await tester.pumpAndSettle();
    expect(trashLoads, 1);
    expect(find.text('Recycle bin'), findsWidgets);
    expect(tester.takeException(), isNull);
  });
}
