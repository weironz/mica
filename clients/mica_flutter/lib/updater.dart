/// In-app self-updater. On desktop (Windows) it checks GitHub Releases and can
/// download + launch the installer, which force-closes this app, updates, and
/// relaunches. On web it resolves to a no-op variant (and keeps `dart:io` out of
/// the web bundle).
library;

export 'updater_desktop.dart' if (dart.library.html) 'updater_web.dart';
