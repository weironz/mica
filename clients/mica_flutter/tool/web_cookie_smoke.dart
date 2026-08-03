// Live smoke for the web credential change: does the server actually hand out
// cookies with the right attributes, and can a socket authenticate on the
// cookie ALONE?
//
// The unit tests cover the parsing and the precedence. What they cannot cover is
// the contract itself — that `Set-Cookie` really carries `HttpOnly` and
// `SameSite=Strict`, and that a WS upgrade with no `Authorization` header and no
// `?token=` in the URL still gets in. Both only exist on a live path.
//
// What this deliberately does NOT test: whether Chrome sends the cookie on the
// handshake. That is specified browser behaviour for a same-origin request, and
// exercising it would mean driving a sign-in form — which means typing a
// password, which is not something to automate here. The half that could
// plausibly be wrong is the server contract, and that is the half this covers.
//
// Run it:
//   docker compose up -d postgres
//   JWT_SECRET=<32+ chars> MICA_REGISTRATION_ENABLED=true APP_ENV=development \
//     DATABASE_URL=postgres://mica:mica@127.0.0.1:5432/mica \
//     cargo run -p mica-api-server
//   dart run tool/web_cookie_smoke.dart          # from clients/mica_flutter
import 'dart:async';
import 'dart:convert';
import 'dart:io';

const _base = 'http://127.0.0.1:8080';

final _client = HttpClient();
int _failures = 0;

void _check(bool ok, String what) {
  stdout.writeln('${ok ? "  ok  " : "  FAIL"}  $what');
  if (!ok) _failures++;
}

Future<HttpClientResponse> _raw(
  String method,
  String path, {
  Object? body,
  String? token,
  String? cookie,
}) async {
  final request = await _client.openUrl(method, Uri.parse('$_base$path'));
  request.headers.contentType = ContentType.json;
  if (token != null) {
    request.headers.set(HttpHeaders.authorizationHeader, 'Bearer $token');
  }
  if (cookie != null) request.headers.set(HttpHeaders.cookieHeader, cookie);
  if (body != null) request.write(jsonEncode(body));
  return request.close();
}

Future<Map<String, dynamic>> _json(HttpClientResponse response) async {
  final text = await response.transform(utf8.decoder).join();
  if (response.statusCode >= 400) {
    throw StateError('${response.statusCode}: $text');
  }
  return text.isEmpty ? const {} : jsonDecode(text) as Map<String, dynamic>;
}

Future<void> _markEmailVerified(String email) async {
  final result = await Process.run('docker', [
    'exec', 'mica-postgres', 'psql', '-U', 'mica', '-d', 'mica', '-c',
    "UPDATE users SET email_verified_at = now() WHERE email = '$email'",
  ]);
  if (result.exitCode != 0) {
    throw StateError('could not confirm the address: ${result.stderr}');
  }
}

Future<void> main() async {
  final email = 'cookie-smoke-${DateTime.now().microsecondsSinceEpoch}@mica.test';
  const password = 'smoke-password-123';

  stdout.writeln('== sign in ==');
  await _json(await _raw('POST', '/api/auth/register', body: {
    'email': email,
    'display_name': 'cookie smoke',
    'password': password,
  }));
  await _markEmailVerified(email);

  final loginResponse = await _raw('POST', '/api/auth/login',
      body: {'email': email, 'password': password});
  final setCookies = loginResponse.headers[HttpHeaders.setCookieHeader] ?? [];
  final session = await _json(loginResponse);
  final token = session['access_token'] as String;

  String? cookieNamed(String name) =>
      setCookies.where((c) => c.startsWith('$name=')).firstOrNull;

  final sessionCookie = cookieNamed('mica_session');
  final refreshCookie = cookieNamed('mica_refresh');

  _check(sessionCookie != null, 'login sets mica_session');
  _check(refreshCookie != null, 'login sets mica_refresh');
  for (final entry in {'mica_session': sessionCookie, 'mica_refresh': refreshCookie}.entries) {
    final raw = entry.value ?? '';
    // HttpOnly is the whole point — without it script reads the cookie and this
    // is no better than localStorage was.
    _check(raw.contains('HttpOnly'), '${entry.key} is HttpOnly');
    // SameSite=Strict is the single-layer CSRF defence this design rests on.
    _check(raw.contains('SameSite=Strict'), '${entry.key} is SameSite=Strict');
    _check(raw.contains('Path=/'), '${entry.key} is scoped to the whole app');
  }

  final sessionValue = sessionCookie!.split(';').first;

  stdout.writeln('== the cookie alone authenticates ==');
  final me = await _raw('GET', '/api/auth/me', cookie: sessionValue);
  _check(me.statusCode == 200, 'REST accepts the cookie with no Authorization header');
  await me.drain<void>();

  // The reload path: no body, no header — the refresh cookie is the only thing
  // presented. This is what keeps a web user signed in across a page load now
  // that the token lives in memory only.
  final refreshValue = refreshCookie!.split(';').first;
  final refreshed = await _raw('POST', '/api/auth/refresh',
      body: const <String, Object?>{}, cookie: refreshValue);
  final refreshedOk = refreshed.statusCode == 200;
  _check(refreshedOk, 'refresh works from the cookie with an empty body');
  if (refreshedOk) await refreshed.drain<void>();

  stdout.writeln('== a socket on the cookie, with nothing in the URL ==');
  final workspace = await _json(await _raw('POST', '/api/workspaces',
      body: {'name': 'cookie smoke'}, token: token));
  final workspaceId = (workspace['workspace'] as Map<String, dynamic>)['id'];
  final created = await _json(await _raw(
      'POST', '/api/workspaces/$workspaceId/documents',
      body: {'name': 'page'}, token: token));
  final documentId = (created['document'] as Map<String, dynamic>)['id'];

  // Raw upgrade, driven by hand: no `Authorization`, no `?token=` — exactly what
  // the browser sends, minus the parts a browser adds for us.
  final socket = await WebSocket.connect(
    'ws://127.0.0.1:8080/ws/workspaces/$workspaceId/documents/$documentId?v=1',
    headers: {'Cookie': sessionValue},
  ).timeout(const Duration(seconds: 10), onTimeout: () {
    throw StateError('the upgrade never completed');
  });

  final bootstrap = Completer<bool>();
  socket.listen(
    (raw) {
      if (raw is String && raw.contains('document.bootstrap') && !bootstrap.isCompleted) {
        bootstrap.complete(true);
      }
    },
    onDone: () => bootstrap.isCompleted ? null : bootstrap.complete(false),
    onError: (_) => bootstrap.isCompleted ? null : bootstrap.complete(false),
  );
  final gotBootstrap = await bootstrap.future
      .timeout(const Duration(seconds: 10), onTimeout: () => false);
  _check(gotBootstrap, 'the WS handshake authenticated on the cookie alone');
  await socket.close();

  stdout.writeln('== sign out clears them ==');
  final out = await _raw('POST', '/api/auth/logout',
      body: const <String, Object?>{}, cookie: refreshValue);
  final cleared = out.headers[HttpHeaders.setCookieHeader] ?? [];
  _check(
    cleared.any((c) => c.startsWith('mica_session=') && c.contains('Max-Age=0')),
    'logout expires mica_session',
  );
  _check(
    cleared.any((c) => c.startsWith('mica_refresh=') && c.contains('Max-Age=0')),
    'logout expires mica_refresh',
  );
  await out.drain<void>();

  _client.close(force: true);
  stdout.writeln(_failures == 0 ? '\nPASS' : '\nFAIL ($_failures)');
  exitCode = _failures == 0 ? 0 : 1;
}
