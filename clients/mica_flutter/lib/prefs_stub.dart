/// Desktop/mobile preference persistence: a single JSON file under the
/// platform's per-user config directory. Mirrors the web variant's
/// localStorage semantics (synchronous string get/set/remove) by holding the
/// whole map in memory and rewriting the file on each mutation. Each rewrite
/// goes to a temp file and renames it over the target (same volume → atomic),
/// so a crash mid-write can't leave a half-written prefs.json that reads back
/// corrupt and silently starts empty. The map is a handful of appearance
/// toggles, so full rewrites stay cheap.
library;

import 'dart:convert';
import 'dart:io';

import 'secret_store_stub.dart';

Map<String, String>? _cache;

/// Which preferences are secrets, and therefore encrypted at rest.
///
/// A rule over key NAMES rather than an explicit `saveSecret` call, on purpose:
/// the opt-in version protects the keys somebody remembered to opt in, and the
/// next token key added — for a second server, a PAT, whatever — would sit in
/// plaintext until someone noticed. Here it is protected by default, and
/// forgetting fails in the safe direction.
///
/// Kept narrow: credentials only. Encrypting the whole file would make the
/// appearance toggles unreadable for no gain, and turn every prefs bug into an
/// opaque one.
bool _isSecret(String key) =>
    key == 'authToken' || // pre-multi-server single token
    key.startsWith('authToken:') ||
    key.startsWith('refreshToken:');

/// The per-user config directory (`{appdata}/mica`). Public so anything that
/// needs to sit beside the preferences — the diagnostics capture — uses this
/// rule instead of writing a second copy of it.
String configDir() {
  final env = Platform.environment;
  if (Platform.isWindows) {
    final appData = env['APPDATA'];
    return '${(appData == null || appData.isEmpty) ? '.' : appData}/mica';
  }
  if (Platform.isMacOS) {
    return '${env['HOME'] ?? '.'}/Library/Application Support/mica';
  }
  final xdg = env['XDG_CONFIG_HOME'];
  return (xdg != null && xdg.isNotEmpty)
      ? '$xdg/mica'
      : '${env['HOME'] ?? '.'}/.config/mica';
}

File _prefsFile() => File('${configDir()}/prefs.json');

Map<String, String> _store() {
  final cached = _cache;
  if (cached != null) return cached;
  final map = <String, String>{};
  try {
    final file = _prefsFile();
    if (file.existsSync()) {
      final decoded = jsonDecode(file.readAsStringSync());
      if (decoded is Map) {
        decoded.forEach((k, v) {
          if (v is String) map['$k'] = v;
        });
      }
    }
  } catch (_) {
    // Corrupt or unreadable file: start empty rather than crash on launch.
  }
  _cache = map;
  _encryptSecretsWrittenBeforeThisExisted();
  return map;
}

/// Upgrade tokens that were stored before encryption existed.
///
/// Done once at first read rather than lazily on next write, because the write
/// that would do it is a login or a token refresh — so a user who simply stays
/// signed in could keep a plaintext token on disk for as long as the session
/// lives, which is precisely the window this is meant to close.
///
/// A no-op where DPAPI is unavailable: rewriting the file to store the same
/// plaintext would be pure churn, and — worse — would make the mtime say
/// something happened.
void _encryptSecretsWrittenBeforeThisExisted() {
  if (!secretsAreEncrypted) return;
  final map = _cache!;
  var changed = false;
  for (final key in map.keys.toList()) {
    if (!_isSecret(key)) continue;
    final value = map[key]!;
    if (value.isEmpty) continue;
    final protectedValue = protect(value);
    // `protect` returns its input unchanged when it could not encrypt; compare
    // rather than assume, so a failure does not get recorded as a migration.
    if (protectedValue == value) continue;
    map[key] = protectedValue;
    changed = true;
  }
  if (changed) _flush();
}

void _flush() {
  try {
    final file = _prefsFile();
    file.parent.createSync(recursive: true);
    // Write the full payload to a sibling temp file first, then rename it over
    // the target. The rename is the only mutation of the real file, so it is
    // either the old complete file or the new complete file that survives a
    // crash — never a truncated one.
    final tmp = File('${file.path}.tmp');
    tmp.writeAsStringSync(jsonEncode(_cache), flush: true);
    try {
      tmp.renameSync(file.path);
    } on FileSystemException {
      // Windows' rename won't overwrite an existing destination. Remove it and
      // retry; the temp still holds the complete new content if we're
      // interrupted between the two calls.
      if (file.existsSync()) file.deleteSync();
      tmp.renameSync(file.path);
    }
  } catch (_) {
    // Best-effort: a failed write just means this preference won't persist.
  }
}

String? loadPref(String key) {
  final stored = _store()[key];
  if (stored == null || !_isSecret(key)) return stored;
  // null here means "there is no usable token" — a ciphertext this user cannot
  // open (copied profile, different account). The caller's own "no token" path
  // sends you to sign in, which is the right answer; returning the raw
  // ciphertext instead would put garbage in an `Authorization` header.
  return unprotect(stored);
}

void savePref(String key, String value) {
  _store()[key] = _isSecret(key) ? protect(value) : value;
  _flush();
}

void removePref(String key) {
  _store().remove(key);
  _flush();
}
