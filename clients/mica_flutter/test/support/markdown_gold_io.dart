import 'dart:io';

/// Locate checked-in gold from both VM tests and the Windows test executable.
Future<String> readMarkdownGold(String name) async {
  for (final start in [
    Directory.current,
    File(Platform.resolvedExecutable).parent,
  ]) {
    var directory = start.absolute;
    while (true) {
      final file = File(
        '${directory.path}/crates/md-core/tests/fixtures/wire/$name.json',
      );
      if (await file.exists()) return file.readAsString();
      final parent = directory.parent;
      if (parent.path == directory.path) break;
      directory = parent;
    }
  }
  throw StateError(
    'Missing wire gold $name.json: integrate WP-R fixtures under crates/md-core/tests/fixtures/wire',
  );
}
