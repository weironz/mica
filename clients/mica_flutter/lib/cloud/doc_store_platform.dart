// P4-2: platform doc-store factory. Desktop's store comes from LocalOffline
// (SQLite via FFI, opened elsewhere); this factory covers the WEB side — an
// IndexedDB-backed [WebIdbDocStore] — and returns null on desktop, where the
// caller already has the FFI store. Facade pattern mirrors cloud_sync.dart.
export 'doc_store_platform_io.dart'
    if (dart.library.html) 'doc_store_platform_web.dart';
