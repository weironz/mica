// Load the Rust FFI library for a test AND prove it was built from this
// checkout.
//
// Nothing else ties the library on disk to the source tree: FRB loads
// whatever sits beside the executable or in `rust/target/release`, and a
// library left over from an older build loads fine and quietly tests old
// code. `rust/build.rs` bakes a fingerprint of its own sources into the
// binary; this recomputes it from the working tree and refuses a mismatch.
// The algorithm mirrors build.rs byte for byte (FNV-1a 64 over the sorted
// files under the roots the library itself reports).

import 'dart:ffi';
import 'dart:io';
import 'dart:typed_data';

import 'package:mica_flutter/local/native_rust.dart';

Future<void> initCheckedNativeRust() async {
  await initNativeRust();
  final library =
      bundledNativeLibrary()?.absolute.path ??
      // FRB's generated fallback (`ioDirectory` in frb_generated.dart).
      File('rust/target/release/$nativeLibraryFileName').absolute.path;
  final reported = _readFingerprint(library);
  final expected = sourceFingerprint(_repoRoot(), reported.roots);
  if (reported.hash != expected) {
    throw StateError(
      'stale native library: $library was built from different sources '
      '(library ${reported.hash}, checkout $expected). Rebuild it: '
      'cargo build --release --manifest-path clients/mica_flutter/rust/Cargo.toml',
    );
  }
}

({String hash, List<String> roots}) _readFingerprint(String library) {
  final Pointer<Uint8> Function() symbol;
  try {
    symbol = DynamicLibrary.open(library)
        .lookupFunction<Pointer<Uint8> Function(), Pointer<Uint8> Function()>(
          'mica_ffi_source_fingerprint',
        );
  } on ArgumentError catch (error) {
    throw StateError(
      'cannot verify $library: no source fingerprint ($error). '
      'It predates the fingerprint or is not the library FRB loaded.',
    );
  }
  final pointer = symbol();
  var length = 0;
  while (pointer[length] != 0) {
    length++;
  }
  final text = String.fromCharCodes(pointer.asTypedList(length));
  final newline = text.indexOf('\n');
  return (
    hash: text.substring(0, newline),
    roots: text.substring(newline + 1).split(';'),
  );
}

/// The repo root: the nearest ancestor holding the FFI crate, searched from
/// the working directory (unit tests run in `clients/mica_flutter`) and then
/// from the executable (an integration-test app under `build/`).
Directory _repoRoot() {
  for (final start in [
    Directory.current,
    File(Platform.resolvedExecutable).parent,
  ]) {
    var dir = start.absolute;
    while (true) {
      if (File(
        '${dir.path}/clients/mica_flutter/rust/Cargo.toml',
      ).existsSync()) {
        return dir;
      }
      final parent = dir.parent;
      if (parent.path == dir.path) break;
      dir = parent;
    }
  }
  throw StateError(
    'no repo root above ${Directory.current.path} or the executable',
  );
}

String sourceFingerprint(Directory repo, List<String> roots) {
  final files = <String>[];
  void collect(String rel) {
    final path = '${repo.path}/$rel';
    if (FileSystemEntity.isDirectorySync(path)) {
      for (final entry in Directory(path).listSync(followLinks: false)) {
        collect(
          '$rel/${entry.uri.pathSegments.lastWhere((s) => s.isNotEmpty)}',
        );
      }
    } else if (FileSystemEntity.isFileSync(path)) {
      files.add(rel);
    }
  }

  roots.forEach(collect);
  // build.rs sorts Rust `String`s (bytes); every path here is ASCII, where
  // UTF-16 code-unit order is the same order.
  files.sort();
  const prime = 0x100000001b3;
  var hash = 0xcbf29ce484222325;
  void feed(List<int> bytes) {
    for (final b in bytes) {
      hash = (hash ^ b) * prime; // Dart ints wrap at 64 bits, as u64 does.
    }
  }

  for (final rel in files) {
    feed(rel.codeUnits);
    feed(Uint8List(1));
    feed(File('${repo.path}/$rel').readAsBytesSync());
    feed(Uint8List(1));
  }
  // Dart ints are signed 64-bit, so print the two unsigned halves.
  String half(int v) => (v & 0xffffffff).toRadixString(16).padLeft(8, '0');
  return half(hash >> 32) + half(hash);
}
