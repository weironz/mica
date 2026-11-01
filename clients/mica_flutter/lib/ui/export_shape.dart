// Which shape a single-page export takes: a lone `.md`, or a `.zip` with the
// page's images under `assets/`.
//
// Decided from the document's own blocks. It used to be decided by scanning
// the exported Markdown TEXT with a regex for `![…](…)` — a Markdown parse in
// Dart, which also counted an image example inside a code block as a real
// image. The blocks say directly which images exist and where they live.

/// True when [blocks] (a snapshot's `payload['blocks']`) hold an image that
/// the `.md` alone could not carry: one stored in Mica (it has a `file_id`
/// and exports as `assets/…`) or one whose `url` is a relative path. Images
/// at `http(s):` / `data:` URLs travel inside the Markdown.
bool pageNeedsAssetBundle(List<Object?> blocks) {
  for (final block in blocks) {
    if (block is! Map || block['type'] != 'image') continue;
    final data = block['data'];
    if (data is! Map) continue;
    final fileId = data['file_id'];
    if (fileId is String && fileId.isNotEmpty) return true;
    final url = data['url'];
    if (url is String &&
        url.isNotEmpty &&
        !url.startsWith('http://') &&
        !url.startsWith('https://') &&
        !url.startsWith('data:')) {
      return true;
    }
  }
  return false;
}
