// P2-M4 (web→yjs) verification hooks (web build only). Exposed on `window` so a
// browser harness (playwright) can drive them:
//   micaYjsSelfTest(b64)      W1: Dart-in-browser reads a real yrs base
//   micaYjsW2Test(b64)        W2: write side — apply ops+marks, return new state
//   micaYjsWebSyncTest(apiUrl) W4: two web sessions converge via the real server
import 'dart:async';
import 'dart:convert';
import 'dart:js_interop';

import 'package:http/http.dart' as http;

import '../cloud/cloud_sync.dart';
import 'mica_ydoc.dart';
import 'yjs_interop.dart';

@JS('micaYjsSelfTest')
external set _selfTest(JSFunction f);

@JS('micaYjsW2Test')
external set _w2Test(JSFunction f);

@JS('micaYjsWebSyncTest')
external set _webSyncTest(JSFunction f);

void registerYjsSelfTest() {
  if (!yjsAvailable) return;

  // W1: Dart-in-browser reads a real yrs-produced base.
  _selfTest = ((JSString b64) {
    try {
      final doc = MicaYDoc.fromState(base64.decode(b64.toDart));
      return jsonEncode({
        'ok': true,
        'root': doc.rootBlockId(),
        'blocks': doc.toBlocks(),
      }).toJS;
    } catch (e) {
      return jsonEncode({'ok': false, 'error': '$e'}).toJS;
    }
  }).toJS;

  // W2: write side — apply editor ops (incl. marks) onto a base, return the
  // re-encoded yjs state so the Rust side can confirm it reads web-written marks.
  _w2Test = ((JSString b64) {
    try {
      final doc = MicaYDoc.fromState(base64.decode(b64.toDart));
      final root = doc.rootBlockId();
      final blocks = doc.toBlocks();
      final target = blocks.firstWhere(
        (b) => (b['text'] as String).length >= 5,
        orElse: () =>
            blocks.firstWhere((b) => b['id'] != root, orElse: () => blocks.first),
      );
      doc.applyOp({
        'type': 'update_block',
        'block_id': target['id'],
        'text': target['text'],
        'data': {
          ...(target['data'] as Map).cast<String, dynamic>(),
          'marks': [
            {'start': 0, 'end': 5, 'type': 'bold'},
          ],
        },
      });
      doc.applyOp({
        'type': 'insert_block',
        'parent_id': root,
        'index': 0,
        'block': {
          'id': 'w2new',
          'type': 'paragraph',
          'text': 'hello link',
          'data': {
            'marks': [
              {'start': 0, 'end': 5, 'type': 'link', 'href': 'http://x', 'title': 'T'},
            ],
          },
          'children': <String>[],
        },
      });
      return jsonEncode({
        'ok': true,
        'state': base64.encode(doc.encodeState()),
        'blocks': doc.toBlocks(),
      }).toJS;
    } catch (e) {
      return jsonEncode({'ok': false, 'error': '$e'}).toJS;
    }
  }).toJS;

  // W4: two web sessions editing the same cloud doc converge through the real
  // server (mirrors the desktop cloud_sync_test, in-browser). Returns a Promise.
  _webSyncTest = ((JSString apiBase) {
    return _runWebSync(apiBase.toDart).toJS;
  }).toJS;
}

Future<JSString> _runWebSync(String apiBase) async {
  try {
    final base = Uri.parse(apiBase);
    Map<String, String> jh([String? t]) => {
      'content-type': 'application/json',
      if (t != null) 'authorization': 'Bearer $t',
    };
    final stamp = DateTime.now().microsecondsSinceEpoch;
    final reg = await http.post(
      base.replace(path: '/api/auth/register'),
      headers: jh(),
      body: jsonEncode({
        'email': 'web$stamp@test.dev',
        'display_name': 'Web',
        'password': 'password123',
      }),
    );
    final token = (jsonDecode(reg.body) as Map)['access_token'] as String;
    final ws = await http.post(
      base.replace(path: '/api/workspaces'),
      headers: jh(token),
      body: jsonEncode({'name': 'WebWS'}),
    );
    final wsId = ((jsonDecode(ws.body) as Map)['workspace'] as Map)['id'] as String;
    final docR = await http.post(
      base.replace(path: '/api/workspaces/$wsId/documents'),
      headers: jh(token),
      body: jsonEncode({'name': 'WebDoc'}),
    );
    final docId =
        ((jsonDecode(docR.body) as Map)['document'] as Map)['id'] as String;

    Uri sock() => base.replace(
      scheme: base.scheme == 'https' ? 'wss' : 'ws',
      path: '/ws/workspaces/$wsId/documents/$docId',
      queryParameters: {'token': token},
    );

    final readyA = Completer<void>();
    final readyB = Completer<void>();
    final a = CloudSyncSession(
      uri: sock(),
      clientId: BigInt.zero,
      onReady: (_, _) => readyA.isCompleted ? null : readyA.complete(),
      onRemoteBlocks: (_) {},
    );
    final b = CloudSyncSession(
      uri: sock(),
      clientId: BigInt.zero,
      onReady: (_, _) => readyB.isCompleted ? null : readyB.complete(),
      onRemoteBlocks: (_) {},
    );
    a.connect();
    b.connect();
    await readyA.future.timeout(const Duration(seconds: 10));
    await readyB.future.timeout(const Duration(seconds: 10));

    a.applyLocalOps([
      {
        'type': 'insert_block',
        'parent_id': a.rootBlockId,
        'index': 0,
        'block': {
          'id': 'wp1',
          'type': 'paragraph',
          'text': 'web hello',
          'data': <String, dynamic>{},
          'children': <String>[],
        },
      },
    ]);

    final deadline = DateTime.now().add(const Duration(seconds: 10));
    bool bHas() =>
        b.childBlocks().any((x) => x['id'] == 'wp1' && x['text'] == 'web hello');
    while (!bHas()) {
      if (DateTime.now().isAfter(deadline)) throw StateError('B did not converge');
      await Future<void>.delayed(const Duration(milliseconds: 50));
    }
    final result = jsonEncode({
      'ok': true,
      'docId': docId,
      'bBlocks': b.childBlocks().map((x) => x['id']).toList(),
    });
    a.dispose();
    b.dispose();
    return result.toJS;
  } catch (e) {
    return jsonEncode({'ok': false, 'error': '$e'}).toJS;
  }
}
