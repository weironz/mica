// Live smoke for the two WS auth changes. Manual, not CI — see the note at the
// bottom for why it cannot be a `cargo test` today.
//
// Proves, against a real server and a real Postgres, the two things the unit
// tests cannot:
//
//   1. a socket authenticated by the `Authorization` header connects at all
//      (the desktop half of "token out of the query string"), and
//   2. when the token's `exp` passes, the SERVER closes the socket with 4401 —
//      not the client giving up, not a silent hang.
//
// The unit tests cover the deadline arithmetic, the close code, and the URL
// rewrite. None of them can tell you the timer is actually armed on the live
// path: `run_connection` is only reachable through a real upgrade.
//
// Run it:
//   docker compose up -d postgres
//   ACCESS_TOKEN_TTL_SECONDS=20 JWT_SECRET=<32+ chars> \
//     MICA_REGISTRATION_ENABLED=true APP_ENV=development \
//     DATABASE_URL=postgres://mica:mica@127.0.0.1:5432/mica \
//     cargo run -p mica-api-server
//   dart run tool/ws_expiry_smoke.dart            # from clients/mica_flutter
//
// Exits non-zero and says which assertion failed; prints elapsed seconds so a
// close that happens for some OTHER reason (server restart, network) is
// distinguishable from the deadline firing.
import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:mica_flutter/api/ws_connect_stub.dart';

const _base = 'http://127.0.0.1:8080';

/// Must match `ACCESS_TOKEN_TTL_SECONDS` the server was started with.
const _ttlSeconds = 20;

final _client = HttpClient();

Future<Map<String, dynamic>> _send(
  String method,
  String path, {
  Object? body,
  String? token,
}) async {
  final request = await _client.openUrl(method, Uri.parse('$_base$path'));
  request.headers.contentType = ContentType.json;
  if (token != null) {
    request.headers.set(HttpHeaders.authorizationHeader, 'Bearer $token');
  }
  if (body != null) request.write(jsonEncode(body));
  final response = await request.close();
  final text = await response.transform(utf8.decoder).join();
  if (response.statusCode >= 400) {
    throw StateError('$method $path -> ${response.statusCode}: $text');
  }
  if (text.isEmpty) return const {};
  return jsonDecode(text) as Map<String, dynamic>;
}

/// Confirm the address the way the emailed link would, straight in the dev
/// database — the smoke has no mailbox, and the first-account bypass only
/// applies to a genuinely empty instance.
Future<void> _markEmailVerified(String email) async {
  final result = await Process.run('docker', [
    'exec',
    'mica-postgres',
    'psql',
    '-U',
    'mica',
    '-d',
    'mica',
    '-c',
    "UPDATE users SET email_verified_at = now() WHERE email = '$email'",
  ]);
  if (result.exitCode != 0) {
    throw StateError('could not confirm the address: ${result.stderr}');
  }
}

void _check(bool ok, String what) {
  stdout.writeln('${ok ? "  ok  " : "  FAIL"}  $what');
  if (!ok) exitCode = 1;
}

Future<void> main() async {
  // A fresh address per run: registration is one-shot per email, and reusing one
  // would make the second run fail for a reason that has nothing to do with WS.
  final email = 'ws-smoke-${DateTime.now().microsecondsSinceEpoch}@mica.test';
  const password = 'smoke-password-123';

  stdout.writeln('== setup ==');
  await _send(
    'POST',
    '/api/auth/register',
    body: {
      'email': email,
      'display_name': 'ws smoke',
      'password': password,
    },
  );
  // Registration answers 204 and the account cannot sign in until its address is
  // confirmed — real behaviour, and the smoke has no mailbox. Flip the column
  // directly rather than weakening the gate for testing: a flag that turns email
  // verification off is a flag that can ship turned off.
  await _markEmailVerified(email);

  final session = await _send(
    'POST',
    '/api/auth/login',
    body: {'email': email, 'password': password},
  );
  final token = session['access_token'] as String;
  stdout.writeln('  signed in, token ttl=${_ttlSeconds}s');

  final workspace = await _send(
    'POST',
    '/api/workspaces',
    body: {'name': 'ws smoke'},
    token: token,
  );
  final workspaceId =
      (workspace['workspace'] as Map<String, dynamic>)['id'] as String;

  final created = await _send(
    'POST',
    '/api/workspaces/$workspaceId/documents',
    body: {'name': 'smoke page'},
    token: token,
  );
  final documentId =
      (created['document'] as Map<String, dynamic>)['id'] as String;
  stdout.writeln('  workspace=$workspaceId document=$documentId');

  // The URI still carries the token; connectAuthedSocket is what moves it into
  // the header on IO. Asserting on the URL it actually requests matters because
  // a header that silently fell back to the query would still connect, and look
  // identical from here.
  final uri = Uri.parse(_base).replace(
    scheme: 'ws',
    path: '/ws/workspaces/$workspaceId/documents/$documentId',
    queryParameters: {'token': token, 'v': '1'},
  );
  final split = splitSocketAuth(uri);
  _check(!split.uri.toString().contains(token), 'the URL carries no token');
  _check(split.token == token, 'the token is handed to the header instead');

  stdout.writeln('== connect ==');
  final started = DateTime.now();
  final channel = connectAuthedSocket(uri);
  await channel.ready;
  stdout.writeln('  connected with Authorization: Bearer (no ?token=)');

  var sawBootstrap = false;
  final closed = Completer<void>();
  channel.stream.listen(
    (raw) {
      if (raw is String && raw.contains('document.bootstrap')) {
        sawBootstrap = true;
      }
    },
    onDone: () {
      if (!closed.isCompleted) closed.complete();
    },
    onError: (_) {},
    cancelOnError: false,
  );

  stdout.writeln('== waiting out the token (${_ttlSeconds}s + margin) ==');
  await closed.future.timeout(
    Duration(seconds: _ttlSeconds + 25),
    onTimeout: () {
      stdout.writeln('  FAIL  the socket was still open past exp');
      exitCode = 1;
    },
  );

  final elapsed = DateTime.now().difference(started).inSeconds;
  _check(sawBootstrap, 'the header alone authenticated (bootstrap arrived)');
  _check(
    channel.closeCode == 4401,
    'the server closed with 4401 (got ${channel.closeCode} '
    '"${channel.closeReason}") after ${elapsed}s',
  );
  // A close landing well before exp means something else dropped it, and
  // passing on that would be a lie about what was proven.
  _check(
    elapsed >= _ttlSeconds - 2,
    'it lived until exp, not longer and not for some other reason',
  );

  _client.close(force: true);
  stdout.writeln(exitCode == 0 ? '\nPASS' : '\nFAIL');
}

// Why this is a script and not a `cargo test`: `mica-api-server` is a bin with
// no lib target, so `crates/api-server/tests/` cannot reach `run_connection` or
// build the router — which is also why every test in that crate today is an
// in-file unit test. Making this a CI regression means giving the crate a lib
// target and moving `main` onto it. That is a real restructuring with its own
// tradeoffs, so it is a separate decision rather than something smuggled in
// alongside a security fix.
