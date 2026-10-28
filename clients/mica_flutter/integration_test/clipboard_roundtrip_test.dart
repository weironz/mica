// S3 T3.10 — the editor's clipboard paths on the REAL desktop app process:
// real Windows clipboard (platform channel, no mock), real FFI engine
// (initCheckedNativeRust verifies this checkout's library), real editor widget
// driven by key events.
//
// Scenarios (S3-editor-boundary.md T3.10), run in ONE testWidgets on purpose:
// the desktop IntegrationTestWidgetsFlutterBinding asserts between tests that
// no async work is pending, and the paste handler is fire-and-forget after the
// key event, so a second testWidgets in the same file failed with
// `'!inTest': is not true` (2026-10-28). Each scenario therefore disposes the
// previous editor, drives its own paste, and waits for the ops it expects.
//
//   1. ChatGPT-style paste: a double-fenced mermaid block becomes a real code
//      block and the rest stays prose (paste_repair heuristic, D7).
//   2. Pasting a `.md` front matter document strips the metadata.
//   3. Rich HTML copied from an external app (CF_HTML on the OS clipboard, the
//      exact shape a browser writes) survives as marks, not literal source.
//   4. Mica-internal Ctrl+A → Ctrl+C → Ctrl+V round trip of a document with a
//      formula, footnote, nested list and table; the clipboard's text/plain
//      AND text/html flavors are both checked.
//   5. AI 「读取当前页」 reads the same Markdown through the engine.
//
// Screenshots of the app window are captured into `build/t3.10/NAME.png`
// (evidence, gitignored).
import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/editor/table.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'package:pasteboard/pasteboard.dart';
import '../test/support/markdown_codec.dart';

Future<void> _key(WidgetTester t, LogicalKeyboardKey k,
    {bool ctrl = false, bool shift = false}) async {
  if (ctrl) await t.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
  if (shift) await t.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
  await t.sendKeyEvent(k);
  if (shift) await t.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
  if (ctrl) await t.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
  await t.pump();
}

