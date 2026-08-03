// The session and refresh tokens used to sit in `prefs.json` in plaintext, so
// any copy of that file — a backup, a synced profile, a support bundle — carried
// the account with it. These pin the properties that make the DPAPI wrapper safe
// to put in front of every token read and write.
//
// Runs on Windows against real DPAPI; elsewhere the encryption is a documented
// no-op, so the round-trip tests would be trivially true and are skipped rather
// than left to pass without exercising anything. The two properties that do NOT
// depend on the platform — legacy plaintext passthrough and the format — run
// everywhere.
@TestOn('vm')
library;

import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/secret_store_stub.dart';

const _token =
    'eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ1c2VyIiwiZXhwIjo5OTk5OTk5OTk5fQ.sig';

void main() {
  group('on Windows, where DPAPI exists', () {
    test('a token survives a round trip', () {
      final sealed = protect(_token);

      expect(sealed, isNot(_token), reason: 'it must not be stored as-is');
      expect(
        sealed,
        isNot(contains(_token)),
        reason: 'and must not merely be wrapped around the plaintext',
      );
      expect(unprotect(sealed), _token);
    }, skip: !secretsAreEncrypted);

    // The bug this guards: `prefs_stub` re-runs its migration on every cold
    // start, so a non-idempotent protect would double-encrypt on the second
    // launch. `unprotect` then peels one layer and returns a `dpapi1:` string,
    // which goes out as an Authorization header — a failure that looks nothing
    // like its cause.
    test('protecting twice is the same as protecting once', () {
      final once = protect(_token);
      final twice = protect(once);

      expect(twice, once);
      expect(unprotect(twice), _token, reason: 'not a ciphertext string');
    }, skip: !secretsAreEncrypted);

    // The copied-profile case. Returning the raw ciphertext here would put
    // garbage in an Authorization header; null means "no token", and the
    // caller's existing path for that is the sign-in screen.
    test('a ciphertext this user cannot open reads as no token', () {
      expect(unprotect('dpapi1:bm90LWEtcmVhbC1ibG9i'), isNull);
      expect(unprotect('dpapi1:not-valid-base64!!'), isNull);
    }, skip: !secretsAreEncrypted);
  });

  test('an empty value stays empty rather than becoming ciphertext', () {
    // Sign-out writes '' to these keys; encrypting that would turn "signed out"
    // into a non-empty value that reads as a token to anything checking
    // `isEmpty`.
    expect(protect(''), '');
  });

  // Platform-independent: about the format, not the crypto.
  group('the upgrade path', () {
    test('a token stored before encryption existed still reads back', () {
      expect(
        unprotect(_token),
        _token,
        reason: 'no prefix means legacy plaintext — signing in again is not '
            'the price of upgrading',
      );
    });

    test('anything without the prefix is passed through untouched', () {
      expect(unprotect(''), '');
      expect(unprotect('dark'), 'dark');
    });
  });

  test('the platform tells the truth about whether it encrypts', () {
    if (Platform.isWindows) {
      expect(
        secretsAreEncrypted,
        isTrue,
        reason: 'crypt32.dll is present on every supported Windows',
      );
    } else {
      expect(
        secretsAreEncrypted,
        isFalse,
        reason: 'and callers must be able to see that, not assume it',
      );
    }
  });
}
