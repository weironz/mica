// Favorite pages: what is stored, what the sidebar shows, and that a
// workspace without favorites keeps its sidebar unchanged.

import 'package:flutter/material.dart';
import 'package:flutter_localizations/flutter_localizations.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'package:mica_flutter/ui/favorites.dart';
import 'package:mica_flutter/ui/theme_tokens.dart';

DocumentView _view(String id, String name, {String type = 'document'}) =>
    DocumentView(
      id: id,
      parentViewId: null,
      objectId: 'obj-$id',
      objectType: type,
      name: name,
      position: id,
    );

Widget _host(Widget child) => MaterialApp(
  locale: const Locale('zh'),
  localizationsDelegates: const [
    AppLocalizations.delegate,
    GlobalMaterialLocalizations.delegate,
    GlobalWidgetsLocalizations.delegate,
    GlobalCupertinoLocalizations.delegate,
  ],
  supportedLocales: const [Locale('zh'), Locale('en')],
  home: Scaffold(
    body: MicaTheme(
      tokens: MicaTokens.light,
      child: SizedBox(width: 280, child: Column(children: [child])),
    ),
  ),
);

void main() {
  test('stars persist per workspace, in starring order', () {
    final store = <String, String>{};
    final a = FavoritePages(
      read: (k) => store[k],
      write: (k, v) => store[k] = v,
    );
    a
      ..toggle('ws1', 'b')
      ..toggle('ws1', 'a')
      ..toggle('ws2', 'c');
    final reopened = FavoritePages(
      read: (k) => store[k],
      write: (k, v) => store[k] = v,
    );
    expect(reopened.idsOf('ws1'), ['b', 'a']);
    expect(reopened.idsOf('ws2'), ['c']);
    reopened.toggle('ws1', 'b');
    expect(reopened.idsOf('ws1'), ['a']);
  });

  test('a corrupt preference reads as no favorites', () {
    final p = FavoritePages(read: (_) => '{not json', write: (_, _) {});
    expect(p.idsOf('ws'), isEmpty);
    final q = FavoritePages(read: (_) => '{"a":1}', write: (_, _) {});
    expect(q.idsOf('ws'), isEmpty);
  });

  test('favoriteViews skips pages that are gone and folders', () {
    final views = [_view('a', 'A'), _view('f', 'F', type: 'folder')];
    expect(favoriteViews(['gone', 'f', 'a'], views).map((v) => v.id), ['a']);
  });

  group('FavoritesSection', () {
    FavoritePages favorites(List<String> ids) {
      final store = <String, String>{};
      final p = FavoritePages(
        read: (k) => store[k],
        write: (k, v) => store[k] = v,
      );
      for (final id in ids) {
        p.toggle('ws', id);
      }
      return p;
    }

    testWidgets('draws nothing without a favorite that still exists', (
      tester,
    ) async {
      await tester.pumpWidget(
        _host(
          FavoritesSection(
            favorites: favorites(['gone']),
            workspaceId: 'ws',
            views: [_view('a', 'Alpha')],
            selectedId: null,
            onOpen: (_) {},
          ),
        ),
      );
      expect(find.text('收藏'), findsNothing);
    });

    testWidgets('lists favorites, opens on tap, and collapses', (tester) async {
      DocumentView? opened;
      final p = favorites(['b', 'a']);
      await tester.pumpWidget(
        _host(
          FavoritesSection(
            favorites: p,
            workspaceId: 'ws',
            views: [_view('a', 'Alpha'), _view('b', 'Beta')],
            selectedId: 'a',
            onOpen: (v) => opened = v,
          ),
        ),
      );
      expect(find.text('收藏'), findsOneWidget);
      expect(find.byIcon(Icons.star_rounded), findsNothing);
      expect(tester.getTopLeft(find.text('收藏')).dx, 4);
      expect(
        tester.getTopLeft(find.text('Beta')).dy,
        lessThan(tester.getTopLeft(find.text('Alpha')).dy),
      );
      await tester.tap(find.text('Alpha'));
      expect(opened?.id, 'a');
      await tester.tap(find.text('收藏'));
      await tester.pump();
      expect(find.text('Alpha'), findsNothing);
      expect(p.collapsed, isTrue);
    });

    testWidgets('right-click removes a favorite', (tester) async {
      final p = favorites(['a']);
      await tester.pumpWidget(
        _host(
          FavoritesSection(
            favorites: p,
            workspaceId: 'ws',
            views: [_view('a', 'Alpha')],
            selectedId: null,
            onOpen: (_) {},
          ),
        ),
      );
      await tester.tap(find.text('Alpha'), buttons: 2);
      await tester.pumpAndSettle();
      await tester.tap(find.text('取消收藏'));
      await tester.pumpAndSettle();
      expect(p.contains('ws', 'a'), isFalse);
      expect(find.text('收藏'), findsNothing);
    });
  });
}
