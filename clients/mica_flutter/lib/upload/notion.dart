/// Notion export adaptation — everything Notion-specific about workspace
/// import lives here. The shared import core stays format-neutral and only
/// consults these helpers when Notion mode is on (chosen in the UI or
/// auto-detected via [looksLikeNotionExport]).
library;

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

/// True when the archive looks like a Notion export: at least half of the
/// markdown files (and at least one) carry an ID suffix. Keeps standard
/// archives with the odd hash-named file from being mangled.
bool looksLikeNotionExport(Iterable<String> mdPaths) {
  var total = 0, ids = 0;
  for (final p in mdPaths) {
    total++;
    final base = p
        .substring(p.lastIndexOf('/') + 1)
        .replaceAll(RegExp(r'\.md$', caseSensitive: false), '');
    if (stripNotionId(base) != base) ids++;
  }
  return ids >= 1 && ids * 2 >= total;
}

/// Map each folder path to the md page that represents it. With [notion]
/// mode on, matching tolerates ID suffixes per segment — folder `apple/`
/// matches the page exported as `apple 31f5<…32 hex>.md`; off, it is exact
/// (`Guide/` ↔ `Guide.md`).
Map<String, String> folderPageIndex(
  Iterable<String> mdPaths, {
  required bool notion,
}) {
  String seg(String s) => notion ? stripNotionId(s) : s;
  final out = <String, String>{};
  for (final p in mdPaths) {
    final cut = p.lastIndexOf('/');
    final dir = cut < 0 ? '' : p.substring(0, cut);
    final base = p
        .substring(cut + 1)
        .replaceAll(RegExp(r'\.md$', caseSensitive: false), '');
    final normDir = dir.split('/').map(seg).join('/');
    final key = dir.isEmpty ? seg(base) : '$normDir/${seg(base)}';
    out.putIfAbsent(key, () => p);
  }
  return out;
}
