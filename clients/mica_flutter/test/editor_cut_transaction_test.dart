import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart' hide TableCell;
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/editor/table.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'support/markdown_codec.dart';

Future<void> key(WidgetTester t, LogicalKeyboardKey k,
    {bool ctrl = false, bool shift = false}) async {
  if (ctrl) await t.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
  if (shift) await t.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
  await t.sendKeyEvent(k);
  if (shift) await t.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
  if (ctrl) await t.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
  await t.pump();
}

Future<void> pumpEditor(WidgetTester t, List<DocOp> ops,
    {bool table = false}) async {
  await t.pumpWidget(MaterialApp(
    localizationsDelegates: AppLocalizations.localizationsDelegates,
    supportedLocales: AppLocalizations.supportedLocales,
    home: Scaffold(body: MicaEditor(
      rootBlockId: 'root',
      nodes: [table
        ? EditorNode(id: 't', kind: 'table', text: '',
            data: TableData([[TableCell('h')], [TableCell('abcdef')]]).toBlockData())
        : EditorNode(id: 'a', kind: 'paragraph', text: 'abcdef')],
      version: 0, canEdit: true,
      onApplyOperations: (batch) async => ops.addAll(batch),
    )),
  ));
  await t.pump();
  final r = t.renderObject<RenderDocument>(find.byType(DocumentSurface));
  final rect = table ? r.tableCellRect(0, 1, 0)! : r.caretRectFor(const DocPosition(0, 0))!;
  await t.tapAt(t.getTopLeft(find.byType(DocumentSurface)) + rect.center);
  await t.pump();
  await t.pump(const Duration(milliseconds: 50));
  if (!table) await key(t, LogicalKeyboardKey.home, ctrl: true);
}

void main() {
  setUpAll(useNativeMarkdownCodec);
  testWidgets('cut deletes only its successful clipboard snapshot, body and cell', (t) async {
    final sandbox = Directory.systemTemp.createTempSync('mica-cut-');
    final signal = '${sandbox.path}/clipboard';
    Process? locker;
    final copied = <String>[];
    var fail = false;
    Completer<void>? pending;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(SystemChannels.platform, (call) async {
      if (call.method == 'Clipboard.setData') {
        copied.add((call.arguments as Map)['text'] as String);
        if (pending != null) await pending.future;
        if (fail) throw PlatformException(code: 'clipboard_denied');
      }
      return null;
    });
    try {
      // Exercise the real native writer's busy-clipboard fallback. The public
      // widget remains the seam; no injection of private clipboard helpers.
      if (Platform.isWindows) {
        await t.runAsync(() async {
          locker = await Process.start('powershell.exe', ['-NoProfile',
            '-ExecutionPolicy', 'Bypass', '-File',
            File('test/support/hold_clipboard.ps1').absolute.path, '-Signal', signal]);
          unawaited(locker!.stdout.drain<void>());
          unawaited(locker!.stderr.drain<void>());
          for (var i = 0; i < 300 && !File('$signal.ready').existsSync(); i++) {
            await Future<void>.delayed(const Duration(milliseconds: 100));
          }
          expect(File('$signal.ready').existsSync(), isTrue);
        });
      }
      final ops = <DocOp>[];
      fail = true;
      await pumpEditor(t, ops);
      await key(t, LogicalKeyboardKey.keyA, ctrl: true);
      final before = t.widget<DocumentSurface>(find.byType(DocumentSurface)).selection;
      await key(t, LogicalKeyboardKey.keyX, ctrl: true);
      await t.pump(const Duration(milliseconds: 500));
      expect(copied, ['abcdef']);
      expect(t.widget<DocumentSurface>(find.byType(DocumentSurface)).nodes.single.text, 'abcdef');
      expect(t.widget<DocumentSurface>(find.byType(DocumentSurface)).selection, before);
      expect(ops, isEmpty);
      await t.pumpWidget(const SizedBox()); await t.pump();

      fail = false; copied.clear(); pending = Completer<void>(); ops.clear();
      await pumpEditor(t, ops);
      await key(t, LogicalKeyboardKey.arrowRight, shift: true);
      await key(t, LogicalKeyboardKey.arrowRight, shift: true);
      await key(t, LogicalKeyboardKey.keyX, ctrl: true);
      // Repeated Ctrl+X while the first copy is pending cannot create a second
      // deletion, and changing the live selection to ef cannot redirect it.
      await key(t, LogicalKeyboardKey.keyX, ctrl: true);
      await key(t, LogicalKeyboardKey.end, ctrl: true);
      await key(t, LogicalKeyboardKey.arrowLeft, shift: true);
      await key(t, LogicalKeyboardKey.arrowLeft, shift: true);
      pending.complete();
      await t.pump(); await t.pump(const Duration(milliseconds: 500));
      expect(copied, ['ab']);
      expect(t.widget<DocumentSurface>(find.byType(DocumentSurface)).nodes.single.text, 'cdef');
      await t.pumpWidget(const SizedBox()); await t.pump();

      fail = true; pending = null; copied.clear(); ops.clear();
      await pumpEditor(t, ops, table: true);
      final ctl = t.widget<TextField>(find.byType(TextField)).controller!;
      ctl.selection = const TextSelection(baseOffset: 0, extentOffset: 6);
      await t.pump();
      await key(t, LogicalKeyboardKey.keyX, ctrl: true);
      await t.pump();
      expect(copied, ['abcdef']);
      expect(ctl.text, 'abcdef');
      expect(ctl.selection, const TextSelection(baseOffset: 0, extentOffset: 6));
      await key(t, LogicalKeyboardKey.escape);
      await t.pumpWidget(const SizedBox()); await t.pump();
    } finally {
      if (locker != null) {
        File('$signal.release').writeAsStringSync('release');
        await t.runAsync(() async { await locker!.exitCode; });
      }
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(SystemChannels.platform, null);
      sandbox.deleteSync(recursive: true);
    }
  });
}
