import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/ui/appearance_preview.dart';
import 'package:mica_flutter/ui/settings_shell.dart';
import 'package:mica_flutter/ui/theme_tokens.dart';

void main() {
  for (final size in [
    const Size(320, 568),
    const Size(390, 844),
    const Size(768, 600),
    const Size(1440, 900),
    const Size(390, 280),
    const Size(768, 280),
  ]) {
    for (final scale in [1.0, 1.8]) {
      testWidgets(
        'settings navigation and close remain reachable: $size $scale',
        (tester) async {
          tester.view.physicalSize = size;
          tester.view.devicePixelRatio = 1;
          addTearDown(tester.view.resetPhysicalSize);
          addTearDown(tester.view.resetDevicePixelRatio);
          var closed = false;
          await tester.pumpWidget(
            _Host(
              textScale: scale,
              onClose: () => closed = true,
              withFooter: true,
            ),
          );
          await tester.pumpAndSettle();
          expect(tester.takeException(), isNull);
          final close = find.byKey(const ValueKey('settings-close'));
          expect(tester.getRect(close).right, lessThanOrEqualTo(size.width));
          expect(tester.getRect(close).bottom, lessThanOrEqualTo(size.height));
          await tester.tap(find.byKey(const ValueKey('settings-category-0')));
          await tester.pumpAndSettle();
          expect(
            find.byKey(const ValueKey('settings-appearance-preview')),
            findsOneWidget,
          );
          expect(tester.takeException(), isNull);
          if (size.width < 700) {
            await tester.tap(find.byKey(const ValueKey('settings-back')));
            await tester.pumpAndSettle();
            expect(
              find.byKey(const ValueKey('settings-navigation')),
              findsOneWidget,
            );
          }
          await tester.tap(close);
          expect(closed, isTrue);
          expect(tester.takeException(), isNull);
        },
      );
    }
  }

  // The About row used to trail the list as a stock ListTile: bigger type
  // than the categories, an icon off their column, and it scrolled away with
  // them. It now shares their row widget and is pinned under a rule.
  testWidgets('the footer is pinned under a rule and matches the categories', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(1440, 900);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    var opened = 0;
    await tester.pumpWidget(
      _Host(onClose: () {}, withFooter: true, onFooter: () => opened++),
    );
    await tester.pumpAndSettle();

    final nav = tester.getRect(
      find.byKey(const ValueKey('settings-navigation')),
    );
    final footer = tester.getRect(find.byKey(const ValueKey('settings-about')));
    final lastCategory = tester.getRect(
      find.byKey(const ValueKey('settings-category-1')),
    );
    expect(footer.top, greaterThan(nav.bottom), reason: 'outside the list');
    expect(footer.top - lastCategory.bottom, greaterThan(200));
    final rule = find.byWidgetPredicate((w) => w is Divider && w.indent == 12);
    expect(rule, findsOneWidget);
    expect(tester.getRect(rule).bottom, lessThanOrEqualTo(footer.top));

    TextStyle style(String text) => tester.widget<Text>(find.text(text)).style!;
    expect(style('About').fontSize, style('AI providers').fontSize);
    expect(style('About').fontWeight, style('AI providers').fontWeight);
    expect(
      tester.getTopLeft(find.byIcon(Icons.info_outline)).dx,
      tester.getTopLeft(find.byIcon(Icons.auto_awesome)).dx,
      reason: 'icons share one column',
    );
    expect(
      tester.getTopLeft(find.text('About')).dx,
      tester.getTopLeft(find.text('AI providers')).dx,
    );

    await tester.tap(find.byKey(const ValueKey('settings-about')));
    expect(opened, 1);
  });

  testWidgets(
    'only the selected settings page scrolls; header stays in place',
    (tester) async {
      tester.view.physicalSize = const Size(390, 844);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      await tester.pumpWidget(_Host(onClose: () {}));
      await tester.tap(find.byKey(const ValueKey('settings-category-0')));
      await tester.pumpAndSettle();
      final headerBefore = tester.getRect(
        find.byKey(const ValueKey('settings-close')),
      );
      await tester.drag(
        find.byKey(const ValueKey('settings-detail-0')),
        const Offset(0, -900),
      );
      await tester.pumpAndSettle();
      expect(
        tester.getRect(find.byKey(const ValueKey('settings-close'))),
        headerBefore,
      );
      expect(find.text('Last setting').hitTestable(), findsOneWidget);
    },
  );

  testWidgets('keyboard inset keeps navigation and close above the keyboard', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(390, 844);
    tester.view.devicePixelRatio = 1;
    tester.view.viewInsets = const FakeViewPadding(bottom: 350);
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    addTearDown(tester.view.resetViewInsets);
    await tester.pumpWidget(_Host(onClose: () {}));
    await tester.tap(find.byKey(const ValueKey('settings-category-0')));
    await tester.pumpAndSettle();
    final close = find.byKey(const ValueKey('settings-close'));
    expect(tester.getRect(close).bottom, lessThanOrEqualTo(494));
    expect(
      find.byKey(const ValueKey('settings-back')).hitTestable(),
      findsOneWidget,
    );
    expect(tester.takeException(), isNull);
  });

  testWidgets(
    'switching category does not retain a previous page scroll offset',
    (tester) async {
      tester.view.physicalSize = const Size(1440, 900);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      await tester.pumpWidget(_Host(onClose: () {}));
      await tester.drag(
        find.byKey(const ValueKey('settings-detail-0')),
        const Offset(0, -900),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('settings-category-1')));
      await tester.pumpAndSettle();
      expect(
        find.text('AI configuration loading').hitTestable(),
        findsOneWidget,
      );
      await tester.tap(find.byKey(const ValueKey('settings-category-0')));
      await tester.pumpAndSettle();
      expect(
        find.text('Typography preview · 800 px').hitTestable(),
        findsOneWidget,
      );
    },
  );

  testWidgets(
    'pending content in one category does not hide other categories',
    (tester) async {
      tester.view.physicalSize = const Size(1440, 900);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      await tester.pumpWidget(_Host(onClose: () {}));
      expect(find.text('Typography preview · 800 px'), findsOneWidget);
      await tester.tap(find.byKey(const ValueKey('settings-category-1')));
      await tester.pump();
      expect(find.text('AI configuration loading'), findsOneWidget);
      await tester.tap(find.byKey(const ValueKey('settings-category-0')));
      await tester.pump();
      expect(find.text('Typography preview · 800 px'), findsOneWidget);
    },
  );

  testWidgets(
    'preview follows editor font metrics, tokens and family updates',
    (tester) async {
      Future<void> preview(EditorAppearance appearance, MicaTokens tokens) =>
          tester.pumpWidget(
            MaterialApp(
              home: MicaTheme(
                tokens: tokens,
                child: Scaffold(
                  body: MicaAppearancePreview(
                    appearance: appearance,
                    pageWidth: 920,
                    title: 'Preview heading',
                    body: '中文与 English',
                    code: 'const idea = 1;',
                    label: 'Typography preview',
                  ),
                ),
              ),
            ),
          );
      await preview(const EditorAppearance(), MicaTokens.light);
      final firstBody = tester.widget<Text>(find.text('中文与 English')).style!;
      final firstCode = tester
          .widget<Text>(find.text('const idea = 1;'))
          .style!;
      await preview(
        const EditorAppearance(fontScale: 1.25, fontFamily: 'serif'),
        MicaTokens.dark_,
      );
      final body = tester.widget<Text>(find.text('中文与 English')).style!;
      final code = tester.widget<Text>(find.text('const idea = 1;')).style!;
      expect(body.fontSize, firstBody.fontSize! * 1.25);
      expect(body.fontFamily, 'serif');
      expect(body.color, MicaTokens.dark_.text.primary);
      expect(code.fontSize, firstCode.fontSize! * 1.25);
      expect(code.fontFamily, kMonoFont);
      expect(body.fontFamilyFallback, EditorAppearance.cjkFallback);
      expect(tester.takeException(), isNull);
    },
  );
}

