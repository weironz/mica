import 'package:mica_flutter/markdown/markdown_codec.dart';
import 'markdown_codec_contract_body.dart';
import 'support/markdown_codec.dart';

void main() {
  runCodecContract(() async {
    await useNativeMarkdownCodec();
    return markdownCodec;
  });
}
