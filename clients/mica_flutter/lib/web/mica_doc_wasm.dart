// The web's CRDT replica: mica-core's document core (Rust yrs) compiled to wasm
// (`crates/doc-wasm`, built by scripts/build-doc-wasm.sh, loaded by index.html
// as `micaDoc`). Same method set and semantics as the desktop FFI
// `MicaDocument`, so the sync adapter, the op replay ([BlockOpMirror]) and every
// edit rule are one implementation on both platforms.
//
// Replaced the yjs engine and its Dart mirror of the edit ops (`mica_ydoc.dart`,
// `marks_diff.dart`, `text_diff.dart`, `yjs_interop.dart`) on 2026-11-01.
// Documents those wrote stay readable: yjs and yrs share the update format, and
// `crates/mica-core/tests/legacy_yjs_state.rs` pins a real yjs-written state.
import 'dart:convert';
import 'dart:js_interop';
import 'dart:typed_data';

import '../cloud/block_op_mirror.dart';

@JS('micaDoc')
external _MicaDocNs? get _micaDoc;
@JS('micaDocReady')
external JSPromise<JSAny?>? get _micaDocReady;
@JS('micaDocError')
external JSString? get _micaDocError;

extension type _MicaDocNs._(JSObject _) implements JSObject {
  @JS('MicaDocument')
  external _DocClass get docClass;
  external String engineInfo();
  external double memoryBytes();
}

extension type _DocClass._(JSObject _) implements JSObject {
  external _JsDoc? fromState(JSUint8Array bytes);
  external _JsDoc? fromBlocksJson(String rootId, String blocksJson);
}

extension type _JsDoc._(JSObject _) implements JSObject {
  external String rootBlockId();
  external String toBlocksJson();
  external void trackChanges();
  external String? takeChangedBlockIds();
  external String blocksJson(String idsJson);
  external JSUint8Array encodeState();
  external JSUint8Array stateVector();
  external JSUint8Array encodeDiffSince(JSUint8Array stateVector);
  external bool applyUpdate(JSUint8Array update);
  external void insertBlockJson(String parentId, int index, String blockJson);
  external void updateBlock(
    String id,
    String? kind,
    String? text,
    String? dataJson,
  );
  external void deleteBlock(String id, bool bringChildren);
  external void moveBlock(String id, String newParent, int index);
  external void beginHistoryBatch();
  external void endHistoryBatch();
  external void resetHistory();
  external bool undo();
  external bool redo();
  external bool canUndo();
  external bool canRedo();
  external JSUint32Array historyDepths();
  external void free();
}

String? _loadError;

/// Await the engine that index.html started loading. Called once at startup
/// (in parallel with the Markdown engine) so that every replica afterwards can
/// be built synchronously, as the sync session requires. A failure is recorded,
/// not thrown: the app still starts, and [MicaDocWasm.fromState] then answers
/// null — the same "undecodable" path the session already handles.
Future<void> initDocEngine() async {
  final ready = _micaDocReady;
  if (ready == null) {
    _loadError = 'mica_doc.js was not loaded (index.html)';
    return;
  }
  try {
    await ready.toDart;
  } catch (error) {
    _loadError = '$error';
    return;
  }
  if (_micaDoc == null) {
    _loadError = _micaDocError?.toDart ?? 'wasm init failed';
  }
}

/// Why the engine is unavailable, or null when it loaded.
String? get docEngineError =>
    _micaDoc == null ? (_loadError ?? 'initDocEngine() was not awaited') : null;

/// The engine's build info (`source_fingerprint`), for freshness checks.
Map<String, dynamic>? docEngineInfo() {
  final ns = _micaDoc;
  if (ns == null) return null;
  return jsonDecode(ns.engineInfo()) as Map<String, dynamic>;
}

/// One document replica in wasm memory.
///
/// Calls into wasm can throw (a Rust panic is a wasm trap, surfacing as a JS
/// `RuntimeError`); the read/merge entry points turn that into the same failure
/// value the desktop FFI returns rather than crashing the caller.
class MicaDocWasm implements BlockOpTarget {
  MicaDocWasm._(this._doc);

