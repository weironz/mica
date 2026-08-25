import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/perf/switch_trace.dart';

/// The instrumentation's own risk is that it becomes a cost, or a crash, on the
/// path it was added to measure. These pin the ways that happens.
void main() {
  test('a trace can be driven to completion without throwing', () {
    // Whether tracing is on depends on the build (kDebugMode / dart-define), so
    // the assertion is about survival, not about output: every call site is
    // unconditional and must be safe in both builds.
    final trace = SwitchTrace.begin('ws-0000-0000-0001', cached: true);
    trace.mark('shell');
    trace.request(
      'GET',
      '/api/workspaces/x/views',
      const Duration(milliseconds: 40),
    );
    trace.mark('tree');
    trace.end();
    // Ending twice is normal — a switch can finish down the mirror path, the
    // error path, or both. It must not throw and must not reopen the trace.
    trace.end();
    expect(
      SwitchTrace.current,
      isNull,
      reason: 'a finished trace is not the one in flight',
    );
  });

  test('marks and requests after the end are dropped', () {
    final trace = SwitchTrace.begin('ws-0000-0000-0002', cached: false);
    trace.end();
    // A late-arriving request (the speculative bootstrap that lost the race)
    // must not attach itself to a trace that already reported.
    trace.request('GET', '/api/late', const Duration(milliseconds: 900));
    trace.mark('late');
    expect(SwitchTrace.current, isNull);
  });

  test('the newest switch owns `current`, and an older one cannot steal it', () {
    final first = SwitchTrace.begin('ws-0000-0000-0003', cached: true);
    final second = SwitchTrace.begin('ws-0000-0000-0004', cached: true);
    if (SwitchTrace.enabled) {
      // Clicking a second workspace before the first finished: request timings
      // must follow the switch the user is actually waiting on.
      expect(identical(SwitchTrace.current, second), isTrue);
      // And the superseded trace must not clear `current` out from under it.
      first.end();
      expect(identical(SwitchTrace.current, second), isTrue);
    }
    second.end();
    expect(SwitchTrace.current, isNull);
  });
}
