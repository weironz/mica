// Regression for session-persistence (no more re-login every restart): the
// startup restore cheaply rejects an expired persisted token by its JWT `exp`
// before hitting the network. This guards that parser.
@TestOn('vm')
library;

import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/main.dart' show jwtExpiry;

/// Build a JWT-shaped string (base64url, no padding) with the given payload.
String _jwt(Map<String, dynamic> payload) {
  String seg(Map<String, dynamic> m) =>
      base64Url.encode(utf8.encode(jsonEncode(m))).replaceAll('=', '');
  return '${seg({'alg': 'HS256', 'typ': 'JWT'})}.${seg(payload)}.sig';
}

void main() {
  test('parses the exp claim as UTC', () {
    const expSec = 1782443714;
    final t = _jwt({'sub': 'u', 'exp': expSec});
    expect(
      jwtExpiry(t),
      DateTime.fromMillisecondsSinceEpoch(expSec * 1000, isUtc: true),
    );
  });

  test('handles a payload whose base64url needs re-padding', () {
    const expSec = 1700000000;
    expect(jwtExpiry(_jwt({'exp': expSec}))!.millisecondsSinceEpoch,
        expSec * 1000);
  });

  test('returns null for malformed / missing-exp / non-JWT input', () {
    expect(jwtExpiry(''), isNull);
    expect(jwtExpiry('onlyonesegment'), isNull);
    expect(jwtExpiry('not.a.jwt'), isNull); // payload not valid base64 JSON
    expect(jwtExpiry(_jwt({'sub': 'u'})), isNull); // no exp claim
    expect(jwtExpiry(_jwt({'exp': 'notanumber'})), isNull); // exp wrong type
  });
}
