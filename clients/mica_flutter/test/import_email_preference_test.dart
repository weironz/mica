import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'package:mica_flutter/ui/import_email_preference.dart';

Widget app({
  required Future<bool> Function() load,
  required Future<void> Function(bool) save,
}) => MaterialApp(
  locale: const Locale('en'),
  localizationsDelegates: AppLocalizations.localizationsDelegates,
  supportedLocales: AppLocalizations.supportedLocales,
  home: Scaffold(
    body: ImportEmailPreference(load: load, save: save),
  ),
);

void main() {
  testWidgets('server preference loads before the toggle can be changed', (
    tester,
  ) async {
    final load = Completer<bool>();
    await tester.pumpWidget(app(load: () => load.future, save: (_) async {}));
    expect(
      tester.widget<SwitchListTile>(find.byType(SwitchListTile)).onChanged,
      isNull,
    );
    load.complete(true);
    await tester.pumpAndSettle();
    expect(
      tester.widget<SwitchListTile>(find.byType(SwitchListTile)).value,
      isTrue,
    );
    expect(
      tester.widget<SwitchListTile>(find.byType(SwitchListTile)).onChanged,
      isNotNull,
    );
  });

  testWidgets('failed save retains the server value and shows an error', (
    tester,
  ) async {
    final sent = <bool>[];
    await tester.pumpWidget(
      app(
        load: () async => true,
        save: (value) async {
          sent.add(value);
          throw StateError('offline');
        },
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.byType(Switch));
    await tester.pumpAndSettle();
    expect(sent, [false]);
    expect(
      tester.widget<SwitchListTile>(find.byType(SwitchListTile)).value,
      isTrue,
    );
    expect(
      find.text('Could not save. The previous setting is unchanged.'),
      findsOneWidget,
    );
  });

  testWidgets('a failed load can be retried', (tester) async {
    var attempts = 0;
    await tester.pumpWidget(
      app(
        load: () async {
          if (++attempts == 1) throw StateError('offline');
          return false;
        },
        save: (_) async {},
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Retry'));
    await tester.pumpAndSettle();
    expect(attempts, 2);
    expect(
      tester.widget<SwitchListTile>(find.byType(SwitchListTile)).value,
      isFalse,
    );
    expect(
      find.text('Could not load email notification settings'),
      findsNothing,
    );
  });

  testWidgets('a delayed server response is harmless after closing settings', (
    tester,
  ) async {
    final load = Completer<bool>();
    await tester.pumpWidget(app(load: () => load.future, save: (_) async {}));
    await tester.pumpWidget(const SizedBox());
    load.complete(true);
    await tester.pump();
    expect(tester.takeException(), isNull);
  });
}
