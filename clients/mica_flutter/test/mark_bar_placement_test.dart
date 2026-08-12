// The floating format bar must never cover the text it describes.
//
// Reported 2026-08-05 with a screenshot: selecting the FIRST line put the bar
// directly on top of that line. The bar was placed unconditionally 44px above
// the selection, and the overlay helper clamps a negative offset to 0 — so "off
// the top of the canvas" silently became "on top of the selection", which is the
// worse of the two outcomes.
//
// Located by ICON rather than tooltip text: the tooltip is localized
// (`context.l10n.shortcutsBold`), so asserting on its string would make this
// test fail in the other language for no reason.
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';

void main() {
  Future<void> pumpEditor(WidgetTester tester, List<EditorNode> nodes) async {
    await tester.pumpWidget(
      MaterialApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: Scaffold(
          body: MicaEditor(
            rootBlockId: 'root',
            nodes: nodes,
            version: 0,
            canEdit: true,
            onApplyOperations: (_) async {},
          ),
        ),
      ),
    );
    await tester.pump();
  }

  /// Select the whole line at [dy] the way a user would: click into it, Home,
  /// then Shift+End.
  Future<void> selectLineAt(WidgetTester tester, double dy) async {
    await tester.tapAt(
      tester.getTopLeft(find.byType(MicaEditor)) + Offset(40, dy),
    );
    await tester.pump();
    await tester.sendKeyEvent(LogicalKeyboardKey.home);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.end);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
    await tester.pumpAndSettle();
  }

  /// The format bar's rectangle on screen, or null when it is not showing.
  Rect? barRect(WidgetTester tester) {
    final bold = find.byIcon(Icons.format_bold);
    if (bold.evaluate().isEmpty) return null;
    final material = find.ancestor(of: bold, matching: find.byType(Material));
    return tester.getRect(material.first);
  }

  testWidgets('selecting the first line does not put the bar over it', (
    tester,
  ) async {
    await pumpEditor(tester, [
      EditorNode(id: 'a', kind: 'paragraph', text: 'first line of the document'),
      EditorNode(id: 'b', kind: 'paragraph', text: 'second line'),
      EditorNode(id: 'c', kind: 'paragraph', text: 'third line'),
    ]);

    await selectLineAt(tester, 14);
    final bar = barRect(tester);
    expect(bar, isNotNull, reason: 'the bar should show for a ranged selection');

    // The first line occupies roughly the top 28px of the canvas.
    final canvas = tester.getRect(find.byType(MicaEditor));
    final firstLine = Rect.fromLTRB(
      canvas.left,
      canvas.top,
      canvas.right,
      canvas.top + 28,
    );

    expect(
      bar!.overlaps(firstLine),
      isFalse,
      reason: 'the bar is sitting on the line it describes: $bar vs $firstLine',
    );
    // ABOVE the canvas, in the app chrome — not below the selection. Flipping
    // below was tried and only moved the damage: the bar then covers the next
    // line AND swallows clicks meant for it.
    expect(
      bar.bottom,
      lessThanOrEqualTo(canvas.top + 1),
      reason: 'it should ride above the canvas, not into the document: $bar',
    );
  });

  testWidgets('a line with room above keeps the bar above it', (tester) async {
    await pumpEditor(tester, [
      for (var i = 0; i < 12; i++)
        EditorNode(id: 'p$i', kind: 'paragraph', text: 'line $i'),
    ]);

    // Well down the document there IS room above, so the bar belongs there —
    // flipping everything below would just be a different regression.
    await selectLineAt(tester, 200);
    final bar = barRect(tester);
    expect(bar, isNotNull);

    final canvas = tester.getRect(find.byType(MicaEditor));
    expect(
      bar!.top,
      lessThan(canvas.top + 200),
      reason: 'should still prefer above when it fits: $bar',
    );
  });

  // Reported 2026-08-12: the AI button was invisible for the selection it was
  // built for. It had been written inside the `singleText` group with bold and
  // italic, which the bar hides whenever the selection spans more than one
  // block — so selecting seven list items to rewrite them showed no AI button.
  group('the AI button', () {
    Future<void> pumpWithAi(
      WidgetTester tester,
      List<EditorNode> nodes, {
      required bool hasAi,
    }) async {
      await tester.pumpWidget(
        MaterialApp(
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          home: Scaffold(
            body: MicaEditor(
              rootBlockId: 'root',
              nodes: nodes,
              version: 0,
              canEdit: true,
              onApplyOperations: (_) async {},
              onAiStream: hasAi
                  ? (_, {String? system}) => const Stream<String>.empty()
                  : null,
            ),
          ),
        ),
      );
      await tester.pump();
    }

    /// Select from the first line into the third — a MULTI-block range.
    Future<void> selectAcrossBlocks(WidgetTester tester) async {
      await tester.tapAt(
        tester.getTopLeft(find.byType(MicaEditor)) + const Offset(40, 14),
      );
      await tester.pump();
      await tester.sendKeyEvent(LogicalKeyboardKey.home);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
      await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
      await tester.sendKeyEvent(LogicalKeyboardKey.end);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
      await tester.pumpAndSettle();
    }

    List<EditorNode> lines() => [
      EditorNode(id: 'a', kind: 'paragraph', text: 'first line of the document'),
      EditorNode(id: 'b', kind: 'paragraph', text: 'second line'),
      EditorNode(id: 'c', kind: 'paragraph', text: 'third line'),
    ];

    testWidgets('shows for a selection spanning several blocks', (tester) async {
      await pumpWithAi(tester, lines(), hasAi: true);
      await selectAcrossBlocks(tester);
      // Bold is hidden for a multi-block selection — that is exactly the group
      // the AI button must NOT be inside.
      expect(find.byIcon(Icons.format_bold), findsNothing);
      expect(find.byIcon(Icons.auto_awesome), findsOneWidget);
    });

    testWidgets('shows for a single-line selection too', (tester) async {
      await pumpWithAi(tester, lines(), hasAi: true);
      await selectLineAt(tester, 14);
      expect(find.byIcon(Icons.auto_awesome), findsOneWidget);
    });

    testWidgets('is absent when the world has no AI', (tester) async {
      // 本地模式 has no provider; a lit button that answers nothing is worse
      // than no button.
      await pumpWithAi(tester, lines(), hasAi: false);
      await selectLineAt(tester, 14);
      expect(find.byIcon(Icons.auto_awesome), findsNothing);
    });
  });
}
