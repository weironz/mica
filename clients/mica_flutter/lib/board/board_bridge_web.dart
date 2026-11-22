// Web bridge: dynamically imports `/mica_board/board.js` WASM strictly on demand
// (0 B fetched on regular Markdown pages).
import 'dart:convert';
import 'dart:js_interop';
import 'dart:js_interop_unsafe';
import 'dart:typed_data';

import 'board_bridge.dart';

@JS('import')
external JSPromise<JSObject> _dynamicImport(JSString url);

JSObject? _wasmModule;
Future<JSObject?>? _loading;

Future<JSObject?> _ensureModule() {
  if (_wasmModule != null) return Future.value(_wasmModule);
  return _loading ??= () async {
    try {
      final mod = await _dynamicImport('./mica_board/board.js'.toJS).toDart;
      final initFn = mod.getProperty<JSFunction?>('default'.toJS);
      if (initFn != null) {
        final p = initFn.callAsFunction(mod) as JSPromise<JSAny?>;
        await p.toDart;
      }
      _wasmModule = mod;
      return mod;
    } catch (_) {
      _loading = null;
      return null;
    }
  }();
}

Future<String?> boardRenderSvg(String source, {BoardThemeInput? theme}) async {
  final mod = await _ensureModule();
  if (mod == null) return null;
  final themeArg = theme == null ? null : jsonEncode(theme.toJson()).toJS;
  final res = mod.callMethodVarArgs<JSString?>(
    'render_board_svg'.toJS,
    [source.toJS, themeArg],
  );
  return res?.toDart;
}

Future<String?> boardRouteEdgesJson(String source) async {
  final mod = await _ensureModule();
  if (mod == null) return null;
  final res = mod.callMethodVarArgs<JSString?>(
    'route_board_edges'.toJS,
    [source.toJS],
  );
  return res?.toDart;
}

Future<String?> boardImportDrawio(Uint8List bytes) async {
  final mod = await _ensureModule();
  if (mod == null) return null;
  final res = mod.callMethodVarArgs<JSString>(
    'import_drawio'.toJS,
    [bytes.toJS],
  );
  return res.toDart;
}

Future<String> boardMergeJson(String base, String local, String remote) async {
  final mod = await _ensureModule();
  if (mod == null) return local;
  final res = mod.callMethodVarArgs<JSString>(
    'merge_board'.toJS,
    [base.toJS, local.toJS, remote.toJS],
  );
  return res.toDart;
}

Future<String?> boardMarkdownToMindmap(String markdown, {String? title}) async {
  final mod = await _ensureModule();
  if (mod == null) return null;
  final res = mod.callMethodVarArgs<JSString>(
    'markdown_to_mindmap_json'.toJS,
    [markdown.toJS, title?.toJS],
  );
  return res.toDart;
}

Future<String?> boardMindmapToMarkdown(String source) async {
  final mod = await _ensureModule();
  if (mod == null) return null;
  final res = mod.callMethodVarArgs<JSString?>(
    'mindmap_json_to_markdown'.toJS,
    [source.toJS],
  );
  return res?.toDart;
}

class _WebBoardSessionBridge implements BoardSessionBridge {
  _WebBoardSessionBridge(this._handle);

  final JSObject _handle;

  @override
  String currentStatePatchJson() =>
      _handle.callMethod<JSString>('currentStatePatchJson'.toJS).toDart;

  @override
  String dispatchJson(String cmdJson) =>
      _handle.callMethodVarArgs<JSString>('dispatchJson'.toJS, [cmdJson.toJS]).toDart;

  @override
  String pointerMoveJson(
    double worldX,
    double worldY,
    double screenX,
    double screenY, {
    bool altPressed = false,
  }) => _handle
      .callMethodVarArgs<JSString>('pointerMoveJson'.toJS, [
        worldX.toJS,
        worldY.toJS,
        screenX.toJS,
        screenY.toJS,
        altPressed.toJS,
      ])
      .toDart;

  @override
  String exportCompactJson() =>
      _handle.callMethod<JSString>('exportCompactJson'.toJS).toDart;

  @override
  String renderSvg({BoardThemeInput? theme}) {
    final themeArg = theme == null ? null : jsonEncode(theme.toJson()).toJS;
    return _handle
        .callMethodVarArgs<JSString>('renderSvg'.toJS, [themeArg])
        .toDart;
  }

  @override
  void dispose() {
    _handle.callMethod<JSAny?>('free'.toJS);
  }
}

Future<BoardSessionBridge?> openBoardSession(String snapshotJson) async {
  final mod = await _ensureModule();
  if (mod == null) return null;
  try {
    final ctor = mod.getProperty<JSFunction?>('WasmBoardSession'.toJS);
    if (ctor == null) return null;
    final handle = ctor.callAsConstructor<JSObject>(snapshotJson.toJS);
    return _WebBoardSessionBridge(handle);
  } catch (_) {
    return null;
  }
}
