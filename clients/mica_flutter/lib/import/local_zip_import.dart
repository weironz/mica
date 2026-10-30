// Local-mode workspace ZIP import: read the archive and decide the new
// workspace's name, before anything is created.
//
// The archive itself is read in Rust (`unpackImportZip`), through the same
// reader and normalization the server import uses, so a ZIP lands as the same
// tree in both worlds. This file only owns what happens around that call:
// reporting an unreadable archive, refusing one with no pages BEFORE an empty
// workspace exists, and the naming rule. A separate library rather than more
// lines in `main.dart`, which the size gate holds to "only shrink".

import 'dart:typed_data';

import 'package:flutter/material.dart';

import '../l10n/app_localizations.dart';
import '../local/local_offline_api.dart';
import '../upload/archive_file.dart';
import 'import_controller.dart';

/// Read [bytes] for a local import. Returns the workspace name and entries, or
/// null after telling the user why there is nothing to import.
Future<({String name, List<ArchiveFile> entries})?> prepareLocalZipImport(
  LocalOfflineApi local,
  String fileName,
  Uint8List bytes, {
  required AppLocalizations l10n,
  required ScaffoldMessengerState? messenger,
}) async {
  final ({
    String? wrapperName,
    int markdownCount,
    List<({String path, List<int> bytes})> entries,
  })
  unpacked;
  try {
    unpacked = await local.unpackImportZip(bytes);
  } catch (e) {
    messenger?.showSnackBar(SnackBar(content: Text(l10n.importFailed('$e'))));
    return null;
  }
  if (unpacked.markdownCount == 0) {
    messenger?.showSnackBar(SnackBar(content: Text(l10n.importNoMarkdownHint)));
    return null;
  }
  return (
    name: localZipWorkspaceName(fileName, unpacked.wrapperName),
    entries: [
      for (final e in unpacked.entries)
        ArchiveFile(
          e.path,
          // Already a Uint8List from the FFI; no copy on the common path.
          e.bytes is Uint8List
              ? e.bytes as Uint8List
              : Uint8List.fromList(e.bytes),
        ),
    ],
  );
}

/// The new workspace's name, by the server's precedence: the archive's file
/// name (Notion's id suffix removed), then the peeled wrapper folder.
String localZipWorkspaceName(String fileName, String? wrapperName) {
  final fromFile = ImportController.cleanArchiveName(fileName);
  if (fromFile.isNotEmpty) return fromFile;
  final wrapper = wrapperName?.trim() ?? '';
  return wrapper.isNotEmpty ? wrapper : 'Imported';
}
