// One dart2js build serves every browser regression in ci.yml's web-e2e job.
// The three harness entrypoints stay separate Dart libraries; this entry
// dispatches on the ?harness= query parameter so the job needs a single
// `flutter build web` (plus the already-built main bundle for the stack
// assertions) instead of four. Keep flags in sync with ci.yml.
import 'package:mica_flutter/markdown/markdown_codec.dart';

import 'clipboard_harness.dart' as clipboard;
import 'sidebar_rename_harness.dart' as sidebar_rename;
import 'ui_experience_harness.dart' as ui_experience;

Future<void> main() async {
  // The harness pages render the real editor, whose copy/paste paths go
  // through the Markdown codec (wasm since S2). Without this the first
  // Ctrl+C/Ctrl+V throws "initMarkdownCodec() was not awaited" and the
  // clipboard regression silently sees plain-only clips. Production does the
  // same call in lib/main.dart before runApp.
  await initMarkdownCodec();
  switch (Uri.base.queryParameters['harness']) {
    case 'clipboard':
      clipboard.main();
    case 'sidebar_rename':
      sidebar_rename.main();
    case 'ui_experience':
      ui_experience.main();
    default:
      throw StateError(
        'Unknown web harness: use ?harness=clipboard|sidebar_rename|ui_experience',
      );
  }
}
