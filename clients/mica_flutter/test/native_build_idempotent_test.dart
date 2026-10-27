@TestOn('vm')
library;

import 'dart:io';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/src/rust/frb_generated.dart';
import 'support/native_build.dart';

void main() {
  test(
    'already initialized native library still verifies source fingerprint',
    () async {
      await initCheckedNativeRust();
      expect(RustLib.instance.initialized, isTrue);
      await initCheckedNativeRust(); // Would throw on a second FRB initialization.
      final wrongRoot = Directory.systemTemp.createTempSync('mica_stale_root_');
      try {
        // The loaded real library reports a nonempty source set. An alternate
        // checkout with missing/different sources must fail even when initialized.
        await expectLater(
          initCheckedNativeRust(fingerprintRoot: wrongRoot),
          throwsA(
            isA<StateError>().having(
              (e) => e.message,
              'message',
              contains('stale native library'),
            ),
          ),
        );
        await initCheckedNativeRust(); // Correct checkout still validates afterward.
      } finally {
        wrongRoot.deleteSync(recursive: true);
      }
    },
  );
}