class _Host extends StatefulWidget {
  const _Host({
    required this.onClose,
    this.textScale = 1,
    this.withFooter = false,
    this.onFooter,
  });
  final VoidCallback onClose;
  final double textScale;
  final bool withFooter;
  final VoidCallback? onFooter;

  @override
  State<_Host> createState() => _HostState();
}

class _HostState extends State<_Host> {
  int selected = 0;

  @override
  Widget build(BuildContext context) => MaterialApp(
    builder: (context, child) => MediaQuery(
      data: MediaQuery.of(
        context,
      ).copyWith(textScaler: TextScaler.linear(widget.textScale)),
      child: MicaTheme(tokens: MicaTokens.light, child: child!),
    ),
    home: Scaffold(
      body: MicaSettingsShell(
        title: 'Settings',
        closeLabel: 'Close',
        backLabel: 'Back',
        onClose: widget.onClose,
        selectedIndex: selected,
        onSelected: (index) => setState(() => selected = index),
        footer: widget.withFooter
            ? MicaSettingsNavItem(
                key: const ValueKey('settings-about'),
                icon: Icons.info_outline,
                title: 'About',
                trailing: const Text('v1.0.0'),
                onTap: widget.onFooter ?? () {},
              )
            : null,
        destinations: [
          MicaSettingsDestination(
            group: 'General',
            title: 'Appearance',
            description: 'Settings apply immediately. 界面与编辑偏好即时生效。',
            icon: Icons.tune,
            content: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                const MicaAppearancePreview(
                  appearance: EditorAppearance(),
                  pageWidth: 800,
                  title: '让想法自然展开',
                  body: 'Keep your ideas clear. 中文内容自然衔接。',
                  code: 'const idea = "Hello";',
                  label: 'Typography preview',
                ),
                for (var i = 0; i < 8; i++)
                  SwitchListTile(
                    value: false,
                    onChanged: (_) {},
                    title: Text('Preference $i'),
                  ),
                const Text('Last setting'),
              ],
            ),
          ),
          const MicaSettingsDestination(
            group: 'Other',
            title: 'AI providers',
            description: 'Server configuration',
            icon: Icons.auto_awesome,
            content: Text('AI configuration loading'),
          ),
        ],
      ),
    ),
  );
}
