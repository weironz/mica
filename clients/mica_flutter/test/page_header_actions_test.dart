// The page header's right-hand cluster: the last-edited wording, which
// timestamp wins, and the share / copy link / favorite controls.

import 'package:flutter/material.dart';
import 'package:flutter_localizations/flutter_localizations.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'package:mica_flutter/ui/favorites.dart';
import 'package:mica_flutter/ui/page_header_actions.dart';
import 'package:mica_flutter/ui/theme_tokens.dart';

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
    body: MicaTheme(tokens: MicaTokens.light, child: child),
  ),
);

FavoritePages _favorites() {
  final store = <String, String>{};
  return FavoritePages(read: (k) => store[k], write: (k, v) => store[k] = v);
}

void main() {
  final zh = lookupAppLocalizations(const Locale('zh'));
  final now = DateTime(2026, 11, 14, 15, 30);

  group('lastEditedWhen', () {
    test('under a minute is 刚刚', () {
      final at = now.subtract(const Duration(seconds: 40));
      expect(zh.pageLastEdited(lastEditedWhen(at, zh, now: now)), '上次编辑 刚刚');
    });

    test('under an hour counts minutes', () {
      final at = now.subtract(const Duration(minutes: 8));
      expect(zh.pageLastEdited(lastEditedWhen(at, zh, now: now)), '上次编辑 8 分钟前');
    });

    test('past the hour is the date, without the year this year', () {
      expect(
        zh.pageLastEdited(
          lastEditedWhen(DateTime(2026, 11, 11, 9), zh, now: now),
        ),
        '上次编辑 11月11日',
      );
      // Earlier today is still a date: there is no "N hours ago" form.
      expect(
        lastEditedWhen(DateTime(2026, 11, 14, 10), zh, now: now),
        '11月14日',
      );
    });

    test('another year carries the year', () {
      expect(lastEditedWhen(DateTime(2025, 3, 2), zh, now: now), '2025年3月2日');
    });

    test('a clock skewed into the future reads as just now', () {
      expect(
        lastEditedWhen(now.add(const Duration(minutes: 3)), zh, now: now),
        '刚刚',
      );
    });
  });

  test('latestEdit takes the newest stamp and ignores missing ones', () {
    final a = DateTime(2026, 1, 1);
    final b = DateTime(2026, 5, 1);
    expect(latestEdit([null, a, b, null]), b);
    expect(latestEdit([null, null]), isNull);
  });

  test('LocalEditClock records every edit but notifies at most every 20s', () {
    var t = DateTime(2026, 11, 14, 12);
    final clock = LocalEditClock(clock: () => t);
    var notified = 0;
    clock.addListener(() => notified++);
    clock.mark('p');
    t = t.add(const Duration(seconds: 5));
    clock.mark('p');
    expect(notified, 1);
    expect(clock.at('p'), t);
    t = t.add(const Duration(seconds: 15));
    clock.mark('p');
    expect(notified, 2);
    for (var i = 0; i < 20; i++) {
      t = t.add(const Duration(seconds: 1));
      clock.mark('p');
    }
    expect(notified, 3, reason: 'continuous typing still refreshes the label');
    clock.mark('other');
    expect(notified, 4, reason: 'a first edit on another page is news');
  });

  group('PageHeaderActions', () {
    Widget actions({
      required FavoritePages favorites,
      List<DateTime?> stamps = const [],
      Future<void> Function()? onShare,
      Future<bool> Function(String)? onCopy,
      bool compact = false,
      LocalEditClock? clock,
    }) => _host(
      PageHeaderActions(
        viewId: 'v1',
        workspaceId: 'ws',
        favorites: favorites,
        editClock: clock ?? LocalEditClock(),
        serverStamps: stamps,
        onShare: onShare,
        onCopyLink: onCopy ?? (_) async => true,
        onMenu: (_) {},
        compact: compact,
      ),
    );

    testWidgets('shows the label, share, link, star and menu in order', (
      tester,
    ) async {
      await tester.pumpWidget(
        actions(
          favorites: _favorites(),
          stamps: [DateTime.now().subtract(const Duration(minutes: 8))],
          onShare: () async {},
        ),
      );
      final xs = [
        tester.getCenter(find.text('上次编辑 8 分钟前')).dx,
        tester.getCenter(find.text('共享')).dx,
        tester.getCenter(find.byIcon(Icons.link)).dx,
        tester.getCenter(find.byIcon(Icons.star_outline_rounded)).dx,
        tester.getCenter(find.byIcon(Icons.more_horiz)).dx,
      ];
      expect([...xs]..sort(), xs);
    });

    testWidgets('no timestamp and no share handler draw neither', (
      tester,
    ) async {
      await tester.pumpWidget(actions(favorites: _favorites()));
      expect(find.textContaining('上次编辑'), findsNothing);
      expect(find.text('共享'), findsNothing);
      expect(find.byIcon(Icons.lock_outline), findsNothing);
    });

    testWidgets('an edit in this window turns the label to 刚刚', (tester) async {
      final clock = LocalEditClock();
      await tester.pumpWidget(
        actions(
          favorites: _favorites(),
          clock: clock,
          stamps: [DateTime(2020, 1, 2)],
        ),
      );
      expect(find.text('上次编辑 2020年1月2日'), findsOneWidget);
      clock.mark('v1');
      await tester.pump();
      expect(find.text('上次编辑 刚刚'), findsOneWidget);
    });

    testWidgets('copy link hands over the page link', (tester) async {
      String? copied;
      await tester.pumpWidget(
        actions(
          favorites: _favorites(),
          onCopy: (link) async {
            copied = link;
            return true;
          },
        ),
      );
      await tester.tap(find.byIcon(Icons.link));
      await tester.pump();
      expect(copied, 'mica://page/v1');
      expect(find.byIcon(Icons.check), findsOneWidget);
      await tester.pump(const Duration(seconds: 2));
      expect(find.byIcon(Icons.link), findsOneWidget);
    });

    testWidgets('the star toggles the page in favorites', (tester) async {
      final favorites = _favorites();
      await tester.pumpWidget(actions(favorites: favorites));
      await tester.tap(find.byIcon(Icons.star_outline_rounded));
      await tester.pump();
      expect(favorites.contains('ws', 'v1'), isTrue);
      expect(find.byIcon(Icons.star_rounded), findsOneWidget);
      await tester.tap(find.byIcon(Icons.star_rounded));
      await tester.pump();
      expect(favorites.contains('ws', 'v1'), isFalse);
    });

    testWidgets('compact drops the label and the share caption', (
      tester,
    ) async {
      await tester.pumpWidget(
        actions(
          favorites: _favorites(),
          stamps: [DateTime.now()],
          onShare: () async {},
          compact: true,
        ),
      );
      expect(find.textContaining('上次编辑'), findsNothing);
      expect(find.text('共享'), findsNothing);
      expect(find.byIcon(Icons.lock_outline), findsOneWidget);
    });
  });
}
