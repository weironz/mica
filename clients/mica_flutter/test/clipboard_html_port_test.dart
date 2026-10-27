@TestOn('vm')
library;

import 'dart:convert';
import 'dart:io';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/highlight.dart';
import 'package:mica_flutter/editor/markdown.dart';
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/markdown/markdown_codec.dart';
import 'package:mica_flutter/markdown/markdown_wire.dart';
import 'support/markdown_codec.dart';

const _goldPath = 'test/fixtures/clipboard_html_port.json';

Map<String, Object?> _oldDartSnapshot(File fixture) {
  final specs = markdownToBlocks(fixture.readAsStringSync());
  final controller = EditorController(rootBlockId: 'root', onOps: (_) async {});
  try {
    controller.load([
      for (var i = 0; i < specs.length; i++)
        EditorNode(
          id: 'b$i',
          kind: specs[i].kind,
          text: specs[i].text,
          data: Map<String, dynamic>.from(specs[i].data),
        ),
    ]);
    final nodes = controller.nodes;
    if (nodes.isNotEmpty) {
      final last = nodes.length - 1;
      controller.selection = DocSelection(
        anchor: const DocPosition(0, 0),
        focus: DocPosition(last, nodes[last].text.length),
      );
    }
    // Full-document slices match _selectionSlices(): every node is complete.
    // Mirror ONLY boundary preparation from _copyLanguage/inlineHtmlOf, never
    // a Markdown/HTML implementation. Expected HTML comes from the old writer.
    final blocks = <BlockSpec>[];
    for (final node in nodes) {
      final data = Map<String, dynamic>.from(node.data);
      if (node.kind == 'code_block') {
        final pinned = (data['language'] as String?)?.trim() ?? '';
        final detected = pinned.isNotEmpty && pinned != 'auto'
            ? pinned
            : detectLanguage(node.text);
        final language =
            detected == 'plaintext' && (pinned.isEmpty || pinned == 'auto')
            ? ''
            : detected;
        if (language.isEmpty) {
          data.remove('language');
        } else {
          data['language'] = language;
        }
      }
      final clipped = <Mark>[];
      for (final mark in marksFromData(data)) {
        final start = mark.start.clamp(0, node.text.length);
        final end = mark.end.clamp(0, node.text.length);
        if (end > start) clipped.add(mark.withRange(start, end));
      }
      if (clipped.isEmpty) {
        data.remove('marks');
      } else {
        data['marks'] = marksToJson(clipped);
      }
      blocks.add((kind: node.kind, text: node.text, data: data));
    }
    return {
      'id': fixture.uri.pathSegments.last,
      'source': fixture.readAsStringSync(),
      'blocks': jsonDecode(encodeBlocks(blocks)),
      'html': controller.selectionHtml(),
    };
  } finally {
    controller.dispose();
  }
}

void main() {
  final directory = Directory(
    '../../crates/markdown/tests/fixtures/conformance',
  );
  final files =
      directory
          .listSync()
          .whereType<File>()
          .where((file) => file.path.endsWith('.md'))
          .toList()
        ..sort((a, b) => a.path.compareTo(b.path));
  // The first 30 named conformance docs cover headings/inline/lists/todo,
  // nested quotes, code/mermaid, tables/images/math/footnotes and escapes.
  final fixtures = files.take(30).toList();
  if (Platform.environment['WRITE_OLD_DART_CLIPBOARD_GOLD'] == '1') {
    test('capture 30 old Dart clipboard snapshots without the native codec', () {
      expect(fixtures.length, 30);
      File(_goldPath).parent.createSync(recursive: true);
      File(_goldPath).writeAsStringSync(
        const JsonEncoder.withIndent(' ').convert({
          'origin':
              'old EditorController.selectionHtml; no Rust used for expected output',
          'cases': fixtures.map(_oldDartSnapshot).toList(),
        }),
      );
    });
    return;
  }
  setUpAll(useNativeMarkdownCodec);
  final gold =
      jsonDecode(File(_goldPath).readAsStringSync()) as Map<String, dynamic>;
  final cases = (gold['cases'] as List).cast<Map<String, dynamic>>();
  test('old Dart clipboard gold covers the selected 30 fixtures', () {
    expect(
      cases.map((item) => item['id']).toList(),
      fixtures.map((file) => file.uri.pathSegments.last).toList(),
    );
  });
  for (var i = 0; i < cases.length; i++) {
    final item = cases[i];
    test('clipboard byte parity ${item['id']}', () {
      final snapshot = _oldDartSnapshot(fixtures[i]);
      expect(
        snapshot,
        item,
        reason: 'Old Dart/source snapshot drift must be reviewed',
      );
      final blocks = decodeBlocks(jsonEncode(item['blocks']));
      expect(
        markdownCodec.renderClipboardHtml(blocks),
        item['html'],
        reason: 'Rust clipboard output must match old Dart byte for byte',
      );
    });
  }
}
