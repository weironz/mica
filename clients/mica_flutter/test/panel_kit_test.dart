import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/ui/panel_kit.dart';

/// The load-bearing constraint in this file is `MicaPickZone`: the mockup draws a
/// drag-and-drop well, the app accepts no drops, and shipping the drag caption
/// first would be a control that lies. The API is shaped so there is nowhere to
/// attach a drop handler.
void main() {
  Future<void> pump(WidgetTester tester, Widget child) => tester.pumpWidget(
    MaterialApp(
      home: Scaffold(body: SingleChildScrollView(child: child)),
    ),
  );

  group('MicaEyebrow', () {
    testWidgets('renders the label muted and tracked, not as a heading', (
      tester,
    ) async {
      await pump(tester, const MicaEyebrow('导入'));

      final text = tester.widget<Text>(find.text('导入'));
      expect(text.style?.fontSize, 12);
      expect(text.style?.fontWeight, FontWeight.w600);
      expect(
        text.style?.letterSpacing,
        0.6,
        reason: 'the tracking is what makes it read as a label',
      );
    });

    testWidgets('an icon is optional and drawn muted, never accent', (
      tester,
    ) async {
      await pump(tester, const MicaEyebrow('导入'));
      expect(find.byType(Icon), findsNothing);

      await pump(tester, const MicaEyebrow('导入', icon: Icons.upload_file));
      final icon = tester.widget<Icon>(find.byType(Icon));
      expect(icon.size, 13);
      // A blue glyph would put the label back at heading weight.
      expect(icon.color, isNot(const Color(0xFF2563EB)));
    });
  });

  group('MicaCard', () {
    testWidgets('is a white card with a hairline border and 14px radius', (
      tester,
    ) async {
      await pump(tester, const MicaCard(child: Text('body')));

      final container = tester.widget<Container>(
        find.ancestor(of: find.text('body'), matching: find.byType(Container)),
      );
      final decoration = container.decoration as BoxDecoration;
      expect(decoration.color, Colors.white);
      // 14, not the mockups' 12 — the system pins 8/14/16.
      expect(decoration.borderRadius, BorderRadius.circular(14));
      expect(decoration.border, isNotNull);
    });

    testWidgets('hosts its child and takes the full width', (tester) async {
      await pump(tester, const MicaCard(child: Text('body')));

      expect(find.text('body'), findsOneWidget);
      final container = tester.widget<Container>(
        find.ancestor(of: find.text('body'), matching: find.byType(Container)),
      );
      expect(container.constraints?.maxWidth, double.infinity);
    });
  });

  group('MicaPickZone', () {
    testWidgets('shows title and optional subtitle', (tester) async {
      await pump(
        tester,
        MicaPickZone(
          icon: Icons.upload_file,
          title: '选择文件',
          subtitle: '.zip 或 .md',
          onTap: () {},
        ),
      );

      expect(find.text('选择文件'), findsOneWidget);
      expect(find.text('.zip 或 .md'), findsOneWidget);
    });

    testWidgets('the whole zone is tappable, not just the glyph', (
      tester,
    ) async {
      var taps = 0;
      await pump(
        tester,
        MicaPickZone(
          icon: Icons.upload_file,
          title: '选择文件',
          onTap: () => taps++,
        ),
      );

      await tester.tap(find.text('选择文件'));
      await tester.pumpAndSettle();

      expect(taps, 1);
    });

    testWidgets('omitting the subtitle renders no second line', (tester) async {
      await pump(
        tester,
        MicaPickZone(icon: Icons.upload_file, title: '选择文件', onTap: () {}),
      );

      // Only the title — no empty Text holding the space a subtitle would take.
      expect(find.byType(Text), findsOneWidget);
    });

    testWidgets('paints its dashed border rather than composing one', (
      tester,
    ) async {
      await pump(
        tester,
        MicaPickZone(icon: Icons.upload_file, title: '选择文件', onTap: () {}),
      );

      // Flutter has no dashed BorderSide; a painter is what survives a resize.
      expect(find.byType(CustomPaint), findsWidgets);
    });
  });
}
