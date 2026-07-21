// The desktop STORE-ZIP path through the REAL FFI: the generated binding must
// carry a CJK filename and fully binary bytes across intact, and the Rust
// output must equal the pure-Dart reference writer byte-for-byte (both are
// separately pinned to the same gold fixture; this checks the live crossing).
//
//   flutter test integration_test/frb_zip_test.dart -d windows
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:mica_flutter/src/rust/frb_generated.dart';
import 'package:mica_flutter/upload/archive_file.dart';
import 'package:mica_flutter/upload/zip_writer_dart.dart' as dart_ref;
import 'package:mica_flutter/upload/zip_writer_io.dart' as ffi_path;

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(() async => RustLib.init());
  tearDownAll(() async => RustLib.dispose());

  test('FFI zip bytes equal the pure-Dart reference', () {
    final files = [
      ArchiveFile('README.md', Uint8List.fromList('# Mica\n'.codeUnits)),
      ArchiveFile(
        '笔记/图 1.png',
        Uint8List.fromList([for (var b = 0; b <= 255; b++) b]),
      ),
      ArchiveFile('empty.txt', Uint8List(0)),
    ];
    expect(
      ffi_path.buildStoreZip(files),
      dart_ref.buildStoreZip(files),
      reason: 'the FFI crossing must not perturb names (UTF-8) or bytes',
    );
  });
}
