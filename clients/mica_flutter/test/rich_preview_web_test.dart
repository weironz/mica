@TestOn('browser')
library;

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/preview_engine_web.dart';

import 'support/rich_preview_cases.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  previewWorkerUrl = 'support/mica_preview_worker.js';

  richPreviewCases();
}
