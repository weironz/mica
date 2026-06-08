// P2 awareness verification helper: connect to a document room as a second
// collaborator and broadcast a caret at a given block, so the running app shows
// a remote cursor. NOT shipped — a dev/test tool.
//
//   dart run tool/fake_collab.dart <wsId> <docId> <blockId> <offset> [apiBase]
import 'dart:async';
import 'dart:convert';

import 'package:http/http.dart' as http;
import 'package:web_socket_channel/web_socket_channel.dart';

Future<void> main(List<String> args) async {
  final wsId = args[0];
  final docId = args[1];
  final blockId = args[2];
  final offset = int.parse(args[3]);
  final base = Uri.parse(args.length > 4 ? args[4] : 'http://127.0.0.1:8090');

  final login = await http.post(
    base.replace(path: '/api/auth/login'),
    headers: {'content-type': 'application/json'},
    body: jsonEncode({'email': 'demo@mica.dev', 'password': 'password123'}),
  );
  final token = (jsonDecode(login.body) as Map)['access_token'] as String;

  final sock = base.replace(
    scheme: base.scheme == 'https' ? 'wss' : 'ws',
    path: '/ws/workspaces/$wsId/documents/$docId',
    queryParameters: {'token': token},
  );
  final ch = WebSocketChannel.connect(sock);
  ch.stream.listen((raw) {
    final m = jsonDecode(raw as String) as Map;
    if (m['type'] == 'document.bootstrap') {
      ch.sink.add(jsonEncode({
        'type': 'presence.update',
        'payload': {
          'name': 'Bot',
          'cursor': {'block': blockId, 'offset': offset},
        },
      }));
      // ignore: avoid_print
      print('Bot connected; cursor at $blockId:$offset');
    }
  });
  await Future<void>.delayed(const Duration(seconds: 180));
}
