import 'dart:convert';
import 'dart:typed_data';

import 'gbk.dart';
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
    final flags = bd.getUint16(off + 8, Endian.little);
    final method = bd.getUint16(off + 10, Endian.little);
    final compSize = bd.getUint32(off + 20, Endian.little);
    final uncompSize = bd.getUint32(off + 24, Endian.little);
    final nameLen = bd.getUint16(off + 28, Endian.little);
    final extraLen = bd.getUint16(off + 30, Endian.little);
    final commentLen = bd.getUint16(off + 32, Endian.little);
    final localOff = bd.getUint32(off + 42, Endian.little);
    final name = _decodeName(
      data.sublist(off + 46, off + 46 + nameLen),
      flags,
      data.sublist(off + 46 + nameLen, off + 46 + nameLen + extraLen),
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

/// Normalize archive entries for import: drop OS metadata (`__MACOSX/`,
/// AppleDouble `._*` files, `.DS_Store`, `Thumbs.db`) and peel wrapper
/// folders — when everything lives under a single top-level folder with no
/// file beside it (a zipped folder, macOS Finder archives, Notion's
/// `Export-<id>/` shell), strip that level, repeatedly.
///
/// Real content is never peeled: a Mica export keeps `manifest.json` (or a
/// root page's `.md`) at the top level, so the single-folder condition fails.
List<ZipFileEntry> normalizeZipEntries(List<ZipFileEntry> entries) {
  var out = [
    for (final e in entries)
      if (!_isJunk(e.name)) e,
  ];
  while (out.isNotEmpty) {
    String? top;
    var single = true;
    for (final e in out) {
      final i = e.name.indexOf('/');
      if (i <= 0) {
        single = false; // a file at the root → not a wrapper
        break;
      }
      final seg = e.name.substring(0, i);
      if (top == null) {
        top = seg;
      } else if (top != seg) {
        single = false;
        break;
      }
    }
    if (!single || top == null) break;
    out = [
      for (final e in out)
        ZipFileEntry(e.name.substring(top.length + 1), e.bytes),
    ];
  }
  return out;
}

bool _isJunk(String path) {
  final parts = path.split('/');
  if (parts.contains('__MACOSX')) return true;
  final base = parts.last;
  return base.startsWith('._') || base == '.DS_Store' || base == 'Thumbs.db';
}

/// Decode a ZIP entry name. Precedence per the ZIP spec and common tools:
/// the UTF-8 flag (bit 11), then the Info-ZIP Unicode Path extra field
/// (0x7075), then strict UTF-8 (most tools write UTF-8 without setting the
/// flag), and finally GBK — what Windows Explorer produces on a Chinese
/// locale.
String _decodeName(List<int> raw, int flags, List<int> extra) {
  if (flags & 0x800 != 0) return utf8.decode(raw, allowMalformed: true);
  var i = 0;
  while (i + 4 <= extra.length) {
    final id = extra[i] | (extra[i + 1] << 8);
    final size = extra[i + 2] | (extra[i + 3] << 8);
    if (id == 0x7075 && size >= 5 && i + 4 + size <= extra.length) {
      // 1-byte version + 4-byte name CRC, then the UTF-8 name.
      return utf8.decode(extra.sublist(i + 9, i + 4 + size),
          allowMalformed: true);
    }
    i += 4 + size;
  }
  try {
    final s = utf8.decode(raw);
    // GBK byte pairs are frequently *also* valid UTF-8, but then decode into
    // blocks essentially absent from real filenames (Latin Extended-B, IPA,
    // Greek symbols: U+0180–U+03FF). If that happens and GBK decodes cleanly,
    // it was GBK all along ("图片" → "ͼƬ").
    if (!s.runes.any((r) => r >= 0x0180 && r <= 0x03FF)) return s;
    final g = decodeGbk(raw);
    return g.contains('�') ? s : g;
  } on FormatException {
    return decodeGbk(raw);
  }
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
    final flags = bd.getUint16(i + 6, Endian.little);
    final method = bd.getUint16(i + 8, Endian.little);
    final compSize = bd.getUint32(i + 18, Endian.little);
    final uncompSize = bd.getUint32(i + 22, Endian.little);
    final nameLen = bd.getUint16(i + 26, Endian.little);
    final extraLen = bd.getUint16(i + 28, Endian.little);
    final nameStart = i + 30;
    final dataStart = nameStart + nameLen + extraLen;
    if (dataStart + compSize > data.length) break;
    final name = _decodeName(
      data.sublist(nameStart, nameStart + nameLen),
      flags,
      data.sublist(nameStart + nameLen, dataStart),
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
