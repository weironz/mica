@TestOn('browser')
library;

import 'package:flutter_test/flutter_test.dart';
import 'markdown_codec_contract_body.dart';
import 'support/markdown_codec_web.dart';

void main() => runCodecContract(loadBrowserMarkdownCodec);
