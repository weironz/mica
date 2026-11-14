@TestOn('vm')
library;

import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/prefs_stub.dart';

// Shell tests seed servers and tokens through the real savePref. Before the
// per-process test profile they wrote to %APPDATA%/mica/prefs.json — the file
// the installed app reads — and replaced the user's server list.
void main() {
  test('tests write prefs to a throwaway profile', () {
    final root = configRootOverrideForTesting;
    expect(root, isNotNull);
    expect(root, startsWith(Directory.systemTemp.path));
    expect(configDir(), root);

    savePref('profileIsolationProbe', 'x');
    expect(
      File('$root/prefs.json').readAsStringSync(),
      contains('profileIsolationProbe'),
    );
    removePref('profileIsolationProbe');
  });

  test('a non-release build never resolves to the installed app profile', () {
    expect(profileDirName, 'mica-dev');
    final saved = configRootOverrideForTesting;
    configRootOverrideForTesting = null;
    try {
      final dir = configDir().replaceAll('\\', '/');
      expect(dir, endsWith('/mica-dev'));
    } finally {
      configRootOverrideForTesting = saved;
    }
  });
}
