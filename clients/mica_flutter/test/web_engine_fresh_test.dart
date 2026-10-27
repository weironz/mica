@TestOn('vm')
library;

import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'support/native_build.dart';

List<String> _fingerprintRoots(Directory repo) {
  final script = File(
    '${repo.path}/crates/md-wasm/build.rs',
  ).readAsStringSync();
  final section = RegExp(
    r'const ROOTS:\s*&\[&str\]\s*=\s*&\[([\s\S]*?)\];',
  ).firstMatch(script);
  if (section == null) throw StateError('Cannot read formal wasm ROOTS');
  final roots = RegExp(
    r'"([^"]+)"',
  ).allMatches(section[1]!).map((match) => match[1]!).toList();
  if (roots.isEmpty) throw StateError('Formal wasm ROOTS is empty');
  return roots;
}

Directory _repoRoot() {
  var dir = Directory.current.absolute;
  while (!File('${dir.path}/crates/md-wasm/build.rs').existsSync()) {
    final parent = dir.parent;
    if (parent.path == dir.path) {
      throw StateError('Formal crates/md-wasm/build.rs is missing');
    }
    dir = parent;
  }
  return dir;
}

void _verifyFingerprint(Directory repo, File stamp) {
  final roots = _fingerprintRoots(repo);
  for (final root in roots) {
    if (!FileSystemEntity.isFileSync('${repo.path}/$root') &&
        !FileSystemEntity.isDirectorySync('${repo.path}/$root')) {
      throw StateError('Missing Web engine fingerprint source: $root');
    }
  }
  final reported = stamp.readAsStringSync().trim();
  final expected = sourceFingerprint(repo, roots);
  if (reported != expected) {
    throw StateError(
      'stale Web Markdown engine: stamp $reported, checkout $expected; '
      'rebuild with bash scripts/build-md-wasm.sh',
    );
  }
}

void _copySource(FileSystemEntity source, String destination) {
  if (source is Directory) {
    Directory(destination).createSync(recursive: true);
    for (final child in source.listSync(followLinks: false)) {
      final name = child.uri.pathSegments.lastWhere((part) => part.isNotEmpty);
      _copySource(child, '$destination/$name');
    }
  } else if (source is File) {
    File(destination).parent.createSync(recursive: true);
    source.copySync(destination);
  }
}

void main() {
  final repo = _repoRoot();
  final roots = _fingerprintRoots(repo);
  test('fingerprint roots are read from the formal wasm build script', () {
    expect(roots, isNotEmpty);
    expect(roots, contains('crates/md-wasm/build.rs'));
  });

  test('built Web Markdown engine matches this checkout', () {
    final stamp = File(
      '${repo.path}/clients/mica_flutter/web/mica_md/SOURCE_FINGERPRINT',
    );
    if (!stamp.existsSync()) {
      markTestSkipped(
        'No production wasm artifact; run bash scripts/build-md-wasm.sh. Freshness gate NOT exercised.',
      );
      return;
    }
    _verifyFingerprint(repo, stamp);
  });

  test('wrong stamp and mutated real source fail; restored source passes', () {
    final temporary = Directory.systemTemp.createTempSync('mica_web_fresh_');
    try {
      // Copy the REAL formal source set, preserving relative paths and bytes.
      for (final root in roots) {
        final path = '${repo.path}/$root';
        _copySource(
          FileSystemEntity.isDirectorySync(path) ? Directory(path) : File(path),
          '${temporary.path}/$root',
        );
      }
      final stamp = File('${temporary.path}/SOURCE_FINGERPRINT');
      final expected = sourceFingerprint(temporary, roots);
      stamp.writeAsStringSync(expected);
      expect(() => _verifyFingerprint(temporary, stamp), returnsNormally);
      stamp.writeAsStringSync('incorrect');
      expect(() => _verifyFingerprint(temporary, stamp), throwsStateError);
      stamp.writeAsStringSync(expected);
      final source = File('${temporary.path}/crates/md-core/src/lib.rs');
      final original = source.readAsBytesSync();
      source.writeAsStringSync(
        '\n// freshness regression probe\n',
        mode: FileMode.append,
      );
      expect(() => _verifyFingerprint(temporary, stamp), throwsStateError);
      source.writeAsBytesSync(original);
      expect(() => _verifyFingerprint(temporary, stamp), returnsNormally);
      source.deleteSync();
      expect(() => _verifyFingerprint(temporary, stamp), throwsStateError);
    } finally {
      temporary.deleteSync(recursive: true);
    }
  });
}
