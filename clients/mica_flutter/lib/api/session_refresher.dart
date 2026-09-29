import 'dart:async';

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

  /// Keyed by the refresh token, whose entry is a [Completer] rather than the
  /// refresh's own future — so the entry exists in the map from the first
  /// synchronous moment of the request, whatever the refresh callback does.
  ///
  /// that ordering is load-bearing. Doing it the obvious way —
  ///
  ///   final f = refresh(key).whenComplete(() => _inFlight.remove(key));
  ///   _inFlight[key] = f;
  ///
  /// — races itself when `refresh` completes without suspending: the completion
  /// callback runs first and removes nothing, and the assignment then files an
  /// ALREADY-COMPLETE future that no later callback will ever clear. The next
  /// caller gets that stale future back — the previous sign-in's tokens — which
  /// is the bug this map exists to prevent, reintroduced by the fix for it.
  /// (Measured: it hung the shared-refresh test outright.)
  final Map<String, Completer<AuthSession?>> _inFlight = {};

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
    // Same sign-in already renewing → await THAT one. Different sign-in → its
    // own entry, so one account's refresh is never handed to another's caller.
    final existing = _inFlight[session.refreshToken];
    if (existing != null) return existing.future;

    final key = session.refreshToken;
    // The entry is created BEFORE the refresh is started, and that ordering is
    // load-bearing. Doing it the obvious way —
    //   final f = refresh(key).whenComplete(() => _inFlight.remove(key));
    //   _inFlight[key] = f;
    // — races itself when `refresh` completes without suspending: the
    // completion callback runs first and removes nothing, and the assignment
    // then files an ALREADY-COMPLETE future that no later callback will ever
    // clear. The next caller gets that stale future back — the previous sign-in's
    // tokens — which is the bug this whole map exists to prevent, reintroduced
    // by the fix for it. (Measured: it hung the shared-refresh test outright.)
    final entry = Completer<AuthSession?>();
    _inFlight[key] = entry;
    refresh(key).then<AuthSession?>((s) => s).then(
      (s) {
        _inFlight.remove(key);
        entry.complete(s);
      },
      onError: (Object e, StackTrace st) {
        _inFlight.remove(key);
        entry.completeError(e, st);
      },
    );
    return entry.future;
  }
}
