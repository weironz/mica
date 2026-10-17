import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:http/http.dart' as http;

import '../api/models.dart';
import '../api/profile_watch.dart';
import '../api/session_refresher.dart';
import '../session/session_controller.dart';

/// The auth domain's session half of `_WorkspaceShellState` — the per-origin
/// credential persistence, the renewal/expiry lifecycle, the profile refresh,
/// the sign-in core and the server readiness probe — extracted verbatim
/// (god-class sixth step; see
/// docs/code-optimization/OPTIMIZATION_PLAN_ROUND3.md §7).
///
/// The SESSION itself is not stored here: it lives in the injected
/// [SessionController] (third step), and every adoption goes through its
/// `adoptSession`/`signIn` transitions — this controller never writes the hub
/// fields directly. What it owns is everything AROUND the session: the prefs
/// keys it is filed under, the guards that decide whether an awaited result
/// still describes the sign-in it was started for, and the probe state the
/// sign-in form reads.
///
/// All remote calls, prefs IO and side effects arrive as injected functions
/// (no `MicaApi` dependency — tests script them directly); refusal RESULTS are
/// enums so the State can localize them — the controller never sees a
/// context. The one user-visible outcome it cannot phrase as a return value
/// (the expired-session banner + local-world fallback) is reported through
/// [onSessionExpired].
class AuthController extends ChangeNotifier {
  AuthController({
    required this.session,
    required this.origin,
    required this.servers,
    required this.readPref,
    required this.savePref,
    required this.isMounted,
    required Future<bool> Function(AuthFormValue form) register,
    required Future<AuthSession> Function(AuthFormValue form) login,
    required Future<List<Workspace>> Function(String token) listWorkspaces,
    required Future<User> Function(String token) fetchMe,
    required Future<AuthSession> Function(String refreshToken) refreshSession,
    required Future<void> Function(String token) removeAvatar,
    required Future<http.Response> Function(Uri url) httpGet,
    required this.avatarUrlFor,
    required this.evictImage,
    this.probeTimeout = const Duration(seconds: 4),
  }) : _register = register,
       _login = login,
       _listWorkspaces = listWorkspaces,
       _fetchMe = fetchMe,
       _refreshSession = refreshSession,
       _removeAvatar = removeAvatar,
       _httpGet = httpGet;

  /// The session/workspace hub — the sign-in itself lives here.
  final SessionController session;

  /// The cloud origin credentials are keyed by (the registry's live binding).
  final String Function() origin;

  /// Every configured server — for [signedInOrigins].
  final List<String> Function() servers;

  final String? Function(String key) readPref;
  final void Function(String key, String value) savePref;

  /// The State's `mounted` — the mid-flight guards below protect DURABLE
  /// writes (the credential prefs), not just setState.
  final bool Function() isMounted;

  final Future<bool> Function(AuthFormValue form) _register;
  final Future<AuthSession> Function(AuthFormValue form) _login;
  final Future<List<Workspace>> Function(String token) _listWorkspaces;
  final Future<User> Function(String token) _fetchMe;
  final Future<AuthSession> Function(String refreshToken) _refreshSession;
  final Future<void> Function(String token) _removeAvatar;
  final Future<http.Response> Function(Uri url) _httpGet;

  /// Compose the avatar address for a version — injected because it reads the
  /// live api base (`ui/avatar_url.dart`), which is the State's to know.
  final String? Function(String? version, {String? userId}) avatarUrlFor;

  /// Evict a cached avatar image (`NetworkImage(url).evict()` — injected so
  /// tests need no image cache).
  final void Function(String url) evictImage;

  /// How long the readiness probe waits before deciding the server is down.
  final Duration probeTimeout;

  // ── Presentation hook (implemented by the State) ─────────────────────────

  /// The sign-in expired and was dropped ([endExpiredSession]): the State
  /// shows the banner and falls back to the local world — both need a context
  /// and the navigation origin, neither of which a controller may touch.
  void Function()? onSessionExpired;

  /// A fingerprint of the CURRENT sign-in: the server it belongs to, the user
  /// it is for, and the refresh token it holds.
  ///
  /// Captured before an await and re-read after, to answer "is the thing that
  /// came back still about the session I asked on behalf of?". A cheap
  /// generation counter would do the same job, but it needs an increment at
  /// every place a session is assigned, and the first one anybody forgets is a
  /// silent resurrection bug again. Reading the identity out of the session
  /// itself cannot be forgotten.
  ///
  /// The refresh token is part of it deliberately: it ROTATES on every refresh,
  /// so a result computed from a token we no longer hold is exactly the stale
  /// one to discard — and it is also what distinguishes "same user, same
  /// server, new token" (accept) from "different sign-in entirely" (drop).
  String? get _sessionIdentity {
    final account = _accountIdentity;
    return account == null ? null : '$account|${session.session!.refreshToken}';
  }