/// Screenshot the app window into `build/t3.10/NAME.png` (evidence, gitignored).
Future<void> _shot(String name) async {
  final dir = Directory('build/t3.10')..createSync(recursive: true);
  final path = '${dir.path}/$name.png';
  final script = r'''
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class W {
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out R r);
  public struct R { public int L, T, Rr, B; }
}
"@
$h = [W]::GetForegroundWindow()
$r = New-Object W+R
[void][W]::GetWindowRect($h, [ref]$r)
$w = $r.Rr - $r.L; $ht = $r.B - $r.T
$bmp = New-Object System.Drawing.Bitmap($w, $ht)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($r.L, $r.T, 0, 0, $bmp.Size)
$bmp.Save("PATH", [System.Drawing.Imaging.ImageFormat]::Png)
''';
  await Process.run('powershell.exe', [
    '-NoProfile',
    '-STA',
    '-Command',
    script.replaceAll('PATH', path.replaceAll('/', r'\')),
  ]);
}

/// Dispose whatever editor is mounted, then pump a fresh one and focus it.
Future<void> _pumpEditor(
  WidgetTester t,
  List<EditorNode> nodes,
  _Doc doc,
) async {
  await t.pumpWidget(const SizedBox.shrink());
  await t.pump();
  doc.reset(nodes);
  await t.pumpWidget(
    MaterialApp(
      localizationsDelegates: AppLocalizations.localizationsDelegates,
      supportedLocales: AppLocalizations.supportedLocales,
      home: Scaffold(
        body: MicaEditor(
          rootBlockId: 'root',
          nodes: nodes,
          version: 0,
          canEdit: true,
          onApplyOperations: (batch) async => doc.apply(batch),
        ),
      ),
    ),
  );
  await t.pump();
  final surface = find.byType(DocumentSurface);
  await t.tapAt(
    t.getTopLeft(surface) +
        t.renderObject<RenderDocument>(surface).caretRectFor(
              const DocPosition(0, 0),
            )!.center,
  );
  await t.pump();
  await t.pump(const Duration(milliseconds: 50));
  await _key(t, LogicalKeyboardKey.home, ctrl: true);
}

/// Ctrl+V reads the clipboard asynchronously on the platform thread; pump
/// until [done] holds (or a bounded deadline says it never did), then drain
/// the handler's trailing awaits so the next scenario starts idle.
Future<void> _pasteAndWait(WidgetTester t, bool Function() done) async {
  await _key(t, LogicalKeyboardKey.keyV, ctrl: true);
  final deadline = DateTime.now().add(const Duration(seconds: 20));
  while (!done() && DateTime.now().isBefore(deadline)) {
    await t.pump(const Duration(milliseconds: 100));
  }
  for (var i = 0; i < 10; i++) {
    await t.pump(const Duration(milliseconds: 50));
  }
  expect(done(), isTrue, reason: 'paste produced no ops before the deadline');
}

/// The document as the backend would see it, folded from the ops the editor
/// emitted. Paste REUSES an empty focused paragraph (the first pasted block
/// arrives as `update_block`, not `insert_block`), so assertions must look at
/// the folded document, not only at inserted blocks.
class _Doc {
  final List<String> _order = [];
  final Map<String, ({String type, String text, Map<String, dynamic> data})>
      _byId = {};

  void reset(List<EditorNode> nodes) {
    _order
      ..clear()
      ..addAll([for (final n in nodes) n.id]);
    _byId.clear();
    for (final n in nodes) {
      _byId[n.id] = (type: n.kind, text: n.text, data: n.data);
    }
  }

  void apply(List<DocOp> ops) {
    for (final op in ops) {
      switch (op['type']) {
        case 'insert_block':
          final b = (op['block'] as Map).cast<String, dynamic>();
          final id = b['id'] as String;
          _order.add(id);
          _byId[id] = (
            type: b['type'] as String,
            text: b['text'] as String? ?? '',
            data: (b['data'] as Map?)?.cast<String, dynamic>() ??
                <String, dynamic>{},
          );
        case 'update_block':
          final id = op['block_id'] as String;
          final prev = _byId[id];
          if (prev == null) continue;
          _byId[id] = (
            type: (op['kind'] as String?) ?? prev.type,
            text: (op['text'] as String?) ?? prev.text,
            data: (op['data'] as Map?)?.cast<String, dynamic>() ?? prev.data,
          );
        case 'delete_block':
          final id = op['block_id'] as String;
          _order.remove(id);
          _byId.remove(id);
      }
    }
  }

  List<({String type, String text, Map<String, dynamic> data})> get blocks =>
      [for (final id in _order) if (_byId[id] != null) _byId[id]!];
}

/// A CF_HTML document, the clipboard format Windows browsers write. Offsets are
/// computed against the fixed-width header; the fragments tag marks the
/// selection so `stripCfHtmlHeader` cuts exactly what was copied.
String _cfHtml(String fragment) {
  String header(int endHtml, int startFrag, int endFrag) =>
      'Version:0.9\r\n'
      'StartHTML:${0.toString().padLeft(10, '0')}\r\n'
      'EndHTML:${endHtml.toString().padLeft(10, '0')}\r\n'
      'StartFragment:${startFrag.toString().padLeft(10, '0')}\r\n'
      'EndFragment:${endFrag.toString().padLeft(10, '0')}\r\n';
  const prefix = '<html><body><!--StartFragment-->';
  const suffix = '<!--EndFragment--></body></html>';
  // CF_HTML offsets are UTF-8 BYTE offsets, not Dart string indices.
  final headerLen = header(0, 0, 0).length; // ASCII header: bytes == length
  final startFrag = headerLen + prefix.length;
  final endFrag = startFrag + utf8.encode(fragment).length;
  final endHtml = endFrag + suffix.length; // ASCII suffix
  return header(endHtml, startFrag, endFrag) + prefix + fragment + suffix;
}

/// Put HTML + plain text on the OS clipboard exactly as an external app does.
///
/// Win32 directly, not System.Windows.Forms.DataObject: `SetData('HTML
/// Format', string)` stores UTF-16 (the pasteboard plugin reads UTF-8 and
/// throws), and passing a byte[] makes .NET ALSO advertise the text format as
/// the literal string "System.Byte[]" — which the paste then reads as the
/// clipboard's plain text (both observed 2026-10-28). CF_HTML is UTF-8 bytes
/// plus a terminating NUL, CF_UNICODETEXT is UTF-16LE; that is what browsers
/// put there.
Future<void> _setHtmlClipboard(String html, String plain) async {
  final b64Html = base64Encode(utf8.encode(html));
  final b64Plain = base64Encode(utf8.encode(plain));
  final dir = Directory('build/t3.10')..createSync(recursive: true);
  final script = File('${dir.path}/set-html-clipboard.ps1')
    ..writeAsStringSync(r'''
param([string]$b64Html, [string]$b64Plain)
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class ClipWriter {
  [DllImport("user32.dll", SetLastError=true)] public static extern bool OpenClipboard(IntPtr h);
  [DllImport("user32.dll")] public static extern bool EmptyClipboard();
  [DllImport("user32.dll")] public static extern IntPtr SetClipboardData(uint fmt, IntPtr h);
  [DllImport("user32.dll")] public static extern bool CloseClipboard();
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern uint RegisterClipboardFormat(string name);
  [DllImport("kernel32.dll")] public static extern IntPtr GlobalAlloc(uint flags, UIntPtr bytes);
  [DllImport("kernel32.dll")] public static extern IntPtr GlobalLock(IntPtr h);
  [DllImport("kernel32.dll")] public static extern bool GlobalUnlock(IntPtr h);
  static void Put(uint fmt, byte[] bytes) {
    IntPtr h = GlobalAlloc(0x42, (UIntPtr)(bytes.Length + 1));
    IntPtr p = GlobalLock(h);
    Marshal.Copy(bytes, 0, p, bytes.Length);
    GlobalUnlock(h);
    SetClipboardData(fmt, h);
  }
  public static void Set(string html, string text) {
    OpenClipboard(IntPtr.Zero);
    EmptyClipboard();
    Put(RegisterClipboardFormat("HTML Format"), System.Text.Encoding.UTF8.GetBytes(html));
    Put(13, System.Text.Encoding.Unicode.GetBytes(text + "\0"));
    CloseClipboard();
  }
}
"@
[ClipWriter]::Set([Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($b64Html)), [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($b64Plain)))
''');
  final result = await Process.run('powershell.exe', [
    '-NoProfile',
    '-STA',
    '-ExecutionPolicy',
    'Bypass',
    '-File',
    script.path,
    b64Html,
    b64Plain,
  ]);
  if (result.exitCode != 0) {
    fail('could not set the HTML clipboard: ${result.stderr}');
  }
}

/// Empty the OS clipboard so a leftover foreign flavor (the user's own copy, or
/// an earlier scenario's HTML) cannot steer the paste under test.
Future<void> _clearClipboard() async {
  await Process.run('powershell.exe', [
    '-NoProfile',
    '-STA',
    '-Command',
    'Add-Type -AssemblyName System.Windows.Forms; '
        '[System.Windows.Forms.Clipboard]::Clear()',
  ]);
}

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(() async {
    await useNativeMarkdownCodec();
  });

  testWidgets('desktop clipboard real paths (S3 T3.10)', (t) async {
    await _clearClipboard();
    // ---------------------------------------------------------------- 1 ----
    // ChatGPT double fence: inner mermaid becomes a real block, prose around.
    const fenceSrc = '建议:\n\n'
        '```\n'
        '```mermaid\n'
        'flowchart LR\n'
        '  A --> B\n'
        '```\n'
        '```\n\n'
        '你们这张拓扑图里\n\n'
        '- 第一条';
    await Clipboard.setData(const ClipboardData(text: fenceSrc));
    final doc = _Doc();
    await _pumpEditor(t, [EditorNode(id: 'a', kind: 'paragraph', text: '')], doc);
    await _pasteAndWait(t, () => doc.blocks.isNotEmpty);
    await _shot('01-double-fence-paste');
    {
      final blocks = doc.blocks;
      final kinds = blocks.map((b) => b.type).toList();
      expect(kinds, contains('code_block'));
      final mermaid = blocks.where((b) => b.type == 'code_block').single;
      expect(mermaid.data['language'], 'mermaid');
      expect(kinds, contains('paragraph'));
      expect(kinds, contains('bulleted_list'));
    }

    // ---------------------------------------------------------------- 2 ----
    // Front matter in a pasted .md is stripped, never a visible block.
    const fmSrc = '---\n'
        'title: Hello\n'
        'tags: [a, b]\n'
        '---\n\n'
        '# Heading\n\n'
        'Body.';
    await Clipboard.setData(const ClipboardData(text: fmSrc));
    await _pumpEditor(t, [EditorNode(id: 'a', kind: 'paragraph', text: '')], doc);
    await _pasteAndWait(t, () => doc.blocks.isNotEmpty);
    await _shot('02-front-matter-paste');
    {
      final blocks = doc.blocks;
      final text = blocks.map((b) => b.text).join('\n');
      expect(text, contains('Heading'));
      expect(text, contains('Body.'));
      expect(text, isNot(contains('title:')));
      expect(text, isNot(contains('tags:')));
      expect(blocks.map((b) => b.type), contains('heading'));
    }

    // ---------------------------------------------------------------- 3 ----
    // Rich HTML copied from an external app keeps real marks, not source.
    const fragment =
        '<p>复制自浏览器的<strong>粗体</strong>与<code>inline_code</code>。</p>';
    await _setHtmlClipboard(_cfHtml(fragment), '复制自浏览器的粗体与inline_code。');
    await _pumpEditor(t, [EditorNode(id: 'a', kind: 'paragraph', text: '')], doc);
    await _pasteAndWait(t, () => doc.blocks.isNotEmpty);
    await _shot('03-rich-html-paste');
    {
      final blocks = doc.blocks;
      final types = <String>[];
      var text = '';
      for (final b in blocks) {
        text += b.text;
        final marks = b.data['marks'];
        if (marks is List) {
          for (final m in marks) {
            types.add('${(m as Map)['type']}');
          }
        }
      }
      expect(text, contains('粗体'));
      expect(types, contains('bold'), reason: 'rich HTML must paste real marks');
      expect(types, contains('code'));
      expect(text, isNot(contains('**')), reason: 'no literal markdown source');
    }

    // ---------------------------------------------------------------- 4 ----
    // Mica copy → paste round-trips formula, footnote, nested list, table.
    final table = TableData([
      ['H1', 'H2'],
      [r'**cell**', 'x'],
    ]);
    final docNodes = [
      EditorNode(
        id: 'p',
        kind: 'paragraph',
        text: '公式 E=mc^2 与脚注x end',
        data: {
          'marks': marksToJson([
            Mark(3, 9, 'math'),
            Mark(15, 16, 'footnote', href: 'n'),
          ]),
        },
      ),
      EditorNode(id: 'b', kind: 'bulleted_list', text: 'one'),
      EditorNode(id: 'b2', kind: 'bulleted_list', text: 'nested', data: {'indent': 1}),
      EditorNode(id: 'n', kind: 'numbered_list', text: 'first', data: {'start': 3}),
      EditorNode(id: 't', kind: 'table', text: '', data: table.toBlockData()),
      EditorNode(id: 'f', kind: 'footnote_def', text: 'note body', data: {'label': 'n'}),
    ];
    await _pumpEditor(t, docNodes, doc);
    await _key(t, LogicalKeyboardKey.keyA, ctrl: true);
    await _key(t, LogicalKeyboardKey.keyC, ctrl: true);
    for (var i = 0; i < 10; i++) {
      await t.pump(const Duration(milliseconds: 30));
    }
    await _shot('04-copy-source');
    // text/plain is the LITERAL flavor (marks and block markers stripped) —
    // a plain-text destination must not see `**` or list markers. The
    // structure travels in the HTML flavor checked next.
    final plain = (await Clipboard.getData(Clipboard.kTextPlain))?.text ?? '';
    // The rich flavor Typora/Obsidian read: real html marks, a real table.
    final html = await Pasteboard.html;
    {
      expect(plain, contains('E=mc^2'), reason: plain);
      expect(plain, contains('one'), reason: plain);
      expect(plain, contains('nested'), reason: plain);
      expect(plain, contains('first'), reason: plain);
      expect(plain, contains('H1\tH2'),
          reason: 'table cells go out as TSV: $plain');
      expect(plain, contains('cell'), reason: plain);
      expect(plain, contains('note body'), reason: plain);
      expect(plain, isNot(contains('**')), reason: plain);
      expect(html, isNotNull);
      expect(html, contains('<table>'));
      expect(html, contains('<strong>cell</strong>'), reason: html);
      expect(html, contains('data-mica-math'), reason: html);
    }

    // Paste back into a FRESH editor: structure must survive.
    await _pumpEditor(t, [EditorNode(id: 'z', kind: 'paragraph', text: '')], doc);
    await _pasteAndWait(t, () => doc.blocks.any((b) => b.type == 'table'));
    await _shot('05-paste-roundtrip');
    {
      final blocks = doc.blocks;
      final kinds = blocks.map((b) => b.type).toList();
      expect(kinds, contains('table'));
      final back = blocks.firstWhere((b) => b.type == 'table');
      final rows = back.data['rows'] as List;
      expect(rows.length, 2);
      expect(rows.first, ['H1', 'H2']);
      expect(kinds, contains('footnote_def'));
      expect(kinds, contains('bulleted_list'));
      expect(kinds, contains('numbered_list'));
      final mathText = blocks.firstWhere(
        (b) =>
            b.data['marks'] is List &&
            (b.data['marks'] as List)
                .any((m) => (m as Map)['type'] == 'math'),
      );
      expect(mathText.text, contains('E=mc^2'));
    }

    // ---------------------------------------------------------------- 5 ----
    // AI current page: the same Markdown the AI dialog sends.
    final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
    c.load([
      EditorNode(id: 'h', kind: 'heading', text: 'Title', data: {'level': 1}),
      EditorNode(id: 'p', kind: 'paragraph', text: 'hi there', data: {
        'marks': marksToJson([Mark(0, 2, 'bold')]),
      }),
      EditorNode(id: 'l', kind: 'bulleted_list', text: 'item'),
      EditorNode(id: 't', kind: 'table', text: '', data: TableData([
        ['A', 'B'],
        ['1', '2'],
      ]).toBlockData()),
    ]);
    final md = c.documentMarkdown();
    c.dispose();
    // ignore: avoid_print
    print('AI current-page markdown:\n$md');
    expect(md, contains('# Title'));
    expect(md, contains('**hi** there'));
    expect(md, contains('- item'));
    expect(md, contains('| A | B |'));

    await t.pumpWidget(const SizedBox.shrink());
    await t.pump();
  });
}