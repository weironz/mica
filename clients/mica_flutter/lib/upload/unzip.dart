import 'dart:convert';
import 'dart:typed_data';

/// One file read out of a ZIP archive.
class ZipFileEntry {
  ZipFileEntry(this.name, this.bytes);
  final String name;
  final Uint8List bytes;
}

/// Read a STORE (uncompressed) ZIP into its files. This matches what Mica's
/// exporter writes; deflate-compressed entries are skipped (no inflater here).
List<ZipFileEntry> readStoreZip(Uint8List data) {
  final bd = ByteData.sublistView(data);
  final out = <ZipFileEntry>[];
  var i = 0;
  while (i + 30 <= data.length) {
    final sig = bd.getUint32(i, Endian.little);
    if (sig != 0x04034b50) break; // central directory / end of entries
    final method = bd.getUint16(i + 8, Endian.little);
    final compSize = bd.getUint32(i + 18, Endian.little);
    final nameLen = bd.getUint16(i + 26, Endian.little);
    final extraLen = bd.getUint16(i + 28, Endian.little);
    final nameStart = i + 30;
    final dataStart = nameStart + nameLen + extraLen;
    if (dataStart + compSize > data.length) break;
    final name = utf8.decode(
      data.sublist(nameStart, nameStart + nameLen),
      allowMalformed: true,
    );
    if (method == 0 && !name.endsWith('/')) {
      out.add(ZipFileEntry(
        name,
        Uint8List.fromList(data.sublist(dataStart, dataStart + compSize)),
      ));
    }
    i = dataStart + compSize;
  }
  return out;
}
