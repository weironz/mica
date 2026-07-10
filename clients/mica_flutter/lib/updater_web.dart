/// Web stub: the web app is served fresh on every load — there is no local
/// binary to update, so the whole updater is a no-op here (and this keeps
/// `dart:io` out of the web bundle).
library;

import 'updater_common.dart';

export 'updater_common.dart';

bool get updateSupported => false;

Future<UpdateInfo?> checkForUpdate(String currentVersion) async => null;

Future<void> downloadAndApplyUpdate(
  UpdateInfo info, {
  void Function(double progress)? onProgress,
}) async {}
