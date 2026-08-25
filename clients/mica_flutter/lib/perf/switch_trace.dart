import 'package:flutter/foundation.dart';

/// Timing for one workspace switch, broken into the parts that can actually be
/// acted on separately.
///
/// This exists because "the switch takes about 170ms" was a number nobody could
/// break down, and every proposal to improve it was therefore a guess — the
/// first round of optimisation shipped on a reading taken with curl from the
/// host, not from inside the app. A switch has at least four distinguishable
/// costs (shell repaint, page tree, document body, and whatever the connection
/// itself costs) and they call for completely different fixes.
///
/// Off unless asked for: `kDebugMode`, or
/// `--dart-define=MICA_TRACE_SWITCH=true` so a RELEASE build can be measured —
/// debug-only timings would say more about the JIT than about the product.
class SwitchTrace {
  SwitchTrace._(this._label, this._cached);

  static const bool _forced = bool.fromEnvironment('MICA_TRACE_SWITCH');

  /// Whether tracing is on at all. Every call site stays unconditional; when
  /// this is false the trace object is a shared do-nothing instance.
  static bool get enabled => kDebugMode || _forced;

  static final SwitchTrace _disabled = SwitchTrace._('', false);

  /// The switch currently in flight, so request timings can be attributed to it
  /// without threading the trace through every method it touches.
  static SwitchTrace? current;

  final String _label;
  final bool _cached;
  final Stopwatch _watch = Stopwatch();
  final List<String> _marks = <String>[];
  final List<String> _requests = <String>[];
  int _lastMs = 0;

  /// Starts a trace. [cached] records whether this workspace's page tree was
  /// already in memory — the hot and cold paths are different enough that
  /// mixing them produces an average that describes neither.
  static SwitchTrace begin(String workspaceId, {required bool cached}) {
    if (!enabled) return _disabled;
    final trace = SwitchTrace._(_short(workspaceId), cached);
    trace._watch.start();
    current = trace;
    return trace;
  }

  /// Records a named point since the switch began, e.g. `shell`, `tree`, `body`.
  void mark(String label) {
    if (!_watch.isRunning) return;
    final now = _watch.elapsedMilliseconds;
    _marks.add('$label ${now - _lastMs}ms');
    _lastMs = now;
  }

  /// Records one HTTP request that happened during this switch.
  void request(String method, String path, Duration headers) {
    if (!_watch.isRunning) return;
    _requests.add('$method ${_short(path)} ${headers.inMilliseconds}ms');
  }

  /// Ends the trace and prints one line. Safe to call twice — a switch can end
  /// down several paths (mirror fallback, error) and none of them should have
  /// to know whether another already finished.
  void end() {
    if (!_watch.isRunning) return;
    _watch.stop();
    if (identical(current, this)) current = null;
    final total = _watch.elapsedMilliseconds;
    final where = _cached ? 'hot' : 'cold';
    debugPrint(
      'switch[$_label $where] total ${total}ms | ${_marks.join(' → ')} '
      '| ${_requests.isEmpty ? 'no requests' : _requests.join(', ')}',
    );
  }

  /// The hook that feeds request timings into whichever switch is in flight.
  /// Null when tracing is off, so the API layer keeps its untimed fast path.
  static void Function(String, String, Duration)? observer() {
    if (!enabled) return null;
    return (method, path, headers) => current?.request(method, path, headers);
  }

  /// Ids and paths are long and the interesting part is at the end.
  static String _short(String value) =>
      value.length <= 12 ? value : '…${value.substring(value.length - 11)}';
}
