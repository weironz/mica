import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/upload/unzip.dart';

/// Minimal STORE-zip builder mirroring the backend writer, for round-trip tests.
Uint8List buildStoreZip(Map<String, List<int>> files) {
  final out = <int>[];
  final central = <int>[];
  final offsets = <int>[];

  void u16(List<int> b, int v) {
    b..add(v & 0xff)..add((v >> 8) & 0xff);
  }

  void u32(List<int> b, int v) {
    b
      ..add(v & 0xff)
      ..add((v >> 8) & 0xff)
      ..add((v >> 16) & 0xff)
      ..add((v >> 24) & 0xff);
  }

  files.forEach((name, data) {
    final nameBytes = utf8.encode(name);
    offsets.add(out.length);
    u32(out, 0x04034b50);
    u16(out, 20);
    u16(out, 0x0800); // UTF-8
    u16(out, 0); // store
    u16(out, 0);
    u16(out, 0);
    u32(out, 0); // crc (reader ignores)
    u32(out, data.length);
    u32(out, data.length);
    u16(out, nameBytes.length);
    u16(out, 0);
    out..addAll(nameBytes)..addAll(data);
  });

  final cdStart = out.length;
  var i = 0;
  files.forEach((name, data) {
    final nameBytes = utf8.encode(name);
    u32(central, 0x02014b50);
    u16(central, 20);
    u16(central, 20);
    u16(central, 0x0800);
    u16(central, 0);
    u16(central, 0);
    u16(central, 0);
    u32(central, 0);
    u32(central, data.length);
    u32(central, data.length);
    u16(central, nameBytes.length);
    u16(central, 0);
    u16(central, 0);
    u16(central, 0);
    u16(central, 0);
    u32(central, 0);
    u32(central, offsets[i]);
    central.addAll(nameBytes);
    i++;
  });
  out.addAll(central);
  u32(out, 0x06054b50);
  u16(out, 0);
  u16(out, 0);
  u16(out, files.length);
  u16(out, files.length);
  u32(out, central.length);
  u32(out, cdStart);
  u16(out, 0);
  return Uint8List.fromList(out);
}

void main() {
  test('readStoreZip round-trips files (incl. CJK names + nested paths)', () {
    final zip = buildStoreZip({
      'Guide.md': utf8.encode('# Guide'),
      'Guide/Setup.md': utf8.encode('## Setup'),
      'assets/图片.png': [1, 2, 3, 4],
    });
    final entries = readStoreZip(zip);
    final byName = {for (final e in entries) e.name: e.bytes};
    expect(byName.keys.toSet(), {'Guide.md', 'Guide/Setup.md', 'assets/图片.png'});
    expect(utf8.decode(byName['Guide.md']!), '# Guide');
    expect(utf8.decode(byName['Guide/Setup.md']!), '## Setup');
    expect(byName['assets/图片.png'], [1, 2, 3, 4]);
  });

  test('readStoreZip skips directory entries', () {
    final zip = buildStoreZip({
      'folder/': const [],
      'folder/a.md': utf8.encode('a'),
    });
    final names = readStoreZip(zip).map((e) => e.name).toList();
    expect(names, ['folder/a.md']);
  });
}
