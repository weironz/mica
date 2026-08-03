// The socket token used to ride in the query string on every platform, so a
// plaintext JWT landed in reverse-proxy access logs and browser history — and a
// log line outlives the session it came from.
//
// The server has preferred `Authorization: Bearer` all along and only fallen
// back to `?token=` (`ws.rs` `token_from_request`, plus its two Rust tests), so
// on desktop this is a client-side fix alone. These tests pin the rewrite, not
// the connect: what can go wrong is dropping the wrong parameter, keeping the
// token in both places, or mangling a URL that has no token at all.
//
// Web is deliberately NOT fixed here — the browser WebSocket API cannot send
// headers — and the last test states that difference so it reads as a decision
// rather than an oversight.
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/sync_client.dart';
import 'package:mica_flutter/api/ws_connect_stub.dart' as io_variant;
import 'package:mica_flutter/api/ws_connect_web.dart' as web_variant;

void main() {
  group('desktop moves the token out of the URL', () {
    test('the token leaves the query and becomes the bearer', () {
      final split = io_variant.splitSocketAuth(
        Uri.parse(
          'wss://mica.example/ws/workspaces/w1/documents/d1?token=jwt.abc&v=1',
        ),
      );

      expect(split.token, 'jwt.abc');
      expect(split.uri.queryParameters.containsKey('token'), isFalse);
      expect(
        split.uri.toString(),
        isNot(contains('jwt.abc')),
        reason: 'the whole point: the token must not appear in the URL at all',
      );
    });

    test('other parameters survive — the protocol version is not collateral', () {
      final split = io_variant.splitSocketAuth(
        Uri.parse('wss://mica.example/ws?token=jwt.abc&v=1'),
      );

      expect(split.uri.queryParameters['v'], '1');
    });

    // An emptied query must become NO query. `queryParameters: {}` still renders
    // a trailing `?`, which is the kind of difference that only shows up as a
    // confusing mismatch somewhere far away.
    test('a token-only query leaves no dangling question mark', () {
      final split = io_variant.splitSocketAuth(
        Uri.parse('wss://mica.example/ws/ai?token=jwt.abc'),
      );

      expect(split.uri.toString(), 'wss://mica.example/ws/ai');
    });

    test('a URL with no token is passed through untouched', () {
      final plain = Uri.parse('wss://mica.example/ws?v=1');
      final split = io_variant.splitSocketAuth(plain);

      expect(split.token, isNull);
      expect(split.uri, plain);
    });

    test('an empty token counts as no token, not as an empty bearer', () {
      final split = io_variant.splitSocketAuth(
        Uri.parse('wss://mica.example/ws?token=&v=1'),
      );

      expect(
        split.token,
        isNull,
        reason: 'Bearer "" would be a 401 with extra steps',
      );
    });
  });

  // This test used to assert the opposite — that web HAD to leave the token in
  // the URL, because the browser WebSocket API cannot set headers. That much is
  // true and it is still true; what was wrong is the conclusion drawn from it.
  // The handshake is an ordinary HTTP request, so the browser attaches the
  // session cookie by itself, and the URL needs to carry nothing at all.
  group('web sends no credential in the URL — the cookie rides the handshake',
      () {
    test('the token is stripped, not relocated', () {
      final split = web_variant.splitSocketAuth(
        Uri.parse('wss://mica.example/ws?token=jwt.abc&v=1'),
      );

      expect(split.uri.queryParameters.containsKey('token'), isFalse);
      expect(split.uri.toString(), isNot(contains('jwt.abc')));
      expect(
        split.token,
        isNull,
        reason: 'nothing to hand anywhere — there is no header to put it in',
      );
    });

    test('the protocol version survives', () {
      final split = web_variant.splitSocketAuth(
        Uri.parse('wss://mica.example/ws?token=jwt.abc&v=1'),
      );

      expect(split.uri.queryParameters['v'], '1');
    });

    test('a token-only query leaves no dangling question mark', () {
      final split = web_variant.splitSocketAuth(
        Uri.parse('wss://mica.example/ws/ai?token=jwt.abc'),
      );

      expect(split.uri.toString(), 'wss://mica.example/ws/ai');
    });

    test('a URL that never had a token is untouched', () {
      final plain = Uri.parse('wss://mica.example/ws?v=1');

      expect(web_variant.splitSocketAuth(plain).uri, plain);
    });
  });

  // The URI builder still owns the token — the header move happens at the
  // connect seam, so this keeps saying what it always said.
  test('documentSocketUri still carries the token for the seam to relocate', () {
    final uri = documentSocketUri(
      Uri.parse('https://mica.example'),
      'w1',
      'd1',
      'jwt.abc',
    );

    expect(uri.scheme, 'wss');
    expect(uri.queryParameters['token'], 'jwt.abc');
    expect(uri.queryParameters['v'], '$kSyncProtocolVersion');
  });
}
