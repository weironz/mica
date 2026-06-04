import 'dart:typed_data';

/// Raw DEFLATE (RFC 1951) decompressor — in-house, no dependencies.
///
/// Follows the classic puff.c structure: canonical Huffman decoding with
/// per-length counts, the three block types (stored / fixed / dynamic), and
/// LZ77 back-reference copying. All bit twiddling stays under 32 bits so it
/// is safe on dart2js.
Uint8List inflate(Uint8List input, {int? expectedSize}) =>
    _Inflater(input, expectedSize).run();

class _Inflater {
  _Inflater(this.input, int? hint)
      : _buf = Uint8List((hint == null || hint < 64) ? 64 : hint);

  final Uint8List input;
  Uint8List _buf;
  int _len = 0; // bytes written to _buf
  int _pos = 0; // byte position in input
  int _bitBuf = 0;
  int _bitCnt = 0;

  Uint8List run() {
    var last = 0;
    do {
      last = _bits(1);
      final type = _bits(2);
      switch (type) {
        case 0:
          _stored();
        case 1:
          _fixedBlock();
        case 2:
          _dynamicBlock();
        default:
          throw const FormatException('inflate: invalid block type');
      }
    } while (last == 0);
    return Uint8List.sublistView(_buf, 0, _len);
  }

  // ---- bit reader (LSB-first) ----

  int _bits(int need) {
    var val = _bitBuf;
    while (_bitCnt < need) {
      if (_pos >= input.length) {
        throw const FormatException('inflate: unexpected end of input');
      }
      val |= input[_pos++] << _bitCnt;
      _bitCnt += 8;
    }
    _bitBuf = val >> need;
    _bitCnt -= need;
    return val & ((1 << need) - 1);
  }

  // ---- output buffer ----

  void _ensure(int extra) {
    if (_len + extra <= _buf.length) return;
    var cap = _buf.length * 2;
    while (cap < _len + extra) {
      cap *= 2;
    }
    _buf = Uint8List(cap)..setRange(0, _len, _buf);
  }

  // ---- block types ----

  void _stored() {
    _bitBuf = 0;
    _bitCnt = 0; // discard bits up to the next byte boundary
    if (_pos + 4 > input.length) {
      throw const FormatException('inflate: truncated stored block');
    }
    final len = input[_pos] | (input[_pos + 1] << 8);
    _pos += 4; // skip LEN + NLEN (complement not verified)
    if (_pos + len > input.length) {
      throw const FormatException('inflate: truncated stored block');
    }
    _ensure(len);
    _buf.setRange(_len, _len + len, input, _pos);
    _len += len;
    _pos += len;
  }

  static _Huffman? _fixedLen;
  static _Huffman? _fixedDist;

  void _fixedBlock() {
    if (_fixedLen == null) {
      final lens = List<int>.filled(288, 8);
      for (var i = 144; i < 256; i++) {
        lens[i] = 9;
      }
      for (var i = 256; i < 280; i++) {
        lens[i] = 7;
      }
      _fixedLen = _Huffman(lens);
      _fixedDist = _Huffman(List<int>.filled(30, 5));
    }
    _codes(_fixedLen!, _fixedDist!);
  }

  /// Order in which code-length code lengths are stored (RFC 1951 §3.2.7).
  static const _clOrder = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
  ];

  void _dynamicBlock() {
    final nlen = _bits(5) + 257;
    final ndist = _bits(5) + 1;
    final ncode = _bits(4) + 4;
    final clLens = List<int>.filled(19, 0);
    for (var i = 0; i < ncode; i++) {
      clLens[_clOrder[i]] = _bits(3);
    }
    final clCode = _Huffman(clLens);

    final lens = List<int>.filled(nlen + ndist, 0);
    var i = 0;
    while (i < nlen + ndist) {
      final sym = _decode(clCode);
      if (sym < 16) {
        lens[i++] = sym;
        continue;
      }
      var value = 0;
      int repeat;
      if (sym == 16) {
        if (i == 0) throw const FormatException('inflate: bad repeat');
        value = lens[i - 1];
        repeat = 3 + _bits(2);
      } else if (sym == 17) {
        repeat = 3 + _bits(3);
      } else {
        repeat = 11 + _bits(7);
      }
      if (i + repeat > nlen + ndist) {
        throw const FormatException('inflate: bad repeat');
      }
      while (repeat-- > 0) {
        lens[i++] = value;
      }
    }
    if (lens[256] == 0) {
      throw const FormatException('inflate: missing end-of-block code');
    }
    _codes(_Huffman(lens.sublist(0, nlen)), _Huffman(lens.sublist(nlen)));
  }

  // ---- literal/length + distance decoding ----

  static const _lenBase = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, //
    67, 83, 99, 115, 131, 163, 195, 227, 258,
  ];
  static const _lenExtra = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, //
    4, 4, 4, 4, 5, 5, 5, 5, 0,
  ];
  static const _distBase = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, //
    513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
  ];
  static const _distExtra = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, //
    9, 9, 10, 10, 11, 11, 12, 12, 13, 13,
  ];

  void _codes(_Huffman lenCode, _Huffman distCode) {
    while (true) {
      var sym = _decode(lenCode);
      if (sym < 256) {
        _ensure(1);
        _buf[_len++] = sym;
        continue;
      }
      if (sym == 256) return; // end of block
      sym -= 257;
      if (sym >= _lenBase.length) {
        throw const FormatException('inflate: bad length code');
      }
      final len = _lenBase[sym] + _bits(_lenExtra[sym]);
      final dsym = _decode(distCode);
      if (dsym >= _distBase.length) {
        throw const FormatException('inflate: bad distance code');
      }
      final dist = _distBase[dsym] + _bits(_distExtra[dsym]);
      if (dist > _len) {
        throw const FormatException('inflate: distance too far back');
      }
      _ensure(len);
      for (var k = 0; k < len; k++) {
        _buf[_len] = _buf[_len - dist];
        _len++;
      }
    }
  }

  int _decode(_Huffman h) {
    var code = 0, first = 0, index = 0;
    for (var len = 1; len <= 15; len++) {
      code |= _bits(1);
      final cnt = h.count[len];
      if (code - first < cnt) return h.symbol[index + (code - first)];
      index += cnt;
      first = (first + cnt) << 1;
      code <<= 1;
    }
    throw const FormatException('inflate: invalid Huffman code');
  }
}

/// Canonical Huffman table: per-length symbol counts + symbols sorted by
/// (length, symbol order), as in puff.c.
class _Huffman {
  _Huffman(List<int> lengths) : count = List<int>.filled(16, 0) {
    for (final l in lengths) {
      count[l]++;
    }
    count[0] = 0;
    final offs = List<int>.filled(16, 0);
    for (var len = 1; len < 16; len++) {
      offs[len] = offs[len - 1] + count[len - 1];
    }
    symbol = List<int>.filled(lengths.length, 0);
    for (var s = 0; s < lengths.length; s++) {
      if (lengths[s] != 0) symbol[offs[lengths[s]]++] = s;
    }
  }

  final List<int> count;
  late final List<int> symbol;
}
