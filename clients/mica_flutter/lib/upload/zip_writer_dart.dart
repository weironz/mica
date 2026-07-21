// The pure-Dart STORE-ZIP encoder — web's implementation (no FFI there) and
// the byte-parity reference for the desktop FFI path.
//
// This is a hand-written twin of `crates/interchange/src/zip/writer.rs` (the
// encoder the server and export paths use). Hand-rolled binary format in two
// places is exactly the double representation CLAUDE.md 原则 #2 forbids — it
// stays ONLY because web can't reach the FFI. The two are pinned byte-for-byte
// by a shared gold fixture: `crates/interchange/tests/zip_gold.rs` generates
// it, `test/zip_writer_conformance_test.dart` holds this file to it. Any edit
// here must keep that test green.
import 'dart:convert';
import 'dart:typed_data';

import 'archive_file.dart';

/// Build a STORE (uncompressed) ZIP — the upload container for server-side
/// import. No compression on purpose: it goes straight to our own backend,
/// and md/images don't gain much anyway. UTF-8 name flag set.
Uint8List buildStoreZip(List<ArchiveFile> files) {
  final out = BytesBuilder();
  final central = BytesBuilder();
  final offsets = <int>[];

  void u16(BytesBuilder b, int v) {
    b.addByte(v & 0xff);
    b.addByte((v >> 8) & 0xff);
  }

  void u32(BytesBuilder b, int v) {
    b.addByte(v & 0xff);
    b.addByte((v >> 8) & 0xff);
    b.addByte((v >> 16) & 0xff);
    b.addByte((v >> 24) & 0xff);
  }

  for (final f in files) {
    final name = utf8.encode(f.name);
    final crc = _crc32(f.bytes);
    offsets.add(out.length);
    u32(out, 0x04034b50);
    u16(out, 20);
    u16(out, 0x0800); // UTF-8 names
    u16(out, 0); // store
    u16(out, 0);
    u16(out, 0);
    u32(out, crc);
    u32(out, f.bytes.length);
    u32(out, f.bytes.length);
    u16(out, name.length);
    u16(out, 0);
    out.add(name);
    out.add(f.bytes);
  }

  final cdStart = out.length;
  for (var i = 0; i < files.length; i++) {
    final f = files[i];
    final name = utf8.encode(f.name);
    final crc = _crc32(f.bytes);
    u32(central, 0x02014b50);
    u16(central, 20);
    u16(central, 20);
    u16(central, 0x0800);
    u16(central, 0);
    u16(central, 0);
    u16(central, 0);
    u32(central, crc);
    u32(central, f.bytes.length);
    u32(central, f.bytes.length);
    u16(central, name.length);
    u16(central, 0);
    u16(central, 0);
    u16(central, 0);
    u16(central, 0);
    u32(central, 0);
    u32(central, offsets[i]);
    central.add(name);
  }
  final cd = central.takeBytes();
  out.add(cd);
  u32(out, 0x06054b50);
  u16(out, 0);
  u16(out, 0);
  u16(out, files.length);
  u16(out, files.length);
  u32(out, cd.length);
  u32(out, cdStart);
  u16(out, 0);
  return out.takeBytes();
}

List<int>? _crcTable;

int _crc32(Uint8List data) {
  final table = _crcTable ??= List<int>.generate(256, (i) {
    var c = i;
    for (var k = 0; k < 8; k++) {
      c = (c & 1) != 0 ? 0xEDB88320 ^ (c >>> 1) : c >>> 1;
    }
    return c;
  });
  var crc = 0xFFFFFFFF;
  for (final b in data) {
    crc = table[(crc ^ b) & 0xff] ^ (crc >>> 8);
  }
  return (crc ^ 0xFFFFFFFF) & 0xFFFFFFFF;
}