  /// Server + user only — for an await whose result describes the ACCOUNT
  /// rather than the sign-in: the profile endpoint answers "who is this user",
  /// and a refresh token rotating underneath it does not make that answer
  /// wrong.
  ///
  /// Using the strict [_sessionIdentity] there would discard a legitimate
  /// profile update every time a refresh landed first, which is a silent
  /// regression of its own (the avatar would simply stop refreshing on a slow
  /// connection, where refreshes overlap polls most).
  String? get _accountIdentity {
    final current = session.session;
    if (current == null) return null;
    return '${origin()}|${current.user.id}';
  }

  /// Renews the access token before it lapses. See [SessionRefresher] for the
  /// two rules it exists to enforce (renew early; never two at once).
  ///
  /// Public because the startup restore (still in the State) renews through
  /// the same single-flight latch rather than talking to the api directly.
  late final SessionRefresher refresher = SessionRefresher(
    refresh: (token) => _refreshSession(token),
  );

  /// Notices a profile edited on another device — see [ProfileWatch] for why
  /// this is a look and not a push.
  late final ProfileWatch _profileWatch = ProfileWatch(
    fetch: (token) => _fetchMe(token),
  );

  /// Persist the access token + user so a restart (desktop) or browser
  /// refresh (web) restores the session instead of forcing re-login.
  /// Plaintext in the same prefs store as other settings — a per-origin
  /// `window.localStorage` entry on web, a JSON file on desktop (DPAPI
  /// encryption is a noted hardening follow-up; the localStorage copy is
  /// likewise XSS-exposed). The token is server-specific — sign-out (which a
  /// server switch also calls) clears it, so a saved token always matches the
  /// configured backend.
  // Credentials are keyed per cloud origin (P3c-2), so pointing the app at a
  // different server doesn't destroy the previous server's session — switching
  // back restores it without retyping.
  void persistSession(AuthSession value) {
    final origin = this.origin();
    savePref('authToken:$origin', value.accessToken);
    savePref('authUser:$origin', jsonEncode(value.user.toJson()));
    // The refresh token rotates on every use, so this write is what keeps the
    // sign-in alive across a restart: persist the replacement or the next
    // launch spends a burnt token and the user is bounced to the login screen.
    savePref('refreshToken:$origin', value.refreshToken);
  }

  void clearPersistedSession() {
    final origin = this.origin();
    savePref('authToken:$origin', '');
    savePref('authUser:$origin', '');
    savePref('refreshToken:$origin', '');
    // Also clear the legacy single-key copies so an explicit sign-out can't be
    // resurrected by a future migration re-run.
    savePref('authToken', '');
    savePref('authUser', '');
  }

  /// Servers we hold credentials for — entering one costs no password.
  ///
  /// Keyed off `authUser:$origin` rather than the access token: the token is
  /// the part that expires (and on web is never on disk at all), while the
  /// stored user is what the State's startup restore needs to rebuild a
  /// session from the refresh token. Using the token here would mark a
  /// perfectly good sign-in as absent every time it lapsed, which is most
  /// mornings.
  Set<String> signedInOrigins() => {
    for (final origin in servers())
      if ((readPref('authUser:$origin') ?? '').isNotEmpty) origin,
  };

  /// The display name stored for [origin], or null if we hold none.
  ///
  /// Read from prefs rather than the live session, because the two disagree
  /// exactly where it matters: entering 本地模式 drops the in-memory session
  /// while the credentials stay on disk. Asking the session there answers
  /// "nobody is signed in", which is how the world picker ended up demanding
  /// a password for a server it could have walked straight into.
  String? storedDisplayName(String origin) {
    final raw = readPref('authUser:$origin');
    if (raw == null || raw.isEmpty) return null;
    try {
      return User.fromJson(jsonDecode(raw) as Map<String, dynamic>).displayName;
    } catch (_) {
      return null; // corrupt value: treat as no stored identity
    }
  }

