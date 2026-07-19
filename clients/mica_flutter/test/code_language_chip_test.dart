import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/render.dart';

// A code block's language is either PINNED (the author chose it, and it never
// moves) or AUTO (re-detected from the content on every layout, so pasting YAML
// relabels it). Both states rendered the same chip — the *resolved* name — so
// there was no way to tell which one you had, and therefore no way to know that
// switching to `auto` was what you wanted.

const yaml = 'services:\n  api:\n    image: mica\n    ports:\n      - "80:80"\n';

void main() {
  Future<String> chipOf(WidgetTester tester, Map<String, dynamic> data) async {
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: DocumentSurface(
            nodes: [
              EditorNode(id: 'c', kind: 'code_block', text: yaml, data: data),
            ],
            selection: null,
            showCaret: false,
            caretBlink: ValueNotifier(false),
            appearance: const EditorAppearance(),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    return tester
        .renderObject<RenderDocument>(find.byType(DocumentSurface))
        .debugLangChipAt(0);
  }

  testWidgets('an auto block shows the detected language, no auto prefix',
      (tester) async {
    // The chip face is just the resolved name now — the auto-vs-pinned
    // distinction moved off the chip (into the ⋯ menu / language picker).
    expect(await chipOf(tester, {}), 'yaml');
  });

  testWidgets('`auto` written out explicitly reads the same', (tester) async {
    expect(await chipOf(tester, {'language': 'auto'}), 'yaml');
  });

  testWidgets('a pinned block shows the bare language', (tester) async {
    expect(await chipOf(tester, {'language': 'python'}), 'python');
  });

  testWidgets('a pinned alias is shown canonically', (tester) async {
    expect(await chipOf(tester, {'language': 'py'}), 'python');
  });
}
