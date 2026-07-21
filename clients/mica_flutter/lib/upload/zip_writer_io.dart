// Desktop STORE-ZIP packing: delegate to the Rust encoder over FFI — the same
// `mica_interchange::zip::writer` the server and the export paths use, so the
// bytes come from the single authoritative implementation (原则 #2). Web keeps
// the pure-Dart twin (zip_writer_dart.dart); a shared gold fixture pins the
// two byte-for-byte.
import 'dart:typed_data';

import '../src/rust/api/zip.dart' as ffi;
import 'archive_file.dart';

/// Build a STORE (uncompressed) ZIP — the upload container for server-side
/// import. See zip_writer_dart.dart for the format notes.
Uint8List buildStoreZip(List<ArchiveFile> files) => ffi.buildStoreZip(
      entries: [
        for (final f in files) ffi.StoreZipEntry(name: f.name, bytes: f.bytes),
      ],
    );
