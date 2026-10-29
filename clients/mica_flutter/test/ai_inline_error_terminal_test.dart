import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'support/markdown_codec.dart';

void main() {
  setUpAll(useNativeMarkdownCodec);
  testWidgets('failed generation never offers replace or insert', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(1400, 1000);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final writes = <Map<String, dynamic>>[];
    final focus = FocusNode();
    addTearDown(focus.dispose);
    final nodes = [
      for (var i = 0; i < 4; i++)
        EditorNode(id: 'pad$i', kind: 'paragraph', text: 'padding line'),
      EditorNode(id: 'target', kind: 'paragraph', text: 'selected text'),
    ];
    final first = StreamController<String>();
    addTearDown(first.close);
    await tester.pumpWidget(
      MaterialApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        locale: const Locale('zh'),
        home: Scaffold(
          body: MicaEditor(
            rootBlockId: 'root',
            nodes: nodes,
            version: 0,
            canEdit: true,
            focusNode: focus,
            onApplyOperations: (ops) async => writes.addAll(ops),
            onAiStream: (prompt, {system}) => first.stream,
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
    final client = tester.state(find.byType(MicaEditor)) as TextInputClient;
    client.updateEditingValue(
      const TextEditingValue(
        text: 'selected text',
        selection: TextSelection(baseOffset: 13, extentOffset: 0),
      ),
    );
    await tester.pump();
    await tester.pump();
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
    await tester.pump();
    // The generation fails mid-stream: a delta, then an error, then the
    // underlying stream still ends normally.
    first.add('Partial result');
    await tester.pump();
    first.addError('probe upstream failed');
    await tester.pump();
    unawaited(first.close());
    await tester.pumpAndSettle();
    expect(find.textContaining('probe upstream failed'), findsOneWidget);
    expect(
      find.text('替换所选'),
      findsNothing,
      reason: 'a failed generation must not be applicable',
    );
    expect(
      find.text('插到下面'),
      findsNothing,
      reason: 'a failed generation must not be insertable either',
    );
    await tester.tap(find.text('取消'));
    await tester.pumpAndSettle();
    expect(writes, isEmpty, reason: 'closing after failure writes nothing');
    await tester.pumpWidget(const SizedBox.shrink());
    await tester.pump();
  });

  testWidgets('a stale generation cannot touch a newer one', (tester) async {
    tester.view.physicalSize = const Size(1400, 1000);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final writes = <Map<String, dynamic>>[];
    final focus = FocusNode();
    addTearDown(focus.dispose);
    final nodes = [
      for (var i = 0; i < 4; i++)
        EditorNode(id: 'pad$i', kind: 'paragraph', text: 'padding line'),
      EditorNode(id: 'target', kind: 'paragraph', text: 'selected text'),
    ];
    final first = StreamController<String>();
    final second = StreamController<String>();
    addTearDown(() {
      unawaited(first.close());
      unawaited(second.close());
    });
    var calls = 0;
    await tester.pumpWidget(
      MaterialApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        locale: const Locale('zh'),
        home: Scaffold(
          body: MicaEditor(
            rootBlockId: 'root',
            nodes: nodes,
            version: 0,
            canEdit: true,
            focusNode: focus,

            onApplyOperations: (ops) async => writes.addAll(ops),
            onAiStream: (prompt, {system}) =>
                (calls++ == 0) ? first.stream : second.stream,

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
    final client = tester.state(find.byType(MicaEditor)) as TextInputClient;
    client.updateEditingValue(
      const TextEditingValue(
        text: 'selected text',
        selection: TextSelection(baseOffset: 13, extentOffset: 0),
      ),
    );
    await tester.pump();
    await tester.pump();
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
    await tester.pump();

    first.add('stale partial');
    await tester.pump();
    first.addError('first attempt failed');
    await tester.pump();

    await tester.pump();
    // Deliberately NOT closed: the first generation's stream stays alive, so
    // any event it still emits belongs to a dead generation.
    await tester.tap(find.text('重新生成'));
    await tester.pump();
    first.add('late stale event');
    await tester.pump();
    second.add('fresh result');
    await tester.pump();
    unawaited(second.close());

    await tester.pump();


    // A late event on the first generation's stream must not corrupt the
    // second generation's buffered output.
    expect(find.textContaining('fresh result'), findsOneWidget);
    expect(
      find.textContaining('stale'),
      findsNothing,
      reason: 'events from a finished generation stay dead',
    );
    await tester.tap(find.text('插到下面'));
    await tester.pumpAndSettle();

    final insert = writes.singleWhere(
      (op) => op['type'] == 'insert_block',
    );
    expect((insert['block'] as Map)['text'], 'fresh result');
    await tester.pumpWidget(const SizedBox.shrink());
    await tester.pump();
  });
}
