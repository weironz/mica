/// A tally of errors this app deliberately drops.
///
/// Some errors genuinely must not propagate. A WebSocket that cannot reach the
/// server is a STATE, not a crash — the handling lives in `_onDone` (offline
/// badge, backoff reconnect), and letting the same failure also escape through
/// the unobserved `ready` future would write a `WebSocketChannelException` into
/// `crash.log` on every offline moment, burying the faults that matter.
///
/// So those sites drop the error. What they had no way to do was say *how often*
/// — and "it never fired" and "it fired four hundred times" looked identical from
/// outside. That is the gap this closes: the drop stays, the count does not.
///
/// **Deliberately not wired to `onFault`.** That callback drives the sync fault
/// banner, and these are not faults; routing an ordinary offline moment there
/// would put a red badge on a state the app already handles correctly. Counting
/// is observation, not escalation.
///
/// Process-lifetime and in-memory: no file, no gating on the diagnostics switch,
/// nothing to persist. The counts are read back in Settings → 诊断.
library;

final Map<String, int> _counts = {};

/// Count one deliberately-dropped error under [tag], returning the new total.
///
/// [tag] is a stable snake_case name for the SITE, not the error — the whole
/// point is to compare "this path fired" against "this path never fired", which
/// needs the same key every time regardless of which exception showed up.
int swallowed(String tag) {
  final next = (_counts[tag] ?? 0) + 1;
  _counts[tag] = next;
  return next;
}

/// Every tag that has fired at least once, with its count. Empty is the normal,
/// healthy answer — and an empty map is itself the signal that none of these
/// paths were taken.
Map<String, int> swallowedCounts() => Map.unmodifiable(_counts);

/// One line for a bug report, or `null` when nothing has been dropped. `null`
/// rather than an empty string so callers can omit the row entirely instead of
/// showing a label with nothing after it.
String? swallowedSummary() {
  if (_counts.isEmpty) return null;
  final tags = _counts.keys.toList()..sort();
  return tags.map((t) => '$t ×${_counts[t]}').join(', ');
}

/// Reset the tally. For tests — nothing in the app clears these, because a count
/// that resets on its own answers a different question than the one asked.
void resetSwallowed() => _counts.clear();
