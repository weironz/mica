// P2 §6: the local→cloud migration sync path, end-to-end against a live stack.
//
// Exercises the load-bearing, net-new path: replaying a local block tree as ops
// onto a freshly-created cloud doc's root (strategy (c)) through the REAL
// [CloudSyncSession.applyLocalOps] → `sync.push` → server fold, then reading it
// back through a SECOND session. Proves the migrated tree (image file_id
// reconciled sha256→UUID, nesting preserved, local root not duplicated) actually
// persists on the server and converges to other clients — not just that the pure
// op builder is correct (that's unit-tested in test/workspace_migration_test.dart).
//
// Requires the dev stack up: `docker compose up -d postgres rustfs api` (api on
// 127.0.0.1:8080) and the demo account (auto-registered on first login attempt).
//
//   flutter test integration_test/migration_sync_test.dart -d windows
import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:integration_test/integration_test.dart';
import 'package:mica_flutter/cloud/cloud_sync.dart';
import 'package:mica_flutter/cloud/workspace_migration.dart';
import 'package:mica_flutter/src/rust/frb_generated.dart';

const _base = 'http://127.0.0.1:8080';
const _email = 'demo@mica.dev';
const _password = 'password123';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(() async => RustLib.init());
  tearDownAll(() async => RustLib.dispose());

  Future<String> login() async {
    Future<http.Response> post(String path, Map<String, dynamic> body) =>
        http.post(
          Uri.parse('$_base$path'),
          headers: {'content-type': 'application/json'},
          body: jsonEncode(body),
        );
    var r = await post('/api/auth/login', {
      'email': _email,
      'password': _password,
    });
    if (r.statusCode != 200) {
      // First run: register the demo account, then it's logged in.
      r = await post('/api/auth/register', {
        'email': _email,
        'display_name': 'Demo',
        'password': _password,
      });
    }
    return (jsonDecode(r.body) as Map)['access_token'] as String;
  }

  Future<T> postJson<T>(String path, String token, Map<String, dynamic> body) async {
    final r = await http.post(
      Uri.parse('$_base$path'),
      headers: {'content-type': 'application/json', 'authorization': 'Bearer $token'},
      body: jsonEncode(body),
    );
    expect(r.statusCode, inInclusiveRange(200, 299), reason: 'POST $path → ${r.body}');
    return jsonDecode(r.body) as T;
  }

  Uri wsUri(String token, String ws, String doc) => Uri.parse(
        '${_base.replaceFirst('http', 'ws')}'
        '/ws/workspaces/$ws/documents/$doc?token=$token',
      );

  Future<CloudSyncSession> openReady(
    String token,
    String ws,
    String doc,
    BigInt clientId,
  ) async {
    final s = CloudSyncSession(
      uri: wsUri(token, ws, doc),
      clientId: clientId,
      onReady: (_, _) {},
      onRemoteBlocks: (_) {},
    );
    s.connect();
    await s.ready.timeout(const Duration(seconds: 20));
    return s;
  }

  test('migration ops replay onto the cloud root, fold on the server, '
      'and read back on a second client', () async {
    final token = await login();
    final ws = ((await postJson<Map>('/api/workspaces', token, {'name': 'mig-e2e'}))
        ['workspace'] as Map)['id'] as String;
    final docId = ((await postJson<Map>(
      '/api/workspaces/$ws/documents',
      token,
      {'name': 'page'},
    ))['document'] as Map)['id'] as String;

    // A local doc: page "Title" → [ paragraph "hello", image(file_id=sha) ].
    final sha = 'b' * 64;
    const cloudUuid = 'mig-uuid-1234';
    final localBlocks = <Map<String, dynamic>>[
      {'id': 'lroot', 'type': 'page', 'text': 'Title', 'data': <String, dynamic>{}, 'children': ['p1', 'img']},
      {'id': 'p1', 'type': 'paragraph', 'text': 'hello', 'data': <String, dynamic>{}, 'children': <String>[]},
      {'id': 'img', 'type': 'image', 'text': '', 'data': {'file_id': sha, 'name': 'pic.png'}, 'children': <String>[]},
    ];

    // ── Session 1: replay the local tree onto the cloud root, drain, dispose ──
    final s1 = await openReady(token, ws, docId, BigInt.from(0x1111));
    final ops = buildMigrationOps(
      blocks: localBlocks,
      localRootId: 'lroot',
      cloudRootId: s1.rootBlockId,
      idMap: {sha: cloudUuid},
    );
    s1.applyLocalOps(ops);
    await s1.drainOutbox();
    s1.dispose();

    // ── Session 2: a fresh client must see the folded, migrated tree ──
    final s2 = await openReady(token, ws, docId, BigInt.from(0x2222));
    // Give the catch-up pull a moment to apply.
    for (var i = 0; i < 40 && s2.allBlocks().length < 3; i++) {
      await Future<void>.delayed(const Duration(milliseconds: 50));
    }
    final blocks = {for (final b in s2.allBlocks()) b['id'] as String: b};
    final root = s2.rootBlockId;

    // The local root was NOT duplicated; its content rode onto the cloud root.
    expect(blocks.containsKey('lroot'), isFalse);
    expect(blocks[root]?['type'], 'page');
    expect(blocks[root]?['text'], 'Title');
    expect((blocks[root]?['children'] as List).cast<String>(), ['p1', 'img']);

    // The paragraph migrated.
    expect(blocks['p1']?['text'], 'hello');

    // The image file_id was reconciled sha256 → cloud UUID, name preserved.
    expect(blocks['img']?['data']['file_id'], cloudUuid);
    expect(blocks['img']?['data']['name'], 'pic.png');

    s2.dispose();
  });
}
