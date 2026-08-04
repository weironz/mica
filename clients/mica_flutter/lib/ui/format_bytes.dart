// Byte counts as people read them.
//
// A pure function in its own file because the rounding is the whole point and it
// is only wrong at the boundaries — exactly what a test can pin and a glance at
// the UI cannot.

/// A byte count as a short human string: `948 KB`, `1.2 MB`, `128 MB`.
///
/// Decimal units (KB = 1000), not binary. That is what every OS file manager and
/// download UI shows, and this number sits next to "export" — the user will
/// compare it against what their browser reports, not against `ls -l`.
///
/// One decimal place below 10 and none above, because `1.2 MB` is worth the
/// character and `128.4 MB` is not: the difference it expresses is smaller than
/// the compression the zip will apply anyway.
String formatBytes(int bytes) {
  if (bytes <= 0) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  var value = bytes.toDouble();
  var unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit++;
  }
  // Bytes are never fractional; above that, one decimal only while it still says
  // something.
  final text = unit == 0
      ? value.toStringAsFixed(0)
      : (value < 10 ? value.toStringAsFixed(1) : value.toStringAsFixed(0));
  return '$text ${units[unit]}';
}

/// A byte count in BINARY units: `948 KiB`, `1.0 GiB`.
///
/// A second formatter rather than a flag on the first, because the choice is
/// not cosmetic — it follows from what the number gets compared against.
/// [formatBytes] sits next to "export", where the user checks it against what
/// their browser reported, and browsers are decimal. THIS one sits next to a
/// quota that is configured, documented and enforced in GiB
/// (`MICA_WORKSPACE_QUOTA_BYTES`, 1 GiB by default). Rendering that with
/// decimal units printed **1.1 GB** for a limit everything else calls 1 GiB —
/// a number the user cannot reconcile with the docs, and the kind of small
/// mismatch that reads as a bug in the quota rather than in the label.
String formatBytesBinary(int bytes) {
  if (bytes <= 0) return '0 B';
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
  var value = bytes.toDouble();
  var unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  final text = unit == 0
      ? value.toStringAsFixed(0)
      : (value < 10 ? value.toStringAsFixed(1) : value.toStringAsFixed(0));
  return '$text ${units[unit]}';
}
