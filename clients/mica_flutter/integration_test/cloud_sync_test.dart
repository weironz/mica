// P2-M4.5: two cloud sessions editing the SAME document converge through the
// real server (WS sync protocol + Postgres yrs stream + CRDT merge). This is the
// end-to-end "双路线打通" proof — desktop ↔ desktop realtime sync.
//
// Requires a running M4.4 server (default http://127.0.0.1:8090). Run:
//   $env:DATABASE_URL=...; cargo run -p mica-api-server   # in another shell
//   flutter test integration_test/cloud_sync_test.dart -d windows \
//     --dart-define=MICA_TEST_API=http://127.0.0.1:8090
//
// Skips (passes) if the server isn't reachable, so it never breaks a no-server run.
import 'dart:async';
import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:integration_test/integration_test.dart';
import 'package:mica_flutter/cloud/cloud_sync.dart';
import 'package:mica_flutter/src/rust/frb_generated.dart';

const _apiBase = String.fromEnvironment(
  'MICA_TEST_API',
  defaultValue: 'http://127.0.0.1:8090',
);

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(() async => RustLib.init());
  tearDownAll(() async => RustLib.dispose());

  test('two cloud sessions converge through the real server', () async {
    final base = Uri.parse(_apiBase);

    // Skip cleanly if no server is up.
    try {
      await http.get(base.replace(path: '/api/health')).timeout(const Duration(seconds: 2));
    } catch (_) {
      // ignore: avoid_print
      print('skipping cloud_sync_test: no server at $_apiBase');
      return;
    }

    Map<String, String> json([String? token]) => {
      'content-type': 'application/json',
      if (token != null) 'authorization': 'Bearer $token',
    };

    // Register a fresh user → token.
    final stamp = DateTime.now().microsecondsSinceEpoch;
    final reg = await http.post(
      base.replace(path: '/api/auth/register'),
      headers: json(),
      body: jsonEncode({
        'email': 'sync$stamp@test.dev',
        'display_name': 'Sync',
      // This password has to clear BOTH of `password_strength.rs`'s rules, and
      // getting it wrong costs a full CI round trip — so it was checked against
      // the real endpoint before landing (2026-08-05: register 204, login 200).
      //
      //   1. not in the common list — `password123` is literally in it
      //   2. no >=4-char run shared with the email's LOCAL PART or the display
      //      name — which rules out anything containing `sync`, including the
      //      first replacement tried here
      //
      // The test had been broken since that guard landed and nobody knew,
      // because this file was excluded from CI. Bringing it in surfaced it on
      // the first real run.
      'password': 'orbital-kettle-2026',
      }),
    );
    expect(reg.statusCode, inInclusiveRange(200, 299), reason: reg.body);

    // `register` answers **204 with no body** — it stopped handing back a
    // session when email verification landed, and this test still parsed the
    // body as JSON (`FormatException: Unexpected end of input`). The session
    // comes from `login`.
    //
    // Which only works because this runs against an EMPTY instance with
    // registration CLOSED: that is the bootstrap exception — the first account
    // is always allowed AND verified on the spot (auth.rs), so it can sign in
    // immediately. With registration OPEN the account would be created but
    // unverified, and login would refuse it. Hence: a fresh database, and do
    // NOT set MICA_REGISTRATION_ENABLED.
    final login = await http.post(
      base.replace(path: '/api/auth/login'),
      headers: json(),
      body: jsonEncode({
        'email': 'sync$stamp@test.dev',
        'password': 'orbital-kettle-2026',
      }),
    );
    expect(login.statusCode, inInclusiveRange(200, 299), reason: login.body);
    final token = (jsonDecode(login.body) as Map)['access_token'] as String;

    // Workspace + document.
    final ws = await http.post(
      base.replace(path: '/api/workspaces'),
      headers: json(token),
      body: jsonEncode({'name': 'SyncWS'}),
    );
    expect(ws.statusCode, inInclusiveRange(200, 299), reason: ws.body);
    final wsId = ((jsonDecode(ws.body) as Map)['workspace'] as Map)['id'] as String;

    final docResp = await http.post(
      base.replace(path: '/api/workspaces/$wsId/documents'),
      headers: json(token),
      body: jsonEncode({'name': 'SyncDoc'}),
    );
    expect(docResp.statusCode, inInclusiveRange(200, 299), reason: docResp.body);
    final docId = ((jsonDecode(docResp.body) as Map)['document'] as Map)['id'] as String;

    Uri sockUri() => base.replace(
      scheme: base.scheme == 'https' ? 'wss' : 'ws',
      path: '/ws/workspaces/$wsId/documents/$docId',
      queryParameters: {'token': token},
    );

    // Two devices (distinct yrs client ids).
    final readyA = Completer<void>();
    final readyB = Completer<void>();
    final a = CloudSyncSession(
      uri: () async => sockUri(),
      clientId: BigInt.from(111),
      onReady: (_, _) => readyA.isCompleted ? null : readyA.complete(),
      onRemoteBlocks: (_) {},
    );
    final b = CloudSyncSession(
      uri: () async => sockUri(),
      clientId: BigInt.from(222),
      onReady: (_, _) => readyB.isCompleted ? null : readyB.complete(),
      onRemoteBlocks: (_) {},
    );
    a.connect();
    b.connect();
    await readyA.future.timeout(const Duration(seconds: 10));
    await readyB.future.timeout(const Duration(seconds: 10));

    // A inserts a paragraph → B should see it.
    a.applyLocalOps([
      {
        'type': 'insert_block',
        'parent_id': a.rootBlockId,
        'index': 0,
        'block': {
          'id': 'p1',
          'type': 'paragraph',
          'text': 'Hello from A',
          'data': <String, dynamic>{},
          'children': <String>[],
        },
      },
    ]);
    await _waitFor(() => b.childBlocks().any(
      (x) => x['id'] == 'p1' && x['text'] == 'Hello from A',
    ));

    // B inserts another → A should see it.
    b.applyLocalOps([
      {
        'type': 'insert_block',
        'parent_id': b.rootBlockId,
        'index': 1,
        'block': {
          'id': 'p2',
          'type': 'paragraph',
          'text': 'Hello from B',
          'data': <String, dynamic>{},
          'children': <String>[],
        },
      },
    ]);
    await _waitFor(() => a.childBlocks().any(
      (x) => x['id'] == 'p2' && x['text'] == 'Hello from B',
    ));

    // Both replicas converged to the same block list.
    Set<String> ids(CloudSyncSession s) =>
        s.childBlocks().map((x) => x['id'] as String).toSet();
    await _waitFor(() => ids(a).containsAll({'p1', 'p2'}) && ids(b).containsAll({'p1', 'p2'}));
    expect(
      a.childBlocks().map((x) => x['id']).toList(),
      b.childBlocks().map((x) => x['id']).toList(),
      reason: 'converged block order matches',
    );

    a.dispose();
    b.dispose();
  });
}

Future<void> _waitFor(
  bool Function() condition, {
  Duration timeout = const Duration(seconds: 10),
}) async {
  final deadline = DateTime.now().add(timeout);
  while (!condition()) {
    if (DateTime.now().isAfter(deadline)) fail('condition not met within $timeout');
    await Future<void>.delayed(const Duration(milliseconds: 50));
  }
}
