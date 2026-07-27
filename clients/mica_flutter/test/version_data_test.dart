import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';
import 'package:mica_flutter/ui/version_data.dart';

/// The trap here is the timezone. The server stamps versions in UTC (`…Z`), so
/// comparing a raw parse against `DateTime.now()` files an evening save under
/// 「更早」 while it is still today for the person looking at it.
///
/// The grouping cases below deliberately use timestamps WITHOUT a `Z`, so they
/// parse as local time and the assertions hold on any machine. A UTC string mixed
/// into those cases would make the expected calendar day depend on the runner's
/// timezone — green in CST, red in UTC-8. The conversion itself is covered by its
/// own case.
void main() {
  DocVersion v(String createdAt, {String? label}) =>
      DocVersion(id: createdAt, label: label, createdAt: createdAt);

  group('versionTime', () {
    test('a UTC stamp is converted to local time, same instant', () {
      final parsed = versionTime(v('2026-07-27T10:00:00Z'));

      expect(parsed, isNotNull);
      expect(parsed!.isUtc, isFalse, reason: 'must be local, not UTC');
      expect(parsed.toUtc(), DateTime.utc(2026, 7, 27, 10));
    });

    test('an unparseable stamp is null rather than a guess', () {
      expect(versionTime(v('')), isNull);
      expect(versionTime(v('not a date')), isNull);
    });
  });

  group('versionDayOf', () {
    final now = DateTime(2026, 7, 27, 14, 30);

    test('any time on the same calendar day is today', () {
      // 23 hours apart, both today — that is what a person means by "today".
      expect(
        versionDayOf(DateTime(2026, 7, 27, 0, 30), now: now),
        VersionDay.today,
      );
      expect(
        versionDayOf(DateTime(2026, 7, 27, 23, 30), now: now),
        VersionDay.today,
      );
    });

    test('the previous calendar day is yesterday, even one minute earlier', () {
      expect(
        versionDayOf(DateTime(2026, 7, 26, 23, 59), now: now),
        VersionDay.yesterday,
      );
    });

    test('further back is earlier', () {
      expect(
        versionDayOf(DateTime(2026, 7, 25, 23, 59), now: now),
        VersionDay.earlier,
      );
      expect(
        versionDayOf(DateTime(2025, 7, 27, 14, 30), now: now),
        VersionDay.earlier,
      );
    });

    test('a month boundary goes by calendar date, not 24h arithmetic', () {
      final firstOfMonth = DateTime(2026, 8, 1, 1, 0);

      expect(
        versionDayOf(DateTime(2026, 7, 31, 23, 0), now: firstOfMonth),
        VersionDay.yesterday,
      );
    });

    test('a clock skewed into the future still reads as today', () {
      // Better than a version filed under a section dated tomorrow.
      expect(
        versionDayOf(DateTime(2026, 7, 28, 9, 0), now: now),
        VersionDay.today,
      );
    });
  });

  group('versionStamp', () {
    final now = DateTime(2026, 7, 27, 14, 30);

    test('today carries a zero-padded clock time', () {
      final s = versionStamp(DateTime(2026, 7, 27, 9, 5), now: now);

      expect(s.day, VersionDay.today);
      expect(s.time, '09:05');
    });

    test('yesterday keeps its own form, with a time', () {
      final s = versionStamp(DateTime(2026, 7, 26, 22, 3), now: now);

      expect(s.day, VersionDay.yesterday);
      expect(s.time, '22:03');
    });

    test('older drops the clock and carries a date', () {
      final s = versionStamp(DateTime(2026, 7, 14, 22, 3), now: now);

      expect(s.day, VersionDay.earlier);
      expect(
        s.time,
        isEmpty,
        reason: 'a clock time on a two-week-old snapshot is noise',
      );
      expect(s.month, 7);
      expect(s.dayOfMonth, 14);
    });
  });

  group('groupVersions', () {
    final now = DateTime(2026, 7, 27, 14, 30);

    test('splits into today and earlier, keeping server order', () {
      final sections = groupVersions([
        v('2026-07-27T14:00:00'),
        v('2026-07-27T10:00:00'),
        v('2026-07-20T10:00:00'),
      ], now: now);

      expect(sections.length, 2);
      expect(sections.first.day, VersionDay.today);
      expect(sections.last.day, VersionDay.earlier);
      // Order within a section is the order it arrived in.
      expect(sections.first.items.map((e) => e.id), [
        '2026-07-27T14:00:00',
        '2026-07-27T10:00:00',
      ]);
    });

    test('an empty section is not emitted', () {
      final onlyOld = groupVersions([v('2020-01-01T00:00:00')], now: now);

      expect(onlyOld.length, 1);
      expect(onlyOld.single.day, VersionDay.earlier);
    });

    test('no versions means no sections, not an empty header', () {
      expect(groupVersions(const [], now: now), isEmpty);
    });

    test('an unparseable timestamp lands in earlier, never in today', () {
      final sections = groupVersions([v('garbage')], now: now);

      expect(sections.single.day, VersionDay.earlier);
    });

    test('yesterday groups under earlier but keeps a yesterday stamp', () {
      final yesterday = v('2026-07-26T14:00:00');
      final sections = groupVersions([yesterday], now: now);

      expect(sections.single.day, VersionDay.earlier);
      // The section is 「更早」; the row itself still says yesterday.
      final when = versionTime(yesterday)!;
      expect(versionStamp(when, now: now).day, VersionDay.yesterday);
    });
  });
}
