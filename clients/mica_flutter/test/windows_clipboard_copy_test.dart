import 'dart:convert';
import 'dart:ffi';
import 'dart:io';

import 'package:ffi/ffi.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/clipboard_copy_stub.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/html_to_markdown.dart';
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';

// Read OS-owned buffers independently of the production writer. No platform
// clipboard mock: this verifies both native formats and CF_HTML byte offsets.
Future<List<int>> readFormat(int format) async {
  final user = DynamicLibrary.open('user32.dll');
  final kernel = DynamicLibrary.open('kernel32.dll');
  final open = user.lookupFunction<Int32 Function(IntPtr), int Function(int)>(
    'OpenClipboard',
  );
  final close = user.lookupFunction<Int32 Function(), int Function()>(
    'CloseClipboard',
  );
  final get = user.lookupFunction<IntPtr Function(Uint32), int Function(int)>(
    'GetClipboardData',
  );
  final lock = kernel
      .lookupFunction<
        Pointer<Uint8> Function(IntPtr),
        Pointer<Uint8> Function(int)
      >('GlobalLock');
  final unlock = kernel
      .lookupFunction<Int32 Function(IntPtr), int Function(int)>(
        'GlobalUnlock',
      );
  final size = kernel
      .lookupFunction<IntPtr Function(IntPtr), int Function(int)>('GlobalSize');
  var opened = false;
  for (var attempt = 0; attempt < 8; attempt++) {
    if (open(0) != 0) {
      opened = true;
      break;
    }
    await Future<void>.delayed(const Duration(milliseconds: 10));
  }
  expect(opened, isTrue);
  try {
    final handle = get(format);
    expect(
      handle,
      isNonZero,
      reason: 'native clipboard format $format must exist',
    );
    final ptr = lock(handle);
    expect(ptr, isNot(nullptr));
    try {
      return List<int>.from(ptr.asTypedList(size(handle)));
    } finally {
      unlock(handle);
    }
  } finally {
    close();
  }
}

