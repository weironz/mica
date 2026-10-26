import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/client.dart';
import 'package:mica_flutter/api/models.dart';

void main() {
  test('AI socket closing without business done fails after its deltas', () async {
    final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    server.listen((request) async {
      final socket = await WebSocketTransformer.upgrade(request);
      await socket.first;
      socket.add(jsonEncode({'type': 'delta', 'text': 'Partial result'}));
      await socket.close();
    });
    try {
      final api = ApiClient()
        ..baseUri = Uri.parse('http://127.0.0.1:${server.port}');
      await expectLater(
        api.aiStream('token', 'rewrite'),
        emitsInOrder([
          'Partial result',
          emitsError(isA<ApiException>()),
          emitsDone,
        ]),
      );
    } finally {
      await server.close(force: true);
    }
  });

  for (final fails in [false, true]) {
    test('AI business terminal frame preserves success=$fails', () async {
      final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
      server.listen((request) async {
        final socket = await WebSocketTransformer.upgrade(request);
        await socket.first;
        socket.add(jsonEncode({'type': 'delta', 'text': 'Partial result'}));
        if (fails) {
          socket.add(jsonEncode({'type': 'error', 'message': 'provider failed'}));
        }
        socket.add(jsonEncode({'type': 'done'}));
        await socket.close();
      });
      try {
        final api = ApiClient()
          ..baseUri = Uri.parse('http://127.0.0.1:${server.port}');
        await expectLater(
          api.aiStream('token', 'rewrite'),
          emitsInOrder([
            'Partial result',
            if (fails)
              emitsError(isA<ApiException>().having(
                (error) => error.toString(), 'message', contains('provider failed'),
              )),
            emitsDone,
          ]),
        );
      } finally {
        await server.close(force: true);
      }
    });
  }
}
