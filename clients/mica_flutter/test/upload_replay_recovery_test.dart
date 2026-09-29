import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/client.dart';
import 'package:mica_flutter/api/models.dart';

void main() {
  test(
    'a conditional PUT 412 retries complete, while another failure does not',
    () async {
      final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
      var putStatus = 412;
      var completed = 0;
      String? ifNoneMatch;
      String? checksum;
      final origin = 'http://127.0.0.1:${server.port}';

      server.listen((request) async {
        await request.drain<void>();
        request.response.headers.contentType = ContentType.json;
        if (request.uri.path.endsWith('/files/presign')) {
          request.response.write(
            jsonEncode({
              'object_key': 'workspaces/test/hash.png',
              'upload': {
                'upload_url': '$origin/blob',
                'if_none_match': '*',
                'checksum_sha256': 'signed-checksum',
              },
            }),
          );
        } else if (request.uri.path == '/blob') {
          ifNoneMatch = request.headers.value('if-none-match');
          checksum = request.headers.value('x-amz-checksum-sha256');
          request.response.statusCode = putStatus;
        } else if (request.uri.path.endsWith('/files/complete')) {
          completed++;
          request.response.write(
            jsonEncode({
              'file': {
                'id': '11111111-1111-1111-1111-111111111111',
                'original_name': 'photo.png',
                'mime_type': 'image/png',
              },
              'download_url': '$origin/blob',
            }),
          );
        } else {
          request.response.statusCode = 404;
        }
        await request.response.close();
      });

      try {
        final api = ApiClient()..baseUri = Uri.parse(origin);
        final bytes = Uint8List.fromList([1, 2, 3]);
        final file = await api.uploadImage(
          'token',
          'workspace',
          fileName: 'photo.png',
          mimeType: 'image/png',
          bytes: bytes,
        );
        expect(file.id, '11111111-1111-1111-1111-111111111111');
        expect(
          completed,
          1,
          reason: '412 means an earlier PUT may have landed',
        );
        expect(ifNoneMatch, '*');
        expect(checksum, 'signed-checksum');

        putStatus = 403;
        await expectLater(
          api.uploadImage(
            'token',
            'workspace',
            fileName: 'photo.png',
            mimeType: 'image/png',
            bytes: bytes,
          ),
          throwsA(isA<ApiException>()),
        );
        expect(
          completed,
          1,
          reason: '403 must not be treated as an uploaded object',
        );
      } finally {
        await server.close(force: true);
      }
    },
  );
}
