// 重连必须重新取 URI —— 因为 URI 里带着会过期的 access token。
//
// 这个 bug 的形状:token 在会话创建时就烤进了 `uri`,而 access TTL 是 1 小时、
// 文档会话却能开一整天。刷新只由用户操作(`_run()`)驱动,没有定时器,所以闲置
// 超过 TTL 后 socket 一断,每次重连都在重放一个死 token:401 → 退避 → 再来,
// 永远。界面上什么都不说,只是"同步悄悄停了",直到切换文档(新会话=新 token)
// 才恢复 —— 最难查的那类故障。
//
// 所以这里钉的不是"能连上",而是**每次连接尝试都重新问一次 URI**。
@TestOn('vm')
library;

import 'dart:async';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/cloud/cloud_sync_session.dart';

void main() {
  /// 一个没人监听的端口:先绑定拿一个空闲的,再放掉。
  Future<int> deadPort() async {
    final probe = await ServerSocket.bind(InternetAddress.loopbackIPv4, 0);
    final port = probe.port;
    await probe.close();
    return port;
  }

  test('每次连接尝试都重新构建 URI,而不是复用构造时的那一个', () async {
    final port = await deadPort();
    // 每次被问就发一个新 token —— 模拟"刷新后拿到的新凭证"。
    var issued = 0;
    final handedOut = <String>[];

    await runZonedGuarded(() async {
      final session = CloudSyncSession(
        uri: () async {
          issued++;
          final token = 'token-$issued';
          handedOut.add(token);
          return Uri.parse('ws://127.0.0.1:$port/ws/doc?token=$token');
        },
        clientId: BigInt.one,
        onReady: (_, _) {},
        onRemoteBlocks: (_) {},
      );
      session.connect();
      // 连不上 → onDone → 退避重连 → 再次 connect()。轮询到第二次尝试发生为止,
      // 不用固定 sleep:死端口的失败要 1.4–3 秒才浮上来(实测),写死时长的话
      // 这个测试就变成一场计时竞赛,在慢一点的机器上会假红。
      final deadline = DateTime.now().add(const Duration(seconds: 15));
      while (issued < 2 && DateTime.now().isBefore(deadline)) {
        await Future<void>.delayed(const Duration(milliseconds: 100));
      }
      session.dispose();
    }, (_, _) {});

    expect(issued, greaterThan(1), reason: '只问一次 = token 被烤死,过期后永远连不上');
    expect(
      handedOut.toSet().length,
      handedOut.length,
      reason: '每次尝试都该拿到当时最新的那个 token',
    );
  });

  test('URI 构建失败当成「连不上」,不炸 zone', () async {
    // 续期失败 / 已登出:builder 抛异常。这必须表现为"离线"这个状态,
    // 而不是把未捕获异常丢进 zone —— crash.log 是留给真故障的。
    final uncaught = <Object>[];

    await runZonedGuarded(() async {
      final session = CloudSyncSession(
        uri: () async => throw StateError('signed out'),
        clientId: BigInt.one,
        onReady: (_, _) {},
        onRemoteBlocks: (_) {},
      );
      session.connect();
      await Future<void>.delayed(const Duration(milliseconds: 300));
      session.dispose();
    }, (error, _) => uncaught.add(error));

    expect(uncaught, isEmpty, reason: '登出不是崩溃 —— 它不该出现在 crash.log 里');
  });
}
