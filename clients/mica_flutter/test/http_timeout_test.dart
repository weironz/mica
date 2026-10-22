import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:mica_flutter/api/client.dart';

/// Issue #45: a server that never answers must surface as an error, not as a
/// request that waits forever — while a large upload is never cut off.
void main() {
  final silent = MockClient((_) => Completer<http.Response>().future);

  test('a small request that gets no response fails with ClientException', () async {
    final client = timedClientForTesting(silent, const Duration(milliseconds: 50));
    await expectLater(
      client.get(Uri.parse('http://test/api/workspaces')),
      throwsA(isA<http.ClientException>()),
    );
  });

  test('a large upload is not subject to the header deadline', () async {
    final slow = MockClient((_) async {
      await Future<void>.delayed(const Duration(milliseconds: 200));
      return http.Response('{}', 200);
    });
    final client = timedClientForTesting(slow, const Duration(milliseconds: 50));
    final res = await client.post(
      Uri.parse('http://test/api/workspaces/import'),
      body: List<int>.filled(2 * 1024 * 1024, 0),
    );
    expect(res.statusCode, 200);
  });
}
