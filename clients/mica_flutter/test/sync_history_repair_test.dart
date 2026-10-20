// B11: a push the server can't integrate (it depends on history the server
// lacks — e.g. our acked edits lost to a restore from backup) used to be acked
// and parked as pending: stored, invisible to every other client, forever. The
// server now says so (`missing` on the ack, `ahead` on the base, each with its
// state vector) and the session answers with its diff against that vector.
// The diff itself is the engine's encodeDiffSince; the server side of the heal
// is `a_push_missing_its_history_asks_for_it_and_the_diff_heals` (sync_pg.rs).
// This pins WHICH frames ask.
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/cloud/cloud_sync_session.dart';

void main() {
  test('an ack for a push left pending asks for history', () {
    expect(
      repairSvFor({
        'type': 'sync.ack',
        'ack_id': '4',
        'missing': true,
        'sv': 'AQID',
      }, outboxEmpty: false),
      'AQID',
    );
  });

  test('an ordinary ack does not', () {
    expect(
      repairSvFor({
        'type': 'sync.ack',
        'ack_id': '4',
        'rid': 9,
      }, outboxEmpty: true),
      isNull,
    );
  });

  test('a base showing us ahead asks only when nothing is queued', () {
    final base = {'type': 'sync.base', 'ahead': true, 'sv': 'AQID'};
    expect(
      repairSvFor(base, outboxEmpty: true),
      'AQID',
      reason: 'acked edits the server lost: only we can send them back',
    );
    expect(
      repairSvFor(base, outboxEmpty: false),
      isNull,
      reason: 'unpushed edits: the outbox replay already sends them',
    );
  });

  test('frames from an old server (no sv) never ask', () {
    expect(
      repairSvFor({'type': 'sync.ack', 'missing': true}, outboxEmpty: true),
      isNull,
    );
    expect(
      repairSvFor({'type': 'sync.base', 'ahead': true}, outboxEmpty: true),
      isNull,
    );
    expect(
      repairSvFor({
        'type': 'sync.base',
        'ahead': false,
        'sv': 'AQID',
      }, outboxEmpty: true),
      isNull,
    );
  });
}
