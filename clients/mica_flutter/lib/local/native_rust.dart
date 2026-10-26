import 'dart:io';

import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart'
    show ExternalLibrary;

import '../src/rust/frb_generated.dart';

/// Load the native library shipped beside the executable before considering
/// FRB's development fallback. Its generated default points at
/// `rust/target/release`, which can exist but belong to an older build when a
/// Windows integration test runs from the project directory.
Future<void> initNativeRust() async {
  final executable = File(Platform.resolvedExecutable);
  final name = Platform.isWindows
      ? 'rust_lib_mica_flutter.dll'
      : Platform.isMacOS
      ? 'librust_lib_mica_flutter.dylib'
      : 'librust_lib_mica_flutter.so';
  final bundled = File('${executable.parent.path}/$name');
  await RustLib.init(
    externalLibrary: bundled.existsSync()
        ? ExternalLibrary.open(bundled.absolute.path)
        : null,
  );
}
