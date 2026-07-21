import 'dart:typed_data';

/// One file headed into an archive (folder/multi-file import packing).
class ArchiveFile {
  ArchiveFile(this.name, this.bytes);
  final String name;
  final Uint8List bytes;
}