  final _JsDoc _doc;

  /// Decode a full state; null when the bytes don't decode or the engine is
  /// not loaded. The CRDT actor id is random per replica (never pinned on the
  /// web: there is no device identity, and each tab must be its own actor).
  static MicaDocWasm? fromState(Uint8List bytes) {
    final ns = _micaDoc;
    if (ns == null) return null;
    try {
      final doc = ns.docClass.fromState(bytes.toJS);
      return doc == null ? null : MicaDocWasm._(doc);
    } catch (_) {
      return null;
    }
  }

  /// Bytes of the engine's wasm memory — its high-water mark, since wasm
  /// memory never shrinks. 0 when the engine is not loaded.
  static double memoryBytes() => _micaDoc?.memoryBytes() ?? 0;

  /// A document holding exactly the given blocks; null when the engine is not
  /// loaded or the JSON is not a block list.
  static MicaDocWasm? fromBlocks(
    String rootId,
    List<Map<String, dynamic>> blocks,
  ) {
    final ns = _micaDoc;
    if (ns == null) return null;
    final doc = ns.docClass.fromBlocksJson(rootId, jsonEncode(blocks));
    return doc == null ? null : MicaDocWasm._(doc);
  }

  /// Release the Rust side now. Every owner calls it when done (scratch
  /// documents, and session replicas via `SyncDocReplica.dispose`); the
  /// `--weak-refs` finalizer is only the safety net. The object must not be
  /// used afterwards.
  void free() => _doc.free();

  String rootBlockId() => _doc.rootBlockId();
  @override
  String toBlocksJson() => _doc.toBlocksJson();
  List<Map<String, dynamic>> toBlocks() =>
      (jsonDecode(_doc.toBlocksJson()) as List).cast<Map<String, dynamic>>();

  /// See mica-core `track_changes` / `take_changed_block_ids`.
  void trackChanges() => _doc.trackChanges();
  List<String>? takeChangedBlockIds() {
    final json = _doc.takeChangedBlockIds();
    return json == null ? null : (jsonDecode(json) as List).cast<String>();
  }

  List<Map<String, dynamic>> blocksById(List<String> ids) =>
      (jsonDecode(_doc.blocksJson(jsonEncode(ids))) as List)
          .cast<Map<String, dynamic>>();
  Uint8List encodeState() => _doc.encodeState().toDart;
  Uint8List stateVector() => _doc.stateVector().toDart;
  Uint8List encodeDiffSince(Uint8List sv) =>
      _doc.encodeDiffSince(sv.toJS).toDart;

  /// Merge a remote update; false when it doesn't apply.
  bool applyUpdate(Uint8List update) {
    try {
      return _doc.applyUpdate(update.toJS);
    } catch (_) {
      return false;
    }
  }

  @override
  void insertBlockJson({
    required String parentId,
    required int index,
    required String blockJson,
  }) => _doc.insertBlockJson(parentId, index, blockJson);
  @override
  void updateBlock({
    required String id,
    String? kind,
    String? text,
    String? dataJson,
  }) => _doc.updateBlock(id, kind, text, dataJson);
  @override
  void deleteBlock({required String id, required bool bringChildren}) =>
      _doc.deleteBlock(id, bringChildren);
  @override
  void moveBlock({
    required String id,
    required String newParent,
    required int index,
  }) => _doc.moveBlock(id, newParent, index);

  void beginHistoryBatch() => _doc.beginHistoryBatch();
  void endHistoryBatch() => _doc.endHistoryBatch();
  void resetHistory() => _doc.resetHistory();
  bool undo() => _doc.undo();
  bool redo() => _doc.redo();
  bool canUndo() => _doc.canUndo();
  bool canRedo() => _doc.canRedo();
  (int, int) historyDepths() {
    final d = _doc.historyDepths().toDart;
    return (d[0], d[1]);
  }
}
