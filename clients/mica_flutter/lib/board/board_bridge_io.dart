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

class _IoBoardSessionBridge implements BoardSessionBridge {
  _IoBoardSessionBridge(this._handle);

  final rust.BoardSessionHandle _handle;

  @override
  String currentStatePatchJson() => _handle.currentStatePatchJson();

  @override
  String dispatchJson(String cmdJson) => _handle.dispatchJson(cmdJson: cmdJson);

  @override
  String pointerMoveJson(
    double worldX,
    double worldY,
    double screenX,
    double screenY, {
    bool altPressed = false,
  }) => _handle.pointerMoveJson(
    worldX: worldX,
    worldY: worldY,
    screenX: screenX,
    screenY: screenY,
    altPressed: altPressed,
  );

  @override
  String exportCompactJson() => _handle.exportCompactJson();

  @override
  String renderSvg({BoardThemeInput? theme}) => _handle.renderSvg(
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

  @override
  void dispose() {
    _handle.dispose();
  }
}

Future<BoardSessionBridge?> openBoardSession(String snapshotJson) async {
  if (!_rustReady) return null;
  try {
    return _IoBoardSessionBridge(
      rust.BoardSessionHandle.fromJson(snapshotJson: snapshotJson),
    );
  } catch (_) {
    return null;
  }
}
