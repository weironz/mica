// Derivations for the version-history timeline: how rows group by day, and how a
// timestamp reads once you know which group it is in.
//
// Pure functions — `_VersionHistoryDialog` is `part of main.dart` and cannot be
// constructed by a test. Wording stays with the caller (same contract as
// `status_kit.dart`), so this file holds no user-facing strings.

import '../api/models.dart';

/// Which day-bucket a version falls into.
enum VersionDay { today, yesterday, earlier }

/// How a version's timestamp should read.
///
/// The shape, not the words: 08 wants `15:08` inside today, `昨天 22:03` for
/// yesterday, and a bare date further back. The caller turns this into text.
typedef VersionStamp = ({
  VersionDay day,

  /// `HH:mm`, zero-padded. Empty when the day is [VersionDay.earlier] — a date is
  /// enough there, and a clock time on a two-week-old snapshot is noise.
  String time,

  /// Local-calendar month and day-of-month, for the [VersionDay.earlier] form.
  int month,
  int dayOfMonth,
});

/// One timeline section: a bucket and the versions in it, newest first.
typedef VersionSection = ({VersionDay day, List<DocVersion> items});

/// Parse [DocVersion.createdAt] into LOCAL time, or null if it is unusable.
///
/// **`.toLocal()` is load-bearing**: the server sends UTC (`…Z`), so comparing a
/// raw parse against `DateTime.now()` drops anything saved in the last few hours
/// into the wrong day for most of the world — an evening edit would file itself
/// under 「更早」 while it is still today.
DateTime? versionTime(DocVersion v) {
  final parsed = DateTime.tryParse(v.createdAt);
  return parsed?.toLocal();
}

/// Which bucket [when] belongs to, by local calendar day.
///
/// Calendar day, not elapsed hours: 00:30 and 23:30 are both "today" even though
/// they are 23 hours apart, and that is what a person means by today.
VersionDay versionDayOf(DateTime when, {DateTime? now}) {
  final ref = now ?? DateTime.now();
  final d = DateTime(when.year, when.month, when.day);
  final today = DateTime(ref.year, ref.month, ref.day);
  final delta = today.difference(d).inDays;
  if (delta <= 0)
    return VersionDay.today; // future clock skew still reads today
  if (delta == 1) return VersionDay.yesterday;
  return VersionDay.earlier;
}

/// How to render [when].
VersionStamp versionStamp(DateTime when, {DateTime? now}) {
  final day = versionDayOf(when, now: now);
  final hh = when.hour.toString().padLeft(2, '0');
  final mm = when.minute.toString().padLeft(2, '0');
  return (
    day: day,
    time: day == VersionDay.earlier ? '' : '$hh:$mm',
    month: when.month,
    dayOfMonth: when.day,
  );
}

/// Split [versions] into 今天 / 更早 sections, preserving the incoming order.
///
/// Yesterday folds into 「更早」 as a *section* while keeping its own stamp form:
/// two sections is what the design draws, and a third one holding a single row on
/// most days is a header that costs more than it explains.
///
/// Order is preserved rather than re-sorted — the server returns newest-first and
/// re-sorting here would silently override it. A version whose timestamp does not
/// parse lands in 「更早」, since an unknown date is certainly not today.
List<VersionSection> groupVersions(List<DocVersion> versions, {DateTime? now}) {
  final today = <DocVersion>[];
  final earlier = <DocVersion>[];
  for (final v in versions) {
    final when = versionTime(v);
    if (when != null && versionDayOf(when, now: now) == VersionDay.today) {
      today.add(v);
    } else {
      earlier.add(v);
    }
  }
  return [
    if (today.isNotEmpty) (day: VersionDay.today, items: today),
    if (earlier.isNotEmpty) (day: VersionDay.earlier, items: earlier),
  ];
}
