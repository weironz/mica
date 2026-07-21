// STORE-ZIP packing facade. Desktop routes to the Rust encoder over FFI
// (zip_writer_io.dart — same `mica_interchange` writer the server uses); web
// keeps the pure-Dart reference implementation (zip_writer_dart.dart). The two
// are pinned byte-for-byte by a shared gold fixture — see
// test/zip_writer_conformance_test.dart.
export 'archive_file.dart';
export 'zip_writer_io.dart' if (dart.library.html) 'zip_writer_dart.dart';
