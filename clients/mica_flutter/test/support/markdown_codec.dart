import 'package:mica_flutter/markdown/markdown_codec.dart';
import 'package:mica_flutter/markdown/markdown_codec_io.dart' as platform;
import 'native_build.dart';

/// Install the real FFI codec only after verifying this checkout's library.
Future<void> useNativeMarkdownCodec() async {
  await initCheckedNativeRust();
  debugSetMarkdownCodec(await platform.createMarkdownCodec());
}
