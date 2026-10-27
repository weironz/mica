@TestOn('vm')
library;

import 'dart:convert';
import 'dart:io';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/markdown.dart';
import 'package:mica_flutter/markdown/markdown_codec.dart';
import 'package:mica_flutter/src/rust/api/document.dart';
import 'support/markdown_codec.dart';
import 'support/markdown_corpus.dart';

// One-time T2.12 migration audit. The legacy CRDT path is intentionally used
// ONLY to explain old baseline differences, never as the new comparison path.
void main() {
  setUpAll(useNativeMarkdownCodec);
  test('new old-dimension entries come from removal of the CRDT boundary', () {
    final evidence = File('test/fixtures/differential/t212_transition.json');
    if (!evidence.existsSync()) {
      throw StateError('Run the old/new baseline transition comparison first');
    }
    final transition =
        jsonDecode(evidence.readAsStringSync()) as Map<String, dynamic>;
    final added = (transition['addedDetails'] as List)
        .cast<Map<String, dynamic>>()
        .where((item) => !(item['category'] as String).startsWith('inline:'))
        .toList();
    expect(added.length, 74);
    final changed = (transition['changedDetails'] as List)
        .cast<Map<String, dynamic>>();
    final inputs = {
      for (final entry in markdownCorpus()) entry.id: entry.markdown,
    };
    Object? canonical(Object? value) {
      if (value is Map) {
        final keys = value.keys.cast<String>().toList()..sort();
        return {for (final key in keys) key: canonical(value[key])};
      }
      if (value is List) return value.map(canonical).toList();
      return value;
    }

    String short(String value) => value.length <= 300
        ? value
        : '${value.substring(0, 300)}…(${value.length} chars)';
    for (final item in changed.where(
      (item) => (item['new'] as Map)['category'] == 'export',
    )) {
      final value = item['new'] as Map;
      final blocks = markdownToBlocks(inputs[value['id']]!);
      final ids = [for (var i = 0; i < blocks.length; i++) 'd$i'];
      final stored = MicaDocument.fromBlocksJson(
        rootId: 'root',
        blocksJson: jsonEncode([
          {
            'id': 'root',
            'type': 'page',
            'text': '',
            'data': {},
            'children': ids,
          },
          for (var i = 0; i < blocks.length; i++)
            {
              'id': ids[i],
              'type': blocks[i].kind,
              'text': blocks[i].text,
              'data': blocks[i].data,
              'children': [],
            },
        ]),
      );
      expect(short(stored.exportMarkdown()), (item['old'] as Map)['rust']);
      expect(short(markdownCodec.serializeBlocks(blocks)), value['rust']);
    }
    // For every changed non-export source, prove the raw engine result becomes
    // the legacy parse result when fed through the SAME old CRDT boundary.
    for (final id
        in changed
            .where((item) => (item['new'] as Map)['category'] != 'export')
            .map((item) => (item['new'] as Map)['id'] as String)
            .toSet()) {
      final blocks = markdownCodec.parseBlocks(inputs[id]!);
      final ids = [for (var i = 0; i < blocks.length; i++) 'd$i'];
      final fromRaw = MicaDocument.fromBlocksJson(
        rootId: 'root',
        blocksJson: jsonEncode([
          {
            'id': 'root',
            'type': 'page',
            'text': '',
            'data': {},
            'children': ids,
          },
          for (var i = 0; i < blocks.length; i++)
            {
              'id': ids[i],
              'type': blocks[i].kind,
              'text': blocks[i].text,
              'data': blocks[i].data,
              'children': [],
            },
        ]),
      );
      List<Object?> project(MicaDocument doc) {
        final all = jsonDecode(doc.toBlocksJson()) as List;
        final byId = {for (final block in all) block['id']: block};
        final root = byId[doc.rootBlockId()];
        return [
          for (final child in root['children'] as List)
            canonical({
              'type': byId[child]['type'],
              'text': byId[child]['text'],
              'data': byId[child]['data'],
            }),
        ];
      }

      expect(
        project(fromRaw),
        project(MicaDocument.fromMarkdown(markdown: inputs[id]!)),
        reason: id,
      );
    }
    final results = <Map<String, dynamic>>[];
    for (final id in added.map((entry) => entry['id'] as String).toSet()) {
      final source = inputs[id]!;
      final raw = markdownCodec.parseBlocks(source);
      final legacy = MicaDocument.fromMarkdown(markdown: source);
      final stored = jsonDecode(legacy.toBlocksJson()) as List;
      final root =
          stored.firstWhere((block) => block['id'] == legacy.rootBlockId())
              as Map;
      final byId = {for (final block in stored) block['id']: block};
      final projected = [
        for (final child in root['children'] as List)
          {
            'type': byId[child]['type'],
            'text': byId[child]['text'],
            'data': byId[child]['data'],
          },
      ];
      if (added.any(
        (entry) => entry['id'] == id && entry['category'] == 'export',
      )) {
        // The two empty-link vectors serialize directly but the old CRDT
        // export path loses the zero-width link, explaining the new records.
        final dart = markdownToBlocks(source);
        expect(markdownCodec.serializeBlocks(dart), isNotEmpty);
        expect(legacy.exportMarkdown(), isEmpty);
      } else {
        final rawMarks = [for (final block in raw) block.data['marks']];
        final storedMarks = [
          for (final block in projected) (block['data'] as Map?)?['marks'],
        ];
        expect(
          jsonEncode(rawMarks),
          isNot(jsonEncode(storedMarks)),
          reason: '$id: must prove a changed CRDT mark representation',
        );
      }
      results.add({
        'id': id,
        'raw': [
          for (final b in raw) {'type': b.kind, 'text': b.text, 'data': b.data},
        ],
        'crdt': projected,
      });
    }
    File(
      'build/differential/t212-crdt-audit.json',
    ).writeAsStringSync(const JsonEncoder.withIndent(' ').convert(results));
  });
}
