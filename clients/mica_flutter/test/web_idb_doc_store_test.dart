// Web-only IndexedDB store tests. The body lives in
// `web_idb_doc_store_test_web.dart` (imports dart:js_interop, so it cannot
// compile on the VM); the VM stub keeps `flutter test` green by running nothing.
// Run the real suite with `flutter test -d chrome test/web_idb_doc_store_test.dart`.
import 'web_idb_doc_store_test_vm.dart'
    if (dart.library.js_interop) 'web_idb_doc_store_test_web.dart';

void main() => main_();
