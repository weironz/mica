// The web CRDT replica (Rust core as wasm) behind the sync seam. Browser-only:
// the body is in `web_doc_replica_test_web.dart`; on the VM this runs nothing.
// Run it with `flutter test --platform chrome test/web_doc_replica_test.dart`
// (scripts/browser-tests.sh does, with a receipt).
import 'web_idb_doc_store_test_vm.dart'
    if (dart.library.js_interop) 'web_doc_replica_test_web.dart';

void main() => main_();
