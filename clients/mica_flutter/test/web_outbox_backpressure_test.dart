// 长离线重连不该是一场推送风暴 —— web 侧那条路以前就是。
//
// 桌面有本地 store,走 append-log,`_pushWindow = 64` 早就把重连开了窗;web 没有
// 本地 store,走的是内存 outbox,`resendAll` 一个同步 for 循环把整条队列灌进
// socket。服务端每条 push 都要**全档解码 + 重编码 + 重写整行**(见 roadmap
// 「每次 push 重建+重编码+重写整档」),所以队列有多长,那一瞬间的服务端开销就是
// 队列长度 × 文档大小。攒了几百条的离线用户,一重连就是他自己制造的一次雪崩。
//
// 这里钉两件事:**节奏**(纯函数,因为真正的发送路径要先 bootstrap,而 bootstrap
// 要 CRDT 引擎 + 活服务端 —— 那是 integration_test/cloud_sync_* 的地盘,CI 里排除),
// 以及**窗口靠什么腾出来**(ack 出队、拒绝让位)。
@TestOn('vm')
library;

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/cloud/cloud_sync_session.dart';

/// 一个最小的 outbox 条目替身:pushSlice 只认这两个谓词。
class _Entry {
  _Entry(this.id);
  final String id;
  bool sent = false;
  bool rejected = false;
}

List<_Entry> _queue(int n) => [for (var i = 0; i < n; i++) _Entry('$i')];

List<_Entry> _slice(List<_Entry> q, {int window = 64}) => pushSlice(
  q,
  alreadySent: (e) => e.sent,
  occupiesSlot: (e) => e.sent && !e.rejected,
  window: window,
);

void main() {
  _mergeRunTests();

  group('pushSlice 是那道闸', () {
    test('积压再长,一次也只放一个窗口出去', () {
      final q = _queue(500);
      final out = _slice(q);
      expect(out.length, 64);
      expect(out.first.id, '0', reason: '顺序不能乱 —— CRDT 收敛不要求顺序,但重放要求');
      expect(out.last.id, '63');
    });

    test('日常打字碰不到闸', () {
      expect(_slice(_queue(3)).length, 3);
    });

    test('在飞的占着窗口,不会二次发送', () {
      final q = _queue(100);
      for (final e in q.take(64)) {
        e.sent = true;
      }
      expect(_slice(q), isEmpty, reason: '窗口已满,尾巴要等 ack');

      // 一条 ack 掉了 → 队列里少一条 → 正好放行一条。
      q.removeAt(0);
      final next = _slice(q);
      expect(next.length, 1);
      expect(next.single.id, '64');
    });

    /// 被拒绝的 push 永远等不到 ack。如果它一直算在飞,攒够一批就把窗口焊死,
    /// 后面的积压再也发不出去 —— 而这条路的重试本来就在重连,不在这里。
    test('被拒绝的那条让出窗口,但不会被立刻重发', () {
      final q = _queue(100);
      for (final e in q.take(64)) {
        e.sent = true;
      }
      q.first.rejected = true;

      final next = _slice(q);
      expect(next.length, 1, reason: '腾出一个位置');
      expect(next.single.id, '64', reason: '让位不等于重发 —— 重发在重连');
    });
  });

  group('会话把窗口腾出来的两条路', () {
    Future<int> deadPort() async {
      final probe = await ServerSocket.bind(InternetAddress.loopbackIPv4, 0);
      final port = probe.port;
      await probe.close();
      return port;
    }

    /// 用 `restoreUnacked` 播种(那正是崩溃恢复走的入口),连一个没人听的端口:
    /// 队列建起来了,但一帧也发不出去 —— 正好只看队列本身怎么变。
    Future<CloudSyncSession> seeded(int n) async {
      final port = await deadPort();
      final s = CloudSyncSession(
        uri: () async => Uri.parse('ws://127.0.0.1:$port/ws/doc'),
        clientId: BigInt.one,
        onReady: (_, _) {},
        onRemoteBlocks: (_) {},
        restoreUnacked: [
          for (var i = 0; i < n; i++) Uint8List.fromList([i]),
        ],
      )..connect();
      return s;
    }

    test('ack 把那条移出 outbox', () async {
      await runZonedGuarded(() async {
        final s = await seeded(3);
        expect(s.debugOutbox.map((e) => e.id), ['0', '1', '2']);

        s.debugHandleFrame(jsonEncode({'type': 'sync.ack', 'ack_id': '1'}));
        expect(s.debugOutbox.map((e) => e.id), ['0', '2']);
        s.dispose();
      }, (_, _) {});
    });

    test('拒绝只打标记,不丢队列 —— 那条编辑还没到服务端', () async {
      await runZonedGuarded(() async {
        final s = await seeded(2);

        s.debugHandleFrame(jsonEncode({'type': 'error', 'ack_id': '0'}));
        expect(s.debugOutbox.length, 2, reason: '被拒 ≠ 已送达,不能出队');
        expect(s.debugOutbox.first.rejected, isTrue);
        expect(s.debugOutbox.last.rejected, isFalse);
        s.dispose();
      }, (_, _) {});
    });
  });
}

/// How much of the queued run folds into one push on reconnect.
///
/// The window (`pushSlice` above) decides how many entries may be in flight;
/// this decides how many of them become a SINGLE message. Both are pure so they
/// can be pinned here — the real send path needs a bootstrapped session and a
/// live server, which is `integration_test/cloud_sync_*`'s job.
void _mergeRunTests() {
  group('mergeRunLength', () {
    test('folds the whole run when it fits', () {
      expect(mergeRunLength([10, 10, 10], maxBytes: 100), 3);
    });

    test('stops before the entry that would blow the cap', () {
      // 40+40 = 80 fits; the third would make 120.
      expect(mergeRunLength([40, 40, 40], maxBytes: 100), 2);
    });

    /// The one that matters for liveness: an entry bigger than the whole cap
    /// still goes, alone. Returning 0 would park it forever, and it is exactly
    /// as sendable as it was before merging existed.
    test('always takes at least one, even when it alone exceeds the cap', () {
      expect(mergeRunLength([500], maxBytes: 100), 1);
      expect(mergeRunLength([500, 10], maxBytes: 100), 1);
    });

    test('an empty queue folds nothing', () {
      expect(mergeRunLength([], maxBytes: 100), 0);
    });

    /// Exactly at the cap is still in — the check is "would exceed", not
    /// "would reach". Off by one here silently halves the merge on a queue of
    /// uniform entries.
    test('an exact fit is included', () {
      expect(mergeRunLength([50, 50], maxBytes: 100), 2);
    });
  });
}
