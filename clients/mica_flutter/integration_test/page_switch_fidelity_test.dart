// A1 (M-R): the page-switch data-loss regression, end-to-end against a live
// stack. Reproduces the cycle that lost content this week: edit doc A → switch
// away (drain + dispose, as _closeDocumentSync now does) → edit doc B → reopen
// A. The content written to A must survive — it was folded on the server and a
// fresh session reads it back. Guards "切页内容丢失" (unpushed edits dropped on
// teardown) at the session/server layer.
//
// Requires the dev stack up: `docker compose up -d postgres rustfs api`
// (api on 127.0.0.1:8080; the demo account auto-registers on first login).
//
//   flutter test integration_test/page_switch_fidelity_test.dart -d windows
import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:integration_test/integration_test.dart';
import 'package:mica_flutter/cloud/cloud_sync.dart';
import 'package:mica_flutter/src/rust/frb_generated.dart';

const _base = 'http://127.0.0.1:8080';
const _email = 'demo@mica.dev';
const _password = 'password123';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(() async => RustLib.init());
  tearDownAll(() async => RustLib.dispose());

  Future<http.Response> post(String path, Map<String, dynamic> body,
          [String? token]) =>
      http.post(
        Uri.parse('$_base$path'),
        headers: {
          'content-type': 'application/json',
          if (token != null) 'authorization': 'Bearer $token',
        },
        body: jsonEncode(body),
      );

  Future<String> login() async {
    var r =
        await post('/api/auth/login', {'email': _email, 'password': _password});
    if (r.statusCode != 200) {
      r = await post('/api/auth/register',
          {'email': _email, 'display_name': 'Demo', 'password': _password});
    }
    return (jsonDecode(r.body) as Map)['access_token'] as String;
  }

  Future<Map> postJson(
      String path, String token, Map<String, dynamic> body) async {
    final r = await post(path, body, token);
    expect(r.statusCode, inInclusiveRange(200, 299),
        reason: 'POST $path → ${r.body}');
    return jsonDecode(r.body) as Map;
  }

  Uri wsUri(String token, String ws, String doc) => Uri.parse(
        '${_base.replaceFirst('http', 'ws')}'
        '/ws/workspaces/$ws/documents/$doc?token=$token',
      );

  Future<CloudSyncSession> openReady(
      String token, String ws, String doc, BigInt clientId) async {
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

  Map<String, dynamic> insertParagraph(String rootId, String id, String text) =>
      {
        'type': 'insert_block',
        'parent_id': rootId,
        'index': 0,
        'block': {
          'id': id,
          'type': 'paragraph',
          'text': text,
          'data': <String, dynamic>{},
          'children': <String>[],
        },
      };

  Future<bool> waitForText(CloudSyncSession s, String text) async {
    for (var i = 0; i < 60; i++) {
      if (s.allBlocks().any((b) => b['text'] == text)) return true;
      await Future<void>.delayed(const Duration(milliseconds: 50));
    }
    return false;
  }

  test('content written to a page survives switching away and back', () async {
    final token = await login();
    final ws = ((await postJson('/api/workspaces', token, {'name': 'switch-e2e'}))
        ['workspace'] as Map)['id'] as String;
    final docA = ((await postJson(
            '/api/workspaces/$ws/documents', token, {'name': 'A'}))['document']
        as Map)['id'] as String;
    final docB = ((await postJson(
            '/api/workspaces/$ws/documents', token, {'name': 'B'}))['document']
        as Map)['id'] as String;

    // One stable device id across the whole cycle — the real switch scenario is
    // the SAME device moving between documents.
    final device = BigInt.from(0xA1CE);

    // ── Edit A, then "switch away": drain (C2) + dispose ──────────────────────
    final a1 = await openReady(token, ws, docA, device);
    a1.applyLocalOps(
        [insertParagraph(a1.rootBlockId, 'pA', 'test1 content')]);
    expect(await a1.drainOutbox(), isTrue,
        reason: 'A edits fold on the server before the switch');
    a1.dispose();

    // ── Switch to B and edit it (a real page switch in between) ───────────────
    final b1 = await openReady(token, ws, docB, device);
    b1.applyLocalOps(
        [insertParagraph(b1.rootBlockId, 'pB', 'doc B content')]);
    expect(await b1.drainOutbox(), isTrue);
    b1.dispose();

    // ── Switch back to A: a fresh session must still see A's content ───────────
    final a2 = await openReady(token, ws, docA, device);
    expect(await waitForText(a2, 'test1 content'), isTrue,
        reason: 'content written to A was lost by switching away and back');
    // And B's content did not leak into A.
    expect(a2.allBlocks().any((b) => b['text'] == 'doc B content'), isFalse,
        reason: 'B content stays in B');
    a2.dispose();
  });
}
