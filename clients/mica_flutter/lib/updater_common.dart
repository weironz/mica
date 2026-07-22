/// Shared, platform-agnostic bits of the self-updater (no dart:io / dart:html),
/// imported by both the desktop and web variants of `updater.dart`.
library;

/// GitHub repo that publishes the releases (owner/name).
const String kUpdateRepo = 'weironz/mica';

/// A newer release worth offering the user.
class UpdateInfo {
  const UpdateInfo({
    required this.version,
    required this.downloadUrl,
    this.notes,
    this.size,
    this.sha256,
  });

  /// Version of the latest release, without the leading `v` (e.g. `0.1.6`).
  final String version;

  /// Direct download URL of the `Mica-Setup-*.exe` asset.
  final String downloadUrl;

  /// The release notes (markdown), if any.
  final String? notes;

  /// Expected byte size of the asset (GitHub's `assets[].size`). The download is
  /// rejected unless it matches — a truncated installer must never be run.
  final int? size;

  /// Expected SHA-256 of the asset as lowercase hex, parsed from GitHub's
  /// `assets[].digest` (`sha256:…`) when present. Verified before the installer
  /// is launched, so a swapped/corrupted download can't be executed. Null when
  /// the release predates GitHub asset digests (then only [size] is checked).
  final String? sha256;
}

/// Compare two dotted versions numerically: >0 if [a] is newer than [b], <0 if
/// older, 0 if equal. Tolerates differing component counts (`0.1` vs `0.1.5`)
/// and trailing non-numeric junk on a component (`1.2.0-rc1` → treats as `1.2.0`).
int compareVersions(String a, String b) {
  final pa = _parts(a);
  final pb = _parts(b);
  final n = pa.length > pb.length ? pa.length : pb.length;
  for (var i = 0; i < n; i++) {
    final x = i < pa.length ? pa[i] : 0;
    final y = i < pb.length ? pb[i] : 0;
    if (x != y) return x < y ? -1 : 1;
  }
  return 0;
}

List<int> _parts(String v) {
  final trimmed = v.trim().replaceFirst(RegExp(r'^[vV]'), '');
  return trimmed.split('.').map((seg) {
    final m = RegExp(r'^\d+').firstMatch(seg);
    return m == null ? 0 : int.parse(m.group(0)!);
  }).toList();
}
