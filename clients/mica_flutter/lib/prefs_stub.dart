/// Desktop/mobile preference persistence: a single JSON file under the
/// platform's per-user config directory. Mirrors the web variant's
/// localStorage semantics (synchronous string get/set/remove) by holding the
/// whole map in memory and rewriting the file on each mutation. The map is a
/// handful of appearance toggles, so full rewrites stay cheap.
library;

import 'dart:convert';
import 'dart:io';

Map<String, String>? _cache;

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
  return _cache = map;
}

void _flush() {
  try {
    final file = _prefsFile();
    file.parent.createSync(recursive: true);
    file.writeAsStringSync(jsonEncode(_cache));
  } catch (_) {
    // Best-effort: a failed write just means this preference won't persist.
  }
}

String? loadPref(String key) => _store()[key];

void savePref(String key, String value) {
  _store()[key] = value;
  _flush();
}

void removePref(String key) {
  _store().remove(key);
  _flush();
}
