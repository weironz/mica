import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/client.dart';
import 'package:mica_flutter/api/models.dart';

/// 上传被拒时,客户端必须拿到服务端的 machine code。
///
/// 这是一条**跨语言契约**的客户端半边:服务端把 `file_too_large` /
/// `workspace_quota_exceeded` 放进错误体的 `code`,客户端 `_decode` 取出来,
/// UI 再按 code 选文案。中间任何一环把 code 丢了,失败的表现不是报错,而是
/// **静默退回转述服务端英文** —— 正是这套 code 要消灭的那个结果,而且两边的
/// 编译器都看不见。
///
/// 服务端那半由 `the_size_refusals_serialize_the_codes_the_client_switches_on`
/// 钉住(断言序列化后的 JSON)。这里用一个真的 HttpServer 钉住客户端这半:
/// 真发请求、真解响应,而不是构造一个 ApiException 自己骗自己。
void main() {
  late HttpServer server;
  late ApiClient api;
  late String lastPath;

  /// 服务端下一次要回的 (状态码, 响应体)。
  late int status;
  late Object body;

  setUp(() async {
    server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    server.listen((req) async {
      lastPath = req.uri.path;
      req.response.statusCode = status;
      req.response.headers.contentType = ContentType.json;
      req.response.write(jsonEncode(body));
      await req.response.close();
    });
    api = ApiClient()..baseUri = Uri.parse('http://127.0.0.1:${server.port}');
  });

  tearDown(() async => server.close(force: true));

  Future<ApiException> upload() async {
    try {
      await api.uploadImage(
        'token',
        '11111111-1111-1111-1111-111111111111',
        fileName: 'big.png',
        mimeType: 'image/png',
        bytes: Uint8List.fromList(List.filled(4096, 7)),
      );
      fail('上传本该被拒');
    } on ApiException catch (e) {
      return e;
    }
  }

  test('单文件超限:code 一路传到 ApiException', () async {
    status = 400;
    body = {
      'code': 'file_too_large',
      'message':
          'bad request: file exceeds the maximum upload size of 1024 bytes',
    };

    final error = await upload();

    expect(lastPath, endsWith('/files/presign'), reason: '拒绝发生在 presign');
    expect(
      error.code,
      'file_too_large',
      reason: 'UI 按这个字符串选文案;丢了就只会转述英文原文',
    );
    expect(error.statusCode, 400);
    // message 仍是服务端那句英文 —— 它是给日志看的,不是给人看的。
    expect(error.message, contains('1024'));
  });

  test('工作区配额满:另一个 code,必须能和超限区分开', () async {
    status = 400;
    body = {
      'code': 'workspace_quota_exceeded',
      'message':
          'bad request: workspace storage is full: 500 of 600 bytes used',
    };

    final error = await upload();

    expect(error.code, 'workspace_quota_exceeded');
    expect(
      error.code,
      isNot('file_too_large'),
      reason: '两种「传不上去」必须分得开,否则文案又会混为一谈',
    );
  });

  test('没有 code 的旧式 400:回落成通用码,而不是假装认识它', () async {
    status = 400;
    body = {'message': 'bad request: something else'};

    final error = await upload();

    expect(error.code, isNull, reason: 'UI 会因此回落到原始 message,这是对的');
  });

  /// `max_upload_bytes` 是超限文案里那个具体数字的唯一来源:presign 只在**成功**时
  /// 回上限,而文件过大时那次 presign 恰恰是失败的。读丢了不会报错,只会让文案
  /// 悄悄退回不带数字的那句。
  group('/usage 带回单文件上限', () {
    test('三个字段都读到,上限和配额是两个独立的数', () async {
      status = 200;
      body = {
        'bytes_used': 3500,
        'quota_bytes': 5368709120,
        'max_upload_bytes': 26214400,
      };

      final usage = await api.workspaceUsage(
        'token',
        '11111111-1111-1111-1111-111111111111',
      );

      expect(usage.used, 3500);
      expect(usage.quota, 5368709120);
      expect(usage.maxUpload, 26214400);
      expect(
        usage.maxUpload,
        isNot(usage.quota),
        reason: '单文件上限不是配额,混起来文案就又分不清了',
      );
    });

    test('旧服务端没有这个字段:落 0(=不知道),不是抛错也不是「不许上传」', () async {
      status = 200;
      body = {'bytes_used': 0, 'quota_bytes': 0};

      final usage = await api.workspaceUsage(
        'token',
        '11111111-1111-1111-1111-111111111111',
      );

      expect(usage.maxUpload, 0, reason: '0 是「没有答案」,UI 据此回落到不带数字的文案');
    });
  });
}
