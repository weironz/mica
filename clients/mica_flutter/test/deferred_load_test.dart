import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'package:mica_flutter/ui/deferred_load.dart';

void main() {
  late BuildContext context;
  Future<void> mount(WidgetTester tester) => tester.pumpWidget(MaterialApp(
    locale: const Locale('zh'),
    localizationsDelegates: AppLocalizations.localizationsDelegates,
    supportedLocales: AppLocalizations.supportedLocales,
    home: Scaffold(body: Builder(builder: (value) {
      context = value;
      return const SizedBox();
    })),
  ));

  testWidgets('a retired document cannot open a newly downloaded module', (tester) async {
    await mount(tester);
    var current = true;
    final download = Completer<void>();
    final result = loadUiModule(context, () => download.future, isCurrent: () => current);
    current = false;
    download.complete();
    await tester.pump();
    expect(await result, isFalse);
  });

  testWidgets('a retired scope does not show a late download error', (tester) async {
    await mount(tester);
    var current = true;
    final download = Completer<void>();
    final result = loadUiModule(context, () => download.future, isCurrent: () => current);
    current = false;
    download.completeError(StateError('offline'));
    await tester.pump();
    expect(await result, isFalse);
    expect(find.byType(SnackBar), findsNothing);
  });

  testWidgets('a failed module can be retried without losing the current page', (tester) async {
    await mount(tester);
    final failed = loadUiModule(context, () async => throw StateError('offline'));
    await tester.pump();
    expect(await failed, isFalse);
    expect(find.byType(SnackBar), findsOneWidget);
    expect(await loadUiModule(context, () async {}), isTrue);
  });
}
