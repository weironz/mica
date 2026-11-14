import 'dart:async';

import 'support/isolated_profile_stub.dart'
    if (dart.library.io) 'support/isolated_profile_io.dart';

/// Every test process gets a throwaway profile. The desktop prefs and local
/// store otherwise resolve to a real per-user profile, and shell tests seed
/// servers and tokens there.
Future<void> testExecutable(FutureOr<void> Function() testMain) async {
  isolateProfileForTests();
  await testMain();
}
