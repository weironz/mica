import 'dart:typed_data';

import '../src/rust/api/board.dart' as rust;
import '../src/rust/frb_generated.dart';
import 'board_bridge.dart';

bool get _rustReady => RustLib.instance.initialized;

Future<String?> boardRenderSvg(String source, {BoardThemeInput? theme}) async {
  if (!_rustReady) return null;
  try {
    return await rust.renderBoardSvg(
      source: source,
      theme: theme == null
          ? null
          : rust.BoardTheme(
              dark: theme.dark,
              canvas: theme.canvas,
              surface: theme.surface,
              text: theme.text,
              subtleText: theme.subtleText,
              border: theme.border,
              accent: theme.accent,
            ),
    );
  } catch (_) {
    return null;
  }
}

Future<String?> boardRouteEdgesJson(String source) async {
  if (!_rustReady) return null;
  try {
    return await rust.routeBoardEdges(source: source);
  } catch (_) {
    return null;
  }
}

Future<String?> boardImportDrawio(Uint8List bytes) async {
  if (!_rustReady) return null;
  return await rust.importDrawioToBoard(bytes: bytes);
}

Future<String> boardMergeJson(String base, String local, String remote) async {
  if (!_rustReady) return local;
  try {
    return await rust.mergeBoardJson(base: base, local: local, remote: remote);
  } catch (_) {
    return local;
  }
}

Future<String?> boardMarkdownToMindmap(String markdown, {String? title}) async {
  if (!_rustReady) return null;
  try {
    return await rust.markdownToMindmapBoard(markdown: markdown, title: title);
  } catch (_) {
    return null;
  }
}

Future<String?> boardMindmapToMarkdown(String source) async {
  if (!_rustReady) return null;
  try {
    return await rust.mindmapBoardToMarkdown(source: source);
  } catch (_) {
    return null;
  }
}
