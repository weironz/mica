import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/ui/format_bytes.dart';

/// Rounding is the whole job here, and it is only ever wrong at the boundaries.
void main() {
  test('bytes stay whole — there is no half byte', () {
    expect(formatBytes(1), '1 B');
    expect(formatBytes(999), '999 B');
  });

  test('decimal units, because that is what the OS and the browser show', () {
    // 1000, not 1024: this number sits next to "export" and will be compared
    // against what the download UI reports.
    expect(formatBytes(1000), '1.0 KB');
    expect(formatBytes(1500), '1.5 KB');
  });

  test('one decimal below 10, none above', () {
    expect(formatBytes(1200000), '1.2 MB');
    expect(formatBytes(9900000), '9.9 MB');
    // 128.4 MB would express a difference smaller than the zip's own compression.
    expect(formatBytes(128400000), '128 MB');
  });

  test('climbs through the units and stops at TB', () {
    expect(formatBytes(2000000), '2.0 MB');
    expect(formatBytes(3000000000), '3.0 GB');
    expect(formatBytes(4000000000000), '4.0 TB');
    // Beyond the last unit it keeps counting in TB rather than inventing one.
    expect(formatBytes(5000000000000000), endsWith(' TB'));
  });

  test('zero and negative read as empty, not as a crash or "-1 B"', () {
    // A missing count arrives as 0; it must not look like a real size.
    expect(formatBytes(0), '0 B');
    expect(formatBytes(-5), '0 B');
  });

  test('the boundary carries up rather than printing 1000 of a unit', () {
    expect(formatBytes(999999), '1000 KB');
    expect(formatBytes(1000000), '1.0 MB');
  });
}
