// Which face the world picker shows: ask for a password, go back, or walk in.
//
// This decision was wrong twice on the same day (2026-08-12), both times in the
// direction of demanding credentials the app already had:
//
//   1. It keyed on the LIVE session. Entering 本地模式 clears that while the
//      stored credentials stay on disk — so the picker offered a login form for
//      a server it could have entered silently.
//   2. The first fix also excluded 本地模式 outright, which hid exactly the case
//      that made the bug visible.
//
// Hence a pure function, with the local-mode rows spelled out.

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/main.dart' show WorldCardMode, worldCardMode;

const _cloud = 'https://mica.example.com';
const _other = 'https://other.example.com';
const _local = 'local';

void main() {
  group('worldCardMode', () {
    test('no stored identity means sign in', () {
      expect(
        worldCardMode(
          migrating: false,
          storedName: null,
          cloudOrigin: _cloud,
          activeOrigin: _cloud,
        ),
        WorldCardMode.signIn,
      );
    });

    test('an empty stored name is no identity either', () {
      // A blank prefs value is what a cleared session leaves behind; reading it
      // as "signed in as ''" would offer a passwordless entry into nothing.
      expect(
        worldCardMode(
          migrating: false,
          storedName: '',
          cloudOrigin: _cloud,
          activeOrigin: _cloud,
        ),
        WorldCardMode.signIn,
      );
    });

    test('already in this world → back', () {
      expect(
        worldCardMode(
          migrating: false,
          storedName: 'willmica',
          cloudOrigin: _cloud,
          activeOrigin: _cloud,
        ),
        WorldCardMode.back,
      );
    });

    test('in 本地模式 with stored credentials → enter, not sign in', () {
      // THE regression. Local mode drops the in-memory session; the credentials
      // are still on disk, so this must not ask for a password.
      expect(
        worldCardMode(
          migrating: false,
          storedName: 'willmica',
          cloudOrigin: _cloud,
          activeOrigin: _local,
        ),
        WorldCardMode.enter,
      );
    });

    test('pointing at a DIFFERENT server we know → enter', () {
      expect(
        worldCardMode(
          migrating: false,
          storedName: 'willmica',
          cloudOrigin: _cloud,
          activeOrigin: _other,
        ),
        WorldCardMode.enter,
      );
    });

    test('a migration always asks, even with credentials in hand', () {
      // That flow needs a real sign-in and says so in its own copy; skipping it
      // would leave the migration with no session to migrate into.
      for (final active in [_cloud, _local, _other]) {
        expect(
          worldCardMode(
            migrating: true,
            storedName: 'willmica',
            cloudOrigin: _cloud,
            activeOrigin: active,
          ),
          WorldCardMode.signIn,
          reason: 'active=$active',
        );
      }
    });
  });
}
