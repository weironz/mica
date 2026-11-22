// Embedded whiteboard / mind map previewer and deferred fullscreen editor launcher.
// Keeps the full board UI (`lib/board/board_editor_screen.dart`) out of the
// initial Web bundle (`deferred as board_ui` -> 0 B cold-start cost on text pages).
import 'package:flutter/material.dart';

import '../board/board_bridge.dart';
import '../board/board_editor_screen.dart' deferred as board_ui;
import '../board/board_models.dart';
import '../ui/deferred_load.dart';
import '../ui/theme_tokens.dart';
import 'board_inline_layout.dart';
import 'mermaid_theme.dart' show cssHex;
import 'preview_raster.dart';
import 'svg_raster.dart';

final _svgWidthAttr = RegExp(r'\bwidth="(\d+(?:\.\d+)?)"');

class BoardPreviewer extends RasterPreviewer {
  const BoardPreviewer();

  @override
  String get id => 'board';

  @override
  Future<RasterPreview?> produce(
    String source,
    double targetWidth,
    MicaTokens tokens,
  ) async {
    try {
      final raw = await boardRenderSvg(
        source,
        theme: BoardThemeInput(
          dark: tokens.dark,
          canvas: cssHex(tokens.surface.base),
          surface: cssHex(tokens.surface.raised),
          text: cssHex(tokens.text.primary),
          subtleText: cssHex(tokens.text.muted),
          border: cssHex(tokens.border.normal),
          accent: cssHex(tokens.border.strong),
        ),
      );
      if (raw == null || raw.trim().isEmpty) return null;
      final wm = _svgWidthAttr.firstMatch(raw);
      if (wm != null) {
        final nw = double.tryParse(wm.group(1)!);
        if (nw != null) rememberBoardNaturalWidth(source, nw);
      }
      final image = await rasterizeSvg(raw, targetWidth * 2);
      return image == null ? null : (image: image, baseline: null);
    } catch (_) {
      return null;
    }
  }
}

/// Starter compact JSON for `/画板`, `/思维导图`, or `/流程图`.
String starterEmbeddedBoardJson(String template) =>
    BoardSnapshot.starter(template).toCompactJson();

/// Open the deferred fullscreen interactive board editor and return the edited
/// compact JSON string (or null if closed without changes / load failed).
Future<String?> openEmbeddedBoardEditor(
  BuildContext context, {
  required String initialJson,
  bool readOnly = false,
}) async {
  if (!await loadUiModule(context, board_ui.loadLibrary)) return null;
  if (!context.mounted) return null;
  return board_ui.showBoardEditorDialog(
    context,
    initialJson: initialJson,
    readOnly: readOnly,
  );
}

/// 3-way semantic merge when a remote update touched the same `mica-board` block
/// while the fullscreen board editor was open.
Future<String> mergeEmbeddedBoardJson(
  String base,
  String local,
  String remote,
) => boardMergeJson(base, local, remote);
