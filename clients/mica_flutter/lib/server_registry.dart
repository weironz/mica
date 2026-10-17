import 'dart:convert';

import 'package:flutter/foundation.dart';

import '../server_list.dart' show knownServers;

/// The server list and which cloud origin is bound, extracted verbatim from
/// `_WorkspaceShellState`'s server-management block (god-class wrap-up;
/// docs/code-optimization/OPTIMIZATION_PLAN_ROUND3.md §7). All prefs IO and
/// side effects (api rebasing, mirror forgetting, session restoring) arrive
/// as injected functions; error RESULTS are enums so the State can localize
/// them — the registry never sees a context.
///
/// Rules pinned by tests, all inherited from the original code:
///  - adding a server only ADDS it — switching is a separate explicit act;
///  - removing a server drains and steps away FIRST when it is the active
///    one, then forgets credentials AND mirror, and
///  - removing the LAST server falls back to the build default and never
///    invents a replacement;
///  - reconnecting to the origin that is already bound still restores the
///    stored session (the cloud → local → cloud round trip must not cost a
///    password).
class ServerRegistry extends ChangeNotifier {
  ServerRegistry({
    required this.readPref,
    required this.savePref,
    required this.forgetOrigin,
    required this.rebaseApi,
    required this.restoreSession,
    this.defaultCloudUrl = '',
  });

  final String? Function(String key) readPref;
  final void Function(String key, String value) savePref;
  final Future<void> Function(String origin) forgetOrigin;
  final void Function(Uri base) rebaseApi;
  final Future<void> Function() restoreSession;
  final String defaultCloudUrl;

  List<String> servers = const [];
  String cloudOrigin = '';

  void load({required String seed}) {
    servers = knownServers(rawPref: readPref('servers'), seed: seed);
    notifyListeners();
  }

  void _persist() => savePref("servers", jsonEncode(servers));

  /// Add [url] to the list — and only that. A duplicate or unparseable URL
  /// is refused rather than becoming a dead entry.
  ServerAddResult addServer(String url) {
    final parsed = Uri.tryParse(url.trim());
    if (parsed == null || !parsed.hasScheme || parsed.host.isEmpty) {
      return ServerAddResult.invalidUrl;
    }
    final normalized = parsed.toString();
    if (servers.contains(normalized)) {
      return ServerAddResult.alreadyAdded;
    }
    servers = [...servers, normalized];
    _persist();
    notifyListeners();
    return ServerAddResult.added;
  }

  /// Forget [origin] completely: its credentials AND its on-device mirror
  /// ("删除就删除干净" — true for anything that reached the server, which is
  /// why [onRemovingActive] gets its chance to push pending edits FIRST,
  /// while a session may still exist). Only called for an origin that is
  /// being switched away from because it is the active one.
  Future<void> removeServer(
    String origin, {
    required Future<bool> Function() onRemovingActive,
  }) async {
    if (!servers.contains(origin)) return;
    if (await onRemovingActive()) {
      // The active world moved to local; nothing else in this method may
      // assume the old session is still meaningful.
    }
    servers = servers.where((s) => s != origin).toList();
    _persist();
    // Credentials, then the mirror. Both are keyed by origin, so this is
    // surgical — no other server's data can be caught by it.
    savePref('authToken:$origin', '');
    savePref('authUser:$origin', '');
    savePref('refreshToken:$origin', '');
    await forgetOrigin(origin);
    // The app must always point at a server that exists. Removing the LAST
    // one leaves none configured — fall back to the build default (empty in
    // production builds), and never invent a server to replace the deleted.
    if (cloudOrigin == origin) {
      final next = servers.firstOrNull ?? defaultCloudUrl;
      if (next.isNotEmpty && !servers.contains(next)) {
        servers = [next];
        _persist();
      }
      bind(next);
    }
    notifyListeners();
  }

  /// Point the cloud side at [url]: persist the origin, rebase the api, and
  /// restore whatever credentials that origin has stored. [onChanging] fires
  /// only when the binding is about to MOVE — the same-origin path keeps any
  /// live session untouched (it only restores one when none exists).
  /// Returns whether the binding CHANGED.
  Future<bool> connect(
    String url, {
    Future<void> Function()? onChanging,
  }) async {
    final parsed = Uri.tryParse(url.trim());
    if (parsed == null || !parsed.hasScheme || parsed.host.isEmpty) {
      return false;
    }
    final normalized = parsed.toString();
    if (normalized == cloudOrigin) {
      // Nothing to rebind — which never meant nothing to do. Local mode
      // drops the session but leaves cloudOrigin pointing here, so coming
      // back lands in this branch with credentials sitting in prefs and no
      // session to use them.
      await restoreSession();
      return false;
    }
    if (onChanging != null) await onChanging();
    bind(normalized);
    await restoreSession(); // the new origin may already have stored credentials
    return true;
  }

  /// Set the bound origin (persist + api rebase). The active-origin bookkeeping
  /// is the State's: which world is on screen involves navigation, not just
  /// this binding.
  void bind(String origin) {
    cloudOrigin = origin;
    savePref('cloudOrigin', origin);
    final base = origin.isEmpty ? null : Uri.tryParse(origin);
    if (base != null) rebaseApi(base);
    notifyListeners();
  }
}

enum ServerAddResult { added, invalidUrl, alreadyAdded }
