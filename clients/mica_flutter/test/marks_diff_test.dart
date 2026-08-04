// The minimal format ops that turn a block's current formatting into its marks.
//
// **This file is deliberately the same table of cases as the Rust side**
// (`crates/mica-core/src/marks.rs`, `mod diff_tests`). Rust is the authority and
// Dart is the mirror; a case added there belongs here too, or the two CRDT
// engines start disagreeing about what a formatting edit MEANS — and a
// disagreement about meaning is exactly the class of bug that converges
// silently into two different documents.
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/web/marks_diff.dart';

MarkRun run(int len, [Map<String, Object?> attrs = const {}]) =>
    (len: len, attrs: attrs);

/// Ops flattened to readable strings, order-independent within an op.
///
/// Strings rather than records-holding-lists: a Dart record compares its fields
/// with `==`, and `List ==` is IDENTITY — so a record with a list inside never
/// equals an equal-looking one, and the failure prints two identical values.
List<String> flat(List<MarkOp> ops) => ops.map((o) {
      final kv = o.attrs.entries.map((e) => '${e.key}=${e.value}').toList()
        ..sort();
      return '${o.start}+${o.len} ${kv.join(",")}';
    }).toList();

void main() {
  test('an unchanged block emits nothing', () {
    final ops = marksDiffFormatOps(
      [run(5, {'bold': true}), run(6)],
      [
        {'start': 0, 'end': 5, 'type': 'bold'},
      ],
    );
    expect(ops, isEmpty, reason: '$ops');
  });

  test('adding a mark touches only its own range', () {
    final ops = marksDiffFormatOps(
      [run(5, {'bold': true}), run(6)],
      [
        {'start': 0, 'end': 5, 'type': 'bold'},
        {'start': 6, 'end': 11, 'type': 'italic'},
      ],
    );
    expect(flat(ops), ['6+5 italic=true']);
  });

  test('removing a mark is a null over just that range', () {
    final ops = marksDiffFormatOps(
      [run(11, {'bold': true})],
      [
        {'start': 6, 'end': 11, 'type': 'bold'},
      ],
    );
    expect(
      flat(ops),
      ['0+6 bold=null'],
      reason: 'only the un-bolded half is written',
    );
  });

  test('one op spans runs that need the same delta', () {
    final ops = marksDiffFormatOps(
      [run(3), run(4), run(4)],
      [
        {'start': 0, 'end': 11, 'type': 'bold'},
      ],
    );
    expect(flat(ops), ['0+11 bold=true']);
  });

  test('a changed href rewrites the link attribute', () {
    final ops = marksDiffFormatOps(
      [
        run(5, {
          'link': {'href': 'http://a'}
        })
      ],
      [
        {'start': 0, 'end': 5, 'type': 'link', 'href': 'http://b'},
      ],
    );
    expect(ops.length, 1, reason: '$ops');
    expect(ops.single.start, 0);
    expect(ops.single.len, 5);
  });

  test('an empty text emits nothing', () {
    expect(
      marksDiffFormatOps(const [], [
        {'start': 0, 'end': 5, 'type': 'bold'},
      ]),
      isEmpty,
    );
  });
}
