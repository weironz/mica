import 'models.dart';

/// Notices that the signed-in user's profile changed on ANOTHER device.
///
/// The session's `user` is a snapshot: written at login, renewed only when the
/// access token is (≤1h), and persisted verbatim across restarts. So changing
/// your avatar on one device left every other device drawing the old face —
/// including after a restart, which reads that same stale snapshot back off
/// disk. Signing in again was the only thing that reliably fixed it, which is
/// how it was reported.
///
/// Deliberately NOT a push: an avatar does not need to arrive within a second,
/// it needs the next look at the profile to notice one changed. So this hangs
/// off work the app was doing anyway.
///
/// Its own class, like [SessionRefresher], because the two rules that keep it
/// from being either useless or rude are only testable if a test can reach them:
///
///  1. **Report a change, not a fetch.** Callers rebuild on what comes back, so
///     handing them an identical user every poll would rebuild the shell (and
///     rewrite prefs) forever.
///  2. **Floor the rate.** It is hung off ordinary user actions, which fire
///     several times a minute; without a floor that is a GET per click.
class ProfileWatch {
  ProfileWatch({
    required this.fetch,
    this.minInterval = const Duration(minutes: 2),
  });

  /// Performs the actual `GET /api/auth/me` round trip.
  final Future<User> Function(String accessToken) fetch;

  /// The shortest gap between two looks. Two minutes because the thing being
  /// noticed is a person editing their own profile on another device — minutes
  /// late is invisible, and anything shorter buys nothing for the traffic.
  final Duration minInterval;

  DateTime? _lastAttempt;

  /// Whether enough time has passed to look again.
  bool dueFor({DateTime? now}) {
    final last = _lastAttempt;
    if (last == null) return true;
    return (now ?? DateTime.now().toUtc()).difference(last) >= minInterval;
  }

  /// The user as the server has them, or null when nothing needs to change:
  /// too soon since the last look, or the profile is already what we show.
  ///
  /// Errors propagate — the caller decides, and for every current caller the
  /// answer is "ignore it": this is a background nicety hung off other work, and
  /// a network blip must not surface over an action that succeeded.
  Future<User?> poll(AuthSession session, {DateTime? now}) async {
    final at = now ?? DateTime.now().toUtc();
    if (!dueFor(now: at)) return null;
    // Stamped BEFORE the await, so two actions in the same moment make one
    // request instead of both racing past a check that only closes on
    // completion.
    _lastAttempt = at;
    final user = await fetch(session.accessToken);
    final same =
        user.avatarVersion == session.user.avatarVersion &&
        user.displayName == session.user.displayName;
    return same ? null : user;
  }
}
