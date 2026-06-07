// P2-M0 pipeline validation: prove the Flutter ↔ Rust (flutter_rust_bridge v2)
// round-trip works on the Windows runner — the native rust_lib_mica_flutter
// dylib (wrapping the shared crates/mica-core) is built by cargokit, bundled,
// loaded, and callable. This is the foundation the whole offline data plane
// (yrs CRDT, local store, sync) sits on; it must be green before any of that.
//
//   flutter test integration_test/frb_roundtrip_test.dart -d windows
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:mica_flutter/src/rust/api/simple.dart';
import 'package:mica_flutter/src/rust/frb_generated.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  setUpAll(() async => RustLib.init());
  tearDownAll(() async => RustLib.dispose());

  test('frb round-trip: string in/out via mica-core', () {
    expect(greet(name: 'Mica'), 'Hello from mica-core, Mica');
  });

  test('frb round-trip: integers across the boundary', () {
    expect(add(a: 2, b: 40), 42);
  });

  test('native core reports a version', () {
    expect(coreVersion(), isNotEmpty);
  });
}
