import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/ui/export_format.dart';

void main() {
  test('a backup is named .mica.zip, a Markdown export plain .zip', () {
    expect(ArchiveFormat.backup.fileName('笔记', 'workspace'), '笔记.mica.zip');
    expect(ArchiveFormat.markdown.fileName(' 笔记 ', 'workspace'), '笔记.zip');
    expect(ArchiveFormat.backup.fileName('  ', 'folder'), 'folder.mica.zip');
  });

  test('the query values are what the server accepts', () {
    expect(ArchiveFormat.values.map((f) => f.query), ['backup', 'markdown']);
  });
}
