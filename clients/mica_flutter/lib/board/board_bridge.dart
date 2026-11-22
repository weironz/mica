// Platform-agnostic bridge to Rust `crates/mica-board` (Desktop FFI / Web WASM).
import 'dart:typed_data';

import 'board_bridge_io.dart'
    if (dart.library.js_interop) 'board_bridge_web.dart'
    as impl;

class BoardThemeInput {
  const BoardThemeInput({
    required this.dark,
    required this.canvas,
    required this.surface,
    required this.text,
    required this.subtleText,
    required this.border,
    required this.accent,
  });

  final bool dark;
  final String canvas;
  final String surface;
  final String text;
  final String subtleText;
  final String border;
  final String accent;

  Map<String, dynamic> toJson() => {
    'dark': dark,
    'canvas': canvas,
    'surface': surface,
    'text': text,
    'subtleText': subtleText,
    'border': border,
    'accent': accent,
  };
}

Future<String?> boardRenderSvg(String source, {BoardThemeInput? theme}) =>
    impl.boardRenderSvg(source, theme: theme);

Future<String?> boardRouteEdgesJson(String source) =>
    impl.boardRouteEdgesJson(source);

Future<String?> boardImportDrawio(Uint8List bytes) =>
    impl.boardImportDrawio(bytes);

Future<String> boardMergeJson(String base, String local, String remote) =>
    impl.boardMergeJson(base, local, remote);

Future<String?> boardMarkdownToMindmap(String markdown, {String? title}) =>
    impl.boardMarkdownToMindmap(markdown, title: title);

Future<String?> boardMindmapToMarkdown(String source) =>
    impl.boardMindmapToMarkdown(source);

/// Cross-platform stateful handle over Rust `crates/mica-board::BoardSession`
/// (synchronous FFI on desktop, lazy on-demand WASM on Web).
abstract class BoardSessionBridge {
  String currentStatePatchJson();
  String dispatchJson(String cmdJson);
  String pointerMoveJson(
    double worldX,
    double worldY,
    double screenX,
    double screenY, {
    bool altPressed = false,
  });
  String exportCompactJson();
  String renderSvg({BoardThemeInput? theme});
  void dispose();
}

Future<BoardSessionBridge?> openBoardSession(String snapshotJson) =>
    impl.openBoardSession(snapshotJson);
