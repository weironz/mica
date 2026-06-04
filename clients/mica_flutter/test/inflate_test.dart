import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/upload/inflate.dart';

// Fixtures produced by Python zlib (raw deflate, wbits=-15) so we verify
// compatibility with a reference compressor rather than ourselves.
Uint8List _b64(String s) => base64.decode(s);

void main() {
  test('inflate decodes a fixed-Huffman block (Z_FIXED)', () {
    final out = inflate(_b64('y0jNyclXyEAiy/OLclIA'));
    expect(utf8.decode(out), 'hello hello hello world');
  });

  test('inflate decodes a dynamic-Huffman block with back-references', () {
    final expected = 'Mica is a cloud-first markdown workspace. ' * 40;
    final out = inflate(
      _b64('881MTlTILFZIVEjOyS9N0U3LLCouUchNLMpOyS/PUyjPL8ouLkhMTtVT8B1VOapy'
          'VOWoylGVoyppphIA'),
      expectedSize: expected.length,
    );
    expect(out.length, 1680);
    expect(utf8.decode(out), expected);
  });

  test('inflate decodes a stored (level-0) block', () {
    final out = inflate(_b64('ARIA7f9wbGFpbiBzdG9yZWQgYnl0ZXM='));
    expect(utf8.decode(out), 'plain stored bytes');
  });

  test('inflate rejects truncated input', () {
    final dynamic_ = _b64(
        '881MTlTILFZIVEjOyS9N0U3LLCouUchNLMpOyS/PUyjPL8ouLkhMTtVT8B1VOapy');
    expect(
      () => inflate(Uint8List.sublistView(dynamic_, 0, 10)),
      throwsFormatException,
    );
  });
}
