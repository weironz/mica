import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/prefs_stub.dart';

void isolateProfileForTests() {
  final dir = Directory.systemTemp.createTempSync('mica-test-profile-');
  configRootOverrideForTesting = dir.path;
  tearDownAll(() {
    try {
      dir.deleteSync(recursive: true);
    } on FileSystemException {
      // A store still open on Windows keeps its files locked; the OS temp
      // cleanup takes the rest.
    }
  });
}
