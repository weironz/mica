import 'dart:convert';

/// Order markdown paths for workspace import.
///
/// Paths listed in the export's `manifest.json` come first, in manifest
/// (pre-order page-tree) order — that restores the original sibling order.
/// Files the manifest doesn't know about (hand-added to the archive) follow,
/// parents-first (shallower paths first) and natural-sorted, so `2 < 10`.
List<String> orderPagePaths(Iterable<String> mdPaths, String? manifestJson) {
  final manifestIndex = <String, int>{};
  if (manifestJson != null) {
    try {
      final m = jsonDecode(manifestJson);
      final pages = m is Map ? m['pages'] : null;
      if (pages is List) {
        var i = 0;
        for (final p in pages) {
          final path = p is Map ? p['path'] : null;
          if (path is String) manifestIndex[path] = i++;
        }
      }
    } catch (_) {
      // Malformed manifest → fall back to depth + natural order.
    }
  }
  final list = mdPaths.toList();
  list.sort((a, b) {
    final ia = manifestIndex[a];
    final ib = manifestIndex[b];
    if (ia != null || ib != null) {
      if (ia == null) return 1;
      if (ib == null) return -1;
      return ia.compareTo(ib);
    }
    final da = '/'.allMatches(a).length;
    final db = '/'.allMatches(b).length;
    if (da != db) return da - db;
    return naturalCompare(a, b);
  });
  return list;
}

/// Strip the ID Notion appends to exported file/folder names —
/// `My Page 1f2e3d4c5b6a7890abcdef1234567890` (32 hex) or a dashed UUID —
/// so imported pages get clean titles. No-op for ordinary names.
String stripNotionId(String segment) {
  return segment
      .replaceFirst(RegExp(r'[ \-_]+[0-9a-fA-F]{32}$'), '')
      .replaceFirst(
        RegExp(r'[ \-_]+[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}'
            r'-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$'),
        '',
      );
}

/// Normalize a path for folder↔page matching: strip the Notion ID from
/// every segment.
String normalizeNotionPath(String path) =>
    path.split('/').map(stripNotionId).join('/');

/// Map each folder (normalized path, Notion IDs stripped) to the md page
/// that represents it. Handles both layouts:
/// - Mica / common: `Guide/...` next to `Guide.md`
/// - Notion: folder `apple/` next to `apple 31f5<…32 hex>.md`
Map<String, String> folderPageIndex(Iterable<String> mdPaths) {
  final out = <String, String>{};
  for (final p in mdPaths) {
    final cut = p.lastIndexOf('/');
    final dir = cut < 0 ? '' : p.substring(0, cut);
    final base = p
        .substring(cut + 1)
        .replaceAll(RegExp(r'\.md$', caseSensitive: false), '');
    final key = dir.isEmpty
        ? stripNotionId(base)
        : '${normalizeNotionPath(dir)}/${stripNotionId(base)}';
    out.putIfAbsent(key, () => p);
  }
  return out;
}

/// Compare strings with digit runs ordered numerically (`2.md` < `10.md`).
int naturalCompare(String a, String b) {
  var i = 0, j = 0;
  bool isDigit(int c) => c >= 0x30 && c <= 0x39;
  while (i < a.length && j < b.length) {
    final ca = a.codeUnitAt(i);
    final cb = b.codeUnitAt(j);
    if (isDigit(ca) && isDigit(cb)) {
      var i2 = i, j2 = j;
      while (i2 < a.length && isDigit(a.codeUnitAt(i2))) {
        i2++;
      }
      while (j2 < b.length && isDigit(b.codeUnitAt(j2))) {
        j2++;
      }
      final na = int.parse(a.substring(i, i2));
      final nb = int.parse(b.substring(j, j2));
      if (na != nb) return na - nb;
      i = i2;
      j = j2;
    } else {
      if (ca != cb) return ca - cb;
      i++;
      j++;
    }
  }
  return (a.length - i) - (b.length - j);
}
