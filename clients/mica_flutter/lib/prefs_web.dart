// Web preference persistence: localStorage, EXCEPT for credentials.
//
// The session and refresh tokens used to be written here like everything else,
// and `localStorage` is readable by any same-origin script — so one stored XSS
// on a shared page was an account takeover, and the token it stole outlived the
// tab it was stolen from.
//
// They now live in an `HttpOnly` cookie the server sets, which script cannot
// read at all. What stays on this side is a MEMORY copy for the current page:
// the app passes a token string around for its `Authorization` headers, and
// keeping that in a variable is no weaker than the cookie it came from — both
// die with the tab, and neither survives to a later visit.
//
// A page reload therefore starts with no token in memory. That is the intended
// shape, not a regression: `main.dart` asks `/api/auth/refresh`, which the
// browser answers with the refresh cookie, and gets a fresh one. The durable
// credential is never handed to script.
// ignore: avoid_web_libraries_in_flutter
import 'dart:html' as html;

const _prefix = 'mica.';

/// Credentials, kept out of localStorage. Same rule as the desktop variant's
/// `_isSecret`, and it has to stay the same rule: a key protected on one
/// platform and persisted on the other is worse than either, because whoever
/// added it would believe it was handled.
bool _isSecret(String key) =>
    key == 'authToken' ||
    key.startsWith('authToken:') ||
    key.startsWith('refreshToken:');

/// This page's tokens. Never written to disk, never readable after a reload.
final Map<String, String> _memory = {};

String? loadPref(String key) => _isSecret(key)
    ? _memory[key]
    : html.window.localStorage['$_prefix$key'];

void savePref(String key, String value) {
  if (_isSecret(key)) {
    _memory[key] = value;
    // Clear any copy an older build left behind. Without this the upgrade is
    // cosmetic: the token stops being written, and the one already sitting in
    // localStorage stays there, stealable, forever.
    html.window.localStorage.remove('$_prefix$key');
    return;
  }
  html.window.localStorage['$_prefix$key'] = value;
}

void removePref(String key) {
  _memory.remove(key);
  html.window.localStorage.remove('$_prefix$key');
}
