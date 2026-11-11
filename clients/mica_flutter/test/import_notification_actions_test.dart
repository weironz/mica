import 'dart:convert';
import 'dart:io';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/client.dart';
import 'package:mica_flutter/session/import_notification_actions.dart';

void main() {
  test('the route keeps its server while using refreshed account credentials', () async {
    final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    addTearDown(() => server.close(force: true));
    String? authorization;
    final served = server.first.then((request) async {
      authorization = request.headers.value(HttpHeaders.authorizationHeader);
      request.response.headers.contentType = ContentType.json;
      request.response.write(jsonEncode({'enabled': true}));
      await request.response.close();
    });
    final api = ApiClient()..baseUri = Uri.parse('http://127.0.0.1:${server.port}');
    var token = 'before-refresh';
    final actions = importNotificationActions(api: api, accessToken: () => token, isCurrent: () => true);
    api.baseUri = Uri.parse('http://127.0.0.1:1');
    token = 'after-refresh';
    expect(await actions.load(), isTrue);
    await served;
    expect(authorization, 'Bearer after-refresh');
  });

  test('retired settings cannot submit another account preference', () async {
    final actions = importNotificationActions(
      api: ApiClient()..baseUri = Uri.parse('http://127.0.0.1:1'),
      accessToken: () => throw StateError('must not read the next account token'),
      isCurrent: () => false,
    );
    await expectLater(actions.save(false), throwsStateError);
  });
}
