import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';
import 'package:mica_flutter/api/profile_watch.dart';

// Changing your avatar on one device left every OTHER device drawing the old
// face until you signed in again. Nothing was broken server-side and the URL
// already carries the version — the client simply never looked at the profile
// again after login, and read the same stale copy back off disk on restart.
//
// ProfileWatch is that look. These lock in the two rules that decide whether it
// is useful or merely expensive.

AuthSession sessionWith({String? avatar, String name = 'A'}) => AuthSession(
  accessToken: 'at',
  refreshToken: 'rt',
  user: User(
    id: 'u1',
    email: 'a@b.c',
    displayName: name,
    avatarVersion: avatar,
  ),
);

User serverUser({String? avatar, String name = 'A'}) =>
    User(id: 'u1', email: 'a@b.c', displayName: name, avatarVersion: avatar);

void main() {
  final t0 = DateTime.utc(2026, 8, 7, 12);

  test('a changed avatar comes back', () async {
    final watch = ProfileWatch(fetch: (_) async => serverUser(avatar: 'v2'));
    final got = await watch.poll(sessionWith(avatar: 'v1'), now: t0);
    expect(got?.avatarVersion, 'v2');
  });

  test('an unchanged profile comes back as null, not as itself', () async {
    // Rule 1. The caller rebuilds on whatever it gets and rewrites prefs, so
    // handing back an identical user would churn the shell on every action.
    final watch = ProfileWatch(fetch: (_) async => serverUser(avatar: 'v1'));
    expect(await watch.poll(sessionWith(avatar: 'v1'), now: t0), isNull);
  });

  test('removing the picture elsewhere counts as a change', () async {
    // null is a real value here, not "unknown" — the peer must go back to the
    // initial-letter circle, which a `!= null` guard would have skipped.
    final watch = ProfileWatch(fetch: (_) async => serverUser(avatar: null));
    final got = await watch.poll(sessionWith(avatar: 'v1'), now: t0);
    expect(got, isNotNull);
    expect(got!.avatarVersion, isNull);
  });

  test('a renamed display name counts too', () async {
    // Same snapshot, same staleness — excluding it would be arbitrary.
    final watch = ProfileWatch(fetch: (_) async => serverUser(name: 'B'));
    expect(await watch.poll(sessionWith(), now: t0), isNotNull);
  });

  test('the rate floor holds, and lifts on time', () async {
    // Rule 2. It hangs off every user action; without this it is a GET a click.
    var calls = 0;
    final watch = ProfileWatch(
      minInterval: const Duration(minutes: 2),
      fetch: (_) async {
        calls++;
        return serverUser(avatar: 'v2');
      },
    );
    final session = sessionWith(avatar: 'v1');

    expect(await watch.poll(session, now: t0), isNotNull);
    expect(calls, 1);

    // Inside the window: no request at all, not merely a discarded answer.
    expect(
      await watch.poll(session, now: t0.add(const Duration(seconds: 90))),
      isNull,
    );
    expect(calls, 1, reason: 'the floor must stop the request, not its result');

    expect(
      await watch.poll(session, now: t0.add(const Duration(minutes: 2))),
      isNotNull,
    );
    expect(calls, 2, reason: 'exactly at the interval counts as due');
  });

  test('two polls in the same instant make one request', () async {
    // The stamp is taken before the await; taking it after would let a burst of
    // actions all pass a check that only closes when the first one returns.
    var calls = 0;
    final watch = ProfileWatch(
      fetch: (_) async {
        calls++;
        return serverUser(avatar: 'v2');
      },
    );
    final session = sessionWith(avatar: 'v1');
    await Future.wait([
      watch.poll(session, now: t0),
      watch.poll(session, now: t0),
    ]);
    expect(calls, 1);
  });

  test('a failed look does not wedge the watch shut', () async {
    // The floor is what re-arms it; a thrown error must not also be a permanent
    // "already looked". It costs one skipped interval, not the feature.
    var calls = 0;
    final watch = ProfileWatch(
      fetch: (_) async {
        calls++;
        throw StateError('offline');
      },
    );
    await expectLater(
      watch.poll(sessionWith(avatar: 'v1'), now: t0),
      throwsStateError,
    );
    await expectLater(
      watch.poll(
        sessionWith(avatar: 'v1'),
        now: t0.add(const Duration(minutes: 3)),
      ),
      throwsStateError,
    );
    expect(calls, 2);
  });
}
