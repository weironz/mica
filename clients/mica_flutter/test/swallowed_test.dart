// The tally of deliberately-dropped errors (lib/swallowed.dart).
//
// What is worth pinning here is not arithmetic — it is the two properties the
// call sites rely on: the tag is the SITE (so the same key accumulates no matter
// which exception arrived), and "nothing dropped" is reported as absence rather
// than as an empty string, because the Settings row is meant to disappear
// entirely in the healthy case.
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/swallowed.dart';

void main() {
  setUp(resetSwallowed);

  test('a tag accumulates across repeats', () {
    expect(swallowed('cloud_ws_stream'), 1);
    expect(swallowed('cloud_ws_stream'), 2);
    expect(swallowed('cloud_ws_stream'), 3);
    expect(swallowedCounts(), {'cloud_ws_stream': 3});
  });

  test('tags are independent', () {
    swallowed('cloud_ws_ready');
    swallowed('cloud_ws_stream');
    swallowed('cloud_ws_stream');
    expect(swallowedCounts(), {'cloud_ws_ready': 1, 'cloud_ws_stream': 2});
  });

  /// The reason `swallowedSummary` returns `String?` and not `''`: a caller that
  /// cannot tell "no drops" from "empty label" ends up rendering a heading with
  /// nothing after it. Absence is the signal.
  test('nothing dropped summarises as null, not as an empty line', () {
    expect(swallowedSummary(), isNull);
    swallowed('ai_ws_ready');
    expect(swallowedSummary(), 'ai_ws_ready ×1');
  });

  test('the summary is ordered so two reports can be compared by eye', () {
    swallowed('presence_ws_stream');
    swallowed('cloud_ws_ready');
    swallowed('cloud_ws_ready');
    expect(swallowedSummary(), 'cloud_ws_ready ×2, presence_ws_stream ×1');
  });

  /// The map handed out must not be a live handle on the tally — a caller that
  /// could mutate it would be editing the record it came to read.
  test('the snapshot cannot be written through', () {
    swallowed('cloud_ws_ready');
    final snapshot = swallowedCounts();
    expect(() => snapshot['cloud_ws_ready'] = 99, throwsUnsupportedError);
    expect(swallowedCounts()['cloud_ws_ready'], 1);
  });
}
