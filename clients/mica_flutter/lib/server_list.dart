// Which servers the app knows about.
//
// Pulled out of `_MicaAppState._loadServers` because it seeded an EMPTY origin
// into the list: the seed is `cloudOrigin`, which is `''` on a fresh install of a
// build with no `MICA_CLOUD_URL` baked in. The account menu then drew a cloud row
// with no label and a delete button, and picking it pointed the whole app at a
// server that does not exist. Worse, it was durable — adding a real server saved
// `['', 'https://…']` back to prefs, so the ghost outlived the condition that
// created it.
//
// A pure function because `_loadServers` sits on a private State inside
// `main.dart` and could not be tested where it was.

import 'dart:convert';

/// The servers to show, from the stored `servers` pref plus the build/session
/// [seed] (`cloudOrigin`).
///
/// [rawPref] is the pref verbatim — null or empty for a fresh install, and
/// possibly garbage, which reads as "no servers" rather than throwing: a corrupt
/// list must not stop the app from starting.
///
/// An empty string is never a server. It is what "no cloud configured" looks
/// like, and the whole point of this function is that such a state produces NO
/// row rather than an unlabeled one. Empties already saved by older builds are
/// dropped on the way in, so the ghost disappears on next launch without a
/// migration.
List<String> knownServers({required String? rawPref, required String seed}) {
  var list = <String>[];
  if (rawPref != null && rawPref.isNotEmpty) {
    try {
      list = [
        for (final entry in jsonDecode(rawPref) as List)
          if (entry is String) entry,
      ];
    } catch (_) {
      list = const [];
    }
  }
  // Order is the user's (the switcher lets them reorder), so dedup keeps the
  // first occurrence rather than sorting.
  final seen = <String>{};
  list = [
    for (final origin in list)
      if (origin.isNotEmpty && seen.add(origin)) origin,
  ];
  if (seed.isNotEmpty && !list.contains(seed)) list = [seed, ...list];
  return list;
}
