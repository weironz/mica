// The selected search row used to be a `Container(color: accent.wash)` wrapped
// around the ListTile. A ListTile paints its background and its ink splash onto
// the nearest Material ancestor, so that coloured box sat *between* the two and
// covered the splash: the highlighted row — the one you are about to click —
// answered a click with no feedback at all, while every unhighlighted row
// (color: null, so no ColoredBox) rippled normally.
//
// Flutter does flag this, but only as a debug-mode assertion into the run log
// ("ListTile background color or ink splashes may be invisible"), which is why
// it shipped through 0.13.9 unnoticed. These tests turn that log line into a
// failure: `takeException` IS that assertion, so putting the wrapper back turns
// a log line nobody reads into a red test. (Asserting "no ColoredBox above the
// tile" was tried and dropped — Scaffold's own background is one, so it fails on
// a correct tree.)
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart' show SearchResult;
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'package:mica_flutter/main.dart';
import 'package:mica_flutter/ui/theme_tokens.dart';

SearchResult _hit({
  String name = '部署流程',
  String snippet = '',
  bool isFolder = false,
  bool titleMatch = true,
}) => SearchResult.fromJson({
  'view_id': 'v1',
  'object_id': 'o1',
  'name': name,
  'snippet': snippet,
  'title_match': titleMatch,
  'is_folder': isFolder,
});

// `context.l10n` is `AppLocalizations.of(context)!`, so the host has to localize
// exactly like the real app or the subtitle branches die on that null check.
Widget _host(Widget tile) => MaterialApp(
  localizationsDelegates: AppLocalizations.localizationsDelegates,
  supportedLocales: AppLocalizations.supportedLocales,
  locale: const Locale('zh'),
  home: Scaffold(body: SizedBox(width: 420, child: tile)),
);

void main() {
  testWidgets('the selected row keeps its ink splash visible', (tester) async {
    await tester.pumpWidget(
      _host(
        SearchResultTile(
          result: _hit(),
          query: '部署',
          selected: true,
          onTap: () {},
        ),
      ),
    );

    // The assertion that fired in the 0.13.9 run log. It is debug-only, so this
    // is the only place it can ever fail a build rather than a reader.
    expect(tester.takeException(), isNull);
  });

  testWidgets('selection colours the tile, not a box around it', (
    tester,
  ) async {
    await tester.pumpWidget(
      _host(
        SearchResultTile(
          result: _hit(),
          query: '部署',
          selected: true,
          onTap: () {},
        ),
      ),
    );

    final tile = tester.widget<ListTile>(find.byType(ListTile));
    expect(tile.tileColor, MicaTokens.light.accent.wash);
  });

  testWidgets('an unselected row carries no background at all', (tester) async {
    await tester.pumpWidget(
      _host(
        SearchResultTile(
          result: _hit(),
          query: '部署',
          selected: false,
          onTap: () {},
        ),
      ),
    );

    expect(tester.takeException(), isNull);
    expect(tester.widget<ListTile>(find.byType(ListTile)).tileColor, isNull);
  });

  // The row still has to say WHY a bodyless hit matched, or it reads as noise.
  testWidgets('a folder hit says it matched on its name', (tester) async {
    await tester.pumpWidget(
      _host(
        SearchResultTile(
          result: _hit(isFolder: true),
          query: '部署',
          selected: false,
          onTap: () {},
        ),
      ),
    );

    expect(find.byIcon(Icons.folder_outlined), findsOneWidget);
    expect(tester.takeException(), isNull);
  });

  // Where a hit LIVES. Two pages called 「问题记录」 under different projects is
  // the normal case, and a list showing only names makes you open both to find
  // out which is which.
  group('the path line', () {
    testWidgets('reads workspace first, folders after', (tester) async {
      await tester.pumpWidget(
        _host(
          SearchResultTile(
            result: _hit(),
            query: '部署',
            path: const ['greenstor', '基础信息'],
            selected: false,
            onTap: () {},
          ),
        ),
      );

      expect(find.text('greenstor / 基础信息'), findsOneWidget);
    });

    testWidgets('no path means nothing is drawn, not an empty separator', (
      tester,
    ) async {
      await tester.pumpWidget(
        _host(
          SearchResultTile(
            result: _hit(),
            query: '部署',
            selected: false,
            onTap: () {},
          ),
        ),
      );

      expect(find.textContaining('/'), findsNothing);
    });

    // A deep path is shown WHOLE. An earlier cut collapsed the middle out
    // (`tools / … / reasonix`) because the path shared the title line and had
    // no room; on its own line it does, and a path with a hole in it is not the
    // path — you cannot tell 「问题记录」 under two projects apart from an
    // ellipsis.
    testWidgets('a deep path is not abbreviated', (tester) async {
      await tester.pumpWidget(
        _host(
          SearchResultTile(
            result: _hit(),
            query: '部署',
            path: const ['tools', 'AI工具', 'deepseek', 'reasonix'],
            selected: false,
            onTap: () {},
          ),
        ),
      );

      expect(find.text('tools / AI工具 / deepseek / reasonix'), findsOneWidget);
      expect(find.textContaining('…'), findsNothing);
    });
  });

  // The three lines answer three different questions, and each needs its own:
  // what it is, where it lives, why it matched.
  testWidgets('name, path and matched text are three separate lines', (
    tester,
  ) async {
    await tester.pumpWidget(
      _host(
        SearchResultTile(
          result: _hit(name: '部署流程', snippet: '一段命中的正文'),
          query: '命中',
          path: const ['greenstor', '基础信息'],
          selected: false,
          onTap: () {},
        ),
      ),
    );

    expect(find.text('部署流程'), findsOneWidget);
    expect(find.text('greenstor / 基础信息'), findsOneWidget);
    // The snippet is rich text (the query is tinted inside it), so it is found
    // by its runs rather than as one string.
    expect(find.textContaining('一段'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });

  testWidgets('tapping the row reaches the host', (tester) async {
    var taps = 0;
    await tester.pumpWidget(
      _host(
        SearchResultTile(
          result: _hit(snippet: '一段命中的正文'),
          query: '命中',
          selected: true,
          onTap: () => taps++,
        ),
      ),
    );

    await tester.tap(find.byType(ListTile));
    expect(taps, 1);
  });
}
