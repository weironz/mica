import 'package:integration_test/integration_test.dart';
import 'package:mica_flutter/markdown/markdown_codec.dart';
import '../test/markdown_codec_contract_body.dart';
import '../test/support/markdown_codec.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  runCodecContract(() async {
    await useNativeMarkdownCodec();
    return markdownCodec;
  });
}
