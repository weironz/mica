// The sync policy in one place (lib/ui/settings_sync.dart): which prefs travel
// with the account, what a cloud blob is allowed to write locally, and what a
// device offers up. The shell's wiring can't be constructed in a test — these
// pins are what keep the policy from silently changing shape.
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/ui/settings_sync.dart';

void main() {
  test('identity and device state never sync', () {
    // The blob is account-level; a token or server list in it would follow the
    // account onto someone else's machine.
    for (final forbidden in [
      'authToken',
      'refreshToken',
      'servers',
      'activeOrigin',
      'cloudOrigin',
      'pendingBlobUploads',
      'lastWorkspaceId',
    ]) {
      expect(kSyncedPrefKeys, isNot(contains(forbidden)));
    }
    // And the reported bug's key is in.
    expect(kSyncedPrefKeys, contains('showPageTitle'));
  });

  test('cloudSettingsToApply takes only known string values', () {
    final apply = cloudSettingsToApply({
      'showPageTitle': 'true',
      'themeMode': 'dark',
      // A newer client's key this build doesn't know: must not leak into prefs.
      'someFutureToggle': 'on',
      // Corrupt / hand-edited values degrade to "not set", never crash.
      'fontScale': 1.2,
      'pageWidth': null,
    });
    expect(apply, {'showPageTitle': 'true', 'themeMode': 'dark'});
  });

  test('settingsPayload omits keys the device never set', () {
    // Absent, not empty: an empty value would overwrite another device's.
    final local = {'showPageTitle': 'false', 'themeMode': 'system'};
    expect(settingsPayload((k) => local[k]), local);
  });
}
