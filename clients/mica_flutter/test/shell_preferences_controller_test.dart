import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';
import 'package:mica_flutter/session/shell_preferences_controller.dart';
import 'package:mica_flutter/ui/interface_text_scale.dart';

void main() {
  const session = AuthSession(
    accessToken: 'old-token',
    user: User(id: 'old', email: 'old@example.test', displayName: 'Old'),
  );
  late Object owner;
  late Map<String, String> prefs;
  late List<Map<String, String>> pushes;
  late ShellPreferencesController controller;
  late Completer<Map<String, dynamic>> cloud;

  setUp(() {
    owner = ('https://first.example.test', 1);
    prefs = {'showPageTitle': 'true'};
    pushes = [];
    cloud = Completer();
    controller = ShellPreferencesController(
      readPref: (key) => prefs[key],
      writePref: (key, value) => prefs[key] = value,
      session: () => session,
      owner: () => owner,
      isLocal: () => false,
      isMounted: () => true,
      fetch: (_) => cloud.future,
      push: (_, value) async => pushes.add(value),
      onCloudApplied: () {},
    );
  });
  tearDown(() {
    controller.dispose();
    uiFontScaleController.value = kUiFontScaleStandard;
  });

  test('interface font scale is loaded, applied, and persisted', () {
    prefs['uiFontScale'] = '1.1';

    controller.loadAppearance();

    expect(controller.uiFontScale, kUiFontScaleLarge);
    expect(uiFontScaleController.value, kUiFontScaleLarge);
    controller.save();
    expect(prefs['uiFontScale'], '1.1');
  });

  test('invalid interface scale falls back to standard', () {
    for (final value in ['not-a-number', 'NaN', 'Infinity']) {
      prefs['uiFontScale'] = value;
      controller.loadAppearance();
      expect(controller.uiFontScale, kUiFontScaleStandard);
    }
  });

  testWidgets('debounce captures preferences when queued', (tester) async {
    controller.schedulePush();
    prefs['showPageTitle'] = 'false';
    await tester.pump(const Duration(seconds: 1));
    expect(pushes, [
      {'showPageTitle': 'true'},
    ]);
  });

  testWidgets('changing server or session retires a queued write', (
    tester,
  ) async {
    controller.schedulePush();
    owner = ('https://second.example.test', 2);
    await tester.pump(const Duration(seconds: 1));
    expect(pushes, isEmpty);
  });

  test('old cloud reply cannot overwrite the next session', () async {
    final sync = controller.syncFromCloud(session);
    owner = ('https://first.example.test', 2);
    cloud.complete({'showPageTitle': 'false'});
    await sync;
    expect(prefs['showPageTitle'], 'true');
    expect(pushes, isEmpty);
  });

  testWidgets('dispose cancels pending work', (tester) async {
    controller.schedulePush();
    controller.dispose();
    await tester.pump(const Duration(seconds: 1));
    expect(pushes, isEmpty);
  });
}
