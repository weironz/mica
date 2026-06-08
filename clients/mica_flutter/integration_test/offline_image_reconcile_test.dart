// P2 §7 upstream differ: the offline-insert image reconcile, end-to-end against a
// live stack.
//
// Exercises the net-new path: an image inserted while "offline" carries a sha256
// CAS placeholder file_id; on reconnect the bytes upload (real presign → PUT →
// complete), and every block referencing that sha256 is rewritten to the cloud
// UUID through the REAL [CloudSyncSession.applyLocalOps] → `sync.push` → server
// fold — proven by reading it back through a SECOND session. The pure rewrite
// builder is unit-tested in test/pending_uploads_test.dart; this proves it folds
// on the server and converges to other clients.
//
// Requires the dev stack up: `docker compose up -d postgres rustfs api` (api on
// 127.0.0.1:8080) and the demo account (auto-registered on first login attempt).
//
//   flutter test integration_test/offline_image_reconcile_test.dart -d windows
import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:integration_test/integration_test.dart';
import 'package:mica_flutter/cloud/cloud_sync.dart';
import 'package:mica_flutter/cloud/pending_uploads.dart';
import 'package:mica_flutter/src/rust/frb_generated.dart';
import 'package:mica_flutter/upload/sha256.dart';

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
    var r = await post('/api/auth/login', {'email': _email, 'password': _password});
    if (r.statusCode != 200) {
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

  /// Upload bytes the way the client does (presign → PUT → complete), returning
  /// the cloud file id (a UUID). Mirrors ApiClient.uploadImage.
  Future<String> uploadImage(
    String token,
    String ws,
    String fileName,
    Uint8List bytes,
  ) async {
    final presign = await postJson<Map>('/api/workspaces/$ws/files/presign', token, {
      'file_name': fileName,
      'mime_type': 'image/png',
      'byte_size': bytes.length,
      'content_hash': sha256Hex(bytes),
    });
    final put = await http.put(
      Uri.parse(presign['upload_url'] as String),
      headers: {'content-type': 'image/png'},
      body: bytes,
    );
    expect(put.statusCode, inInclusiveRange(200, 299), reason: 'PUT blob → ${put.statusCode}');
    final complete = await postJson<Map>('/api/workspaces/$ws/files/complete', token, {
      'object_key': presign['object_key'],
      'file_name': fileName,
      'mime_type': 'image/png',
      'byte_size': bytes.length,
    });
    return (complete['file'] as Map)['id'] as String;
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

  test('offline image (sha256 placeholder) reconciles to a cloud UUID, folds on '
      'the server, and reads back on a second client', () async {
    final token = await login();
    final ws = ((await postJson<Map>('/api/workspaces', token, {'name': 'offimg-e2e'}))
        ['workspace'] as Map)['id'] as String;
    final docId = ((await postJson<Map>(
      '/api/workspaces/$ws/documents',
      token,
      {'name': 'page'},
    ))['document'] as Map)['id'] as String;

    // Distinct bytes so the content hash is unique to this run.
    final bytes = Uint8List.fromList(
      utf8.encode('mica-offline-image-${DateTime.now().microsecondsSinceEpoch}'),
    );
    final sha = sha256Hex(bytes); // the offline placeholder file_id

    // ── Session 1: insert the image with the sha256 placeholder (as the offline
    //    editor would), drain it to the server. ────────────────────────────────
    final s1 = await openReady(token, ws, docId, BigInt.from(0x1111));
    s1.applyLocalOps([
      {
        'type': 'insert_block',
        'parent_id': s1.rootBlockId,
        'index': 0,
        'block': {
          'id': 'img',
          'type': 'image',
          'text': '',
          'data': {'file_id': sha, 'name': 'pic.png'},
          'children': <String>[],
        },
      },
    ]);
    await s1.drainOutbox();

    // Sanity: the placeholder really is sha256-shaped (a local CAS id), not a UUID.
    expect(RegExp(r'^[0-9a-f]{64}$').hasMatch(sha), isTrue);

    // ── Reconnect reconcile: upload the bytes, rewrite sha256 → UUID on the live
    //    replica via the same content-addressed builder main.dart uses. ─────────
    final uuid = await uploadImage(token, ws, 'pic.png', bytes);
    final ops = buildImageIdRewriteOps(
      blocks: s1.allBlocks(),
      fromId: sha,
      toId: uuid,
    );
    expect(ops, isNotEmpty, reason: 'the placeholder block should be found for rewrite');
    s1.applyLocalOps(ops);
    await s1.drainOutbox();
    s1.dispose();

    // ── Session 2: a fresh client must see the reconciled file_id. ─────────────
    final s2 = await openReady(token, ws, docId, BigInt.from(0x2222));
    Map<String, dynamic>? img;
    for (var i = 0; i < 60; i++) {
      for (final b in s2.allBlocks()) {
        if (b['id'] == 'img') {
          img = b;
          break;
        }
      }
      if (img != null && (img['data'] as Map)['file_id'] == uuid) break;
      img = null;
      await Future<void>.delayed(const Duration(milliseconds: 50));
    }

    expect(img, isNotNull, reason: 'the image block should sync to the second client');
    final fileId = (img!['data'] as Map)['file_id'] as String;
    expect(fileId, uuid, reason: 'file_id should be reconciled sha256 → cloud UUID');
    expect(fileId, isNot(sha), reason: 'the sha256 placeholder must be gone');
    expect((img['data'] as Map)['name'], 'pic.png'); // name preserved

    s2.dispose();
  });
}
