import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:mica_flutter/api/client.dart';

/// Issue #30: an admin's not-yet-saved provider key must travel in the body,
/// never the URL — query strings are written to the proxy's access log.
void main() {
  tearDown(resetHttpClientForTesting);

  test('listAiModels sends the key in a POST body, not the query', () async {
    late http.Request seen;
    useHttpClientForTesting(
      MockClient((request) async {
        seen = request;
        return http.Response(jsonEncode({'models': ['m1'], 'endpoint': 'x'}), 200);
      }),
    );
    final api = ApiClient()..baseUri = Uri.parse('http://test');
    final res = await api.listAiModels(
      'tok',
      provider: 'openai',
      baseUrl: 'https://api.example.com/v1',
      apiKey: 'sk-secret',
    );

    expect(res['models'], ['m1']);
    expect(seen.method, 'POST');
    expect(seen.url.path, '/api/ai/models');
    expect(seen.url.query, isEmpty);
    expect(seen.url.toString(), isNot(contains('sk-secret')));
    expect(jsonDecode(seen.body)['api_key'], 'sk-secret');
  });
}
