import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/sync_client.dart';

/// 客户端必须**声明**自己说的协议版本,服务端才有可能在需要时把太老的客户端挡在
/// 门外(`client_too_old`)而不是让它连上来再以谁也解释不清的方式出错。
///
/// 今天服务端的地板是 0,谁都不拒 —— 所以这条声明现在一分钱不值,而这正是它的意义:
/// 桌面端是用户自装的二进制,更新节奏不由我们定。等 op 模型退役(S4)把那条默默兜住
/// 老客户端的 REST 后路撤掉时,还在连的恰恰是这些从不更新的安装 —— 到那时才加声明
/// 已经晚了,因为要挡的就是那批没有声明的客户端。
void main() {
  test('WS URI 带上协议版本', () {
    final uri = documentSocketUri(
      Uri.parse('https://mica.example'),
      'ws-1',
      'doc-1',
      'tok',
    );

    expect(uri.queryParameters['v'], '$kSyncProtocolVersion');
    expect(uri.queryParameters['token'], 'tok', reason: 'token 不能被挤掉');
    expect(uri.scheme, 'wss', reason: 'https → wss');
    expect(uri.path, '/ws/workspaces/ws-1/documents/doc-1');
  });

  test('版本号跟服务端对齐', () {
    // 服务端那侧有一条对称的断言(ws.rs `the_current_version_is_one`)。
    // 两个数字活在两种语言里,会无声漂移,这是最便宜的发现点。
    expect(
      kSyncProtocolVersion,
      1,
      reason: '改这里就要同时改 ws.rs 的 WS_PROTOCOL_VERSION',
    );
  });

  test('http 降级成 ws,不是硬编码 wss', () {
    // 自托管/本地开发跑在明文 http 上,写死 wss 会让它们连不上。
    final uri = documentSocketUri(
      Uri.parse('http://127.0.0.1:8080'),
      'w',
      'd',
      't',
    );
    expect(uri.scheme, 'ws');
    expect(uri.queryParameters['v'], '$kSyncProtocolVersion');
  });
}
