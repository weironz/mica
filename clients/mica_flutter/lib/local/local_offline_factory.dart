import 'package:flutter/foundation.dart' show visibleForTesting;

import 'local_offline.dart';

/// Whole-shell cloud widget tests can supply their storage dependency without
/// loading the native bridge. Production always constructs the real facade.
@visibleForTesting
LocalOffline Function()? localOfflineFactoryForTesting;

LocalOffline createLocalOffline() =>
    localOfflineFactoryForTesting?.call() ?? LocalOffline();