Future<String> readHtml() async {
  final name = 'HTML Format'.toNativeUtf16();
  final register = DynamicLibrary.open('user32.dll')
      .lookupFunction<
        Uint32 Function(Pointer<Utf16>),
        int Function(Pointer<Utf16>)
      >('RegisterClipboardFormatW');
  final format = register(name);
  malloc.free(name);
  final bytes = await readFormat(format);
  final header = ascii.decode(
    bytes.takeWhile((b) => b != 0).take(105).toList(),
  );
  int offset(String field) =>
      int.parse(RegExp('$field:(\\d+)').firstMatch(header)!.group(1)!);
  expect(bytes[offset('EndHTML')], 0);
  return utf8.decode(
    bytes.sublist(offset('StartFragment'), offset('EndFragment')),
  );
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  test(
    'native Windows clipboard carries literal text and rich code semantics',
    () async {
      var fallback = false;
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(SystemChannels.platform, (call) async {
            if (call.method == 'Clipboard.setData') fallback = true;
            return null;
          });
      addTearDown(
        () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
            .setMockMethodCallHandler(SystemChannels.platform, null),
      );
      const plain = '中文 😀 a`b and `literal`';
      const html = '<p>中文 😀 <code>a`b</code> and `literal`</p>';
      expect(await copyRichToClipboard(plain: plain, richHtml: html), isTrue);
      expect(
        fallback,
        isFalse,
        reason: 'must exercise Win32, not the Flutter fallback',
      );
      final textBytes = await readFormat(13);
      final units = <int>[];
      for (var i = 0; i + 1 < textBytes.length; i += 2) {
        final unit = textBytes[i] | (textBytes[i + 1] << 8);
        if (unit == 0) break;
        units.add(unit);
      }
      expect(String.fromCharCodes(units), plain);
      final fragment = await readHtml();
      expect(fragment, html);
      final parsed = parseInline(htmlToMarkdown(fragment));
      expect(parsed.text, plain);
      expect(parsed.marks.single.type, 'code');
    },
    skip: !Platform.isWindows,
  );
  for (final cut in [false, true]) {
    testWidgets(
      'cell context menu ${cut ? 'cut' : 'copy'} writes semantic HTML',
      (tester) async {
        await tester.pumpWidget(
          MaterialApp(
            localizationsDelegates: AppLocalizations.localizationsDelegates,
            supportedLocales: AppLocalizations.supportedLocales,
            locale: const Locale('en'),
            home: Scaffold(
              body: MicaEditor(
                rootBlockId: 'root',
                nodes: [
                  EditorNode(
                    id: 't',
                    kind: 'table',
                    text: '',
                    data: {
                      'rows': [
                        [r'`command` then \`literal\`', 'end'],
                      ],
                    },
                  ),
                ],
                version: 0,
                canEdit: true,
                onApplyOperations: (_) async {},
              ),
            ),
          ),
        );
        await tester.pump();
        final render = tester.renderObject<RenderDocument>(
          find.byType(DocumentSurface),
        );
        final origin = tester.getTopLeft(find.byType(DocumentSurface));
        await tester.tapAt(origin + render.tableCellRect(0, 0, 0)!.center);
        await tester.pump();
        await tester.pump(const Duration(milliseconds: 50));
        final field = tester.widget<TextField>(find.byType(TextField));
        final controller = field.controller!;
        controller.selection = TextSelection(
          baseOffset: 0,
          extentOffset: controller.text.length,
        );
        await tester.pump();
        final state = tester.state<EditableTextState>(
          find.byType(EditableText),
        );
        final toolbar =
            field.contextMenuBuilder!(
                  tester.element(find.byType(TextField)),
                  state,
                )
                as AdaptiveTextSelectionToolbar;
        toolbar.buttonItems!
            .singleWhere(
              (item) =>
                  item.type ==
                  (cut
                      ? ContextMenuButtonType.cut
                      : ContextMenuButtonType.copy),
            )
            .onPressed!();
        await tester.pump();
        final html = await tester.runAsync(readHtml);
        expect(html, '<code>command</code> then `literal`');
        expect(controller.text, cut ? '' : 'command then `literal`');
        await tester.pumpWidget(const SizedBox());
        await tester.pump();
      },
      skip: !Platform.isWindows,
    );
  }

  testWidgets('Windows rich paste restores Gemini math and keeps bold', (
    tester,
  ) async {
    final fixture =
        jsonDecode(
              File('test/fixtures/gemini_inline_math.json').readAsStringSync(),
            )
            as Map<String, dynamic>;
    final plain = fixture['plain'] as String;
    expect(
      await copyRichToClipboard(
        plain: plain,
        richHtml: fixture['html'] as String,
      ),
      isTrue,
    );
    final messenger =
        TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
    // Widget tests have no native plugin host. Bridge its reads to the actual
    // CF_HTML buffer written above; only the platform channel is substituted.
    messenger.setMockMethodCallHandler(const MethodChannel('pasteboard'), (
      call,
    ) async {
      if (call.method == 'html') return readHtml();
      return null;
    });
    messenger.setMockMethodCallHandler(SystemChannels.platform, (call) async {
      if (call.method == 'Clipboard.getData') return {'text': plain};
      return null;
    });
    addTearDown(() {
      messenger.setMockMethodCallHandler(
        const MethodChannel('pasteboard'),
        null,
      );
      messenger.setMockMethodCallHandler(SystemChannels.platform, null);
    });
    final commands = EditorCommandHook();
    await tester.pumpWidget(
      MaterialApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: Scaffold(
          body: MicaEditor(
            rootBlockId: 'root',
            nodes: [EditorNode(id: 'empty', kind: 'paragraph', text: '')],
            version: 0,
            canEdit: true,
            commandHook: commands,
            onApplyOperations: (_) async {},
          ),
        ),
      ),
    );
    commands.focusFirstLine();
    await tester.pump();
    await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyV);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    // Platform reads and toImage complete on real time; wait for their result
    // instead of assuming 30 zero-duration frames also finish native I/O.
    for (var i = 0; i < 100; i++) {
      await tester.pump(const Duration(milliseconds: 20));
      await tester.runAsync(
        () => Future<void>.delayed(const Duration(milliseconds: 10)),
      );
      final surface = tester.widget<DocumentSurface>(
        find.byType(DocumentSurface),
      );
      if (surface.nodes.single.text.isNotEmpty &&
          surface.previewImages['math']?[r'\rightarrow'] != null) {
        break;
      }
    }
    final surface = tester.widget<DocumentSurface>(
      find.byType(DocumentSurface),
    );
    final node = surface.nodes.single;
    expect(node.kind, 'paragraph');
    expect(node.text, plain.replaceAll(r'$\rightarrow$', r'\rightarrow'));
    final marks = marksFromData(node.data);
    expect(marks.where((m) => m.type == 'math'), hasLength(1));
    expect(
      marks.where((m) => m.type == 'bold'),
      hasLength(1),
      reason: 'rich HTML must win over single-line plain math fast path',
    );
    expect(
      surface.previewImages['math']?[r'\rightarrow'],
      isNotNull,
      reason: 'the real editor must rasterize the arrow, not just store a mark',
    );
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox());
    await tester.pump();
  }, skip: !Platform.isWindows);

  testWidgets('cut accepts an atomic image with only an HTML flavor', (
    tester,
  ) async {
    final commands = EditorCommandHook();
    final ops = <Map<String, dynamic>>[];
    await tester.pumpWidget(
      MaterialApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: Scaffold(
          body: MicaEditor(
            rootBlockId: 'root',
            nodes: [EditorNode(id: 'image', kind: 'image', text: '')],
            version: 0,
            canEdit: true,
            commandHook: commands,
            onApplyOperations: (batch) async => ops.addAll(batch),
          ),
        ),
      ),
    );
    await tester.pump();
    commands.focusFirstLine();
    await tester.pump();
    await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyX);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    await tester.pump();
    expect(
      ops.any(
        (op) => op['type'] == 'delete_block' && op['block_id'] == 'image',
      ),
      isTrue,
    );
    await tester.pumpWidget(const SizedBox());
    await tester.pump();
  }, skip: !Platform.isWindows);
}
