// The Dart half of a two-engine invariant.
//
// Desktop and the server run yrs (Rust); web runs yjs (JS). Both now write a
// block's text as a MINIMAL splice rather than "delete everything, insert
// everything" — that is what lets two people type into one paragraph and get
// both edits instead of a duplicated paragraph. But a splice only means the
// same thing on both engines if both compute the same one, so this file is
// deliberately the SAME table of cases as `crates/mica-core/src/text_diff.rs`.
// If you add a case there, add it here.
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/web/text_diff.dart';

/// Apply the splice the way the CRDT would, so every case also proves the
/// splice reproduces the target — the property the rest of the file leans on.
String applied(String old, String next) {
  final d = textDiffUtf16(old, next);
  if (d == null) return old;
  final a = old.codeUnits;
  return String.fromCharCodes([
    ...a.take(d.at),
    ...d.ins.codeUnits,
    ...a.skip(d.at + d.del),
  ]);
}

(int, int, String)? diff(String old, String next) {
  final d = textDiffUtf16(old, next);
  return d == null ? null : (d.at, d.del, d.ins);
}

void main() {
  test('no change is no write', () {
    expect(diff('Hello', 'Hello'), isNull);
    expect(diff('', ''), isNull);
  });

  /// The two edits from the concurrency case, each seen on its own replica.
  test('a keystroke is one character', () {
    expect(diff('Hello', 'AHello'), (0, 0, 'A'));
    expect(diff('Hello', 'HelloB'), (5, 0, 'B'));
    expect(diff('Hello', 'HeXllo'), (2, 0, 'X'));
  });

  test('deletion and replacement', () {
    expect(diff('Hello', 'Helo'), (3, 1, ''));
    expect(diff('Hello', ''), (0, 5, ''));
    expect(diff('', 'Hi'), (0, 0, 'Hi'));
    expect(diff('abc', 'aXc'), (1, 1, 'X'));
  });

  /// CJK inside the BMP is 1 unit per character, so offsets are character
  /// counts here — the ordinary case for this product.
  test('CJK offsets are units, not bytes', () {
    expect(diff('你好', '你好吗'), (2, 0, '吗'));
    expect(diff('你好', '你们好'), (1, 0, '们'));
    expect(applied('笔记软件', '笔记软件很好'), '笔记软件很好');
  });

  /// Outside the BMP one character is TWO units. A boundary that lands between
  /// them would hand the CRDT half a character.
  test('surrogate pairs are never split', () {
    final d = textDiffUtf16('😀', '😁')!;
    expect(d.at, 0, reason: 'the whole character is replaced, not half of it');
    expect(d.del, 2);
    expect(d.ins, '😁');
    expect(applied('😀', '😁'), '😁');

    expect(applied('a😀b', 'a😀Xb'), 'a😀Xb');
    expect(applied('😀😀', '😀'), '😀');
    expect(applied('hi 😀', 'hi 😀!'), 'hi 😀!');
    expect(applied('x😀', 'y😀'), 'y😀');
  });

  test('the splice always reproduces the target', () {
    const alphabet = ['', 'a', 'ab', '你', '你好', '😀', 'a😀', '😀a', '😀😀'];
    for (final old in alphabet) {
      for (final next in alphabet) {
        expect(applied(old, next), next, reason: '$old -> $next');
      }
    }
  });
}
