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
  test('readZip round-trips files (incl. CJK names + nested paths)', () {
    final zip = buildStoreZip({
      'Guide.md': utf8.encode('# Guide'),
      'Guide/Setup.md': utf8.encode('## Setup'),
      'assets/图片.png': [1, 2, 3, 4],
    });
    final entries = readZip(zip);
    final byName = {for (final e in entries) e.name: e.bytes};
    expect(byName.keys.toSet(), {'Guide.md', 'Guide/Setup.md', 'assets/图片.png'});
    expect(utf8.decode(byName['Guide.md']!), '# Guide');
    expect(utf8.decode(byName['Guide/Setup.md']!), '## Setup');
    expect(byName['assets/图片.png'], [1, 2, 3, 4]);
  });

  test('readZip skips directory entries', () {
    final zip = buildStoreZip({
      'folder/': const [],
      'folder/a.md': utf8.encode('a'),
    });
    final names = readZip(zip).map((e) => e.name).toList();
    expect(names, ['folder/a.md']);
  });

  test('readZip reads a DEFLATE zip made by an external tool', () {
    // Built with Python zipfile (ZIP_DEFLATED, level 9): nested paths, a CJK
    // name, repeated text that actually compresses, and an empty file.
    final zip = base64.decode(
      'UEsDBBQAAAAIAEVMxFy0mEsXGAAAAHABAAAIAAAAR3VpZGUubWRTVnAvzUxJ5SpKLUhN'
      'LFHIycwbZdOSDQBQSwMEFAAAAAgARUzEXMIS3MkKAAAACAAAAA4AAABHdWlkZS9TZXR1'
      'cC5tZFNWVghOLSktAABQSwMEFAAACAgARUzEXDgV/woJAAAAKAAAABEAAABhc3NldHMv'
      '5Zu+54mHLnBuZ2NkYmZhJAIDAFBLAwQUAAAACABFTMRcAAAAAAIAAAAAAAAACAAAAGVt'
      'cHR5Lm1kAwBQSwECFAMUAAAACABFTMRctJhLFxgAAABwAQAACAAAAAAAAAAAAAAAgAEA'
      'AAAAR3VpZGUubWRQSwECFAMUAAAACABFTMRcwhLcyQoAAAAIAAAADgAAAAAAAAAAAAAA'
      'gAE+AAAAR3VpZGUvU2V0dXAubWRQSwECFAMUAAAICABFTMRcOBX/CgkAAAAoAAAAEQAA'
      'AAAAAAAAAAAAgAF0AAAAYXNzZXRzL+WbvueJhy5wbmdQSwECFAMUAAAACABFTMRcAAAA'
      'AAIAAAAAAAAACAAAAAAAAAAAAAAAgAGsAAAAZW1wdHkubWRQSwUGAAAAAAQABADnAAAA'
      '1AAAAAAA',
    );
    final entries = readZip(Uint8List.fromList(zip));
    final byName = {for (final e in entries) e.name: e.bytes};
    expect(byName.keys.toSet(),
        {'Guide.md', 'Guide/Setup.md', 'assets/图片.png', 'empty.md'});
    expect(
        utf8.decode(byName['Guide.md']!), '# Guide\n${'repeat line\n' * 30}');
    expect(utf8.decode(byName['Guide/Setup.md']!), '## Setup');
    expect(byName['assets/图片.png'],
        List.filled(10, [1, 2, 3, 4]).expand((x) => x).toList());
    expect(byName['empty.md'], isEmpty);
  });

  test('readZip decodes GBK entry names (Windows Explorer, CJK locale)', () {
    // Hand-crafted STORE zip: names are GBK bytes, UTF-8 flag NOT set —
    // exactly what Explorer's "compress" produces on a Chinese Windows.
    final zip = base64.decode(
      'UEsDBBQAAAAAAAAAAAAHuXpbCgAAAAoAAAAQAAAA1tDOxMS/wrwv0rPD5i5tZEdCSyDl'
      'hoXlrrlQSwMEFAAAAAAAAAAAABIRx+kCAAAAAgAAAAgAAADNvMasLnBuZwkJUEsBAhQA'
      'FAAAAAAAAAAAAAe5elsKAAAACgAAABAAAAAAAAAAAAAAAAAAAAAAANbQzsTEv8K8L9Kz'
      'w+YubWRQSwECFAAUAAAAAAAAAAAAEhHH6QIAAAACAAAACAAAAAAAAAAAAAAAAAA4AAAA'
      'zbzGrC5wbmdQSwUGAAAAAAIAAgB0AAAAYAAAAAAA',
    );
    final entries = readZip(Uint8List.fromList(zip));
    final byName = {for (final e in entries) e.name: e.bytes};
    expect(byName.keys.toSet(), {'中文目录/页面.md', '图片.png'});
    expect(utf8.decode(byName['中文目录/页面.md']!), 'GBK 内容');
  });

  test('readZip prefers the Info-ZIP Unicode Path extra field (0x7075)', () {
    final zip = base64.decode(
      'UEsDBBQAAAAAAAAAAACsKpPYAgAAAAIAAAAHABIAX19fXy5tZHVwDgABrWBbNOa1i+iv'
      'lS5tZGhpUEsBAhQAFAAAAAAAAAAAAKwqk9gCAAAAAgAAAAcAEgAAAAAAAAAAAAAAAAAA'
      'AF9fX18ubWR1cA4AAa1gWzTmtYvor5UubWRQSwUGAAAAAAEAAQBHAAAAOQAAAAAA',
    );
    final entries = readZip(Uint8List.fromList(zip));
    expect(entries.single.name, '测试.md');
    expect(utf8.decode(entries.single.bytes), 'hi');
  });

  group('normalizeZipEntries', () {
    ZipFileEntry e(String name) => ZipFileEntry(name, Uint8List(0));
    List<String> names(List<ZipFileEntry> l) => l.map((x) => x.name).toList();

    test('peels a single wrapper folder (Notion export shell)', () {
      final out = normalizeZipEntries([
        e('Export-1f2e3d4c/Guide 0123456789abcdef0123456789abcdef.md'),
        e('Export-1f2e3d4c/Guide 0123456789abcdef0123456789abcdef/img.png'),
      ]);
      expect(names(out), [
        'Guide 0123456789abcdef0123456789abcdef.md',
        'Guide 0123456789abcdef0123456789abcdef/img.png',
      ]);
    });

    test('peels repeatedly through double wrappers', () {
      final out = normalizeZipEntries([
        e('outer/inner/x.md'),
        e('outer/inner/sub/y.md'),
      ]);
      expect(names(out), ['x.md', 'sub/y.md']);
    });

    test('keeps a root-level file from being peeled (Mica exports)', () {
      final out = normalizeZipEntries([
        e('manifest.json'),
        e('Guide.md'),
        e('Guide/Setup.md'),
      ]);
      expect(names(out), ['manifest.json', 'Guide.md', 'Guide/Setup.md']);
    });

    test('a lone root page with children is not a wrapper', () {
      final out = normalizeZipEntries([e('Page.md'), e('Page/Child.md')]);
      expect(names(out), ['Page.md', 'Page/Child.md']);
    });

    test('two top folders are not peeled', () {
      final out = normalizeZipEntries([e('A/x.md'), e('B/y.md')]);
      expect(names(out), ['A/x.md', 'B/y.md']);
    });

    test('drops macOS metadata, then peels the remaining wrapper', () {
      final out = normalizeZipEntries([
        e('__MACOSX/notes/._a.md'),
        e('notes/.DS_Store'),
        e('notes/._a.md'),
        e('notes/Thumbs.db'),
        e('notes/a.md'),
        e('notes/pics/b.png'),
      ]);
      expect(names(out), ['a.md', 'pics/b.png']);
    });
  });

  group('resolveZipPath', () {
    final paths = {
      'assets/图片.png',
      'Guide/pics/a.png',
      'Guide/Setup/shot.png',
    };

    test('resolves relative to the md file folder', () {
      expect(resolveZipPath('Guide/Setup.md', 'pics/a.png', paths),
          'Guide/pics/a.png');
      expect(resolveZipPath('Guide/Setup/Linux.md', './shot.png', paths),
          'Guide/Setup/shot.png');
    });

    test('resolves ../ chains (Mica export layout)', () {
      expect(resolveZipPath('Guide/Setup.md', '../assets/图片.png', paths),
          'assets/图片.png');
      expect(
          resolveZipPath('Guide/Setup/Linux.md', '../../assets/图片.png', paths),
          'assets/图片.png');
    });

    test('falls back to archive root for root-relative refs', () {
      expect(resolveZipPath('Guide/Setup/Linux.md', 'assets/图片.png', paths),
          'assets/图片.png');
      // Excess ../ is tolerated rather than failing.
      expect(resolveZipPath('Guide.md', '../../assets/图片.png', paths),
          'assets/图片.png');
    });

    test('decodes percent-encoded names', () {
      expect(
          resolveZipPath('Guide/Setup.md', '../assets/%E5%9B%BE%E7%89%87.png',
              paths),
          'assets/图片.png');
    });

    test('external URLs and missing files return null', () {
      expect(resolveZipPath('a.md', 'https://x.com/i.png', paths), isNull);
      expect(resolveZipPath('a.md', 'data:image/png;base64,xx', paths), isNull);
      expect(resolveZipPath('a.md', 'nope.png', paths), isNull);
    });
  });
}
