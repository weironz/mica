import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/upload/sha256.dart';

void main() {
  test('sha256 known vectors', () {
    expect(sha256Hex(const []),
        'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855');
    expect(sha256Hex(utf8.encode('abc')),
        'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad');
    expect(
      sha256Hex(utf8.encode(
          'abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq')),
      '248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1',
    );
  });

  test('sha256 handles a multi-block payload', () {
    // 1000 'a' bytes crosses several 64-byte blocks and the padding boundary.
    final data = List<int>.filled(1000, 0x61);
    expect(sha256Hex(data),
        '41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3');
  });
}
