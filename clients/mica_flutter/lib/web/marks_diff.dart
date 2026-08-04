// The minimal FORMAT ops that turn a block's current inline formatting into the
// marks it should carry — the Dart mirror of Rust
// `crates/mica-core/src/marks.rs` (`marks_diff_format_ops`).
//
// Two copies exist because the two CRDT engines do: desktop/server run yrs
// (Rust), web runs yjs (JS). They must emit the SAME ops, or a desktop and a
// web client formatting one paragraph would disagree about what their updates
// mean. Kept honest by the same table of cases on both sides — the Rust unit
// tests and `test/marks_diff_test.dart`.
//
// Why a diff at all: the previous writer said "clear the whole block's
// formatting, then replay every mark". That is correct for one writer and a
// guaranteed clobber for two, because `format(0, len, …)` is an operation over
// the ENTIRE text — B's bold on words 6-10 died to A's re-format of word 1, and
// a removal ("this got un-bolded") could not be expressed at all except by
// wiping everything. Emitting only the ranges that differ puts two writers on
// disjoint ranges, exactly as the minimal text splice does for characters.
//
// Kept free of `dart:js_interop` on purpose: the caller parses the yjs delta,
// this decides. That is what lets it be unit-tested in the VM, where the web
// engine itself cannot run.
library;

/// One run of the current text: how many UTF-16 code units it covers, and the
/// formatting attributes on it (a `null` value means the attribute is absent).
typedef MarkRun = ({int len, Map<String, Object?> attrs});

/// A format op to apply: `attrs` maps a mark type to its value, where `null`
/// means REMOVE that attribute over this range.
typedef MarkOp = ({int start, int len, Map<String, Object?> attrs});

/// Comparable form of an attribute value, independent of map key order.
///
/// Values are either `true` (a plain mark) or `{href, title}` (a link). The
/// current side arrives through JSON and the wanted side is built here, so
/// comparing encodings directly would make key order load-bearing.
String _key(Object? v) => v is Map ? 'm:${v['href']}|${v['title']}' : 'b:$v';

/// The value a mark contributes as a yjs attribute: `{href,title}` when it
/// carries link metadata, otherwise `true`. Mirrors `Mark::attr_value`.
Object markAttrValue(Map<String, dynamic> m) {
  final href = m['href'] as String?;
  final title = m['title'] as String?;
  if (href == null && title == null) return true;
  return <String, String>{'href': ?href, 'title': ?title};
}

/// The minimal ops turning [runs] into [target].
///
/// Segments are cut at every run boundary AND every mark edge, so inside a
/// segment a mark either covers all of it or none of it. Adjacent segments with
/// an identical delta are merged, which keeps a whole-block bold to one op
/// rather than one per pre-existing run.
List<MarkOp> marksDiffFormatOps(
  List<MarkRun> runs,
  List<Map<String, dynamic>> target,
) {
  // Current formatting as (endOffset, attrs), plus the total length.
  final ends = <int>[];
  final attrsPerRun = <Map<String, Object?>>[];
  var total = 0;
  for (final r in runs) {
    total += r.len;
    ends.add(total);
    attrsPerRun.add(r.attrs);
  }
  if (total == 0) return const [];

  final live = target.where((m) {
    final s = m['start'] as int?, e = m['end'] as int?;
    return s != null && e != null && e > s;
  }).toList();

  final cuts = <int>{0, total, ...ends};
  for (final m in live) {
    cuts.add((m['start'] as int).clamp(0, total));
    cuts.add((m['end'] as int).clamp(0, total));
  }
  final bounds = cuts.toList()..sort();

  Map<String, Object?> attrsAt(int p) {
    for (var i = 0; i < ends.length; i++) {
      if (p < ends[i]) return attrsPerRun[i];
    }
    return const {};
  }

  final ops = <MarkOp>[];
  for (var i = 0; i + 1 < bounds.length; i++) {
    final a = bounds[i], b = bounds[i + 1];
    if (a >= b) continue;

    // What `target` says this segment should carry. Cuts land on every mark
    // edge, so an overlap here is total coverage.
    final want = <String, Object?>{};
    for (final m in live) {
      if ((m['start'] as int) <= a && (m['end'] as int) >= b) {
        want[m['type'] as String] = markAttrValue(m);
      }
    }
    final now = attrsAt(a);
    final delta = <String, Object?>{};
    want.forEach((k, v) {
      // A `null` in the current run means the attribute is absent, not that it
      // is present holding null.
      final n = now[k];
      if (n == null || _key(n) != _key(v)) delta[k] = v;
    });
    now.forEach((k, v) {
      if (v != null && !want.containsKey(k)) delta[k] = null;
    });
    if (delta.isEmpty) continue;

    if (ops.isNotEmpty &&
        ops.last.start + ops.last.len == a &&
        _sameDelta(ops.last.attrs, delta)) {
      final prev = ops.removeLast();
      ops.add((start: prev.start, len: prev.len + (b - a), attrs: prev.attrs));
    } else {
      ops.add((start: a, len: b - a, attrs: delta));
    }
  }
  return ops;
}

bool _sameDelta(Map<String, Object?> a, Map<String, Object?> b) {
  if (a.length != b.length) return false;
  for (final e in a.entries) {
    if (!b.containsKey(e.key) || _key(b[e.key]) != _key(e.value)) return false;
  }
  return true;
}
