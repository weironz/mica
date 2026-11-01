// Browser harness hooks (web build only), exposed on `window` for playwright
// (`e2e/web_e2e.mjs`):
//   micaDocEngineInfo()                        the loaded document engine's
//     build info (JSON), or `{"error": …}` — proves the wasm core is live
//   micaWebSyncTest(apiUrl, email, password)   two web sessions editing one
//     document converge through the real server (Promise of JSON)
//
// The W1/W2 hooks that checked the old yjs engine against yrs went with yjs
// (2026-11-01): the web now runs the same Rust core as the server, and the
// stored-data side of that question is `crates/mica-core/tests/legacy_yjs_state.rs`.
import 'dart:async';
import 'dart:convert';
import 'dart:js_interop';

import 'package:http/http.dart' as http;

import '../api/doc_socket_uri.dart';
import '../cloud/cloud_sync.dart';
import 'mica_doc_wasm.dart';

export 'mica_doc_wasm.dart' show initDocEngine;

@JS('micaDocEngineInfo')
external set _engineInfo(JSFunction f);

@JS('micaWebSyncTest')
external set _webSyncTest(JSFunction f);

void registerWebProbes() {
  _engineInfo = (() {
    final error = docEngineError;
    return jsonEncode(error != null ? {'error': error} : docEngineInfo()).toJS;
  }).toJS;

  // Credentials come from the CALLER rather than being registered here:
  // registration is closed by default, and a fresh account must confirm its
  // address first. The server's `MICA_SEED_TEST_USER` upserts a known,
  // verified account for exactly this harness. No credential ships in the
  // bundle.
  _webSyncTest = ((JSString apiBase, JSString email, JSString password) {
    return _runWebSync(apiBase.toDart, email.toDart, password.toDart).toJS;
  }).toJS;
}

Future<JSString> _runWebSync(String apiBase, String email, String password) async {
  try {
    final base = Uri.parse(apiBase);
    Map<String, String> jh([String? t]) => {
      'content-type': 'application/json',
      if (t != null) 'authorization': 'Bearer $t',
    };
    final signIn = await http.post(
      base.replace(path: '/api/auth/login'),
      headers: jh(),
      body: jsonEncode({'email': email, 'password': password}),
    );
    if (signIn.statusCode != 200) {
      // Fail loudly with the server's own words. A harness that cannot sign in
      // must not look like a convergence failure — that would send whoever reads
      // the CI log hunting through the CRDT layer for a configuration problem.
      return jsonEncode({
        'ok': false,
        'error': 'sign-in failed (${signIn.statusCode}): ${signIn.body}',
      }).toJS;
    }
    final token = (jsonDecode(signIn.body) as Map)['access_token'] as String;
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

    // Same builder the app itself uses — it announces the sync protocol
    // version, which the server's floor (1 since op-model retirement)
    // requires: a versionless URL is refused with `client_too_old`.
    Uri sock() => documentSocketUri(base, wsId, docId, token);

    final readyA = Completer<void>();
    final readyB = Completer<void>();
    final a = CloudSyncSession(
      uri: () async => sock(),
      clientId: BigInt.zero,
      onReady: (_, _) => readyA.isCompleted ? null : readyA.complete(),
      onRemoteChange: () {},
    );
    final b = CloudSyncSession(
      uri: () async => sock(),
      clientId: BigInt.zero,
      onReady: (_, _) => readyB.isCompleted ? null : readyB.complete(),
      onRemoteChange: () {},
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
