// Backup vs Markdown export (native format N3, docs/single-source/01-background.md §9.1).
//
// Both are ZIPs of the same `.md` files, assets and manifest. The BACKUP also
// carries each page's `.mica.json` — native blocks, block ids, comments — and
// is what restores a workspace losslessly; it is named `<name>.mica.zip` so a
// person can tell the two apart in a downloads folder. The MARKDOWN export is
// for other tools: no Mica-only files inside. Either imports back into Mica
// (the Markdown one through the Markdown path, so comments and block ids are
// not carried).
import 'dart:typed_data';

import 'package:flutter/material.dart';

import '../api/models.dart' show ApiException;
import '../editor/image_actions.dart' show downloadImage;
import '../l10n/locale_controller.dart';

enum ArchiveFormat {
  backup,
  markdown;

  /// The `?format=` value the server's tree exports take.
  String get query => name;

  /// `<base>.mica.zip` for a backup, `<base>.zip` for Markdown.
  String fileName(String name, String fallback) {
    final base = name.trim().isEmpty ? fallback : name.trim();
    return this == backup ? '$base.mica.zip' : '$base.zip';
  }
}

/// Build an archive and hand it to the user as a download, or say why not.
/// An empty archive is a failure, not a file worth saving.
Future<void> saveArchive(
  BuildContext context, {
  required Future<Uint8List> Function() build,
  required ArchiveFormat format,
  required String name,
  required String fallback,
}) async {
  final l10n = context.l10n;
  final messenger = ScaffoldMessenger.of(context);
  try {
    final bytes = await build();
    if (bytes.isEmpty) throw ApiException(l10n.exportEmptyContent);
    downloadImage(bytes, format.fileName(name, fallback), 'application/zip');
  } catch (error) {
    messenger.showSnackBar(SnackBar(content: Text(l10n.exportFailed('$error'))));
  }
}

/// The "Export" submenu with one entry per [ArchiveFormat]. [item] builds an
/// entry in the host menu's own style; [icon] / [iconColor] match its siblings.
Widget archiveFormatSubmenu(
  BuildContext context, {
  required Widget Function(IconData icon, String label, VoidCallback onTap) item,
  required void Function(ArchiveFormat format) onPick,
  Color? iconColor,
}) => SubmenuButton(
  leadingIcon: Icon(Icons.folder_zip_outlined, size: 18, color: iconColor),
  menuChildren: [
    for (final f in ArchiveFormat.values)
      item(
        Icons.folder_zip_outlined,
        context.l10n.archiveFormatLabel(f.name),
        () => onPick(f),
      ),
  ],
  child: Text(context.l10n.commonExport),
);