  /// Adopt a profile change made elsewhere. Rate-limited and change-gated by
  /// [ProfileWatch], so calling it from the app's hot paths is cheap and does
  /// not churn state.
  ///
  /// Silent on failure by design: it rides along on other work, and a blip
  /// here must not put a banner over an action that succeeded.
  Future<void> refreshProfile() async {
    final current = session.session;
    if (current == null) return;
    // Which ACCOUNT this poll is about. Loose on purpose: see
    // [_accountIdentity].
    final identity = _accountIdentity;
    try {
      final user = await _profileWatch.poll(current);
      // Re-read the session: the poll awaited a round trip, and a sign-out or a
      // token rotation in that window would make `current` the wrong thing to
      // build on — copyWith on the stale one would resurrect a dead token.
      //
      // The identity check is the other half, and the one the re-read cannot
      // do on its own: a session that changed USER or SERVER is a different
      // account, and writing this user's profile into it would show the wrong
      // name and avatar — then persist them under the new server's key.
      final now = session.session;
      if (user == null || now == null || !isMounted()) return;
      if (_accountIdentity != identity) return;
      final updated = now.copyWith(user: user);
      session.adoptSession(updated);
      persistSession(updated);
      // No ImageCache evict needed: the address carries the version
      // (`ui/avatar_url.dart`), so a new picture is a new URL and the stale
      // entry is simply never asked for again.
    } catch (_) {
      // Offline, or a 401 that the surrounding action already handles.
    }
  }

  Future<void> ensureFreshSession() async {
    final current = session.session;
    if (current == null) return;
    // Bind the round trip to the sign-in it was started for. The refresh takes
    // a network round trip, and a sign-out, a server switch, or signing in as
    // somebody else all land inside that window — after which installing the
    // OLD account's tokens over the new session, and persisting them under
    // whatever origin is current by then, would be a resurrection, not a
    // stale read: the user is back on an account they left.
    final identity = _sessionIdentity;
    try {
      final next = await refresher.ensureFresh(current);
      if (next == null || !isMounted()) return;
      if (_sessionIdentity != identity) return;
      session.adoptSession(next);
      // Persist immediately: the refresh token just rotated, and the copy on
      // disk is now burnt. Losing this write costs the sign-in at next launch.
      persistSession(next);
    } on ApiException catch (error) {
      // The server refused the refresh token: 30 days idle, revoked, or its
      // family burnt by reuse detection. Nothing to recover — say so, rather
      // than firing doomed requests and leaving the user at `unauthorized`.
      // Only for the session that made the call: a refusal aimed at a sign-in
      // the user has already left must not end the one they are on now.
      if (error.isUnauthorized && _sessionIdentity == identity) {
        endExpiredSession();
      }
    } catch (_) {
      // Offline / server down: keep the session. The token may well still be
      // good, and local-first means there is work to do without a network.
    }
  }

  /// The sign-in is over and cannot be renewed. Drop it and say so plainly.
  void endExpiredSession() {
    if (!isMounted() || session.session == null) return;
    clearPersistedSession();
    // Only the session drops — the workspace list survives so the local
    // world can fall back coherently (SessionController.expireSession).
    session.expireSession();
    // The banner and the local-world fallback are the State's: they need a
    // context and the navigation origin, neither of which a controller may
    // touch.
    onSessionExpired?.call();
  }

  /// The sign-in form's flow (the old `_authenticate` core): register stops at
  /// the round trip, a refused login comes back as an [AuthRefusal] for the
  /// State to localize, and anything else propagates raw for the busy
  /// wrapper's banner.
  Future<AuthOutcome> authenticate(AuthMode mode, AuthFormValue form) async {
    // Registering no longer signs you in: the address has to be confirmed
    // first, so there is no session to take. Say so and stop here —
    // navigating into the app would be a lie about what just happened.
    if (mode == AuthMode.register) {
      // The FIRST account on an empty instance is verified server-side on the
      // spot, so telling it to go check its email is two lies in one
      // sentence: nothing was sent, and nothing needs clicking. Ask the
      // server which case this is instead of assuming the common one.
      final verified = await _register(form);
      return AuthRegistered(verified: verified);
    }
    final AuthSession signedIn;
    try {
      signedIn = await _login(form);
    } on ApiException catch (error) {
      // Map the two failures a person can actually act on. Done HERE and not
      // in the api-message helper, because these codes mean different things
      // elsewhere: a 401 with a live session is an expired token (handled by
      // the busy wrapper), and `conflict` covers stale writes too. Left alone,
      // the form showed the server's bare English word — 「unauthorized」 in
      // a Chinese UI, which is barely better than the nothing it used to show.
      if (error.isUnauthorized) {
        return const AuthRefused(AuthRefusal.badCredentials);
      }
      if (error.statusCode == 409) {
        return const AuthRefused(AuthRefusal.emailTaken);
      }
      // The server distinguishes "confirm your email" from "wrong password"
      // with a code precisely so this branch can exist: one of them has a
      // next step the person can take, and it is not the same next step.
      if (error.code == 'email_not_verified') {
        return const AuthRefused(AuthRefusal.emailNotVerified);
      }
      rethrow;
    }
    await _completeSignIn(signedIn);
    return AuthSignedIn(signedIn);
  }

  /// The switcher/account-UI sign-in core (the old `_promptSignIn` body): the
  /// same sequence as [authenticate]'s login path but with NO refusal
  /// mapping — a failure surfaces through the caller's busy wrapper as the
  /// server worded it.
  Future<AuthSession> signIn(AuthFormValue form) async {
    final signedIn = await _login(form);
    return _completeSignIn(signedIn);
  }

