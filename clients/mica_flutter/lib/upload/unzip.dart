import 'dart:convert';
import 'dart:typed_data';

import 'inflate.dart';

/// One file read out of a ZIP archive.
class ZipFileEntry {
  ZipFileEntry(this.name, this.bytes);
  final String name;
  final Uint8List bytes;
}

/// Read a ZIP archive into its files. Supports STORE (Mica's own exports) and
/// DEFLATE (what `zip`, Windows Explorer, macOS Finder etc. produce), driven
/// by the central directory so data-descriptor entries also work. Directories
/// and entries with other compression methods are skipped.
List<ZipFileEntry> readZip(Uint8List data) {
  final bd = ByteData.sublistView(data);

  // Locate the end-of-central-directory record, scanning back over a
  // possible archive comment (up to 64 KiB).
  const eocdLen = 22;
  var eocd = -1;
  final stop =
      (data.length - eocdLen - 0xffff) < 0 ? 0 : data.length - eocdLen - 0xffff;
  for (var i = data.length - eocdLen; i >= stop; i--) {
    if (bd.getUint32(i, Endian.little) == 0x06054b50) {
      eocd = i;
      break;
    }
  }
  if (eocd < 0) return _readLocalEntries(data, bd);

  final total = bd.getUint16(eocd + 10, Endian.little);
  var off = bd.getUint32(eocd + 16, Endian.little);
  final out = <ZipFileEntry>[];
  for (var n = 0; n < total; n++) {
    if (off + 46 > data.length ||
        bd.getUint32(off, Endian.little) != 0x02014b50) {
      break;
    }
    final method = bd.getUint16(off + 10, Endian.little);
    final compSize = bd.getUint32(off + 20, Endian.little);
    final uncompSize = bd.getUint32(off + 24, Endian.little);
    final nameLen = bd.getUint16(off + 28, Endian.little);
    final extraLen = bd.getUint16(off + 30, Endian.little);
    final commentLen = bd.getUint16(off + 32, Endian.little);
    final localOff = bd.getUint32(off + 42, Endian.little);
    final name = utf8.decode(
      data.sublist(off + 46, off + 46 + nameLen),
      allowMalformed: true,
    );
    off += 46 + nameLen + extraLen + commentLen;
    if (name.endsWith('/')) continue; // directory

    // The local header's name/extra lengths may differ from the central
    // directory's — read them to find where the data actually starts.
    if (localOff + 30 > data.length ||
        bd.getUint32(localOff, Endian.little) != 0x04034b50) {
      continue;
    }
    final lNameLen = bd.getUint16(localOff + 26, Endian.little);
    final lExtraLen = bd.getUint16(localOff + 28, Endian.little);
    final dataStart = localOff + 30 + lNameLen + lExtraLen;
    if (dataStart + compSize > data.length) continue;
    final entry = _decodeEntry(
      Uint8List.sublistView(data, dataStart, dataStart + compSize),
      method,
      uncompSize,
    );
    if (entry != null) out.add(ZipFileEntry(name, entry));
  }
  return out;
}

/// Resolve a Markdown reference (`../assets/图 1.png`) found inside
/// [fromFile] (a path within the archive) to an archive path. Returns null
/// when the reference is external (has a URL scheme) or matches nothing in
/// [paths]. Tries the md file's own folder first, then the archive root.
String? resolveZipPath(String fromFile, String ref, Set<String> paths) {
  var u = ref.trim();
  if (u.isEmpty) return null;
  if (RegExp(r'^[a-zA-Z][a-zA-Z0-9+.-]*:').hasMatch(u)) return null; // URL
  u = u.split('#').first.split('?').first;
  try {
    u = Uri.decodeFull(u);
  } catch (_) {}
  final dir = fromFile.contains('/')
      ? fromFile.substring(0, fromFile.lastIndexOf('/')).split('/')
      : const <String>[];
  for (final base in [dir, const <String>[]]) {
    final stack = [...base];
    for (final seg in u.split('/')) {
      if (seg.isEmpty || seg == '.') continue;
      if (seg == '..') {
        if (stack.isNotEmpty) stack.removeLast();
      } else {
        stack.add(seg);
      }
    }
    final p = stack.join('/');
    if (paths.contains(p)) return p;
  }
  return null;
}

Uint8List? _decodeEntry(Uint8List comp, int method, int uncompSize) {
  switch (method) {
    case 0:
      return Uint8List.fromList(comp);
    case 8:
      return inflate(comp, expectedSize: uncompSize);
    default:
      return null; // unsupported compression method
  }
}

/// Fallback for archives without a readable central directory: walk local
/// headers front-to-back. Cannot handle data-descriptor entries (their local
/// sizes are zero), but covers truncated-yet-salvageable archives.
List<ZipFileEntry> _readLocalEntries(Uint8List data, ByteData bd) {
  final out = <ZipFileEntry>[];
  var i = 0;
  while (i + 30 <= data.length) {
    if (bd.getUint32(i, Endian.little) != 0x04034b50) break;
    final method = bd.getUint16(i + 8, Endian.little);
    final compSize = bd.getUint32(i + 18, Endian.little);
    final uncompSize = bd.getUint32(i + 22, Endian.little);
    final nameLen = bd.getUint16(i + 26, Endian.little);
    final extraLen = bd.getUint16(i + 28, Endian.little);
    final nameStart = i + 30;
    final dataStart = nameStart + nameLen + extraLen;
    if (dataStart + compSize > data.length) break;
    final name = utf8.decode(
      data.sublist(nameStart, nameStart + nameLen),
      allowMalformed: true,
    );
    if (!name.endsWith('/')) {
      final entry = _decodeEntry(
        Uint8List.sublistView(data, dataStart, dataStart + compSize),
        method,
        uncompSize,
      );
      if (entry != null) out.add(ZipFileEntry(name, entry));
    }
    i = dataStart + compSize;
  }
  return out;
}
