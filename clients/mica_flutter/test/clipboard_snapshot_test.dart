// S0 clipboard snapshot: the Dart copy output BEFORE S3 moves it to Rust.
//
// S3 replaces `selectionText` / `selectionHtml` / `selectionClipboardText` with
// the engine. To report what a user would see change, the pre-switch output is
// captured here from a fixed set of conformance fixtures. It is a snapshot, not
// a golden: the file is written under `build/` (gitignored) and compared by
// hand at S3, because the point is the DELTA, not the bytes.
//
// Regenerate on any commit before S3:
//
//   cd clients/mica_flutter
//   flutter test test/clipboard_snapshot_test.dart
//   # -> build/differential/clipboard_baseline.json
//
// The fixture set is the first 30 `*.md` files under
// `crates/markdown/tests/fixtures/conformance/` in sorted order.
//
// @TestOn('vm') — file output.
@TestOn('vm')
library;

import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/markdown.dart';
import 'package:mica_flutter/editor/model.dart';

import 'support/markdown_corpus.dart';

void main() {
  test('capture clipboard output for 30 fixtures', () {
    final dir = Directory('$markdownFixtureRoot/conformance');
    final files = dir
        .listSync()
        .whereType<File>()
        .where((f) => f.path.endsWith('.md'))
        .toList()
      ..sort((a, b) => a.path.compareTo(b.path));
    expect(files.length, greaterThanOrEqualTo(30),
        reason: 'the snapshot set is defined as the first 30 fixtures');

    final out = <String, Map<String, String>>{};
    for (final file in files.take(30)) {
      final name = file.uri.pathSegments.last;
      final controller =
          EditorController(rootBlockId: 'root', onOps: (_) async {});
      try {
        final specs = markdownToBlocks(file.readAsStringSync());
        controller.load([
          for (var i = 0; i < specs.length; i++)
            EditorNode(
              id: 'b$i',
              kind: specs[i].kind,
              text: specs[i].text,
              data: Map<String, dynamic>.from(specs[i].data),
            ),
        ]);
        if (controller.nodes.isEmpty) {
          out[name] = {'text': '', 'html': '', 'clipboard_text': ''};
          continue;
        }
        final last = controller.nodes.length - 1;
        controller.selection = DocSelection(
          anchor: const DocPosition(0, 0),
          focus: DocPosition(last, controller.nodes[last].text.length),
        );
        out[name] = {
          'text': controller.selectionText(),
          'html': controller.selectionHtml(),
          'clipboard_text': controller.selectionClipboardText(),
        };
      } finally {
        controller.dispose();
      }
    }

    final output = File('build/differential/clipboard_baseline.json');
    output.parent.createSync(recursive: true);
    output.writeAsStringSync(const JsonEncoder.withIndent(' ').convert(out));

    expect(out.length, 30);
    expect(output.existsSync(), isTrue);
    // ignore: avoid_print
    print('clipboard snapshot: ${output.absolute.path}');
  });
}