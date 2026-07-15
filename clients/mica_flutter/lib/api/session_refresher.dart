import 'models.dart';

/// Keeps a signed-in session's access token fresh.
///
/// Its own class, rather than a couple of fields on the app state, because the
/// two rules it enforces are easy to state, easy to get wrong, and expensive
/// when wrong — and both are only testable if they live somewhere a test can
/// reach:
///
///  1. **Renew ahead of expiry, don't wait to be refused.** Being refused is
///     the whole failure being fixed.
///  2. **Never two refreshes at once.** A refresh token is single-use; the
///     server cannot tell our own second spend from a stolen one, so it burns
///     the entire sign-in (reuse detection). Every API call funnels through one
///     wrapper, so two overlapping calls near expiry would sign the user out by
///     our own hand. All callers await the same future.
class SessionRefresher {
  SessionRefresher({required this.refresh, this.lead = const Duration(minutes: 5)});

  /// Performs the actual `/auth/refresh` round trip.
  final Future<AuthSession> Function(String refreshToken) refresh;

  /// How far ahead of expiry to renew. Long enough that a slow request started
  /// just after the check can't outlive the token it was issued under.
  final Duration lead;

  Future<AuthSession?>? _inFlight;

  /// Whether [session]'s access token is close enough to death to renew now.
  ///
  /// False without a refresh token (nothing to renew with) and false when the
  /// token carries no readable `exp` — otherwise every call would refresh, and
  /// since each refresh rotates, that would be a token-burning treadmill.
  bool needsRenewal(AuthSession session, {DateTime? now}) {
    if (session.refreshToken.isEmpty) return false;
    final expiry = session.expiresAt;
    if (expiry == null) return false;
    return !expiry.isAfter((now ?? DateTime.now().toUtc()).add(lead));
  }

  /// The renewed session, or null if [session] didn't need renewing.
  ///
  /// Concurrent callers share one refresh — see rule 2. Errors propagate: the
  /// caller decides whether a 401 means "the sign-in is over" or a network
  /// blip means "keep it and try later".
  Future<AuthSession?> ensureFresh(AuthSession session, {DateTime? now}) {
    if (!needsRenewal(session, now: now)) return Future.value(null);
    // whenComplete, not then: a failed refresh MUST clear the latch, or every
    // later renewal silently no-ops and the session dies anyway.
    return _inFlight ??= refresh(session.refreshToken)
        .then<AuthSession?>((s) => s)
        .whenComplete(() => _inFlight = null);
  }
}
