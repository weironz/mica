import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'package:mica_flutter/ui/theme_tokens.dart';
import 'package:mica_flutter/ui/version_history_dialog.dart';
import 'support/markdown_codec.dart';

void main() {
  setUpAll(useNativeMarkdownCodec);
  for (final tokens in [MicaTokens.light, MicaTokens.dark_]) {
    testWidgets(
      'history preview paints with the host palette dark=${tokens.dark}',
      (tester) async {
        tester.view.physicalSize = const Size(1440, 1000);
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.resetPhysicalSize);
        addTearDown(tester.view.resetDevicePixelRatio);
        await tester.pumpWidget(
          MaterialApp(
            theme: tokens.toMaterialTheme(),
            locale: const Locale('zh'),
            localizationsDelegates: AppLocalizations.localizationsDelegates,
            supportedLocales: AppLocalizations.supportedLocales,
            builder: (context, child) =>
                MicaTheme(tokens: tokens, child: child!),
            home: Scaffold(
              body: Builder(
                builder: (context) => TextButton(
                  onPressed: () => showDialog<void>(
                    context: context,
                    builder: (_) => VersionHistoryDialog(
                      onList: () async => [
                        const DocVersion(
                          id: 'v1',
                          label: 'checkpoint',
                          createdAt: '2026-11-12T00:00:00Z',
                        ),
                      ],
                      onCreate: (_) async {},
                      onRestore: (_) async {},
                      onLoadContent: (_) async => (
                        rootBlockId: 'root',
                        blocks: [
                          {
                            'id': 'root',
                            'type': 'page',
                            'text': '',
                            'children': ['body'],
                          },
                          {
                            'id': 'body',
                            'type': 'paragraph',
                            'text': '版本预览 Preview',
                            'children': <String>[],
                          },
                        ],
                      ),
                    ),
                  ),
                  child: const Text('Open history'),
                ),
              ),
            ),
          ),
        );
        await tester.tap(find.text('Open history'));
        await tester.pumpAndSettle();
        final preview = tester.widget<MicaEditor>(find.byType(MicaEditor));
        expect(preview.canEdit, isFalse);
        final surface = tester.widget<DocumentSurface>(
          find.byType(DocumentSurface),
        );
        expect(
          surface.appearance.tokens,
          same(tokens),
          reason:
              'A dark dialog must not paint the preview using the default light ink.',
        );
        expect(
          EditorTheme.styleFor(
            surface.nodes.first,
            surface.appearance.tokens,
          ).color,
          tokens.text.primary,
        );
        expect(tester.takeException(), isNull);
        await tester.pumpWidget(const SizedBox.shrink());
        await tester.pump();
      },
    );
  }
}
