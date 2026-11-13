// Temporary Windows-native acceptance harness, outside the production source.
import 'dart:convert';
import 'dart:ffi' as ffi;
import 'dart:io';
import 'dart:ui' as ui;
import 'package:ffi/ffi.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:window_manager/window_manager.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:mica_flutter/main.dart' as app;
import 'package:mica_flutter/api/client.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'package:mica_flutter/l10n/locale_controller.dart';
import 'package:mica_flutter/local/local_offline.dart';
import 'package:mica_flutter/markdown/markdown_codec.dart';
import 'package:mica_flutter/prefs.dart';
import 'package:mica_flutter/theme_controller.dart';
import 'package:mica_flutter/ui/font_warmup.dart';
import 'package:mica_flutter/ui/segmented_choice.dart';
import 'package:mica_flutter/ui/settings_dialog.dart';
import 'package:mica_flutter/window_setup.dart';
import 'package:mica_flutter/web/web_probe.dart';
import '../test/support/native_build.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  testWidgets('Windows settings groups fit their choices at 100 percent', (t) async {
    final output=Directory(Platform.environment['MICA_UI_OUTPUT']!)..createSync(recursive:true);
    final boundary=GlobalKey();
    await windowManager.ensureInitialized();
    await windowManager.setMaximumSize(const Size(2000,1600));
    await windowManager.setSize(const Size(1440,1000));
    await windowManager.show();
    await windowManager.setTitle('Mica isolated settings follow-up');
    await warmUpFonts();
    await initMarkdownCodec();
    await initCheckedNativeRust();
    await initDocEngine();
    final local=LocalOffline();await local.open();
    final ws=local.createLocalWorkspace('设置验收');
    await local.importMarkdownPage('设置验收.md','# 设置验收\n\n中文 English，独立配置。',ws,fallbackTitle:'设置验收');
    useHttpClientForTesting(MockClient((r) async => http.Response(jsonEncode({'status':'ok','registration_open':true}),200)));
    savePref('cloudOrigin','https://visual.fixture.test');
    savePref('servers',jsonEncode(['https://visual.fixture.test']));
    savePref('activeOrigin','local');savePref('lastWorkspaceId:local',ws);
    final metrics=<Map<String,Object?>>[];
    for(final language in ['zh','en']) {
      localeController.value=Locale(language);
      for(final mode in [MicaThemeMode.light,MicaThemeMode.dark]) {
        themeModeController.value=mode;
        await t.pumpWidget(RepaintBoundary(key:boundary,child:const app.MicaApp()));
        await t.pumpAndSettle();
        final l=AppLocalizations.of(t.element(find.byType(MicaEditor)));
        await t.tap(find.text(l.identityLocalName).last);await t.pumpAndSettle();
        await t.tap(find.text(l.settingsTitle));await t.pumpAndSettle();
        expect(find.byType(SettingsDialog),findsOneWidget);
        for(final size in [const Size(1440,1000),const Size(900,800)]) {
          await windowManager.unmaximize();await windowManager.setSize(size);
          await t.pumpAndSettle();
          expect(t.view.devicePixelRatio,1);
          expect(ui.PlatformDispatcher.instance.views.first.devicePixelRatio,1);
          final groups=find.byType(MicaSegmentGroup);
          expect(groups,findsNWidgets(3));
          final rows=<Map<String,Object?>>[];
          for(var i=0;i<3;i++) {
            final group=groups.at(i);
            final background=find.descendant(of:group,matching:find.byType(DecoratedBox)).first;
            final bounds=t.getRect(background);
            final options=find.descendant(of:group,matching:find.byType(MicaSegmentButton));
            final rects=[for(var j=0;j<options.evaluate().length;j++) t.getRect(options.at(j))];
            expect(bounds.left,t.getRect(group).left);
            expect(rects.first.left,bounds.left+2);
            expect(bounds.right,rects.map((r)=>r.right).reduce((a,b)=>a>b?a:b)+2);
            expect(bounds.bottom,rects.map((r)=>r.bottom).reduce((a,b)=>a>b?a:b)+2);
            if(size.width==1440)expect(bounds.width,lessThan(t.getSize(group).width));
            rows.add({'backgroundWidth':bounds.width,'availableWidth':t.getSize(group).width,'height':bounds.height});
          }
          final name='$language-${mode.name}-${size.width.toInt()}';
          final image=await (boundary.currentContext!.findRenderObject()! as RenderRepaintBoundary).toImage(pixelRatio:1);
          try{
            final bytes=await image.toByteData(format:ui.ImageByteFormat.png);
            File('${output.path}/$name.png').writeAsBytesSync(bytes!.buffer.asUint8List());
          }finally{image.dispose();}
          metrics.add({'scene':name,'dpr':t.view.devicePixelRatio,'clientWidth':t.view.physicalSize.width,'clientHeight':t.view.physicalSize.height,'groups':rows});
          expect(t.takeException(),isNull);
        }
        await windowManager.setSize(const Size(1440,1000));await t.pumpAndSettle();
        appNavigatorKey.currentState!.pop();await t.pumpAndSettle();
        await t.pumpWidget(const SizedBox.shrink());await t.pump();
      }
    }
    final native=ffi.DynamicLibrary.open('user32.dll');
    final findWindow=native.lookupFunction<ffi.Pointer<ffi.Void> Function(ffi.Pointer<Utf16>,ffi.Pointer<Utf16>),ffi.Pointer<ffi.Void> Function(ffi.Pointer<Utf16>,ffi.Pointer<Utf16>)>('FindWindowW');
    final dpi=native.lookupFunction<ffi.Uint32 Function(ffi.Pointer<ffi.Void>),int Function(ffi.Pointer<ffi.Void>)>('GetDpiForWindow');
    final title='Mica isolated settings follow-up'.toNativeUtf16();
    try{expect(dpi(findWindow(ffi.nullptr,title)),96);}finally{calloc.free(title);}
    File('${output.path}/settings-metrics.json').writeAsStringSync(const JsonEncoder.withIndent('  ').convert(metrics));
  },timeout:const Timeout(Duration(minutes:5)));
}