  /// The raw registration round trip — the switcher's prompt path calls it
  /// without caring whether the instance auto-verified the account.
  Future<bool> register(AuthFormValue form) => _register(form);

  /// Login → workspace list → the state writes both sign-in entry points
  /// share. A listWorkspaces failure propagates BEFORE anything is signed in
  /// or persisted.
  Future<AuthSession> _completeSignIn(AuthSession signedIn) async {
    final workspaces = await _listWorkspaces(signedIn.accessToken);
    session.signIn(signedIn, workspaces);
    persistSession(signedIn);
    return signedIn;
  }

  /// Whether the server at [regOrigin] accepts new registrations, as of the
  /// last probe. Null — including "we asked a server too old to answer" —
  /// means DON'T KNOW, and the registration entry then stays exactly as it
  /// was. Guessing "closed" would hide the only way into a fresh instance;
  /// guessing "open" would show a door that 403s.
  bool? registrationOpen;

  /// Which origin [registrationOpen] describes. Kept beside it so switching
  /// servers cannot show the previous server's answer: the State's
  /// active-registration getter only trusts it while the two still agree.
  String? regOrigin;

  /// Does this server answer? Straight to `/api/health`, unauthenticated —
  /// the point is to tell "wrong address / not running" apart from "wrong
  /// password", which is the failure a first-time user actually hits.
  Future<bool> probeServer(String origin) async {
    final base = Uri.tryParse(origin);
    if (base == null || base.host.isEmpty) return false;
    try {
      // `/api/ready`, not `/api/health`. The sign-in screen's real question is
      // "can you serve me", not "is your process alive" — an instance whose
      // database is down cannot sign anyone in. And only the readiness probe
      // touches the database, which is exactly what answering
      // `registration_open` requires (a brand-new instance with no users
      // still accepts its first account, however the flag is set).
      final r = await _httpGet(base.resolve('/api/ready')).timeout(
        probeTimeout,
      );
      if (r.statusCode != 200) return false;
      bool? open;
      try {
        final body = jsonDecode(r.body);
        if (body is Map && body['registration_open'] is bool) {
          open = body['registration_open'] as bool;
        }
      } catch (_) {
        // A 200 with a body we cannot read still proves reachability, and a
        // server older than this field simply doesn't send it. Either way the
        // answer is "don't know", never "closed".
      }
      // Publish only on the way OUT: this runs inside the pane's probe, and
      // touching state on the way in would land during its build.
      if (isMounted()) {
        regOrigin = origin;
        registrationOpen = open;
        notifyListeners();
      }
      return true;
    } catch (_) {
      return false;
    }
  }

  Future<String?> removeAvatar(AuthSession session) async {
    await _removeAvatar(session.accessToken);
    return applyAvatarVersion(session, null);
  }

  /// Same copyWith discipline as a profile update: rebuilding the session
  /// here would drop the refresh token, and changing your picture would cost
  /// you the ability to renew.
  String? applyAvatarVersion(AuthSession session, String? version) {
    final updated = session.copyWith(
      user: session.user.withAvatarVersion(version),
    );
    if (isMounted()) {
      this.session.adoptSession(updated);
      persistSession(updated);
    }
    // Derived from the version we just set, not read back out of state — the
    // answer must be right even when this widget is already gone.
    final url = avatarUrlFor(version, userId: updated.user.id);
    // The version is a content hash, so setting → removing → setting the SAME
    // picture lands back on the exact URL that 404'd in between, and
    // Flutter's ImageCache holds that failure for the rest of the run: the
    // app would say you have a picture and draw your initial. Evicting here
    // is the one moment anything knows the bytes behind this URL just
    // changed.
    if (url != null) evictImage(url);
    return url;
  }
}

/// What an authenticate attempt ended in. The refusal KINDS are enums, not
/// sentences: the words live in the State (localization needs a context) while
/// the decision — WHICH failure it was — stays here, testable.
sealed class AuthOutcome {
  const AuthOutcome();
}

/// Registration succeeded without a session — [verified] distinguishes the
/// first account on an empty instance (usable immediately) from the ordinary
/// case that still has to confirm its email.
class AuthRegistered extends AuthOutcome {
  const AuthRegistered({required this.verified});

  final bool verified;
}

class AuthSignedIn extends AuthOutcome {
  const AuthSignedIn(this.session);

  final AuthSession session;
}

class AuthRefused extends AuthOutcome {
  const AuthRefused(this.refusal);

  final AuthRefusal refusal;
}

enum AuthRefusal { badCredentials, emailTaken, emailNotVerified }
