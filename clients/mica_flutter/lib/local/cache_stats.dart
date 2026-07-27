// Telling the on-device cache apart from the on-device originals.
//
// The local store holds two different things in one place, and only one of them
// is a cache:
//
//   * **Cloud mirrors** — pages and images copied down from a server so they can
//     be read offline. Deleting them costs a re-download and nothing else.
//   * **Local workspaces** — pages that exist ONLY on this device. No server has
//     ever seen them. Deleting them is data loss.
//
// Reporting one number over both would invite someone to "clear the cache" and
// lose the only copy of something, so the split has to be decidable rather than
// guessed. It is: the blob store keys the two by different id formats (see
// `local_offline_io.dart`) — a local image is content-addressed by its sha256,
// while a mirrored one keeps the server's UUID file id.

/// Whether a blob id in `blobs/` belongs to a cloud mirror (and is therefore
/// re-downloadable) rather than to a local-only document.
///
/// Decided by id SHAPE, which is what the store already relies on to let the two
/// schemes share one directory: `8-4-4-4-12` hex with dashes is a server file id;
/// 64 unbroken hex characters is a local sha256. Anything else counts as local —
/// the safe direction, because being wrong costs a re-download one way and a lost
/// original the other.
bool isCloudMirrorBlobId(String id) {
  if (id.length != 36) return false;
  const dashes = [8, 13, 18, 23];
  for (final i in dashes) {
    if (id[i] != '-') return false;
  }
  for (var i = 0; i < id.length; i++) {
    if (dashes.contains(i)) continue;
    if (!_isHexDigit(id.codeUnitAt(i))) return false;
  }
  return true;
}

bool _isHexDigit(int c) =>
    (c >= 0x30 && c <= 0x39) || // 0-9
    (c >= 0x61 && c <= 0x66) || // a-f
    (c >= 0x41 && c <= 0x46); // A-F

/// What the on-device store is holding, split so the cache part can be reported
/// — and one day reclaimed — without touching the originals.
typedef LocalCacheStats = ({
  /// Pages mirrored from a server: re-downloadable.
  int mirroredPages,

  /// Bytes of mirrored images: re-downloadable.
  int mirroredBytes,

  /// Pages that live only on this device. Reported so the total is honest about
  /// what the store holds, never offered for deletion.
  int localOnlyPages,

  /// Bytes of images belonging to local-only documents.
  int localOnlyBytes,
});
