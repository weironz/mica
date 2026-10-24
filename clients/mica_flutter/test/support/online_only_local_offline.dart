import 'package:mica_flutter/local/local_offline.dart';
import 'package:mica_flutter/local/local_offline_factory.dart';

/// Cloud shell scenarios exercise HTTP and navigation, not native storage.
/// Native store behavior has separate FFI/integration coverage.
class _OnlineOnlyLocalOffline extends LocalOffline {
  @override
  bool get available => false;

  @override
  Future<BigInt?> deviceClientId() async => null;
}

void useOnlineOnlyLocalOffline() {
  localOfflineFactoryForTesting = _OnlineOnlyLocalOffline.new;
}

void resetLocalOfflineFactory() {
  localOfflineFactoryForTesting = null;
}
