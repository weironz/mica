import 'dart:js_interop';
import 'package:web/web.dart' as web;

Future<String> readMarkdownGold(String name) async {
  final response = await web.window
      .fetch('support/mica_md/gold/$name.json'.toJS)
      .toDart;
  if (!response.ok) {
    throw StateError(
      'Missing wire gold $name.json (HTTP ${response.status}); run scripts/build-md-wasm.sh',
    );
  }
  return (await response.text().toDart).toDart;
}
