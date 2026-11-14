import 'package:flutter/material.dart';
import 'package:flutter_localizations/flutter_localizations.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'package:mica_flutter/ui/home_data.dart';
import 'package:mica_flutter/ui/recycle_bin_section.dart';
import 'package:mica_flutter/ui/theme_tokens.dart';

void main() {
  const deleted = DocumentView(
    id: 'deleted',
    parentViewId: null,
    objectId: 'object-deleted',
    objectType: 'document',
    name: 'Deleted note',
    position: 'a',
  );

  testWidgets('restore reloads the list in the settings section', (
    tester,
  ) async {
    var loads = 0;
    var restores = 0;
    await tester.pumpWidget(
      _host(
        RecycleBinSection(
          onLoad: () async => ++loads == 1 ? [deleted] : const [],
          onRestore: (_) async => restores++,
          onPurge: (_) async {},
          canEdit: true,
          liveViews: const [],
          relativeStrings: _relativeStrings,
        ),
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Deleted note'), findsOneWidget);
    await tester.tap(find.widgetWithIcon(IconButton, Icons.restore));
    await tester.pumpAndSettle();

    expect(restores, 1);
    expect(loads, 2);
    expect(find.text('Deleted note'), findsNothing);
    expect(tester.takeException(), isNull);
  });

  testWidgets('viewers can inspect trash but cannot restore or purge', (
    tester,
  ) async {
    await tester.pumpWidget(
      _host(
        RecycleBinSection(
          onLoad: () async => [deleted],
          onRestore: (_) async {},
          onPurge: (_) async {},
          canEdit: false,
          liveViews: const [],
          relativeStrings: _relativeStrings,
        ),
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Deleted note'), findsOneWidget);
    expect(find.widgetWithIcon(IconButton, Icons.restore), findsNothing);
    expect(find.widgetWithIcon(IconButton, Icons.delete_forever), findsNothing);
    expect(tester.takeException(), isNull);
  });
}

final _relativeStrings = RelativeTimeStrings(
  justNow: 'Just now',
  minutesAgo: (n) => '$n minutes ago',
  hoursAgo: (n) => '$n hours ago',
  yesterday: 'Yesterday',
  daysAgo: (n) => '$n days ago',
);

Widget _host(Widget child) => MaterialApp(
  locale: const Locale('en'),
  localizationsDelegates: const [
    AppLocalizations.delegate,
    GlobalMaterialLocalizations.delegate,
    GlobalWidgetsLocalizations.delegate,
    GlobalCupertinoLocalizations.delegate,
  ],
  supportedLocales: const [Locale('en')],
  home: Scaffold(
    body: MicaTheme(tokens: MicaTokens.light, child: child),
  ),
);
