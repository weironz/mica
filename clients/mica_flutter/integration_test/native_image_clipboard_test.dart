// Native OS clipboard -> production image reader -> actual editor paste op.
// Run only on a test desktop with its clipboard preserved by the runner.
import 'dart:convert';
import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/image_actions_stub.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/editor/rich_paste_stub.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';

import '../test/support/native_build.dart';

Future<void> _externalPngClipboard(List<int> bytes) async {
  if (!Platform.isWindows) {
    expect(
      await copyImageToClipboard(Uint8List.fromList(bytes), 'image/png'),
      isTrue,
    );
    return;
  }
  final script =
      r'''
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class PngClipboardFixture {
 [DllImport("user32.dll")] public static extern bool OpenClipboard(IntPtr h);
 [DllImport("user32.dll")] public static extern bool EmptyClipboard();
 [DllImport("user32.dll")] public static extern bool CloseClipboard();
 [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern uint RegisterClipboardFormat(string name);
 [DllImport("user32.dll")] public static extern IntPtr SetClipboardData(uint format, IntPtr h);
 [DllImport("kernel32.dll")] public static extern IntPtr GlobalAlloc(uint flags, UIntPtr bytes);
 [DllImport("kernel32.dll")] public static extern IntPtr GlobalLock(IntPtr h);
 [DllImport("kernel32.dll")] public static extern bool GlobalUnlock(IntPtr h);
 [DllImport("kernel32.dll")] public static extern IntPtr GlobalFree(IntPtr h);
}
"@
$b=[Convert]::FromBase64String('PNG_BYTES')
if(-not [PngClipboardFixture]::OpenClipboard([IntPtr]::Zero)){throw 'clipboard busy'}
$h=[IntPtr]::Zero
try {
 [void][PngClipboardFixture]::EmptyClipboard()
 $h=[PngClipboardFixture]::GlobalAlloc(2,[UIntPtr]::new([uint64]$b.Length))
 if($h -eq [IntPtr]::Zero){throw 'allocation failed'}
 $p=[PngClipboardFixture]::GlobalLock($h)
 if($p -eq [IntPtr]::Zero){throw 'lock failed'}
 [Runtime.InteropServices.Marshal]::Copy($b,0,$p,$b.Length)
 [void][PngClipboardFixture]::GlobalUnlock($h)
 $format=[PngClipboardFixture]::RegisterClipboardFormat('PNG')
 if([PngClipboardFixture]::SetClipboardData($format,$h) -eq [IntPtr]::Zero){throw 'PNG clipboard write failed'}
 $h=[IntPtr]::Zero
} finally {
 if($h -ne [IntPtr]::Zero){[void][PngClipboardFixture]::GlobalFree($h)}
 [void][PngClipboardFixture]::CloseClipboard()
}
'''
          .replaceAll('PNG_BYTES', base64Encode(bytes));
  final powershell =
      '${Platform.environment['SystemRoot']}\\System32\\WindowsPowerShell\\v1.0\\powershell.exe';
  final result = await Process.run(powershell, [
    '-NoProfile',
    '-STA',
    '-Command',
    script,
  ]);
  expect(
    result.exitCode,
    0,
    reason: 'synthetic external PNG clipboard fixture: ${result.stderr}',
  );
}

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(initCheckedNativeRust);
  testWidgets('native image clipboard preserves pixels and inserts an image', (
    tester,
  ) async {
    // Fixed red/transparent PNG: the fixture itself must not depend on GPU paint.
    final png = base64Decode(
      'iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAEklEQVR4nGP4z8AARAxgEswAACvmA/0Sp46gAAAAAElFTkSuQmCC',
    );
    // The Windows plugin writes CF_BITMAP, whose contract is opaque. The
    // registered PNG fixture below represents transparent images copied by browsers.
    expect(await copyImageToClipboard(png, 'image/png'), isTrue);
    final pluginRead = await readClipboardImage();
    expect(pluginRead, isNotNull, reason: 'native pasteboard image round trip');
    await _externalPngClipboard(png);
    final pasted = await readClipboardImage();
    expect(pasted, isNotNull);
    await tester.runAsync(() async {
      final codec = await ui.instantiateImageCodec(pasted!);
      final frame = await codec.getNextFrame();
      codec.dispose();
      try {
        expect([frame.image.width, frame.image.height], [2, 2]);
        final data = (await frame.image.toByteData(
          format: ui.ImageByteFormat.rawRgba,
        ))!;
        final bytes = data.buffer.asUint8List(
          data.offsetInBytes,
          data.lengthInBytes,
        );
        expect(
          [bytes[0], bytes[3], bytes[7]],
          [255, 255, 0],
          reason: 'red and transparent PNG pixels survive',
        );
      } finally {
        frame.image.dispose();
      }
    });
    final operations = <Map<String, dynamic>>[];
    Uint8List? uploaded;
    await tester.pumpWidget(
      MaterialApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: Scaffold(
          body: MicaEditor(
            rootBlockId: 'root',
            nodes: [
              EditorNode(id: 'p', kind: 'paragraph', text: 'paste image'),
            ],
            version: 0,
            canEdit: true,
            onApplyOperations: (batch) async => operations.addAll(batch),
            onUploadImage: (bytes, name, mime) async {
              uploaded = bytes;
              expect(mime, 'image/png');
              return (fileId: 'clipboard-image-fixture', name: name);
            },
            onLoadImageBytes: (_) async => png,
          ),
        ),
      ),
    );
    await tester.pump();
    final surface = find.byType(DocumentSurface);
    await tester.tapAt(
      tester.getTopLeft(surface) +
          tester
              .renderObject<RenderDocument>(surface)
              .caretRectFor(const DocPosition(0, 0))!
              .center,
    );
    await tester.pump();
    await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyV);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    final deadline = DateTime.now().add(const Duration(seconds: 20));
    while (operations.isEmpty && DateTime.now().isBefore(deadline)) {
      await tester.pump(const Duration(milliseconds: 50));
    }
    expect(uploaded, isNotNull);
    expect(jsonEncode(operations), contains('clipboard-image-fixture'));
    await tester.pumpWidget(const SizedBox.shrink());
    await tester.pump();
  });
}
