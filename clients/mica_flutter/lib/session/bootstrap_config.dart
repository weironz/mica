import 'package:flutter/foundation.dart' show kReleaseMode;
import '../api/client.dart';

/// The cloud server a fresh install starts with — EMPTY on purpose.
///
/// A shipped binary must not carry anyone's server address: baking one in means
/// every downloaded client points at that host on first launch and can sign up
/// against it, which is somebody's private deployment, not a default. So a new
/// install starts in local mode with no server configured, and the user adds
/// one through "添加服务器…".
///
/// Anyone running a hosted flavour can bake their own in at build time with
/// `--dart-define=MICA_CLOUD_URL=https://…`; the public release sets nothing.
const String kDefaultCloudUrl = String.fromEnvironment('MICA_CLOUD_URL');

/// One-time migration (P3c-2) of the legacy world-switch prefs
/// (`serverMode`/`serverUrl`, the pre-P3 ServerMode model) into the dissolved
/// model: which cloud server is configured ([cloudOrigin]) and which world
/// starts active ([activeOrigin], `'local'` or the cloud origin). Pure — no
/// I/O — so it is unit-testable. Semantics carried over from the old resolve():
/// legacy `cloud`/`self` fold into a URL; a desktop fresh install is
/// local-first UNLESS the user had signed in / set a URL before (they were
/// online users — don't strand them); web is always cloud-active.
({String cloudOrigin, String activeOrigin}) resolveLegacyCloudSetup({
  required String? savedMode,
  required String savedUrl,
  required String authToken,
  required bool isWeb,
}) {
  // Everything downstream keys off Dart-normalized URL strings
  // (_api.baseUri.toString(): lowercased host, default ports stripped) — so the
  // migration MUST emit the same normal form, or a legacy raw URL (mixed-case
  // host, explicit :443) would file the auth token under a key nothing ever
  // reads again (= silent sign-out on the first Settings save).
  String normalize(String url) => Uri.tryParse(url)?.toString() ?? url;
  final onlineUrl = normalize(
    savedUrl.isEmpty ? ApiClient.defaultBaseUri().toString() : savedUrl,
  );
  switch (savedMode) {
    case 'local':
      // Keep the user's configured server (they just weren't ACTIVE on it) so
      // any stale token files under the RIGHT origin — hardcoding Mica Cloud
      // here would send a self-hosted token to the wrong server on restore.
      return (
        cloudOrigin: savedUrl.isEmpty ? kDefaultCloudUrl : onlineUrl,
        activeOrigin: 'local',
      );
    case 'cloud':
      // Legacy: a fixed "Mica Cloud" preset that carried no URL of its own.
      // There is no built-in host to resolve it to any more, so fall back to
      // whatever URL the install had (empty on a build with no
      // MICA_CLOUD_URL) and start local. Such a user re-adds their server once
      // — better than shipping someone's address to everyone to avoid it.
      return kDefaultCloudUrl.isEmpty
          ? (
              cloudOrigin: savedUrl.isEmpty ? '' : onlineUrl,
              activeOrigin: 'local',
            )
          : (cloudOrigin: kDefaultCloudUrl, activeOrigin: kDefaultCloudUrl);
    case 'online':
    case 'self': // legacy self-hosted → same thing, keep its URL
      return (cloudOrigin: onlineUrl, activeOrigin: onlineUrl);
    default:
      final usedOnlineBefore = authToken.isNotEmpty || savedUrl.isNotEmpty;
      return (isWeb || usedOnlineBefore)
          ? (cloudOrigin: onlineUrl, activeOrigin: onlineUrl)
          // Fresh desktop install: local-first, and NO server preconfigured.
          : (cloudOrigin: kDefaultCloudUrl, activeOrigin: 'local');
  }
}

/// Dev convenience: when true, the app signs in automatically on startup so you
/// don't have to log in on every reload. Turn it off for a real login screen:
/// `flutter run --dart-define=MICA_DEV_AUTOLOGIN=false`.
///
/// Never in a release build: no release recipe passes the define, so every
/// shipped app used to try demo/password123 — and then REGISTER it — against
/// any localhost backend, held back only by the server's weak-password list
/// (2026-10-21 review).
const bool kDevAutoLogin =
    !kReleaseMode &&
    bool.fromEnvironment('MICA_DEV_AUTOLOGIN', defaultValue: true);
const String kDevEmail = String.fromEnvironment(
  'MICA_DEV_EMAIL',
  defaultValue: 'demo@mica.dev',
);
const String kDevPassword = String.fromEnvironment(
  'MICA_DEV_PASSWORD',
  defaultValue: 'password123',
);
