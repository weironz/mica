import 'package:flutter_test/flutter_test.dart';

import 'support/native_build.dart';
import 'support/rich_preview_cases.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(initCheckedNativeRust);
  richPreviewCases();
}
