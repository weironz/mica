import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'support/markdown_codec.dart';

void main() {
  setUpAll(useNativeMarkdownCodec);
  for (final replace in [false, true]) {
    for (final prefix in ['', 'remote ']) {
      testWidgets(
        'AI uses remapped selection after remote reorder, replace=$replace, prefix=$prefix',
        (tester) async {
          final writes = <Map<String, dynamic>>[];
          final focus = FocusNode();
          addTearDown(focus.dispose);
          var version = 0;
          var nodes = [
            for (var i = 0; i < 4; i++)
              EditorNode(id: 'pad$i', kind: 'paragraph', text: 'padding line'),
            EditorNode(id: 'target', kind: 'paragraph', text: 'selected text'),
            EditorNode(id: 'tail', kind: 'paragraph', text: 'tail text'),
          ];
          late StateSetter update;
          await tester.pumpWidget(
            MaterialApp(
              localizationsDelegates: AppLocalizations.localizationsDelegates,
              supportedLocales: AppLocalizations.supportedLocales,
              locale: const Locale('zh'),
              home: Scaffold(
                body: StatefulBuilder(
                  builder: (context, setState) {
                    update = setState;
                    return MicaEditor(
                      rootBlockId: 'root',
                      nodes: nodes,
                      version: version,
                      canEdit: true,
                      focusNode: focus,
                      onApplyOperations: (ops) async => writes.addAll(ops),
                      onAiStream: (prompt, {system}) =>
                          Stream.value('rewritten'),
                    );
                  },
                ),
              ),
            ),
          );
          await tester.pump();
          await tester.tapAt(
            tester.getTopLeft(find.byType(MicaEditor)) + const Offset(40, 170),
          );
          await tester.pump();
          focus.requestFocus();
          await tester.pump();
          final client =
              tester.state(find.byType(MicaEditor)) as TextInputClient;
          expect(client.currentTextEditingValue!.text, 'selected text');
          client.updateEditingValue(
            const TextEditingValue(
              text: 'selected text',
              selection: TextSelection(baseOffset: 13, extentOffset: 0),
            ),
          );
          await tester.pump();
          await tester.pump();
          expect(
            client.currentTextEditingValue!.selection.isCollapsed,
            isFalse,
          );
          await tester.tap(find.widgetWithIcon(IconButton, Icons.auto_awesome));
          await tester.pumpAndSettle();
          await tester.enterText(
            find.descendant(
              of: find.byType(AlertDialog),
              matching: find.byType(TextField),
            ),
            'rewrite',
          );
          await tester.tap(find.text('生成'));
          await tester.pumpAndSettle();
          // Inserting a remote block changes every old node index. The dialog
          // remains open, so only the reconciled live selection knows its end.
          update(() {
            nodes = [
              EditorNode(id: 'remote', kind: 'paragraph', text: 'remote text'),
              ...nodes.map(
                (node) => node.id == 'target'
                    ? EditorNode(
                        id: node.id,
                        kind: node.kind,
                        text: '${prefix}selected text',
                      )
                    : node,
              ),
            ];
            version++;
          });
          await tester.pump();
          await tester.tap(find.text(replace ? '替换所选' : '插到下面'));
          await tester.pumpAndSettle();
          if (replace) {
            expect(
              writes.any(
                (op) =>
                    op['block_id'] == 'target' &&
                    op['text'] == (prefix.isEmpty ? 'rewritten' : prefix),
              ),
              isTrue,
            );
            expect(writes.any((op) => op['block_id'] == 'pad3'), isFalse);
          }
          if (!replace || prefix.isNotEmpty) {
            final insert = writes.singleWhere(
              (op) => op['type'] == 'insert_block',
            );
            expect(insert['index'], 6);
            expect((insert['block'] as Map)['text'], 'rewritten');
          }
          await tester.pumpWidget(const SizedBox.shrink());
          await tester.pump();
        },
      );
    }
  }
}
