import 'dart:async';
import 'dart:ui' as ui;

import 'package:flutter/foundation.dart' show kIsWeb;
import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/scheduler.dart';
import 'package:flutter/services.dart';

import 'controller.dart';
import 'highlight.dart';
import 'markdown.dart';
import 'marks.dart';
import 'clipboard_copy.dart';
import 'image_actions.dart';
import 'image_animator.dart';
import 'mermaid_preview.dart';
import 'model.dart';
import 'preview_raster.dart';
import 'open_url.dart';
import 'pick_image.dart';
import 'render.dart';
import 'rich_paste.dart';
import 'cell_edit_controller.dart';
import 'table.dart';

export 'controller.dart' show DocOp, ApplyOps;
export 'markdown.dart' show markdownToBlocks, BlockSpec;
export 'model.dart' show EditorNode;
export 'render.dart' show EditorAppearance, EditorTheme;

/// The in-house editor: one editing surface for the whole document.
///
/// Drop-in replacement for the old per-block `TextField` editor. It owns a
/// single [Focus], a single OS text-input connection (so IME/Chinese input
/// works), one document-wide selection, and renders every node itself through
/// [DocumentSurface]. The caret travels the document as if it were one
/// continuous page — no block chrome, Word/Typora feel (see docs/editor.md).
/// Lets a host (e.g. the page outline) ask the editor to scroll a block into
/// view. The editor wires its scroll implementation into [_scroll] on init.
class EditorScrollHook {
  void Function(String blockId)? _scroll;
  void scrollToBlock(String blockId) => _scroll?.call(blockId);
}

/// A single heading in the live document outline (table of contents).
class OutlineEntry {
  const OutlineEntry({
    required this.id,
    required this.text,
    required this.level,
  });
  final String id;
  final String text;
  final int level;

  @override
  bool operator ==(Object other) =>
      other is OutlineEntry &&
      other.id == id &&
      other.text == text &&
      other.level == level;

  @override
  int get hashCode => Object.hash(id, text, level);
}

/// Live table-of-contents feed. The editor republishes the current headings (in
/// document order) on every model change; the host's outline panel listens and
/// rebuilds. Decouples the outline from the frozen bootstrap snapshot so it
/// tracks the user's own edits in real time (not just navigation / remote sync).
class EditorOutlineHook extends ChangeNotifier {
  List<OutlineEntry> _headings = const [];
  List<OutlineEntry> get headings => _headings;

  void publish(List<OutlineEntry> next) {
    if (_same(next)) return;
    _headings = List.unmodifiable(next);
    notifyListeners();
  }

  bool _same(List<OutlineEntry> next) {
    if (next.length != _headings.length) return false;
    for (var i = 0; i < next.length; i++) {
      if (next[i] != _headings[i]) return false;
    }
    return true;
  }
}

/// Lets the host open the editor's in-page find bar (Ctrl+F) even when focus is
/// outside the editor. The editor wires [_open] on init; a no-op otherwise.
class EditorFindHook {
  void Function()? _open;
  void open() => _open?.call();
}

/// Publishes the focused block's kind (+ heading level) to the host, so the
/// optional format toolbar can highlight the button for the current block type
/// — e.g. which heading level the caret sits in. Change-detected.
class EditorActiveBlockHook extends ChangeNotifier {
  String? _kind;
  int? _level;
  String? get kind => _kind;
  int? get level => _level;

  void publish(String? kind, int? level) {
    if (kind == _kind && level == _level) return;
    _kind = kind;
    _level = level;
    notifyListeners();
  }
}

/// Case-insensitive, non-overlapping occurrences of [query] across [texts] (one
/// entry per document node), in document order — the enumerator behind the
/// in-page find bar. Empty query → no matches. Pure; exposed for testing.
List<({int node, int start, int end})> findTextMatches(
  List<String> texts,
  String query,
) {
  final matches = <({int node, int start, int end})>[];
  if (query.isEmpty) return matches;
  final needle = query.toLowerCase();
  for (var i = 0; i < texts.length; i++) {
    final hay = texts[i].toLowerCase();
    var from = 0;
    while (from <= hay.length - needle.length) {
      final at = hay.indexOf(needle, from);
      if (at < 0) break;
      matches.add((node: i, start: at, end: at + query.length));
      from = at + query.length;
    }
  }
  return matches;
}

/// Lets the host's formatting toolbar drive editor commands (block converts,
/// inline marks, inserts, undo/redo) without owning the controller. The
/// editor registers itself while mounted; calls are no-ops otherwise.
class EditorCommandHook {
  void Function(String type)? _toggleMark;
  void Function(String kind, Map<String, dynamic> data)? _setBlock;
  void Function(String kind)? _insert; // 'divider' | 'table' | 'image'
  VoidCallback? _editLink;
  VoidCallback? _undo;
  VoidCallback? _redo;
  VoidCallback? _focusFirstLine;
  void Function(String text)? _insertTopParagraph;
  VoidCallback? _resetDiagramViews;
  Future<void> Function()? _flush;

  void toggleMark(String type) => _toggleMark?.call(type);
  void setBlock(String kind, [Map<String, dynamic> data = const {}]) =>
      _setBlock?.call(kind, data);
  void insert(String kind) => _insert?.call(kind);
  void editLink() => _editLink?.call();
  void undo() => _undo?.call();
  void redo() => _redo?.call();

  /// Move the caret to the start of the first body line (ArrowDown from the
  /// page title).
  void focusFirstLine() => _focusFirstLine?.call();

  /// Insert a new paragraph at the very top (Enter in the page title pushes
  /// the body down; [text] is the title remainder after the caret).
  void insertTopParagraph(String text) => _insertTopParagraph?.call(text);

  /// Restore all diagram previews to their natural zoom/pan — the page host
  /// calls this when a click lands outside the editor canvas (page margins).
  void resetDiagramViews() => _resetDiagramViews?.call();

  /// Flush any debounced/pending text edits to the backend NOW. The host calls
  /// this before navigating away (page/workspace switch) so the last <=400ms of
  /// typing isn't dropped when the editor is torn down. Returns when the edits
  /// have been emitted (and, for the cloud path, the round-trip is in flight).
  Future<void> flush() => _flush?.call() ?? Future.value();
}

class MicaEditor extends StatefulWidget {
  const MicaEditor({
    required this.rootBlockId,
    required this.nodes,
    required this.version,
    required this.canEdit,
    required this.onApplyOperations,
    this.onSelectionChanged,
    this.remoteCursors = const [],
    this.onAiStream,
    this.onUploadImage,
    this.onImportImageUrl,
    this.onLoadImageBytes,
    this.onResolveImageUrls,
    this.reHostImages = true,
    this.focusNode,
    this.scrollHook,
    this.commandHook,
    this.outlineHook,
    this.findHook,
    this.activeBlockHook,
    this.onExitTop,
    this.appearance = const EditorAppearance(),
    this.onOpenPage,
    this.pageLinks,
    super.key,
  });

  final String rootBlockId;

  /// Latest server view of the document's blocks (converted to [EditorNode]s by
  /// the caller). Re-supplied on every snapshot; reconciled against local edits.
  final List<EditorNode> nodes;

  /// Bumped whenever a new server snapshot arrives (our own write or a remote
  /// edit); triggers reconciliation.
  final int version;
  final bool canEdit;
  final ApplyOps onApplyOperations;

  /// Fired (debounced by the host) when the local caret moves — `(blockId,
  /// offset)`, or `(null, null)` when there's no selection — so the host can
  /// broadcast it as awareness.
  final void Function(String? blockId, int? offset)? onSelectionChanged;

  /// Other collaborators' carets to paint on the canvas (awareness).
  final List<RemoteCursor> remoteCursors;

  /// Streams Markdown from a prompt for the in-editor "Ask AI" command (deltas
  /// shown live). When null, the AI slash entry is hidden.
  final Stream<String> Function(String prompt, {String? system})? onAiStream;

  /// Upload image bytes, returning the new `(file_id, name)`. When null, image
  /// insertion is disabled.
  final Future<({String fileId, String name})?> Function(
    Uint8List bytes,
    String fileName,
    String mimeType,
  )?
  onUploadImage;

  /// Re-host a pasted image URL server-side (avoids dead links), returning the
  /// new `(file_id, name)`. When null, pasted image URLs are left as text.
  final Future<({String fileId, String name})?> Function(String url)?
  onImportImageUrl;

  /// Resolve an image `file_id` to its bytes (the host resolves a fresh signed
  /// URL and fetches it). Used to paint image nodes on the canvas.
  final Future<Uint8List?> Function(String fileId)? onLoadImageBytes;

  /// Batch-resolve image `file_id`s to fresh download URLs (for copy/export so
  /// pasted Markdown links resolve). Cached eagerly when the document changes.
  final Future<Map<String, String>> Function(List<String> fileIds)?
  onResolveImageUrls;

  /// When true, pasted/imported external image URLs are re-hosted into Mica's
  /// storage; when false they stay as standard external Markdown links.
  final bool reHostImages;

  /// Optional external focus node so the host can move focus into the editor
  /// (e.g. pressing Enter in the page title jumps to the first body line).
  final FocusNode? focusNode;

  /// Optional hook the page outline uses to scroll a heading block into view.
  final EditorScrollHook? scrollHook;

  /// Optional hook the host's formatting toolbar uses to run editor commands.
  final EditorCommandHook? commandHook;

  /// Optional hook that receives the live heading list (document outline / TOC)
  /// on every edit, so the host's outline panel updates without navigation.
  final EditorOutlineHook? outlineHook;

  /// Optional hook the host uses to open the in-page find bar (Ctrl+F).
  final EditorFindHook? findHook;

  /// Optional hook that receives the focused block's kind + heading level, so
  /// the host toolbar can highlight the current block type.
  final EditorActiveBlockHook? activeBlockHook;

  /// Called when the caret tries to leave the document upward (ArrowUp on the
  /// first line, Backspace at the very start) — the host focuses the page
  /// title.
  final VoidCallback? onExitTop;

  /// User-adjustable font appearance.
  final EditorAppearance appearance;

  /// Open an internal page link (`mica://page/<viewId>`). When null, page
  /// links are inert.
  final void Function(String viewId)? onOpenPage;

  /// Pages offered by the `[[` link picker. When null, the picker is hidden.
  final List<PageLinkTarget> Function()? pageLinks;

  @override
  State<MicaEditor> createState() => _MicaEditorState();
}

/// The bare LaTeX of a pasted line that is *entirely* one **display** formula
/// (`$$…$$` or `\[…\]`), or null. Display formulas become their own math block;
/// inline forms (`$…$`, `\(…\)`) are handled separately so they stay in the
/// text flow (see [parseInlineMath]). A line with surrounding prose fails the
/// full-line anchors and returns null, staying literal text.
String? pastedFormulaSource(String line) {
  final m =
      RegExp(r'^\$\$(.+)\$\$$').firstMatch(line) ??
      RegExp(r'^\\\[(.+)\\\]$').firstMatch(line);
  return m?.group(1)?.trim();
}

/// A linkable page for the `[[` picker.
class PageLinkTarget {
  const PageLinkTarget({required this.id, required this.title});
  final String id;
  final String title;
}

class _MicaEditorState extends State<MicaEditor> implements TextInputClient {
  late final EditorController _controller;
  late final FocusNode _focus =
      widget.focusNode ?? FocusNode(debugLabel: 'MicaEditor');
  final GlobalKey _surfaceKey = GlobalKey();

  TextInputConnection? _conn;
  // True while an IME composition (e.g. pinyin) is in progress. Desktop
  // backspace/delete must defer to the IME then, or the composition desyncs and
  // raw pinyin leaks into the document (seen with Microsoft Pinyin).
  bool _imeComposing = false;
  // The composing (marked-text) range within the focused block. It MUST be
  // reflected back to the OS via currentTextEditingValue/_imeValue, or a
  // composition-style IME (Microsoft Pinyin) diverges and accumulates garbage.
  TextRange _composing = TextRange.empty;
  TextEditingValue _lastSentIme = TextEditingValue.empty;

  Timer? _blink;
  bool _caretOn = true;
  DocPosition? _dragAnchor;
  int? _scrollbarDrag; // code-block index whose scrollbar is being dragged
  int? _blockDrag; // block index being moved via its gutter drag handle
  int? _diagramPan; // diagram block index being panned by drag
  int? _panDownDiagram; // diagram under the REAL pointer-down position

  // Source → picture previews (math, mermaid): the pipeline owns the cache /
  // pending / off-screen-capture lifecycle, the previewers say how one source
  // becomes a picture (docs/render-architecture.md). Mermaid only registers
  // where its JS engine exists (web) — elsewhere ```mermaid blocks stay
  // highlighted source via the renderer's null-decline.
  late final RasterPreviewPipeline _previews = RasterPreviewPipeline(
    previewers: [
      const MathPreviewer(),
      if (mermaidAvailable) const MermaidPreviewer(),
    ],
    requestRebuild: (fn) {
      if (mounted) setState(fn);
    },
  );
  int? _imageResize; // image node index whose width is being dragged
  double? _imageResizeWidth; // last previewed width during an image resize
  // Auto-scroll the surrounding page while drag-selecting near the viewport edge.
  Timer? _autoScrollTimer;
  Offset? _lastDragGlobal;
  OverlayEntry? _cellEntry; // active table cell editor
  VoidCallback? _cellFocusListener;
  // Commit-and-close hook for the active cell editor; called when a drag
  // begins so the floating field never detaches from its (re-laid-out) cell.
  VoidCallback? _commitCellEditor;
  // The table cell currently being edited (controller + address), so the
  // floating format bar can act on the cell's selection like it does on the body.
  ({CellEditController ctl, int node, int row, int col})? _activeCell;
  // A live drag selecting text inside a cell: the cell's controller + the anchor
  // text offset. Drives the cell's selection from the canvas pointer (the field
  // can't receive the canvas-owned drag itself).
  ({CellEditController ctl, int node, int row, int col, int anchor})? _cellDrag;
  // A live cross-cell drag selecting a rectangular AREA of cells (starts when a
  // cell drag crosses into another cell — AFFiNE-style): the anchor cell.
  ({int node, int row, int col})? _areaDrag;
  OverlayEntry? _markBar; // floating inline-format toolbar over a selection
  MouseCursor _cursor = SystemMouseCursors.text;

  // Decoded images keyed by file_id, painted on the canvas by RenderDocument.
  // For an animated one (GIF / animated WebP) this holds the frame currently
  // on screen and _imageAnims holds the loop feeding it.
  final Map<String, ui.Image> _imageCache = {};
  final Set<String> _imageErrors = {};
  final Set<String> _imageLoading = {};
  final Map<String, ImageAnimator> _imageAnims = {};
  // Keys the canvas drew since each one's last frame — see [_onImageFrame].
  final Set<String> _imagePainted = {};
  // Bumped per animated frame, so an open fullscreen viewer moves too.
  final ValueNotifier<int> _imageFrameTick = ValueNotifier(0);
  // file_id -> fresh download URL, for copy/export of images.
  final Map<String, String> _imageUrlCache = {};
  // Active table column resize: node, right-column index, start x, start
  // weights, full available width, and the table's width fraction at start.
  // col == weights.length means the table's right edge (overall width drag).
  ({
    int node,
    int col,
    double startX,
    List<double> weights,
    double avail,
    double frac,
  })?
  _colResize;

  // Slash (`/`) insert menu — transient; never shown at rest.
  OverlayEntry? _slashEntry;
  int _slashStart = 0;
  String _slashQuery = '';
  int _slashIndex = 0;

  // `[[` page-link picker — same lifecycle as the slash menu.
  OverlayEntry? _pageEntry;
  int _pageStart = 0;
  String _pageQuery = '';
  int _pageIndex = 0;

  // Hover toolbar over a link (edit / copy / remove).
  OverlayEntry? _linkBar;
  Timer? _linkBarHide;
  int _linkBarNode = -1;
  Mark? _linkBarMark;
  bool _pointerOverLinkBar = false;

  // Multi-tap tracking for word (double) / block (triple) selection. The
  // platform GestureDetector only reports double-taps without a position and
  // has no triple-tap, so we count taps ourselves from _onTapDown: taps within
  // [_multiTapSlop] pixels and [_multiTapWindow] of the previous one escalate
  // the count (2 = word, 3 = block); anything else resets to a single tap.
  static const Duration _multiTapWindow = Duration(milliseconds: 400);
  static const double _multiTapSlop = 12.0;
  int _tapCount = 0;
  Offset? _lastTapLocal;
  // The pointer-event timestamp of the previous down (binding clock, not wall
  // clock) so the window is deterministic under the test fake-async clock.
  Duration? _lastTapStamp;
  Duration _downStamp = Duration.zero; // stamp of the in-flight pointer down

  RenderDocument? get _render =>
      _surfaceKey.currentContext?.findRenderObject() as RenderDocument?;

  @override
  void initState() {
    super.initState();
    _controller = EditorController(
      rootBlockId: widget.rootBlockId,
      onOps: widget.onApplyOperations,
    );
    // Load before subscribing so the initial notify doesn't setState in init.
    _controller.load(widget.nodes);
    _controller.addListener(_onControllerChanged);
    _focus.addListener(_onFocusChange);
    widget.scrollHook?._scroll = _scrollToBlock;
    widget.findHook?._open = _openFind;
    _registerCommandHook();
    _ensureNotEmptyDeferred();
    // Seed the outline once the first frame is up (publishing during init could
    // notify the host's outline panel mid-build).
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      _publishOutline();
      _publishActiveBlock();
    });
  }

  /// Push the current headings (id + text + level, document order) to the host
  /// outline hook. Change-detected, so it only rebuilds the outline when a
  /// heading is actually added / removed / retitled / re-leveled.
  void _publishOutline() {
    final hook = widget.outlineHook;
    if (hook == null) return;
    final items = <OutlineEntry>[];
    for (final n in _controller.nodes) {
      if (n.kind == 'heading') {
        items.add(
          OutlineEntry(
            id: n.id,
            text: n.text,
            level: (n.data['level'] as num?)?.toInt() ?? 1,
          ),
        );
      }
    }
    hook.publish(items);
  }

  /// Publish the focused block's kind + heading level for the host toolbar.
  void _publishActiveBlock() {
    final hook = widget.activeBlockHook;
    if (hook == null) return;
    final n = _controller.focusedNode;
    final kind = n?.kind;
    hook.publish(kind, kind == 'heading' ? n!.headingLevel : null);
  }

  // ---- In-page find (Ctrl+F) -----------------------------------------------
  bool _findOpen = false;
  final TextEditingController _findCtrl = TextEditingController();
  final FocusNode _findFocus = FocusNode(debugLabel: 'editorFind');
  List<({int node, int start, int end})> _findMatches = const [];
  int _findIndex = 0;

  /// Open (or re-focus) the in-page find bar. Seeds the query from a simple
  /// single-node selection, like a browser's Ctrl+F.
  void _openFind() {
    if (!mounted) return;
    final sel = _controller.selection;
    if (sel != null && !sel.isCollapsed && !sel.isMultiNode) {
      final n = _controller.nodes[sel.start.node];
      final s = sel.start.offset.clamp(0, n.text.length);
      final e = sel.end.offset.clamp(0, n.text.length);
      if (e > s) _findCtrl.text = n.text.substring(s, e);
    }
    setState(() => _findOpen = true);
    _recomputeFind(reveal: true);
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      _findFocus.requestFocus();
      _findCtrl.selection = TextSelection(
        baseOffset: 0,
        extentOffset: _findCtrl.text.length,
      );
    });
  }

  void _closeFind() {
    if (!_findOpen) return;
    setState(() => _findOpen = false);
    _focus.requestFocus();
  }

  /// Case-insensitive, non-overlapping scan of every node's text for the query.
  void _recomputeFind({bool reveal = false}) {
    final matches = findTextMatches([
      for (final n in _controller.nodes) n.text,
    ], _findCtrl.text);
    setState(() {
      _findMatches = matches;
      _findIndex = 0;
    });
    if (reveal && matches.isNotEmpty) _revealFind(0);
  }

  /// Move to the next/previous match (wrapping) and reveal it.
  void _findStep(int delta) {
    if (_findMatches.isEmpty) return;
    setState(() {
      _findIndex = (_findIndex + delta) % _findMatches.length;
      if (_findIndex < 0) _findIndex += _findMatches.length;
    });
    _revealFind(_findIndex);
  }

  /// Select + scroll the i-th match into view. The existing selection paint
  /// highlights it (visible even though the find field holds focus).
  void _revealFind(int i) {
    if (i < 0 || i >= _findMatches.length) return;
    final m = _findMatches[i];
    if (m.node >= _controller.nodes.length) return;
    _controller.setSelection(
      DocSelection(
        anchor: DocPosition(m.node, m.start),
        focus: DocPosition(m.node, m.end),
      ),
    );
    _scrollToBlock(_controller.nodes[m.node].id);
  }

  Widget _buildFindBar() {
    final total = _findMatches.length;
    final label = _findCtrl.text.isEmpty
        ? ''
        : (total == 0 ? '无结果' : '${_findIndex + 1}/$total');
    Widget iconBtn(IconData icon, String tip, VoidCallback? onTap) =>
        IconButton(
          icon: Icon(icon, size: 18),
          tooltip: tip,
          onPressed: onTap,
          visualDensity: VisualDensity.compact,
          padding: EdgeInsets.zero,
          constraints: const BoxConstraints(minWidth: 30, minHeight: 30),
          color: const Color(0xFF475569),
        );
    return Material(
      elevation: 4,
      borderRadius: BorderRadius.circular(8),
      color: Colors.white,
      child: CallbackShortcuts(
        bindings: <ShortcutActivator, VoidCallback>{
          const SingleActivator(LogicalKeyboardKey.escape): _closeFind,
          const SingleActivator(LogicalKeyboardKey.enter, shift: true): () =>
              _findStep(-1),
        },
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 2),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              const Icon(Icons.search, size: 16, color: Color(0xFF64748B)),
              const SizedBox(width: 6),
              SizedBox(
                width: 170,
                child: TextField(
                  controller: _findCtrl,
                  focusNode: _findFocus,
                  onChanged: (_) => _recomputeFind(reveal: true),
                  onSubmitted: (_) => _findStep(1),
                  textInputAction: TextInputAction.search,
                  style: const TextStyle(fontSize: 14),
                  decoration: const InputDecoration(
                    isDense: true,
                    border: InputBorder.none,
                    hintText: '页内查找',
                  ),
                ),
              ),
              const SizedBox(width: 6),
              SizedBox(
                width: 44,
                child: Text(
                  label,
                  textAlign: TextAlign.right,
                  style: const TextStyle(
                    fontSize: 12,
                    color: Color(0xFF94A3B8),
                  ),
                ),
              ),
              iconBtn(
                Icons.keyboard_arrow_up,
                '上一个 (Shift+Enter)',
                total == 0 ? null : () => _findStep(-1),
              ),
              iconBtn(
                Icons.keyboard_arrow_down,
                '下一个 (Enter)',
                total == 0 ? null : () => _findStep(1),
              ),
              iconBtn(Icons.close, '关闭 (Esc)', _closeFind),
            ],
          ),
        ),
      ),
    );
  }

  /// Wire the host formatting toolbar's commands to the controller. Every
  /// command restores editor focus first — toolbar clicks blur the canvas.
  void _registerCommandHook() {
    final hook = widget.commandHook;
    if (hook == null) return;
    void refocus() {
      _focus.requestFocus();
      _syncImeFromSelection(force: true);
    }

    hook
      .._toggleMark = (type) {
        if (!mounted || !widget.canEdit) return;
        // A table cell is being edited: act on the cell's own selection and do
        // NOT refocus the canvas — that would blur the cell field, committing
        // and closing it before the mark could apply.
        if (_activeCell != null) {
          _toggleMarkCtx(type);
          return;
        }
        _focus.requestFocus();
        _controller.toggleMark(type);
        _syncImeFromSelection(force: true);
      }
      .._setBlock = (kind, data) {
        if (!mounted || !widget.canEdit) return;
        _focus.requestFocus();
        _controller.setSelectedBlocksKind(kind, data: data);
        _syncImeFromSelection(force: true);
      }
      .._insert = (kind) {
        if (!mounted || !widget.canEdit) return;
        switch (kind) {
          case 'divider':
            _focus.requestFocus();
            _controller.insertDivider();
            _syncImeFromSelection(force: true);
          case 'table':
            _focus.requestFocus();
            _controller.insertBlocksAfterFocus([
              (kind: 'table', text: '', data: TableData.empty().toBlockData()),
            ]);
            _syncImeFromSelection(force: true);
          case 'image':
            _pickAndInsertImage();
        }
      }
      .._editLink = () {
        if (!mounted || !widget.canEdit) return;
        _focus.requestFocus();
        _promptLink();
      }
      .._undo = () {
        if (!mounted || !widget.canEdit) return;
        _focus.requestFocus();
        _controller.undo();
        refocus();
      }
      .._redo = () {
        if (!mounted || !widget.canEdit) return;
        _focus.requestFocus();
        _controller.redo();
        refocus();
      }
      .._focusFirstLine = () {
        if (!mounted) return;
        _focus.requestFocus();
        if (_controller.nodes.isNotEmpty) {
          _controller.collapseTo(const DocPosition(0, 0));
        }
        _syncImeFromSelection(force: true);
      }
      .._insertTopParagraph = (text) {
        if (!mounted || !widget.canEdit) return;
        _focus.requestFocus();
        _controller.insertParagraphAtTop(text);
        _syncImeFromSelection(force: true);
      }
      .._resetDiagramViews = () {
        if (!mounted) return;
        _render?.resetAllPreviewViews();
      }
      .._flush = _controller.flushPending;
  }

  /// Scroll the surrounding page so the block [blockId] is near the top.
  void _scrollToBlock(String blockId) {
    final r = _render;
    if (r == null) return;
    final top = r.nodeBoxTop(blockId);
    if (top == null) return;
    final scrollable = Scrollable.maybeOf(context);
    if (scrollable == null) return;
    final box = scrollable.context.findRenderObject() as RenderBox?;
    if (box == null) return;
    final globalY = r.localToGlobal(Offset(0, top)).dy;
    final viewTop = box.localToGlobal(Offset.zero).dy;
    final pos = scrollable.position;
    final target = (pos.pixels + (globalY - viewTop) - 16).clamp(
      pos.minScrollExtent,
      pos.maxScrollExtent,
    );
    pos.animateTo(
      target,
      duration: const Duration(milliseconds: 300),
      curve: Curves.easeOut,
    );
  }

  @override
  void didUpdateWidget(covariant MicaEditor oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (widget.version != oldWidget.version) {
      // A routine snapshot (our own save, a remote edit, presence) must NOT
      // disturb the editing session: keep focus, keep the caret, and keep the
      // slash menu open if its `/` survived the reconcile. Re-evaluating the
      // slash session is deferred so we don't mutate the overlay during build.
      _controller.reconcile(widget.nodes);
      _syncImeFromSelection();
      _ensureNotEmptyDeferred();
      if (_slashEntry != null) {
        WidgetsBinding.instance.addPostFrameCallback((_) {
          if (mounted) _refreshSlash();
        });
      }
      if (_pageEntry != null) {
        WidgetsBinding.instance.addPostFrameCallback((_) {
          if (mounted) _refreshPageLink();
        });
      }
    }
  }

  @override
  void dispose() {
    _slashEntry?.remove();
    _slashEntry = null;
    _pageEntry?.remove();
    _pageEntry = null;
    _linkBarHide?.cancel();
    _linkBar?.remove();
    _linkBar = null;
    _cellEntry?.remove();
    _cellEntry = null;
    _markBar?.remove();
    _markBar = null;
    for (final anim in _imageAnims.values) {
      anim.dispose();
    }
    _imageAnims.clear();
    for (final img in _imageCache.values) {
      img.dispose();
    }
    _imageCache.clear();
    _imageFrameTick.dispose();
    setRichPasteHandler(null);
    setRichImagePasteHandler(null);
    _stopAutoScroll();
    _blink?.cancel();
    _conn?.close();
    _focus.removeListener(_onFocusChange);
    // Only dispose a focus node we created; an external one is owned by the host.
    if (widget.focusNode == null) _focus.dispose();
    _controller.removeListener(_onControllerChanged);
    // Unwire the find hook if it still points at us, so a later Ctrl+F on a
    // view with no editor (e.g. a folder) doesn't call into a dead State.
    if (widget.findHook?._open == _openFind) widget.findHook?._open = null;
    _findCtrl.dispose();
    _findFocus.dispose();
    _controller.dispose();
    super.dispose();
  }

  void _ensureNotEmptyDeferred() {
    if (!widget.canEdit) return;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      if (_controller.nodes.isEmpty) {
        _controller.ensureNotEmpty();
        _syncImeFromSelection();
      }
    });
  }

  // ---------------------------------------------------------------------------
  // Controller / focus plumbing
  // ---------------------------------------------------------------------------

  String? _lastCursorKey;

  /// Report the local caret (block id + offset) to the host for awareness, when
  /// it actually moved.
  void _reportCursor() {
    final cb = widget.onSelectionChanged;
    if (cb == null) return;
    final sel = _controller.selection;
    String? blockId;
    int? offset;
    if (sel != null &&
        sel.focus.node >= 0 &&
        sel.focus.node < _controller.nodes.length) {
      blockId = _controller.nodes[sel.focus.node].id;
      offset = sel.focus.offset;
    }
    final key = '$blockId:$offset';
    if (key == _lastCursorKey) return;
    _lastCursorKey = key;
    cb(blockId, offset);
  }

  void _onControllerChanged() {
    // Repaint only. The OS input connection is the source of truth while
    // typing, so we never push editing state back from here — that is done
    // explicitly at the call sites that move the caret programmatically
    // (arrows, click, structural edits, slash apply). Pushing here would echo
    // an `updateEditingValue` back and, e.g., dismiss the slash menu.
    void apply() {
      if (!mounted) return;
      // Any document change invalidates a table cell-area selection: it stores
      // a node INDEX, and edits/undo/menu ops/remote ops shift indices — a
      // stale area made Delete blank the WRONG table and stole Backspace/
      // Ctrl+C from ordinary body editing. (Cell-edit previews don't get here
      // with an area set: opening a cell editor already clears it.)
      _render?.tableBlockSelection = null;
      setState(() {});
      _restartBlink();
      _refreshMarkBar();
      _cacheImageUrls();
      _reportCursor();
      _publishOutline();
      _publishActiveBlock();
    }

    // notifyListeners may fire during the build/layout phase (load/reconcile).
    if (SchedulerBinding.instance.schedulerPhase ==
        SchedulerPhase.persistentCallbacks) {
      SchedulerBinding.instance.addPostFrameCallback((_) => apply());
    } else {
      apply();
    }
  }

  void _onFocusChange() {
    if (_focus.hasFocus && widget.canEdit) {
      // Focus gained without a caret (e.g. Enter from the page title) → land on
      // the first line so typing starts at the top of the body.
      if (_controller.selection == null && _controller.nodes.isNotEmpty) {
        _controller.collapseTo(const DocPosition(0, 0));
      }
      _attachIme();
      _restartBlink();
      setRichPasteHandler(_handleRichPaste);
      setRichImagePasteHandler(_handlePasteImage);
      _syncImeFromSelection(force: true);
    } else {
      _detachIme();
      _blink?.cancel();
      _closeSlash();
      _closePageLink();
      _closeLinkBar();
      _hideMarkBar();
      setRichPasteHandler(null);
      setRichImagePasteHandler(null);
    }
    if (mounted) setState(() {});
  }

  /// Native paste interceptor (web): rich HTML or multi-line text is converted to
  /// Markdown and inserted as structured blocks; a single line of plain text
  /// falls through to the normal inline paste. Returns true when consumed.
  bool _handleRichPaste(
    String markdown,
    String plain,
    bool rich, {
    bool requireFocus = true,
  }) {
    if (!mounted || (requireFocus && !_focus.hasFocus) || !widget.canEdit) {
      return false;
    }
    // The document model is \n-only; Windows clipboards deliver \r\n and this
    // was the one paste path that stored the \r (code blocks kept literal
    // carriage returns, breaking the round-trip and caret math).
    markdown = markdown.replaceAll('\r\n', '\n').replaceAll('\r', '\n');
    plain = plain.replaceAll('\r\n', '\n').replaceAll('\r', '\n');
    // Repair the LLM "double-fence" artifact (ChatGPT/Codex wrap a code or
    // mermaid block in a SECOND equal-length fence). Paste-only — the core
    // CommonMark parser and file import stay strict.
    markdown = unwrapNestedFences(markdown);
    // ChatGPT ships Python as ```bash (highlight.js auto-detect's favourite
    // wrong guess). Paste-only, and only when the code structurally contradicts
    // the label — see [retagMislabeledFences].
    markdown = retagMislabeledFences(markdown);

    // Inside a code block, paste raw text verbatim (keep newlines, stay
    // inside) — but only when the selection is confined to that block: a
    // cross-block selection ending in a code block must fall through to the
    // block path, or only the code half got replaced (keyed, absurdly, to
    // the drag direction).
    final node = _controller.focusedNode;
    final sel0 = _controller.selection;
    if (node != null && node.isCode && !(sel0 != null && sel0.isMultiNode)) {
      final raw = plain.isNotEmpty ? plain : markdown;
      if (raw.isEmpty) return false;
      _controller.insertTextAtCaret(raw);
      _syncImeFromSelection(force: true);
      return true;
    }

    // A bare image URL: re-host it (server-side fetch) so the link can't rot,
    // then insert an image block instead of pasting the raw link text.
    final trimmed = plain.trim();
    if (!rich &&
        widget.onImportImageUrl != null &&
        _looksLikeImageUrl(trimmed)) {
      _importAndInsertImageUrl(trimmed);
      return true;
    }

    // A bare URL pastes as a real link: over a ranged selection it links the
    // selected text (Notion-style); at a caret it inserts the URL as linked
    // text instead of plain text.
    if (!rich &&
        _looksLikeUrl(trimmed) &&
        node != null &&
        node.kind != 'table' &&
        !node.isAtomic) {
      final sel = _controller.selection;
      if (sel != null && !sel.isCollapsed && !sel.isMultiNode) {
        _controller.setLinkRange(
          sel.focus.node,
          sel.start.offset,
          sel.end.offset,
          trimmed,
        );
      } else {
        if (sel != null && !sel.isCollapsed) _controller.deleteSelection();
        final at = _controller.selection?.focus.offset ?? 0;
        _controller.insertPageLink(at, at, trimmed, trimmed);
      }
      _syncImeFromSelection(force: true);
      return true;
    }

    // A pasted line that is entirely one *display* formula ($$…$$ or \[…\])
    // becomes its own math block — a display formula is, by definition, set on
    // its own line. (The single-line fast path below would keep it literal.)
    if (node != null && !node.isAtomic && node.kind != 'table') {
      final src = pastedFormulaSource(trimmed);
      if (src != null && src.isNotEmpty && !src.contains('\n')) {
        _controller.insertBlocksAfterFocus([
          (kind: 'math_block', text: src, data: {}),
        ]);
        _syncImeFromSelection(force: true);
        return true;
      }
    }

    // Inline math ($…$ / \(…\)) anywhere in a single pasted line is woven into
    // the text as inline-math marks at the caret — so "see $x$ here" keeps its
    // prose and the formula renders inline instead of jumping onto its own
    // line. Only fires when the line actually carries a formula; plain text
    // falls through to the literal single-line paste below.
    if (node != null &&
        !node.isAtomic &&
        !node.isCode &&
        node.kind != 'table' &&
        !plain.contains('\n')) {
      final parsed = parseInlineMath(plain);
      if (parsed.marks.isNotEmpty) {
        final sel = _controller.selection;
        if (sel != null) {
          final from = sel.start.node == sel.focus.node ? sel.start.offset : 0;
          final to = sel.end.node == sel.focus.node ? sel.end.offset : from;
          _controller.insertInlineSpan(
            sel.focus.node,
            from,
            to,
            parsed.text,
            parsed.marks,
          );
          _syncImeFromSelection(force: true);
          return true;
        }
      }
    }

    if (markdown.trim().isEmpty) return false;
    // A single-line RICH fragment (a few words copied from a page or from
    // mica itself) weaves INLINE at the caret — landing it as a fresh
    // paragraph below broke the most common paste gesture. Real block
    // constructs (a heading, a list item, a table row) still take the block
    // path: only a lone paragraph spec qualifies.
    if (rich &&
        !markdown.contains('\n') &&
        node != null &&
        !node.isAtomic &&
        !node.isCode &&
        node.kind != 'table') {
      final specs = markdownToBlocks(markdown);
      if (specs.length == 1 && specs.single.kind == 'paragraph') {
        final sel = _controller.selection;
        if (sel != null && !sel.isMultiNode) {
          final from = sel.start.node == sel.focus.node ? sel.start.offset : 0;
          final to = sel.end.node == sel.focus.node ? sel.end.offset : from;
          final spec = specs.single;
          _controller.insertInlineSpan(
            sel.focus.node,
            from,
            to,
            spec.text,
            marksFromData(spec.data),
          );
          _syncImeFromSelection(force: true);
          return true;
        }
      }
    }
    if (rich || markdown.contains('\n')) {
      // Paste replaces the current selection (Ctrl+A → paste swaps the doc).
      _controller.insertBlocksReplacingSelection(markdownToBlocks(markdown));
      _rehostExternalImages();
      _syncImeFromSelection(force: true);
      return true;
    }
    return false;
  }

  /// Desktop Ctrl+V: pull the clipboard's plain text and run it through the
  /// shared paste pipeline. _handleRichPaste consumes the rich cases (code,
  /// URL/image links, formulas, multi-line markdown); a plain single line falls
  /// through (returns false) and is inserted inline, replacing any selection —
  /// the same outcome the web textarea produced.
  /// Copy the current ranged selection in both flavors (stripped text/plain +
  /// rich text/html). False when there is nothing to copy. Shared by Ctrl+C
  /// and the context menu.
  bool _copySelection() {
    final plain = _controller.selectionPlainText(imageUrls: _imageUrlCache);
    if (plain.isEmpty) return false;
    final richHtml = _controller.selectionHtml(imageUrls: _imageUrlCache);
    copyRichToClipboard(plain: plain, richHtml: richHtml).then((_) {
      if (mounted) _focus.requestFocus();
    });
    return true;
  }

  /// Cut = copy both flavors, then delete the selection.
  bool _cutSelection() {
    final plain = _controller.selectionPlainText(imageUrls: _imageUrlCache);
    if (plain.isEmpty) return false;
    final richHtml = _controller.selectionHtml(imageUrls: _imageUrlCache);
    copyRichToClipboard(plain: plain, richHtml: richHtml).then((_) {
      if (!mounted) return;
      _focus.requestFocus();
      _controller.deleteSelection();
      _syncImeFromSelection();
    });
    return true;
  }

  /// Paste the clipboard's PLAIN text literally: no HTML conversion and no
  /// Markdown parsing — `**x**` stays `**x**`, a pipe row stays a pipe row.
  /// Multi-line content becomes plain paragraphs (or keeps its real newlines
  /// when pasting into a code block).
  Future<void> _pastePlainText() async {
    if (!mounted || !widget.canEdit) return;
    final data = await Clipboard.getData(Clipboard.kTextPlain);
    final text = (data?.text ?? '')
        .replaceAll('\r\n', '\n')
        .replaceAll('\r', '\n');
    if (text.isEmpty || !mounted) return;
    final node = _controller.focusedNode;
    final sel0 = _controller.selection;
    // Same guard as _handleRichPaste: a cross-block selection ending in a
    // code block must not take the in-block arm.
    if (node != null && node.isCode && !(sel0 != null && sel0.isMultiNode)) {
      _controller.insertTextAtCaret(text); // keeps newlines inside the block
      _syncImeFromSelection(force: true);
      return;
    }
    final lines = text.split('\n');
    if (lines.length == 1) {
      final sel = _controller.selection;
      if (sel != null && !sel.isCollapsed) _controller.deleteSelection();
      _controller.insertTextAtCaret(text);
    } else {
      _controller.insertBlocksReplacingSelection([
        for (final l in lines)
          (kind: 'paragraph', text: l, data: <String, dynamic>{}),
      ]);
    }
    _syncImeFromSelection(force: true);
  }

  /// [requireFocus] false = called from the context menu, where the canvas may
  /// not have regained focus yet by the time the async clipboard reads land.
  Future<void> _pasteFromClipboard({bool requireFocus = true}) async {
    bool live() => mounted && (!requireFocus || _focus.hasFocus);
    if (!live() || !widget.canEdit) return;
    // 0) Clipboard HTML that carries a real <table> (Excel / Google Sheets /
    //    a web table) → a Markdown table. Checked BEFORE the bitmap: Excel
    //    also puts a picture of the copied cells on the clipboard, and the
    //    image-first order pasted spreadsheets as screenshots.
    final tableMd = await readClipboardTableAsMarkdown();
    if (tableMd != null && live()) {
      final tData = await Clipboard.getData(Clipboard.kTextPlain);
      if (_handleRichPaste(
        tableMd,
        tData?.text ?? '',
        true,
        requireFocus: requireFocus,
      )) {
        return;
      }
    }
    // 1) A bitmap (screenshot / copied image) → upload as an image block.
    final img = await readClipboardImage();
    if (img != null && img.isNotEmpty) {
      if (!live() || !widget.canEdit) return;
      _handlePasteImage(img, 'image/png', 'pasted-image.png');
      return;
    }
    final data = await Clipboard.getData(Clipboard.kTextPlain);
    final plain = data?.text ?? '';
    // 2) Rich HTML (browser/Word) → Markdown so structure survives, like web.
    final md = await readClipboardHtmlAsMarkdown();
    if (md != null && live()) {
      if (_handleRichPaste(md, plain, true, requireFocus: requireFocus)) {
        return;
      }
    }
    // 3) Plain text: multi-line markdown-parses, single line inserts inline.
    if (plain.isEmpty || !live()) return;
    if (!_handleRichPaste(plain, plain, false, requireFocus: requireFocus)) {
      final sel = _controller.selection;
      if (sel != null && !sel.isCollapsed) _controller.deleteSelection();
      _controller.insertTextAtCaret(plain);
      _syncImeFromSelection(force: true);
    }
  }

  static final RegExp _bareUrlRe = RegExp(
    r'^https?://\S+$',
    caseSensitive: false,
  );

  bool _looksLikeUrl(String text) =>
      text.isNotEmpty && _bareUrlRe.hasMatch(text);

  static final RegExp _imageUrlRe = RegExp(
    r'^https?://\S+\.(png|jpe?g|gif|webp|bmp|svg|avif)(\?\S*)?$',
    caseSensitive: false,
  );

  bool _looksLikeImageUrl(String text) =>
      !text.contains(RegExp(r'\s')) && _imageUrlRe.hasMatch(text);

  Future<void> _importAndInsertImageUrl(String url) async {
    final import = widget.onImportImageUrl;
    if (import == null) return;
    final result = await import(url);
    if (!mounted) return;
    if (result == null) {
      // Re-hosting failed (commonly: the SERVER can't reach that host, even
      // though this client can). Dropping the paste on the floor lost the
      // content outright — keep the block on its external url instead, and let
      // [_requestImage] load it directly.
      _controller.insertImage(url: url);
      _syncImeFromSelection(force: true);
      return;
    }
    _controller.insertImage(fileId: result.fileId, name: result.name);
    _syncImeFromSelection(force: true);
  }

  /// External image urls with a re-host CURRENTLY in flight. [_requestImage]
  /// skips these (the url is about to become a file_id — fetching it would be
  /// a wasted CORS round trip), and only these: gating on the global
  /// `reHostImages` flag instead meant a url was skipped even when no re-host
  /// was running or possible — re-hosting only ever fires right after a paste,
  /// so an image block arriving any other way (a synced doc, a reopened page,
  /// a failed re-host) stayed blank forever, waiting on a file_id nobody was
  /// coming to create.
  final Set<String> _rehostPending = {};

  /// Re-host any image blocks that still reference an external `url` (e.g. from
  /// pasted/AI Markdown) into our own storage, so they render on the canvas and
  /// can't rot. Runs in the background; each conversion repaints when done.
  /// Best-effort: on failure the block keeps its url and renders from there.
  void _rehostExternalImages() {
    if (!widget.reHostImages) return;
    final import = widget.onImportImageUrl;
    if (import == null) return;
    for (final node in [..._controller.nodes]) {
      if (node.kind != 'image') continue;
      final url = node.data['url'] as String?;
      if (node.data['file_id'] != null ||
          url == null ||
          !url.startsWith('http')) {
        continue;
      }
      if (!_rehostPending.add(url)) continue; // already in flight
      _rehostOne(node.id, url).whenComplete(() {
        if (!mounted) {
          _rehostPending.remove(url);
          return;
        }
        // Repaint either way: on success the block now has a file_id; on
        // failure clearing `pending` lets [_requestImage] load the url.
        setState(() => _rehostPending.remove(url));
      });
    }
  }

  /// Move one external image into our storage. Tries the SERVER first (it can
  /// stream straight into storage), then falls back to doing it from HERE:
  /// fetch the bytes and upload them like any picked/pasted image.
  ///
  /// The fallback is the whole point. Server-side import fails routinely for
  /// reasons that have nothing to do with the link being bad — a CN-hosted
  /// server has no route to medium/imgur/… while this client reaches them
  /// fine. Without it the doc silently keeps depending on a link that can rot.
  /// Returns true when the block ended up on a file_id.
  Future<bool> _rehostOne(String nodeId, String url) async {
    final import = widget.onImportImageUrl;
    if (import != null) {
      try {
        final result = await import(url);
        if (result != null) {
          if (!mounted) return false;
          _controller.setImageSource(
            nodeId,
            fileId: result.fileId,
            name: result.name,
          );
          return true;
        }
      } catch (_) {
        // fall through to the client-side attempt
      }
    }
    final load = widget.onLoadImageBytes;
    final upload = widget.onUploadImage;
    if (load == null || upload == null) return false;
    try {
      // On web this can be blocked by the source's CORS policy — then the
      // bytes are unreadable here too and the block stays on its url.
      final bytes = await load(url);
      if (bytes == null || bytes.isEmpty || !mounted) return false;
      final name = _imageNameFromUrl(url);
      final result = await upload(bytes, name, _mimeFromName(name));
      if (result == null || !mounted) return false;
      _primeImage(result.fileId, bytes);
      _controller.setImageSource(
        nodeId,
        fileId: result.fileId,
        name: result.name,
      );
      return true;
    } catch (_) {
      return false;
    }
  }

  /// A filename for an image url: its last path segment (query stripped), or a
  /// generic fallback when the url carries no usable name.
  static String _imageNameFromUrl(String url) {
    final path = Uri.tryParse(url)?.path ?? '';
    final seg = path.split('/').where((s) => s.isNotEmpty).lastOrNull ?? '';
    final name = Uri.decodeComponent(seg);
    return name.contains('.') ? name : 'image.png';
  }

  // ---------------------------------------------------------------------------
  // Caret blink
  // ---------------------------------------------------------------------------

  void _restartBlink() {
    _blink?.cancel();
    _caretOn = true;
    if (_focus.hasFocus && widget.canEdit) {
      _blink = Timer.periodic(const Duration(milliseconds: 530), (_) {
        if (!mounted) return;
        setState(() => _caretOn = !_caretOn);
      });
    }
  }

  // ---------------------------------------------------------------------------
  // IME / text input connection
  // ---------------------------------------------------------------------------

  void _attachIme() {
    // Desktop (Flutter 3.44 multi-view): TextInput.attach resolves the view id
    // from the focused node. Attaching before requestFocus() has actually
    // landed (hasFocus still false) yields a zombie connection — the engine
    // rejects setClient with "view id is null", yet the Dart side reports
    // attached=true, so every later setEditingState fails "no client set".
    // Only attach once focus is genuinely held; the focus listener re-runs this.
    if (!_focus.hasFocus) return;
    if (_conn != null && _conn!.attached) {
      _syncImeFromSelection();
      return;
    }
    _conn = TextInput.attach(
      this,
      TextInputConfiguration(
        // Multi-view (Flutter 3.4x+): the engine's setClient requires the target
        // FlutterView's id. Standard EditableText sets it; a raw
        // TextInput.attach must too, or desktop rejects with "view id is null"
        // (the connection then silently no-ops every setEditingState).
        viewId: View.of(context).viewId,
        inputType: TextInputType.multiline,
        inputAction: TextInputAction.newline,
        autocorrect: false,
        enableSuggestions: false,
        keyboardAppearance: Brightness.light,
      ),
    );
    _syncImeFromSelection(force: true);
    _conn!.show();
  }

  void _detachIme() {
    _conn?.close();
    _conn = null;
  }

  /// Result for keys we want the platform text-input connection to act on
  /// (typed characters, within-node backspace/delete). On desktop these must
  /// NOT be marked consumed, or the engine won't route them to the IME
  /// connection (the character/deletion is silently dropped). On web the hidden
  /// DOM textarea bypasses this, so skipRemainingHandlers is safe there and also
  /// stops the key bubbling to app-level shortcuts (Space-scroll, input rules).
  KeyEventResult get _passToTextInput =>
      kIsWeb ? KeyEventResult.skipRemainingHandlers : KeyEventResult.ignored;

  TextEditingValue _imeValue() {
    final sel = _controller.selection;
    final nodes = _controller.nodes;
    if (sel == null || sel.focus.node >= nodes.length) {
      return TextEditingValue.empty;
    }
    final i = sel.focus.node;
    final text = nodes[i].text;
    final base = sel.anchor.node == i ? sel.anchor.offset : sel.focus.offset;
    return TextEditingValue(
      text: text,
      selection: TextSelection(
        baseOffset: base.clamp(0, text.length),
        extentOffset: sel.focus.offset.clamp(0, text.length),
      ),
      composing: (_composing.isValid && _composing.end <= text.length)
          ? _composing
          : TextRange.empty,
    );
  }

  /// Push our selection/text down to the OS connection (after we move the caret
  /// or change structure). Not called during typing — the OS is the source then.
  void _syncImeFromSelection({bool force = false}) {
    final conn = _conn;
    if (conn == null || !conn.attached) return;
    final value = _imeValue();
    if (!force && value == _lastSentIme) return;
    _lastSentIme = value;
    conn.setEditingState(value);
    final sel = _controller.selection;
    final rect = sel == null ? null : _render?.caretRectFor(sel.focus);
    if (rect != null) conn.setCaretRect(rect);
  }

  // TextInputClient ----------------------------------------------------------

  @override
  TextEditingValue get currentTextEditingValue => _imeValue();

  @override
  AutofillScope? get currentAutofillScope => null;

  @override
  void updateEditingValue(TextEditingValue value) {
    _imeComposing = value.composing.isValid;
    _composing = value.composing.isValid ? value.composing : TextRange.empty;
    final node = _controller.focusedNode;
    if (node == null) return;

    // IME composition in progress (pinyin/kana): mirror the marked text into the
    // focused block so it shows, and keep our editing value — including the
    // composing range, via _imeValue/currentTextEditingValue — identical to what
    // the OS sent. That parity is what stops a composition-style IME (Microsoft
    // Pinyin) from desyncing and accumulating garbage. Crucially, do NOT run
    // commit-time logic (newline split, paste detection, Markdown input rules)
    // on a transient composition — that waits until composing collapses.
    if (value.composing.isValid && !value.composing.isCollapsed) {
      final cbase = value.selection.baseOffset;
      final cext = value.selection.extentOffset;
      _controller.setFocusedText(
        value.text,
        cbase < 0 ? value.text.length : cbase,
        cext < 0 ? value.text.length : cext,
      );
      _lastSentIme = value;
      _closeLinkBar();
      final f = _controller.selection?.focus;
      final rect = f == null ? null : _render?.caretRectFor(f);
      if (rect != null) _conn?.setCaretRect(rect);
      return;
    }

    final shift = HardwareKeyboard.instance.isShiftPressed;
    final text = value.text;

    // Newline accounting is INCREMENTAL against the node's current text:
    // pasted multi-line quotes legitimately live as one multi-line block, so
    // absolute counts misread any edit inside them as a paste (re-parsing
    // the bare lines stripped their quote identity) and the old
    // first-newline split cut them at the wrong line.
    final oldBreaks = '\n'.allMatches(node.text).length;
    final newBreaks = '\n'.allMatches(text).length;

    // A chunk bringing 2+ NEW newlines at once is a paste: parse it as
    // Markdown into structured blocks instead of one literal block.
    if (!node.isCode && !shift && newBreaks >= oldBreaks + 2) {
      _controller.replaceFocusedWithBlocks(markdownToBlocks(text));
      _rehostExternalImages();
      _lastSentIme = _imeValue();
      _syncImeFromSelection(force: true);
      return;
    }

    // Exactly one NEW newline is Enter: split at the just-typed newline (the
    // one at the caret — NOT the first in the text, which in a multi-line
    // block is some older soft break). Code blocks and Shift+Enter keep the
    // newline as a soft break inside the node.
    if (newBreaks == oldBreaks + 1 && !node.isCode && !shift) {
      final caret = value.selection.baseOffset.clamp(0, text.length);
      final idx = (caret > 0 && text[caret - 1] == '\n')
          ? caret - 1
          : text.indexOf('\n');
      _controller.applyNewlineSplit(
        text.substring(0, idx),
        text.substring(idx + 1),
      );
      _lastSentIme = _imeValue();
      _syncImeFromSelection(force: true);
      return;
    }

    // A newline typed inside a code block is a soft break that copies the
    // previous line's leading whitespace (auto-indent), so nested code keeps
    // its column. Detect the single `\n` the IME just inserted at the caret.
    final base = value.selection.baseOffset;
    final ext = value.selection.extentOffset;
    if (node.isCode &&
        text.contains('\n') &&
        base == ext &&
        base > 0 &&
        text.length == node.text.length + 1 &&
        text[base - 1] == '\n') {
      _controller.insertCodeNewline(base);
      _lastSentIme = _imeValue();
      _syncImeFromSelection(force: true);
      return;
    }

    _controller.setFocusedText(
      text,
      base < 0 ? text.length : base,
      ext < 0 ? text.length : ext,
    );
    _lastSentIme = value;

    // Any text change can shift mark ranges — drop the hover link bar.
    _closeLinkBar();

    // Markdown input rules take precedence; a conversion strips the marker.
    if (_controller.applyInputRules()) {
      _closeSlash();
      _closePageLink();
      _syncImeFromSelection(force: true);
      return;
    }
    _refreshSlash();
    _refreshPageLink();
  }

  @override
  void performAction(TextInputAction action) {
    // Enter arrives as a newline in the editing value (handled there); nothing
    // else to do for the newline action.
  }

  @override
  void performPrivateCommand(String action, Map<String, dynamic> data) {}

  @override
  void updateFloatingCursor(RawFloatingCursorPoint point) {}

  @override
  void showAutocorrectionPromptRect(int start, int end) {}

  @override
  void insertTextPlaceholder(Size size) {}

  @override
  void removeTextPlaceholder() {}

  @override
  bool onFocusReceived() => false;

  @override
  void connectionClosed() {
    _conn = null;
    // The engine drops the connection on its own at times (web rebuilds its
    // hidden textarea; overlay TextFields borrow the singleton TextInput and
    // hand it back closed). Our FocusNode never blinked, so _onFocusChange
    // will NOT re-attach — without this the editor keeps a blinking caret
    // that hears nothing until the next focus round-trip.
    if (mounted && _focus.hasFocus && widget.canEdit) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted && _focus.hasFocus && widget.canEdit && _conn == null) {
          _attachIme();
        }
      });
    }
  }

  @override
  void didChangeInputControl(
    TextInputControl? oldControl,
    TextInputControl? newControl,
  ) {}

  @override
  void performSelector(String selectorName) {}

  @override
  void insertContent(KeyboardInsertedContent content) {}

  @override
  void showToolbar() {}

  // ---------------------------------------------------------------------------
  // Keyboard
  // ---------------------------------------------------------------------------

  KeyEventResult _onKey(FocusNode node, KeyEvent event) {
    if (!widget.canEdit) return KeyEventResult.ignored;
    if (event is! KeyDownEvent && event is! KeyRepeatEvent) {
      return KeyEventResult.ignored;
    }
    final sel = _controller.selection;
    if (sel == null) return KeyEventResult.ignored;

    final key = event.logicalKey;
    final hw = HardwareKeyboard.instance;
    final shift = hw.isShiftPressed;
    final accel = hw.isControlPressed || hw.isMetaPressed;

    // The `[[` page-link picker, when open, captures the same keys as the
    // slash menu.
    if (_pageEntry != null) {
      final items = _filteredPages();
      if (key == LogicalKeyboardKey.escape) {
        _closePageLink();
        return KeyEventResult.handled;
      }
      if (key == LogicalKeyboardKey.arrowDown) {
        if (items.isNotEmpty) _pageIndex = (_pageIndex + 1) % items.length;
        _pageEntry?.markNeedsBuild();
        return KeyEventResult.handled;
      }
      if (key == LogicalKeyboardKey.arrowUp) {
        if (items.isNotEmpty) {
          _pageIndex = (_pageIndex - 1 + items.length) % items.length;
        }
        _pageEntry?.markNeedsBuild();
        return KeyEventResult.handled;
      }
      if (key == LogicalKeyboardKey.enter || key == LogicalKeyboardKey.tab) {
        if (items.isNotEmpty) {
          _applyPageLink(items[_pageIndex.clamp(0, items.length - 1)]);
        } else {
          _closePageLink();
        }
        return KeyEventResult.handled;
      }
    }

    // The slash menu, when open, captures navigation and commit/dismiss keys.
    if (_slashEntry != null) {
      final items = _filteredSlash();
      if (key == LogicalKeyboardKey.escape) {
        _closeSlash();
        return KeyEventResult.handled;
      }
      if (key == LogicalKeyboardKey.arrowDown) {
        if (items.isNotEmpty) _slashIndex = (_slashIndex + 1) % items.length;
        _slashEntry?.markNeedsBuild();
        return KeyEventResult.handled;
      }
      if (key == LogicalKeyboardKey.arrowUp) {
        if (items.isNotEmpty) {
          _slashIndex = (_slashIndex - 1 + items.length) % items.length;
        }
        _slashEntry?.markNeedsBuild();
        return KeyEventResult.handled;
      }
      if (key == LogicalKeyboardKey.enter || key == LogicalKeyboardKey.tab) {
        if (items.isNotEmpty) {
          _applySlash(items[_slashIndex.clamp(0, items.length - 1)]);
        } else {
          _closeSlash();
        }
        return KeyEventResult.handled;
      }
    }

    // Undo / redo: Ctrl/Cmd+Z, and Ctrl/Cmd+Shift+Z or Ctrl+Y to redo.
    if (accel && key == LogicalKeyboardKey.keyZ) {
      if (shift) {
        _controller.redo();
      } else {
        _controller.undo();
      }
      _syncImeFromSelection(force: true);
      return KeyEventResult.handled;
    }
    if (accel && key == LogicalKeyboardKey.keyY) {
      _controller.redo();
      _syncImeFromSelection(force: true);
      return KeyEventResult.handled;
    }

    if (accel && key == LogicalKeyboardKey.keyA) {
      _selectAll();
      return KeyEventResult.handled;
    }

    // A table cell-area selection (row / column / dragged rectangle) captures
    // the clipboard + delete keys: copy as TSV+HTML, cut/delete blanks cells.
    final area = _render?.tableBlockSelection;
    if (area != null) {
      if (key == LogicalKeyboardKey.escape) {
        _render?.tableBlockSelection = null;
        return KeyEventResult.handled;
      }
      if (accel &&
          (key == LogicalKeyboardKey.keyC || key == LogicalKeyboardKey.keyX)) {
        _copyTableArea(area, cut: key == LogicalKeyboardKey.keyX);
        return KeyEventResult.handled;
      }
      if (key == LogicalKeyboardKey.delete ||
          key == LogicalKeyboardKey.backspace) {
        _controller.clearTableCells(
          area.node,
          area.r0,
          area.c0,
          area.r1,
          area.c1,
        );
        return KeyEventResult.handled;
      }
    }

    if (accel && key == LogicalKeyboardKey.keyC) {
      return _copySelection() ? KeyEventResult.handled : KeyEventResult.ignored;
    }

    if (accel && key == LogicalKeyboardKey.keyX) {
      return _cutSelection() ? KeyEventResult.handled : KeyEventResult.ignored;
    }

    // Web's DOM paste interceptor (setRichPasteHandler) handles Ctrl+V; on
    // desktop there is no such hook, so read the clipboard and route it through
    // the same paste logic ourselves. Ctrl+Shift+V pastes as PLAIN text.
    if (accel && key == LogicalKeyboardKey.keyV && !kIsWeb) {
      if (shift) {
        _pastePlainText();
      } else {
        _pasteFromClipboard();
      }
      return KeyEventResult.handled;
    }

    // Inline formatting over a ranged selection.
    if (accel && key == LogicalKeyboardKey.keyB) {
      _controller.toggleMark('bold');
      return KeyEventResult.handled;
    }
    if (accel && key == LogicalKeyboardKey.keyI) {
      _controller.toggleMark('italic');
      return KeyEventResult.handled;
    }
    if (accel && key == LogicalKeyboardKey.keyE) {
      _controller.toggleMark('code');
      return KeyEventResult.handled;
    }
    if (accel && key == LogicalKeyboardKey.keyK) {
      _promptLink();
      return KeyEventResult.handled;
    }

    // Heading level shortcuts (Notion/Word convention): Ctrl/Cmd+Alt+1…6 sets
    // H1–H6 on the selected block(s), Ctrl/Cmd+Alt+0 back to plain text. Not
    // Typora's bare Ctrl+digit — the browser owns Ctrl+1…9 for tab switching
    // on the web build, so the app would never see it there.
    if (accel && HardwareKeyboard.instance.isAltPressed) {
      final int? digit = switch (key) {
        LogicalKeyboardKey.digit0 || LogicalKeyboardKey.numpad0 => 0,
        LogicalKeyboardKey.digit1 || LogicalKeyboardKey.numpad1 => 1,
        LogicalKeyboardKey.digit2 || LogicalKeyboardKey.numpad2 => 2,
        LogicalKeyboardKey.digit3 || LogicalKeyboardKey.numpad3 => 3,
        LogicalKeyboardKey.digit4 || LogicalKeyboardKey.numpad4 => 4,
        LogicalKeyboardKey.digit5 || LogicalKeyboardKey.numpad5 => 5,
        LogicalKeyboardKey.digit6 || LogicalKeyboardKey.numpad6 => 6,
        _ => null,
      };
      if (digit != null) {
        if (digit == 0) {
          _controller.setSelectedBlocksKind('paragraph');
        } else {
          _controller.setSelectedBlocksKind('heading', data: {'level': digit});
        }
        _syncImeFromSelection(force: true);
        return KeyEventResult.handled;
      }
    }

    // When a whole atomic block (image/divider/table) is the caret stop: delete
    // removes it; arrows step off it to the adjacent node.
    final atomic = sel.isCollapsed && sel.focus.node < _controller.nodes.length
        ? _controller.nodes[sel.focus.node]
        : null;
    if (atomic != null && atomic.isAtomic) {
      if (key == LogicalKeyboardKey.backspace ||
          key == LogicalKeyboardKey.delete) {
        _controller.deleteNode(sel.focus.node);
        _syncImeFromSelection(force: true);
        return KeyEventResult.handled;
      }
      if (key == LogicalKeyboardKey.arrowUp ||
          key == LogicalKeyboardKey.arrowLeft) {
        _moveVertical(-1, shift);
        return KeyEventResult.handled;
      }
      if (key == LogicalKeyboardKey.arrowDown ||
          key == LogicalKeyboardKey.arrowRight) {
        _moveVertical(1, shift);
        return KeyEventResult.handled;
      }
    }

    // A ranged selection (possibly across blocks) is deleted as a unit — the
    // OS input only knows the focused node, so we must do it ourselves.
    if ((key == LogicalKeyboardKey.backspace ||
            key == LogicalKeyboardKey.delete) &&
        !sel.isCollapsed) {
      if (_controller.deleteSelection()) {
        _syncImeFromSelection();
        return KeyEventResult.handled;
      }
    }

    // Tab / Shift+Tab: indent or outdent list items; inside a code block a
    // Tab inserts two spaces. Always handled — the browser must never steal
    // focus from the editor on Tab.
    if (key == LogicalKeyboardKey.tab) {
      final n = _controller.focusedNode;
      if (n != null && n.isCode && sel.isCollapsed && !shift) {
        // At the very start of a free-standing code block, Tab attaches it
        // to the list item above; anywhere else it indents the code text.
        final attach =
            sel.focus.offset == 0 && _controller.canAttachToItem(n.id);
        if (!attach) {
          _controller.insertTextAtCaret('  ');
          _syncImeFromSelection(force: true);
          return KeyEventResult.handled;
        }
      }
      if (_controller.indentSelection(shift ? -1 : 1)) {
        _syncImeFromSelection(force: true);
      }
      return KeyEventResult.handled;
    }

    if (key == LogicalKeyboardKey.backspace) {
      if (sel.isCollapsed && sel.focus.offset == 0) {
        // A nested list item outdents before any merge happens.
        final n = _controller.focusedNode;
        if (n != null && n.isListKind && n.indent > 0) {
          _controller.indentSelection(-1);
          _syncImeFromSelection(force: true);
          return KeyEventResult.handled;
        }
        if (_controller.mergeBackward()) {
          _syncImeFromSelection();
          return KeyEventResult.handled;
        }
        // Nothing above to merge into → step up into the page title.
        if (sel.focus.node == 0 && widget.onExitTop != null) {
          widget.onExitTop!();
          return KeyEventResult.handled;
        }
        return KeyEventResult.skipRemainingHandlers;
      }
      // A caret at a typeset formula's trailing edge deletes the whole formula
      // — its source is an atom, not backspaceable one character at a time.
      // Before both the web and desktop grapheme paths, since it replaces them.
      if (sel.isCollapsed && _controller.deleteMathAtomBackward()) {
        _syncImeFromSelection(force: true);
        return KeyEventResult.handled;
      }
      // Within-node backspace. The desktop embedder does NOT route Backspace to
      // a raw text-input client (only typed characters reach updateEditingValue),
      // so delete the grapheme before the caret ourselves. Web's hidden textarea
      // still handles it, so delegate there.
      if (!kIsWeb && !_imeComposing && sel.isCollapsed) {
        final n = _controller.focusedNode;
        final o = sel.focus.offset;
        if (n != null && o > 0 && o <= n.text.length) {
          final head = n.text.substring(0, o).characters.skipLast(1).toString();
          _controller.setFocusedText(
            head + n.text.substring(o),
            head.length,
            head.length,
          );
          _syncImeFromSelection(force: true);
          return KeyEventResult.handled;
        }
      }
      return _passToTextInput;
    }

    if (key == LogicalKeyboardKey.delete) {
      final n = _controller.focusedNode;
      if (sel.isCollapsed && n != null && sel.focus.offset == n.text.length) {
        if (_controller.mergeForward()) {
          _syncImeFromSelection();
          return KeyEventResult.handled;
        }
        return KeyEventResult.skipRemainingHandlers;
      }
      // Delete at a formula's leading edge removes it whole (mirror of the
      // Backspace case above).
      if (sel.isCollapsed && _controller.deleteMathAtomForward()) {
        _syncImeFromSelection(force: true);
        return KeyEventResult.handled;
      }
      // Within-node forward delete: same desktop caveat as backspace.
      if (!kIsWeb && !_imeComposing && sel.isCollapsed) {
        final n = _controller.focusedNode;
        final o = sel.focus.offset;
        if (n != null && o < n.text.length) {
          final tail = n.text.substring(o).characters.skip(1).toString();
          _controller.setFocusedText(n.text.substring(0, o) + tail, o, o);
          _syncImeFromSelection(force: true);
          return KeyEventResult.handled;
        }
      }
      return _passToTextInput;
    }

    if (key == LogicalKeyboardKey.arrowLeft) {
      _moveHorizontal(-1, shift);
      return KeyEventResult.handled;
    }
    if (key == LogicalKeyboardKey.arrowRight) {
      _moveHorizontal(1, shift);
      return KeyEventResult.handled;
    }
    if (key == LogicalKeyboardKey.arrowUp) {
      _moveVertical(-1, shift);
      return KeyEventResult.handled;
    }
    if (key == LogicalKeyboardKey.arrowDown) {
      _moveVertical(1, shift);
      return KeyEventResult.handled;
    }
    if (key == LogicalKeyboardKey.home) {
      _moveLineEdge(false, shift);
      return KeyEventResult.handled;
    }
    if (key == LogicalKeyboardKey.end) {
      _moveLineEdge(true, shift);
      return KeyEventResult.handled;
    }

    // Text-producing keys (including Space) must reach the platform text input
    // so the character is inserted, but must NOT bubble to the app's default
    // shortcuts — Space is bound to scroll/activate at the app level and would
    // otherwise be swallowed (breaking `# `, `- `, `1. `, `> ` input rules).
    //
    // Web vs desktop divergence: on web, skipRemainingHandlers stops the app
    // shortcuts while the hidden DOM textarea still feeds the character to the
    // IME connection. On desktop there is no such bypass — skipRemainingHandlers
    // (like handled) marks the key consumed, so the engine never generates the
    // text-input event and the character is dropped. Return ignored on desktop
    // so typing reaches updateEditingValue.
    if (event.character != null && event.character!.isNotEmpty) {
      return _passToTextInput;
    }
    return KeyEventResult.ignored;
  }

  void _apply(DocPosition target, bool shift, {bool keepGoalX = false}) {
    final sel = _controller.selection!;
    _controller.setSelection(
      shift
          ? DocSelection(anchor: sel.anchor, focus: target)
          : DocSelection.collapsed(target),
      keepGoalX: keepGoalX,
    );
    _syncImeFromSelection();
  }

  void _moveHorizontal(int dir, bool shift) {
    final sel = _controller.selection!;
    final nodes = _controller.nodes;
    if (!shift && !sel.isCollapsed) {
      _apply(dir < 0 ? sel.start : sel.end, false);
      return;
    }
    final cur = sel.focus;
    DocPosition target = cur;
    if (dir < 0) {
      if (cur.offset > 0) {
        target = cur.withOffset(cur.offset - 1);
      } else if (cur.node > 0) {
        target = DocPosition(cur.node - 1, nodes[cur.node - 1].text.length);
      }
    } else {
      final len = nodes[cur.node].text.length;
      if (cur.offset < len) {
        target = cur.withOffset(cur.offset + 1);
      } else if (cur.node + 1 < nodes.length) {
        target = DocPosition(cur.node + 1, 0);
      }
    }
    _apply(target, shift);
  }

  void _moveVertical(int dir, bool shift) {
    final r = _render;
    if (r == null) return;
    final focus = _controller.selection!.focus;
    _controller.goalX ??= r.caretRectFor(focus)?.left;
    final target = dir < 0
        ? r.positionAbove(focus, _controller.goalX)
        : r.positionBelow(focus, _controller.goalX);

    // Upward escape from the very top: Up on the document's first line moves
    // focus into the page title.
    if (dir < 0 &&
        !shift &&
        focus.node == 0 &&
        (target == null || target == focus) &&
        widget.onExitTop != null) {
      widget.onExitTop!();
      return;
    }

    // Downward escape from a trailing code block: Enter inside code inserts a
    // newline, so Down on the last line of a last-node code block creates a new
    // paragraph below instead of going nowhere.
    if (dir > 0 && !shift && (target == null || target == focus)) {
      final nodes = _controller.nodes;
      final node = focus.node < nodes.length ? nodes[focus.node] : null;
      if (node != null && node.isCode && focus.node == nodes.length - 1) {
        _controller.addParagraphAfterLast();
        _syncImeFromSelection();
        return;
      }
    }

    if (target == null) return;
    _apply(target, shift, keepGoalX: true);
  }

  void _moveLineEdge(bool end, bool shift) {
    final r = _render;
    if (r == null) return;
    final focus = _controller.selection!.focus;
    final target = end ? r.lineEnd(focus) : r.lineStart(focus);
    _apply(target, shift);
  }

  void _selectAll() {
    final nodes = _controller.nodes;
    if (nodes.isEmpty) return;
    final sel = _controller.selection;
    final node = _controller.focusedNode;

    // In a code block, Ctrl/Cmd+A first selects only the code; press again (when
    // the whole code node is already selected) to escalate to the whole document.
    if (node != null && node.isCode && sel != null) {
      final i = sel.focus.node;
      final wholeNode =
          sel.start.node == i &&
          sel.end.node == i &&
          sel.start.offset == 0 &&
          sel.end.offset == node.text.length;
      if (!wholeNode) {
        _controller.setSelection(
          DocSelection(
            anchor: DocPosition(i, 0),
            focus: DocPosition(i, node.text.length),
          ),
        );
        _syncImeFromSelection();
        return;
      }
    }

    _controller.setSelection(
      DocSelection(
        anchor: const DocPosition(0, 0),
        focus: DocPosition(nodes.length - 1, nodes.last.text.length),
      ),
    );
    _syncImeFromSelection();
  }

  // ---------------------------------------------------------------------------
  // Pointer
  // ---------------------------------------------------------------------------

  void _onTapDown(TapDownDetails d) {
    final r = _render;
    if (r == null) return;
    final local = r.globalToLocal(d.globalPosition);
    // A plain click on a link opens it (also on read-only pages); placing the
    // caret inside link text is keyboard/edge territory, like Notion.
    final href = _linkHitAt(r, local);
    if (href != null) {
      _openHref(href);
      return;
    }
    _closeLinkBar();
    if (!widget.canEdit) return;
    _closeSlash();
    _closePageLink();
    // Any click clears a table row/column block-selection; the row/column
    // handle branches below re-set it when that's what was clicked.
    r.tableBlockSelection = null;
    final mathIdx = r.blockAt(local);
    if (mathIdx != null &&
        mathIdx < _controller.nodes.length &&
        _controller.nodes[mathIdx].kind == 'math_block') {
      _editMathBlock(_controller.nodes[mathIdx]);
      return;
    }
    // A typeset inline formula is an atom: clicking it opens the source editor
    // rather than placing a caret inside (AppFlowy/AFFiNE/Notion all do this).
    final inlineMath = r.inlineMathAt(local);
    if (inlineMath != null) {
      _focus.requestFocus();
      _editInlineMath(
        inlineMath.node,
        inlineMath.start,
        inlineMath.end,
        inlineMath.source,
      );
      return;
    }
    final langNode = r.codeLanguageAt(local);
    if (langNode != null) {
      _openLanguageMenu(langNode, d.globalPosition);
      return;
    }
    final copyNode = r.codeCopyAt(local);
    if (copyNode != null) {
      _copyCode(copyNode);
      return;
    }
    // Image hover toolbar (expand / align / delete).
    final imageAction = r.imageActionAt(local);
    if (imageAction != null) {
      _onImageAction(imageAction.node, imageAction.action);
      return;
    }
    // Double-click the picture itself → fullscreen viewer. The toolbar's
    // expand button is easy to miss, and double-click-to-zoom is what every
    // other image surface does. Single click still just selects the block.
    final imageNode = r.imageAt(local);
    if (imageNode != null) {
      _focus.requestFocus();
      _controller.collapseTo(DocPosition(imageNode, 0));
      if (_bumpTapCount(local) >= 2) {
        _tapCount = 0; // consumed: a third click shouldn't re-open it
        _openImageViewer(imageNode);
      }
      _syncImeFromSelection(force: true);
      return;
    }
    // A block with nothing to click into (a divider) takes the click as
    // "select me", so it can be seen and deleted at all.
    final blockNode = r.blockSelectAt(local);
    if (blockNode != null) {
      _focus.requestFocus();
      _controller.collapseTo(DocPosition(blockNode, 0));
      _syncImeFromSelection(force: true);
      return;
    }
    // A click anywhere outside the diagram blocks restores their natural
    // zoom/pan (the explicit reset gesture — hover-leave was too eager).
    r.resetPreviewViewsOutside(local);
    final viewTab = r.viewTabAt(local);
    if (viewTab != null) {
      _controller.setCodeView(viewTab.node, viewTab.view);
      return;
    }
    final wrapNode = r.codeWrapAt(local);
    if (wrapNode != null) {
      _controller.toggleCodeWrap(wrapNode);
      return;
    }
    final bar = r.scrollbarAt(local);
    if (bar != null) {
      r.setCodeScrollByTrackX(bar, local.dx);
      return;
    }
    final tableDel = r.tableDeleteAt(local);
    if (tableDel != null) {
      _closeCellEditor();
      _controller.deleteNode(tableDel);
      return;
    }
    final tableHandle = r.tableHandleAt(local);
    if (tableHandle != null) {
      _openTableMenu(tableHandle, d.globalPosition);
      return;
    }
    final rowHandle = r.tableRowHandleAt(local);
    if (rowHandle != null) {
      // Clicking a row handle block-selects the whole row (highlight) and opens
      // its menu — "select by row" without a cross-cell text selection.
      final t = TableData.fromBlock(_controller.nodes[rowHandle.node].data);
      r.tableBlockSelection = (
        node: rowHandle.node,
        r0: rowHandle.row,
        c0: 0,
        r1: rowHandle.row,
        c1: (t.columns - 1).clamp(0, 1 << 30),
      );
      _openRowMenu(rowHandle.node, rowHandle.row, d.globalPosition);
      return;
    }
    final colHandle = r.tableColHandleAt(local);
    if (colHandle != null) {
      final t = TableData.fromBlock(_controller.nodes[colHandle.node].data);
      r.tableBlockSelection = (
        node: colHandle.node,
        r0: 0,
        c0: colHandle.col,
        r1: (t.rowCount - 1).clamp(0, 1 << 30),
        c1: colHandle.col,
      );
      _openColumnMenu(colHandle.node, colHandle.col, d.globalPosition);
      return;
    }
    final add = r.tableAddAt(local);
    if (add != null) {
      if (add.column) {
        _controller.insertTableColumn(add.node, 1 << 30);
      } else {
        _controller.insertTableRow(add.node, 1 << 30);
      }
      return;
    }
    final cell = r.tableCellAt(local);
    if (cell != null) {
      _openCellEditor(cell.node, cell.row, cell.col);
      return;
    }
    _focus.requestFocus();
    // Re-arm a dead IME connection (engine-side closes leave the caret
    // blinking but deaf; requestFocus is a no-op when focus never moved).
    _attachIme();
    final cb = r.checkboxAt(local);
    if (cb != null) {
      _controller.toggleTodo(cb);
      return;
    }
    if (local.dy > r.contentBottom) {
      _tapCount = 0;
      _controller.appendOrFocusLast();
      _syncImeFromSelection();
      return;
    }
    // Count repeated taps at the same spot: 2 → select the word under the
    // caret, 3 → select the whole block's text. Atomic blocks were handled
    // above (e.g. a math double-tap opened its editor), so by here we are on a
    // text block and multi-tap means text selection.
    final count = _bumpTapCount(local);
    final pos = r.positionAt(local);
    var handled = false;
    if (count == 2) {
      handled = _controller.selectWordAt(pos);
    } else if (count >= 3) {
      handled = _controller.selectBlockText(pos.node);
    }
    if (!handled) _controller.collapseTo(pos);
    _syncImeFromSelection();
  }

  /// Update and return the running tap count for word/block selection. Taps
  /// close in time and position escalate the count; otherwise it resets to 1.
  /// Uses the pointer-down timestamp captured in [_downStamp] (binding clock).
  int _bumpTapCount(Offset local) {
    final now = _downStamp;
    final last = _lastTapStamp;
    final lastPos = _lastTapLocal;
    final near = lastPos != null && (lastPos - local).distance <= _multiTapSlop;
    final soon = last != null && (now - last) <= _multiTapWindow;
    _tapCount = (near && soon) ? _tapCount + 1 : 1;
    _lastTapLocal = local;
    _lastTapStamp = now;
    return _tapCount;
  }

  void _closeCellEditor() {
    // Commit the active editor (saves the edit + disposes safely) rather than
    // just yanking the overlay, which would drop the edit and orphan its
    // controller. `commit` guards re-entry and clears `_cellEntry` itself.
    _commitCellEditor?.call();
    _cellEntry?.remove();
    _cellEntry = null;
    _render?.editingCell = null;
  }

  /// Edit a table cell inline: a borderless field placed exactly over the cell
  /// (so it reads as the cell itself, no nested box). Commits on Enter/tap-out;
  /// Tab / Shift+Tab move to the next / previous cell.
  void _openCellEditor(int node, int row, int col) {
    _closeCellEditor();
    final r = _render;
    if (r == null || node >= _controller.nodes.length) return;
    r.tableBlockSelection =
        null; // editing a cell clears a row/column selection
    final localRect = r.tableCellRect(node, row, col);
    if (localRect == null) return;
    final table = TableData.fromBlock(_controller.nodes[node].data);
    final text = (row < table.rows.length && col < table.rows[row].length)
        ? table.rows[row][col]
        : '';
    final topLeft = r.localToGlobal(localRect.topLeft);
    // WYSIWYG cell editor: renders inline marks (bold/italic/code) live instead
    // of the raw Markdown source, and serializes back to Markdown on commit.
    final controller = CellEditController(text);
    final focus = FocusNode();
    var committed = false;
    late OverlayEntry entry;

    // Grow/shrink the cell's row live as its text changes — a preview relayout
    // (no op) so pressing Enter expands the table immediately, not only after
    // clicking away. Also keeps the floating format bar in step with the cell's
    // selection. The final value is persisted by [commit].
    void onCellChanged() {
      _controller.previewTableCell(node, row, col, controller.serialize());
      _refreshMarkBar(); // show/route the floating format bar over the cell
      // The preview relayout can MOVE/RESIZE this cell (auto-fit column widths,
      // row growth) — re-anchor the overlay after that layout lands.
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!committed && entry.mounted) entry.markNeedsBuild();
      });
    }

    controller.addListener(onCellChanged);

    void commit() {
      if (committed) return;
      committed = true;
      controller.removeListener(onCellChanged);
      _render?.editingCell = null;
      _activeCell = null;
      // A commit can land MID-DRAG (Esc / Tab / arrow-exit / focus loss while
      // the button is still down); the live drag must not keep driving this
      // controller — it is disposed a frame from now.
      if (_cellDrag != null && identical(_cellDrag!.ctl, controller)) {
        _cellDrag = null;
      }
      _hideMarkBar();
      _controller.setTableCell(node, row, col, controller.serialize());
      focus.removeListener(_cellFocusListener!);
      // Tear the field down FIRST, then dispose its controller/focus after this
      // frame. Disposing them synchronously here races the removed TextField's
      // own teardown (which still reads them) → "used after being disposed".
      if (entry.mounted) entry.remove();
      if (identical(_cellEntry, entry)) _cellEntry = null;
      _commitCellEditor = null;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        controller.dispose();
        focus.dispose();
      });
    }

    _commitCellEditor = commit;

    void moveTo(int nextRow, int nextCol) {
      commit();
      _openCellEditor(node, nextRow, nextCol);
    }

    // Commit when focus is lost (clicking elsewhere) — not on the opening tap.
    _cellFocusListener = () {
      if (!focus.hasFocus) commit();
    };
    focus.addListener(_cellFocusListener!);

    KeyEventResult onKey(FocusNode n, KeyEvent e) {
      if (e is! KeyDownEvent) return KeyEventResult.ignored;
      final cols = table.columns;
      final rows = table.rows.length;
      final sel = controller.selection;
      final atStart = sel.baseOffset == 0 && sel.isCollapsed;
      final atEnd = sel.baseOffset == controller.text.length && sel.isCollapsed;
      final key = e.logicalKey;

      // Inline formatting inside the cell (WYSIWYG marks), same accelerators as
      // the body: Ctrl/Cmd+B/I/E over a ranged selection.
      final accel =
          HardwareKeyboard.instance.isControlPressed ||
          HardwareKeyboard.instance.isMetaPressed;
      if (accel) {
        final mark = switch (key) {
          LogicalKeyboardKey.keyB => 'bold',
          LogicalKeyboardKey.keyI => 'italic',
          LogicalKeyboardKey.keyE => 'code',
          _ => null,
        };
        if (mark != null) {
          controller.toggleMark(mark);
          return KeyEventResult.handled;
        }
      }

      if (key == LogicalKeyboardKey.tab) {
        final shift = HardwareKeyboard.instance.isShiftPressed;
        var nr = row;
        var nc = col + (shift ? -1 : 1);
        if (nc >= cols) {
          nc = 0;
          nr++;
        } else if (nc < 0) {
          nc = cols - 1;
          nr--;
        }
        if (nr >= 0 && nr < rows) moveTo(nr, nc);
        return KeyEventResult.handled;
      }
      if (key == LogicalKeyboardKey.escape) {
        commit();
        return KeyEventResult.handled;
      }
      // Up/Down are line-aware in a multi-line cell: they move within the
      // cell's own VISUAL lines first (soft-wrapped lines count, not just hard
      // \n); only at the first/last visual line do they jump to the cell
      // above/below — and past the table's edge they EXIT the table back into
      // the document. Shift+arrows always stay with the field (selection
      // extension), and a ranged selection collapses natively first.
      if (key == LogicalKeyboardKey.arrowDown ||
          key == LogicalKeyboardKey.arrowUp) {
        final up = key == LogicalKeyboardKey.arrowUp;
        if (HardwareKeyboard.instance.isShiftPressed || !sel.isCollapsed) {
          return KeyEventResult.ignored;
        }
        if (!_cellCaretOnEdgeLine(
          node,
          row,
          col,
          controller,
          sel.baseOffset,
          last: !up,
        )) {
          return KeyEventResult.ignored; // move within the cell's lines
        }
        if (up ? row > 0 : row + 1 < rows) {
          moveTo(up ? row - 1 : row + 1, col);
        } else {
          commit();
          _exitTableCaret(node, up: up);
        }
        return KeyEventResult.handled;
      }
      if (key == LogicalKeyboardKey.arrowLeft && atStart && col > 0) {
        moveTo(row, col - 1);
        return KeyEventResult.handled;
      }
      if (key == LogicalKeyboardKey.arrowRight && atEnd && col + 1 < cols) {
        moveTo(row, col + 1);
        return KeyEventResult.handled;
      }
      return KeyEventResult.ignored;
    }

    entry = OverlayEntry(
      builder: (context) {
        // Anchor to the cell's CURRENT rect each build — the table relayouts
        // under the field while typing (live row growth + auto-fit column
        // widths), and a stale position would visibly detach the editor.
        final rr = _render;
        final liveRect = rr?.tableCellRect(node, row, col) ?? localRect;
        final liveTopLeft = rr != null
            ? rr.localToGlobal(liveRect.topLeft)
            : topLeft;
        return Positioned(
          left: liveTopLeft.dx,
          top: liveTopLeft.dy,
          width: liveRect.width,
          child: Focus(
            onKeyEvent: onKey,
            // The overlay mounts above the app's Material, but TextField needs a
            // Material ancestor (ink/selection) — a transparent one adds no chrome.
            child: Material(
              type: MaterialType.transparency,
              child: Container(
                constraints: BoxConstraints(minHeight: liveRect.height),
                color: Colors.transparent,
                padding: const EdgeInsets.symmetric(
                  horizontal: 10,
                  vertical: 9,
                ),
                child: TextField(
                  controller: controller,
                  focusNode: focus,
                  autofocus: true,
                  maxLines: null,
                  // Enter inserts a newline inside the cell (the cell grows); it
                  // does NOT commit. Commit is click-away / Esc / Tab / arrows.
                  keyboardType: TextInputType.multiline,
                  textInputAction: TextInputAction.newline,
                  cursorColor: const Color(0xFF2563EB),
                  style: const TextStyle(fontSize: 15, height: 1.4),
                  decoration: const InputDecoration(
                    isDense: true,
                    contentPadding: EdgeInsets.zero,
                    border: InputBorder.none,
                    isCollapsed: true,
                  ),
                ),
              ),
            ),
          ),
        );
      },
    );
    _cellEntry = entry;
    _render?.editingCell = (node: node, row: row, col: col);
    _activeCell = (ctl: controller, node: node, row: row, col: col);
    Overlay.of(context).insert(entry);
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!committed) focus.requestFocus();
    });
  }

  /// Lazily fetch + decode an image for [fileId] (called from layout). Repaints
  /// when the image is ready or fails. No-op if already cached/loading/errored.
  void _requestImage(String fileId) {
    if (_imageCache.containsKey(fileId) ||
        _imageLoading.contains(fileId) ||
        _imageErrors.contains(fileId)) {
      return;
    }
    // An external url with a re-host in flight: don't CORS-fetch it, it will
    // become a file_id shortly. Any other url — re-hosting off, already
    // failed, or a block that never went through the paste path at all — is
    // fetched best-effort. See [_rehostPending].
    final isUrl = fileId.startsWith('http://') || fileId.startsWith('https://');
    if (isUrl && _rehostPending.contains(fileId)) return;
    final load = widget.onLoadImageBytes;
    if (load == null) return;
    _imageLoading.add(fileId);
    load(fileId).then((bytes) async {
      if (!mounted) return;
      if (bytes == null) {
        _imageLoading.remove(fileId);
        setState(() => _imageErrors.add(fileId));
        return;
      }
      try {
        await _decodeInto(fileId, bytes);
      } catch (_) {
        if (!mounted) return;
        _imageLoading.remove(fileId);
        setState(() => _imageErrors.add(fileId));
      }
    });
  }

  /// Decode [bytes] under [key] and put them on the canvas: a still image once,
  /// an animated one (GIF / animated WebP) frame by frame via [ImageAnimator].
  Future<void> _decodeInto(String key, Uint8List bytes) async {
    // Claim the key before the first await, so it is claimed synchronously with
    // the call. _primeImage decodes bytes we already hold, but inserting the
    // block relayouts immediately — and the renderer, seeing nothing decoded
    // yet, would ask the host to FETCH the very image we just uploaded. That
    // second decode then overwrote this one's ui.Image without disposing it.
    _imageLoading.add(key);
    final codec = await ui.instantiateImageCodec(bytes);
    if (!mounted) {
      codec.dispose();
      return;
    }
    if (codec.frameCount > 1) {
      _imageAnims.remove(key)?.dispose();
      // Seed the paint mark: nothing has drawn this yet, and the very first
      // frame is what gives the canvas something to draw.
      _imagePainted.add(key);
      final anim = ImageAnimator(
        CodecFrameSource(codec),
        onFrame: (frame) => _onImageFrame(key, frame),
      );
      _imageAnims[key] = anim;
      anim.start();
      return;
    }
    final frame = await codec.getNextFrame();
    codec.dispose();
    if (!mounted) {
      frame.image.dispose();
      return;
    }
    final previous = _imageCache[key];
    _imageLoading.remove(key);
    _imageErrors.remove(key);
    setState(() => _imageCache[key] = frame.image);
    previous?.dispose(); // never orphan the handle we just replaced
  }

  /// One frame of an animated image, owned by us from here on.
  void _onImageFrame(String key, ui.Image frame) {
    if (!mounted) {
      frame.dispose();
      return;
    }
    final first = !_imageCache.containsKey(key);
    if (first) {
      _imageLoading.remove(key);
      _imageErrors.remove(key);
      // The first frame settles the block's natural size, so it has to go
      // through layout. Every frame after is the same size — paint only.
      setState(() => _imageCache[key] = frame);
      return;
    }
    final previous = _imageCache[key];
    _imageCache[key] = frame;
    _render?.replaceImage(key, frame);
    _imageFrameTick.value++;
    previous?.dispose();

    // Nothing drew the last frame: the block was deleted or its source
    // replaced, and decoding on would burn CPU on a picture no one sees. The
    // canvas re-arms the mark whenever it paints, so a block that comes back
    // (undo) revives the loop in [_onImagePainted].
    if (!_imagePainted.remove(key)) _imageAnims[key]?.pause();
  }

  void _onImagePainted(String key) {
    _imagePainted.add(key);
    final anim = _imageAnims[key];
    if (anim != null && !anim.isPlaying) anim.start();
  }

  /// The image name to hang off a blob url, url-encoded. Cosmetic only (the
  /// server ignores it), so a shared link reads as an image instead of ending
  /// in a bare `/blob`. Null when the block has no usable name.
  static String? _blobNameSuffix(EditorNode node) {
    final name = (node.data['name'] as String?)?.trim();
    if (name == null || name.isEmpty || !name.contains('.')) return null;
    // Only the last path segment, and never a traversal — this lands in a url.
    final leaf = name.split(RegExp(r'[/\\]')).last;
    if (leaf.isEmpty || leaf == '.' || leaf == '..') return null;
    return Uri.encodeComponent(leaf);
  }

  /// Resolve fresh download URLs for any image file_ids not yet cached, so copy
  /// can emit working Markdown links synchronously (no gesture-breaking await).
  void _cacheImageUrls() {
    final resolve = widget.onResolveImageUrls;
    if (resolve == null) return;
    final ids = <String>[];
    final names = <String, String>{}; // file_id -> encoded filename
    for (final n in _controller.nodes) {
      if (n.kind != 'image') continue;
      final id = n.data['file_id'] as String?;
      if (id == null) continue;
      final suffix = _blobNameSuffix(n);
      if (suffix != null) names[id] = suffix;
      if (!_imageUrlCache.containsKey(id)) ids.add(id);
    }
    if (ids.isEmpty) return;
    resolve(ids).then((map) {
      if (!mounted) return;
      // Hang the (cosmetic) filename off the blob url so a copied/exported
      // link reads as an image: `…/blob/diagram.png`, not a bare `…/blob`.
      _imageUrlCache.addAll({
        for (final e in map.entries)
          e.key: names[e.key] == null || !e.value.endsWith('/blob')
              ? e.value
              : '${e.value}/${names[e.key]}',
      });
    });
  }

  /// The href of a link mark covering [pos], or null if none.
  String? _linkAt(DocPosition pos) {
    if (pos.node < 0 || pos.node >= _controller.nodes.length) return null;
    final node = _controller.nodes[pos.node];
    for (final m in marksFromData(node.data)) {
      if (m.type == 'link' &&
          m.href != null &&
          pos.offset >= m.start &&
          pos.offset < m.end) {
        return m.href;
      }
    }
    return null;
  }

  /// The link under the pointer, but only when the pointer is actually on the
  /// text — positionAt snaps to the nearest offset, so clicking the empty
  /// margin past a line that ends in a link must NOT count as a hit.
  String? _linkHitAt(RenderDocument r, Offset local) {
    final pos = r.positionAt(local);
    final href = _linkAt(pos);
    if (href == null) return null;
    final rect = r.caretRectFor(pos);
    if (rect == null) return null;
    if (local.dy < rect.top - 2 || local.dy > rect.bottom + 2) return null;
    if ((local.dx - rect.left).abs() > 24) return null;
    return href;
  }

  /// Like [_linkHitAt] but returns the node index and the full link mark, for
  /// the hover toolbar (its actions need the mark's whole range).
  ({int node, Mark mark})? _linkMarkHitAt(RenderDocument r, Offset local) {
    final pos = r.positionAt(local);
    if (pos.node < 0 || pos.node >= _controller.nodes.length) return null;
    final rect = r.caretRectFor(pos);
    if (rect == null) return null;
    if (local.dy < rect.top - 2 || local.dy > rect.bottom + 2) return null;
    if ((local.dx - rect.left).abs() > 24) return null;
    final node = _controller.nodes[pos.node];
    for (final m in marksFromData(node.data)) {
      if (m.type == 'link' &&
          m.href != null &&
          pos.offset >= m.start &&
          pos.offset < m.end) {
        return (node: pos.node, mark: m);
      }
    }
    return null;
  }

  static const _pageScheme = 'mica://page/';

  void _openHref(String href) {
    if (href.startsWith(_pageScheme)) {
      widget.onOpenPage?.call(href.substring(_pageScheme.length));
      return;
    }
    openUrl(href);
  }

  Future<void> _promptLink() async {
    final sel = _controller.selection;
    final node = _controller.focusedNode;
    if (sel == null || sel.isCollapsed || sel.isMultiNode || node == null)
      return;
    if (rangeHasMark(
      marksFromData(node.data),
      sel.start.offset,
      sel.end.offset,
      'link',
    )) {
      _controller.toggleMark('link'); // remove existing link
      return;
    }
    final field = TextEditingController();
    final url = await showDialog<String>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Add link'),
        content: TextField(
          controller: field,
          autofocus: true,
          decoration: const InputDecoration(
            hintText: 'https://…',
            border: OutlineInputBorder(),
          ),
          onSubmitted: (value) => Navigator.of(context).pop(value),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(),
            child: const Text('Cancel'),
          ),
          FilledButton(
            onPressed: () => Navigator.of(context).pop(field.text),
            child: const Text('Add'),
          ),
        ],
      ),
    );
    field.dispose();
    if (url != null && url.trim().isNotEmpty) {
      _controller.toggleMark('link', href: url.trim());
    }
  }

  void _refreshMarkBar() {
    final sel = _controller.selection;
    // A ranged selection inside a table cell shows the bar too (inline marks
    // only), so selecting cell text — not just Ctrl+B — can format it.
    final ac = _activeCell;
    final cellSel = ac?.ctl.selection;
    final inCell =
        ac != null &&
        cellSel != null &&
        cellSel.isValid &&
        !cellSel.isCollapsed;
    // Show on any ranged selection: inline marks for a single text block, plus
    // block-type conversion (list/quote/heading/code) for any selection.
    final show =
        widget.canEdit &&
        (inCell ||
            (_focus.hasFocus &&
                sel != null &&
                !sel.isCollapsed &&
                _render?.caretRectFor(sel.start) != null));
    if (!show) {
      _hideMarkBar();
      return;
    }
    if (_markBar == null) {
      _markBar = OverlayEntry(builder: _buildMarkBar);
      Overlay.of(context).insert(_markBar!);
    } else {
      _markBar!.markNeedsBuild();
    }
  }

  void _hideMarkBar() {
    _markBar?.remove();
    _markBar = null;
  }

  /// Toggle an inline mark on whatever is focused: the active table cell (its
  /// own selection) if one is being edited, otherwise the document selection.
  /// This is what makes the format bar + shortcuts act on cell content.
  void _toggleMarkCtx(String type, {String? href}) {
    final ac = _activeCell;
    if (ac != null) {
      // With a cell open the command belongs to the CELL — a collapsed cell
      // selection is a no-op, never a fall-through: the (hidden) document
      // selection may still hold an old range, and mutating it here would
      // silently bold off-screen text while the user is looking at the cell.
      if (ac.ctl.selection.isValid && !ac.ctl.selection.isCollapsed) {
        ac.ctl.toggleMark(type, href: href);
      }
      return;
    }
    _controller.toggleMark(type, href: href);
  }

  Widget _buildMarkBar(BuildContext context) {
    final r = _render;
    if (r == null) return const SizedBox.shrink();
    // Cell mode: a table cell is being edited with a ranged selection — anchor
    // the bar over the cell and route marks to the cell (block-type conversions
    // are hidden: a cell holds inline text only).
    final ac = _activeCell;
    final cellSel = ac?.ctl.selection;
    final inCell =
        ac != null &&
        cellSel != null &&
        cellSel.isValid &&
        !cellSel.isCollapsed;
    final Offset origin;
    if (inCell) {
      final cellRect = r.tableCellRect(ac.node, ac.row, ac.col);
      if (cellRect == null) return const SizedBox.shrink();
      origin = r.localToGlobal(cellRect.topLeft);
    } else {
      final sel = _controller.selection;
      if (sel == null) return const SizedBox.shrink();
      final rect = r.caretRectFor(sel.start);
      if (rect == null) return const SizedBox.shrink();
      origin = r.localToGlobal(rect.topLeft);
    }
    final screen = MediaQuery.of(context).size;
    final left = origin.dx.clamp(8.0, screen.width - 420);
    final top = (origin.dy - 44).clamp(8.0, screen.height - 8);

    final docSel = _controller.selection;
    final singleText =
        inCell ||
        (docSel != null &&
            !docSel.isMultiNode &&
            _controller.focusedNode != null &&
            _controller.focusedNode!.kind != 'code_block' &&
            _controller.focusedNode!.kind != 'table');
    // Block-type conversions (headings/lists/quote/code block) only apply to
    // document blocks, never to inline cell content.
    final showBlocks = !inCell;

    Widget markBtn(
      IconData icon,
      String type,
      String tip, {
      VoidCallback? custom,
    }) {
      return IconButton(
        iconSize: 18,
        visualDensity: VisualDensity.compact,
        tooltip: tip,
        icon: Icon(icon, color: EditorTheme.text),
        onPressed: custom ?? () => _toggleMarkCtx(type),
      );
    }

    Widget blockBtn(
      IconData icon,
      String kind,
      String tip, {
      Map<String, dynamic>? data,
    }) {
      return IconButton(
        iconSize: 18,
        visualDensity: VisualDensity.compact,
        tooltip: tip,
        icon: Icon(icon, color: EditorTheme.muted),
        onPressed: () => _controller.setSelectedBlocksKind(kind, data: data),
      );
    }

    // The focused block's heading level lights up the matching control, so
    // a selected title tells you its level at a glance.
    final focused = _controller.focusedNode;
    final currentLevel = focused != null && focused.kind == 'heading'
        ? focused.headingLevel
        : 0;

    // T1/T2/T3 as labeled buttons — heavier label for bigger headings, a
    // tinted pill when it is the selection's current level.
    Widget headingBtn(int level) {
      final active = level == currentLevel;
      return IconButton(
        iconSize: 18,
        visualDensity: VisualDensity.compact,
        tooltip: 'Heading $level (Ctrl+Alt+$level)',
        style: active
            ? IconButton.styleFrom(backgroundColor: const Color(0xFFE2E8F0))
            : null,
        icon: Text(
          'T$level',
          style: TextStyle(
            fontSize: 13,
            fontWeight: level == 1 ? FontWeight.w800 : FontWeight.w600,
            color: active ? EditorTheme.text : EditorTheme.muted,
          ),
        ),
        onPressed: () => _controller.setSelectedBlocksKind(
          'heading',
          data: {'level': level},
        ),
      );
    }

    // The rare H4–H6 live behind a chevron dropdown; the chevron tints when
    // one of them is current, and the menu checks it.
    Widget moreHeadingsBtn() {
      final activeDeep = currentLevel >= 4;
      return Builder(
        builder: (btnContext) => IconButton(
          iconSize: 16,
          visualDensity: VisualDensity.compact,
          tooltip: activeDeep ? 'Heading $currentLevel' : 'More headings',
          style: activeDeep
              ? IconButton.styleFrom(backgroundColor: const Color(0xFFE2E8F0))
              : null,
          icon: Icon(
            Icons.expand_more,
            color: activeDeep ? EditorTheme.text : EditorTheme.muted,
          ),
          onPressed: () async {
            final box = btnContext.findRenderObject() as RenderBox?;
            if (box == null) return;
            final at = box.localToGlobal(Offset(0, box.size.height));
            final pick = await _showSmallMenu(at, [
              for (var l = 4; l <= 6; l++)
                ('$l', '${l == currentLevel ? '✓ ' : '   '}Heading $l'),
            ]);
            if (pick != null) {
              _controller.setSelectedBlocksKind(
                'heading',
                data: {'level': int.parse(pick)},
              );
            }
          },
        ),
      );
    }

    return Positioned(
      left: left,
      top: top,
      // TextFieldTapRegion: clicking the bar must not count as a tap OUTSIDE
      // the cell editor's TextField — Flutter's onTapOutside would unfocus it
      // on pointer-DOWN, committing and closing the cell before the button's
      // onPressed (pointer-up) could apply the mark.
      child: TextFieldTapRegion(
        child: ExcludeFocus(
          child: Material(
            elevation: 6,
            borderRadius: BorderRadius.circular(8),
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 2),
              child: Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  if (singleText) ...[
                    markBtn(Icons.format_bold, 'bold', 'Bold'),
                    markBtn(Icons.format_italic, 'italic', 'Italic'),
                  ],
                  // Code block sits just before inline code — the two code
                  // affordances read as one group (document blocks only).
                  if (showBlocks)
                    blockBtn(Icons.terminal, 'code_block', 'Code block'),
                  if (singleText) ...[
                    markBtn(Icons.code, 'code', 'Inline code'),
                    markBtn(Icons.strikethrough_s, 'strike', 'Strikethrough'),
                    markBtn(Icons.link, 'link', 'Link', custom: _promptLink),
                  ],
                  if (showBlocks) ...[
                    const VerticalDivider(width: 9, indent: 8, endIndent: 8),
                    // The three everyday heading levels are one click; the rare
                    // H4–H6 hide behind the chevron.
                    for (var level = 1; level <= 3; level++) headingBtn(level),
                    moreHeadingsBtn(),
                    blockBtn(Icons.notes, 'paragraph', 'Text'),
                    blockBtn(
                      Icons.format_list_bulleted,
                      'bulleted_list',
                      'Bulleted list',
                    ),
                    blockBtn(
                      Icons.format_list_numbered,
                      'numbered_list',
                      'Numbered list',
                    ),
                    blockBtn(
                      Icons.check_box_outlined,
                      'todo',
                      'To-do',
                      data: {'checked': false},
                    ),
                    blockBtn(Icons.format_quote, 'quote', 'Quote'),
                  ],
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }

  Future<void> _openTableMenu(int node, Offset globalPosition) async {
    final selected = await _showSmallMenu(globalPosition, const [
      ('left', 'Align left'),
      ('center', 'Align center'),
      ('right', 'Align right'),
      ('autofit', 'Auto-fit columns'),
      ('copy', 'Copy table'),
      ('delete', 'Delete table'),
    ]);
    switch (selected) {
      case 'left':
      case 'center':
      case 'right':
        _controller.setTableAlign(node, selected!);
      case 'autofit':
        // Equal weights = the renderer's content-driven auto width mode.
        _controller.resetTableColumnWidths(node);
      case 'copy':
        final table = TableData.fromBlock(_controller.nodes[node].data);
        await copyTextToClipboard(tableToMarkdown(table));
        if (mounted) {
          ScaffoldMessenger.of(context).showSnackBar(
            const SnackBar(
              content: Text('Table copied'),
              duration: Duration(seconds: 1),
            ),
          );
        }
      case 'delete':
        _closeCellEditor();
        _controller.deleteNode(node);
    }
  }

  Future<void> _openRowMenu(int node, int row, Offset globalPosition) async {
    final selected = await _showSmallMenu(globalPosition, const [
      ('moveUp', 'Move up'),
      ('moveDown', 'Move down'),
      ('above', 'Insert row above'),
      ('below', 'Insert row below'),
      ('delete', 'Delete row'),
    ]);
    switch (selected) {
      case 'moveUp':
        _controller.moveTableRow(node, row, -1);
      case 'moveDown':
        _controller.moveTableRow(node, row, 1);
      case 'above':
        _controller.insertTableRow(node, row);
      case 'below':
        _controller.insertTableRow(node, row + 1);
      case 'delete':
        _controller.deleteTableRow(node, row);
    }
  }

  Future<void> _openColumnMenu(int node, int col, Offset globalPosition) async {
    final selected = await _showSmallMenu(globalPosition, const [
      ('moveLeft', 'Move left'),
      ('moveRight', 'Move right'),
      ('left', 'Insert column left'),
      ('right', 'Insert column right'),
      ('delete', 'Delete column'),
    ]);
    switch (selected) {
      case 'moveLeft':
        _controller.moveTableColumn(node, col, -1);
      case 'moveRight':
        _controller.moveTableColumn(node, col, 1);
      case 'left':
        _controller.insertTableColumn(node, col);
      case 'right':
        _controller.insertTableColumn(node, col + 1);
      case 'delete':
        _controller.deleteTableColumn(node, col);
    }
  }

  Future<String?> _showSmallMenu(
    Offset globalPosition,
    List<(String, String)> items,
  ) {
    final overlay =
        Overlay.of(context).context.findRenderObject() as RenderBox?;
    if (overlay == null) return Future.value(null);
    return showMenu<String>(
      context: context,
      position: RelativeRect.fromRect(
        globalPosition & const Size(40, 40),
        Offset.zero & overlay.size,
      ),
      items: [
        for (final (value, label) in items)
          PopupMenuItem<String>(value: value, child: Text(label)),
      ],
    );
  }

  void _copyCode(int nodeIndex) {
    final nodes = _controller.nodes;
    if (nodeIndex < 0 || nodeIndex >= nodes.length) return;
    copyTextToClipboard(nodes[nodeIndex].text);
    if (mounted) {
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(
          content: Text('Code copied'),
          duration: Duration(seconds: 1),
        ),
      );
    }
  }

  void _onHover(PointerHoverEvent event) {
    final r = _render;
    if (r == null) return;
    final local = r.globalToLocal(event.position);
    r.setHover(local);
    final cursor = _cursorFor(r, local);
    if (cursor != _cursor) setState(() => _cursor = cursor);

    // Link hover toolbar: show while the pointer is on link text, keep it
    // while the pointer is over the bar itself, hide after a short grace.
    final hit = _linkMarkHitAt(r, local);
    if (hit != null) {
      _linkBarHide?.cancel();
      final same =
          _linkBar != null &&
          _linkBarNode == hit.node &&
          _linkBarMark?.start == hit.mark.start &&
          _linkBarMark?.href == hit.mark.href;
      if (!same) _showLinkBar(hit.node, hit.mark);
    } else if (_linkBar != null && !_pointerOverLinkBar) {
      _scheduleLinkBarHide();
    }
  }

  void _showLinkBar(int node, Mark mark) {
    _linkBarNode = node;
    _linkBarMark = mark;
    if (_linkBar == null) {
      _linkBar = OverlayEntry(builder: _buildLinkBar);
      Overlay.of(context).insert(_linkBar!);
    } else {
      _linkBar!.markNeedsBuild();
    }
  }

  void _scheduleLinkBarHide() {
    _linkBarHide?.cancel();
    _linkBarHide = Timer(const Duration(milliseconds: 350), () {
      if (!_pointerOverLinkBar) _closeLinkBar();
    });
  }

  void _closeLinkBar() {
    _linkBarHide?.cancel();
    _linkBarHide = null;
    _linkBar?.remove();
    _linkBar = null;
    _linkBarMark = null;
    _linkBarNode = -1;
    _pointerOverLinkBar = false;
  }

  Widget _buildLinkBar(BuildContext context) {
    final r = _render;
    final mark = _linkBarMark;
    if (r == null || mark == null) return const SizedBox.shrink();
    final rect = r.caretRectFor(DocPosition(_linkBarNode, mark.start));
    if (rect == null) return const SizedBox.shrink();
    final origin = r.localToGlobal(rect.bottomLeft);
    final screen = MediaQuery.of(context).size;
    final left = origin.dx.clamp(8.0, screen.width - 320);
    final top = (origin.dy + 4).clamp(8.0, screen.height - 48);

    Widget action(IconData icon, String label, VoidCallback onTap) {
      return InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(6),
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 7),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              Icon(icon, size: 15, color: const Color(0xFF475569)),
              const SizedBox(width: 6),
              Text(
                label,
                style: const TextStyle(fontSize: 13, color: Color(0xFF0F172A)),
              ),
            ],
          ),
        ),
      );
    }

    return Positioned(
      left: left,
      top: top,
      child: MouseRegion(
        onEnter: (_) {
          _pointerOverLinkBar = true;
          _linkBarHide?.cancel();
        },
        onExit: (_) {
          _pointerOverLinkBar = false;
          _scheduleLinkBarHide();
        },
        child: ExcludeFocus(
          child: Material(
            elevation: 6,
            borderRadius: BorderRadius.circular(8),
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 2),
              child: Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  action(Icons.copy_outlined, 'Copy link', _copyLinkFromBar),
                  if (widget.canEdit) ...[
                    Container(
                      width: 1,
                      height: 18,
                      color: const Color(0xFFE2E8F0),
                    ),
                    action(Icons.edit_outlined, 'Edit link', _editLinkFromBar),
                    Container(
                      width: 1,
                      height: 18,
                      color: const Color(0xFFE2E8F0),
                    ),
                    action(Icons.link_off, 'Remove link', _removeLinkFromBar),
                  ],
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }

  void _copyLinkFromBar() {
    final href = _linkBarMark?.href;
    _closeLinkBar();
    if (href == null) return;
    copyTextToClipboard(href).then((_) {
      if (mounted) _focus.requestFocus();
    });
  }

  void _removeLinkFromBar() {
    final mark = _linkBarMark;
    final node = _linkBarNode;
    _closeLinkBar();
    if (mark == null) return;
    _controller.setLinkRange(node, mark.start, mark.end, null);
  }

  Future<void> _editLinkFromBar() async {
    final mark = _linkBarMark;
    final nodeIdx = _linkBarNode;
    _closeLinkBar();
    if (mark == null || nodeIdx < 0 || nodeIdx >= _controller.nodes.length) {
      return;
    }
    final node = _controller.nodes[nodeIdx];
    final start = mark.start.clamp(0, node.text.length);
    final end = mark.end.clamp(start, node.text.length);
    final currentText = node.text.substring(start, end);

    final textField = TextEditingController(text: currentText);
    final urlField = TextEditingController(text: mark.href ?? '');
    final saved = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Edit link', style: TextStyle(fontSize: 15)),
        titlePadding: const EdgeInsets.fromLTRB(16, 14, 16, 0),
        contentPadding: const EdgeInsets.fromLTRB(16, 12, 16, 0),
        actionsPadding: const EdgeInsets.fromLTRB(16, 4, 12, 8),
        content: SizedBox(
          width: 320,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              TextField(
                controller: textField,
                autofocus: true,
                style: const TextStyle(fontSize: 13),
                decoration: const InputDecoration(
                  labelText: 'Text',
                  isDense: true,
                  border: OutlineInputBorder(),
                ),
              ),
              const SizedBox(height: 10),
              TextField(
                controller: urlField,
                style: const TextStyle(fontSize: 13),
                decoration: const InputDecoration(
                  labelText: 'URL',
                  hintText: 'https://…',
                  isDense: true,
                  border: OutlineInputBorder(),
                ),
                onSubmitted: (_) => Navigator.of(context).pop(true),
              ),
            ],
          ),
        ),
        actions: [
          TextButton(
            style: TextButton.styleFrom(visualDensity: VisualDensity.compact),
            onPressed: () => Navigator.of(context).pop(),
            child: const Text('Cancel'),
          ),
          FilledButton(
            style: FilledButton.styleFrom(visualDensity: VisualDensity.compact),
            onPressed: () => Navigator.of(context).pop(true),
            child: const Text('Save'),
          ),
        ],
      ),
    );
    final newText = textField.text.trim();
    final newUrl = urlField.text.trim();
    textField.dispose();
    urlField.dispose();
    if (saved != true || newUrl.isEmpty) return;
    _controller.replaceLink(
      nodeIdx,
      start,
      end,
      newText.isEmpty ? currentText : newText,
      newUrl,
    );
  }

  MouseCursor _cursorFor(RenderDocument r, Offset local) {
    if (_blockDrag != null || r.dragHandleAt(local) != null) {
      return _blockDrag != null
          ? SystemMouseCursors.grabbing
          : SystemMouseCursors.grab;
    }
    // Rendered diagrams pan by drag (AFFiNE-style): hand cursor invites it.
    if (_diagramPan != null) return SystemMouseCursors.grabbing;
    if (r.diagramAt(local) != null && r.viewTabAt(local) == null) {
      return SystemMouseCursors.grab;
    }
    if (r.tableColBorderAt(local) != null || r.imageResizeAt(local) != null) {
      return SystemMouseCursors.resizeLeftRight;
    }
    final clickable =
        r.codeLanguageAt(local) != null ||
        r.codeCopyAt(local) != null ||
        r.codeWrapAt(local) != null ||
        r.scrollbarAt(local) != null ||
        r.tableHandleAt(local) != null ||
        r.tableDeleteAt(local) != null ||
        r.tableRowHandleAt(local) != null ||
        r.tableColHandleAt(local) != null ||
        r.tableAddAt(local) != null ||
        r.imageActionAt(local) != null ||
        _linkHitAt(r, local) != null;
    return clickable ? SystemMouseCursors.click : SystemMouseCursors.text;
  }

  void _onPointerSignal(PointerSignalEvent event) {
    final r = _render;
    if (r == null) return;
    // Ctrl+wheel over a rendered diagram zooms it (AFFiNE-style). The web
    // engine surfaces ctrl+wheel as a SCALE signal; desktops keep it a
    // scroll with the ctrl modifier down — accept both.
    if (event is PointerScaleEvent) {
      r.zoomPreviewBy(r.globalToLocal(event.position), event.scale);
      return;
    }
    if (event is! PointerScrollEvent) return;
    if (HardwareKeyboard.instance.isControlPressed) {
      if (r.zoomPreviewBy(
        r.globalToLocal(event.position),
        event.scrollDelta.dy > 0 ? 0.9 : 1.1,
      )) {
        return;
      }
    }
    final shift = HardwareKeyboard.instance.isShiftPressed;
    final dx = event.scrollDelta.dx != 0
        ? event.scrollDelta.dx
        : (shift ? event.scrollDelta.dy : 0.0);
    if (dx == 0) return;
    r.scrollCodeAt(r.globalToLocal(event.position), dx);
  }

  Future<void> _openLanguageMenu(int nodeIndex, Offset globalPosition) async {
    if (nodeIndex < 0 || nodeIndex >= _controller.nodes.length) return;
    final pinned = canonicalCodeLanguage(
      (_controller.nodes[nodeIndex].data['language'] as String?) ?? '',
    );
    final selected = await showDialog<String>(
      context: context,
      barrierColor: Colors.transparent,
      builder: (context) => _LanguagePicker(
        anchor: globalPosition,
        current: pinned.isEmpty ? 'auto' : pinned,
      ),
    );
    if (selected != null) {
      _controller.setCodeLanguage(nodeIndex, selected);
    }
  }

  /// Copy the selected cell area to the clipboard in two flavors: plain TSV
  /// (tab-separated — pastes back into Excel/Sheets as cells) and an HTML
  /// `<table>` with the cells' inline marks (pastes into Mica/Typora as a
  /// table, via the HTML-table paste path). [cut] also blanks the cells.
  void _copyTableArea(
    ({int node, int r0, int c0, int r1, int c1}) area, {
    required bool cut,
  }) {
    if (area.node < 0 || area.node >= _controller.nodes.length) return;
    // A stale index could land on a non-table node — TableData.fromBlock would
    // fabricate the empty-table placeholder and overwrite the clipboard with it.
    if (_controller.nodes[area.node].kind != 'table') return;
    final table = TableData.fromBlock(_controller.nodes[area.node].data);
    final plainRows = <String>[];
    final htmlRows = StringBuffer();
    for (var r = area.r0; r <= area.r1 && r < table.rows.length; r++) {
      if (r < 0) continue;
      final plainCells = <String>[];
      htmlRows.write('<tr>');
      for (var c = area.c0; c <= area.c1 && c < table.rows[r].length; c++) {
        if (c < 0) continue;
        final parsed = parseInline(table.rows[r][c]);
        plainCells.add(parsed.text.replaceAll('\t', ' ').replaceAll('\n', ' '));
        htmlRows.write('<td>${inlineToHtml(parsed.text, parsed.marks)}</td>');
      }
      htmlRows.write('</tr>');
      plainRows.add(plainCells.join('\t'));
    }
    copyRichToClipboard(
      plain: plainRows.join('\n'),
      richHtml: '<table>$htmlRows</table>',
    );
    if (cut) {
      _controller.clearTableCells(
        area.node,
        area.r0,
        area.c0,
        area.r1,
        area.c1,
      );
    }
  }

  /// Whether the caret at [offset] sits on the first ([last] false) or last
  /// ([last] true) VISUAL line of the cell — soft-wrapped lines included, which
  /// a plain `\n` scan misses (a wrapped cell would eject the caret from the
  /// table instead of moving down a visual line). Measured with the same span
  /// + width the cell's field renders with.
  bool _cellCaretOnEdgeLine(
    int node,
    int row,
    int col,
    CellEditController controller,
    int offset, {
    required bool last,
  }) {
    final rect = _render?.tableCellRect(node, row, col);
    final caret = offset.clamp(0, controller.text.length);
    final tp =
        TextPainter(
          text: buildMarkedSpan(
            controller.text,
            controller.marks,
            const TextStyle(fontSize: 15, height: 1.4),
          ),
          textDirection: TextDirection.ltr,
        )..layout(
          maxWidth: ((rect?.width ?? double.infinity) - 20).clamp(
            1.0,
            double.infinity,
          ),
        );
    final lines = tp.computeLineMetrics();
    final caretY = tp
        .getOffsetForCaret(TextPosition(offset: caret), Rect.zero)
        .dy;
    tp.dispose();
    if (lines.length <= 1) return true;
    // Find the caret's line index by walking cumulative line heights.
    var top = 0.0;
    var line = 0;
    for (var i = 0; i < lines.length; i++) {
      if (caretY < top + lines[i].height - 0.1) {
        line = i;
        break;
      }
      top += lines[i].height;
      line = i;
    }
    return last ? line == lines.length - 1 : line == 0;
  }

  /// Move the document caret out of table [node]: [up] lands at the end of the
  /// block above, else at the start of the block below (appending an empty
  /// paragraph when the table is the last block, so ↓ always has somewhere to
  /// go). Restores canvas focus + IME — the caret just left an overlay field.
  void _exitTableCaret(int node, {required bool up}) {
    _focus.requestFocus();
    _attachIme();
    final nodes = _controller.nodes;
    if (up) {
      if (node > 0) {
        final target = nodes[node - 1];
        final offset = target.isAtomic ? 0 : target.text.length;
        _controller.setSelection(
          DocSelection.collapsed(DocPosition(node - 1, offset)),
        );
      } else {
        // Table is the first block: select it (there is nothing above).
        _controller.setSelection(DocSelection.collapsed(DocPosition(node, 0)));
      }
    } else if (node + 1 < nodes.length) {
      _controller.setSelection(
        DocSelection.collapsed(DocPosition(node + 1, 0)),
      );
    } else {
      _controller.insertParagraphAfter(node); // also places the caret in it
    }
    _syncImeFromSelection(force: true);
  }

  /// Map a render-local pointer to a text offset inside the active cell, using a
  /// throwaway painter of the cell's clean text + marks (the same style the field
  /// renders with) so a drag-selection lands on the right character.
  int _cellOffsetAt(
    RenderDocument r,
    ({CellEditController ctl, int node, int row, int col}) ac,
    Offset local,
  ) {
    final rect = r.tableCellRect(ac.node, ac.row, ac.col);
    if (rect == null) return ac.ctl.text.length;
    const padH = 10.0, padV = 9.0;
    final origin = rect.topLeft + const Offset(padH, padV);
    final tp = TextPainter(
      text: buildMarkedSpan(
        ac.ctl.text,
        ac.ctl.marks,
        const TextStyle(fontSize: 15, height: 1.4),
      ),
      textDirection: TextDirection.ltr,
    )..layout(maxWidth: (rect.width - padH * 2).clamp(1.0, double.infinity));
    final pos = tp.getPositionForOffset(local - origin);
    tp.dispose();
    return pos.offset.clamp(0, ac.ctl.text.length);
  }

  void _onPanDown(DragDownDetails d) {
    // Captured at the REAL pointer-down spot: by the time onPanStart fires
    // the pointer has already travelled past the touch slop, and a drag that
    // STARTED on a paragraph above a diagram would read as inside it —
    // hijacking the text selection into a pan and yanking the picture out
    // of its viewport.
    final r = _render;
    _panDownDiagram = r == null
        ? null
        : r.diagramAt(r.globalToLocal(d.globalPosition));
  }

  void _onPanStart(DragStartDetails d) {
    if (!widget.canEdit) return;
    _tapCount = 0; // a drag ends any tap sequence (no stray triple-click)
    final r = _render;
    if (r == null) return;
    // A drag (column resize, selection…) relayouts under the floating cell
    // editor — commit it first so it never hangs detached from its cell.
    _commitCellEditor?.call();
    final local = r.globalToLocal(d.globalPosition);
    // Dragging a block's gutter handle moves the block.
    final handle = r.dragHandleAt(local);
    if (handle != null) {
      _blockDrag = handle;
      r.setDropIndicator(r.dropIndexAt(local.dy));
      return;
    }
    // Dragging a table column border resizes columns.
    final colBorder = r.tableColBorderAt(local);
    if (colBorder != null) {
      // Seed from the EFFECTIVE (rendered) widths, not the stored weights: in
      // auto-fit mode the two differ, and starting from the stored equal
      // weights made the first drag tick snap every column to the equal split.
      final effective = r.tableEffectiveWeights(colBorder.node);
      _colResize = (
        node: colBorder.node,
        col: colBorder.col,
        startX: local.dx,
        weights: effective.isNotEmpty
            ? effective
            : [...r.tableWeights(colBorder.node)],
        avail: r.tableAvailWidth(),
        frac: r.tableWidthFraction(colBorder.node),
      );
      return;
    }
    // Dragging a rendered diagram pans it inside its fixed viewport — only
    // when the gesture truly began on the diagram (see _onPanDown).
    final diagram = _panDownDiagram;
    if (diagram != null) {
      _diagramPan = diagram;
      return;
    }
    // Dragging an image's right-edge handle resizes it.
    final imageResize = r.imageResizeAt(local);
    if (imageResize != null) {
      _imageResize = imageResize;
      _imageResizeWidth = r.imageWidthFor(imageResize, local.dx);
      return;
    }
    // Dragging the horizontal scrollbar scrolls the code, not selects text.
    final bar = r.scrollbarAt(local);
    if (bar != null) {
      _scrollbarDrag = bar;
      r.setCodeScrollByTrackX(bar, local.dx);
      return;
    }
    // A drag that begins inside a table cell enters that cell's editor rather
    // than starting a whole-table block selection (dragging over the atomic
    // table used to select the entire table). Text is then drag-selected inside
    // the cell's own field. _dragAnchor stays null so _onPanUpdate no-ops.
    final tcell = r.tableCellAt(local);
    if (tcell != null) {
      _dragAnchor = null;
      _openCellEditor(tcell.node, tcell.row, tcell.col);
      // Begin a text drag-selection inside the cell — the field can't receive
      // the canvas-owned drag, so we drive its selection from the pointer.
      final ac = _activeCell;
      if (ac != null) {
        final off = _cellOffsetAt(r, ac, local);
        ac.ctl.selection = TextSelection.collapsed(offset: off);
        _cellDrag = (
          ctl: ac.ctl,
          node: ac.node,
          row: ac.row,
          col: ac.col,
          anchor: off,
        );
      }
      return;
    }
    _focus.requestFocus();
    // requestFocus is a no-op when we already own focus, so a dead IME
    // connection (engine-side close) would stay dead — re-arm explicitly;
    // attach is idempotent when the connection is healthy.
    _attachIme();
    final p = r.positionAt(local);
    _dragAnchor = p;
    _controller.setSelection(DocSelection.collapsed(p));
  }

  void _onPanUpdate(DragUpdateDetails d) {
    final r = _render;
    if (r == null) return;
    final local = r.globalToLocal(d.globalPosition);
    // Extend a cross-cell AREA selection (rectangle of whole cells).
    final ad = _areaDrag;
    if (ad != null) {
      final cur = r.tableCellNear(ad.node, local);
      if (cur != null) {
        r.tableBlockSelection = (
          node: ad.node,
          r0: ad.row < cur.row ? ad.row : cur.row,
          c0: ad.col < cur.col ? ad.col : cur.col,
          r1: ad.row > cur.row ? ad.row : cur.row,
          c1: ad.col > cur.col ? ad.col : cur.col,
        );
      }
      return;
    }
    // Extend a text drag-selection inside the active cell — until the pointer
    // crosses into ANOTHER cell, which escalates to an area selection (in-cell
    // text OR whole-cell area, never a cross-cell text range).
    final cd = _cellDrag;
    if (cd != null) {
      final hit = r.tableCellAt(local);
      if (hit != null &&
          hit.node == cd.node &&
          (hit.row != cd.row || hit.col != cd.col)) {
        _commitCellEditor?.call(); // close the cell field (commits its text)
        _focus.requestFocus(); // area actions (Ctrl+C / Delete) need key focus
        _areaDrag = (node: cd.node, row: cd.row, col: cd.col);
        r.tableBlockSelection = (
          node: cd.node,
          r0: cd.row < hit.row ? cd.row : hit.row,
          c0: cd.col < hit.col ? cd.col : hit.col,
          r1: cd.row > hit.row ? cd.row : hit.row,
          c1: cd.col > hit.col ? cd.col : hit.col,
        );
        _cellDrag = null;
        return;
      }
      final off = _cellOffsetAt(r, (
        ctl: cd.ctl,
        node: cd.node,
        row: cd.row,
        col: cd.col,
      ), local);
      cd.ctl.selection = TextSelection(
        baseOffset: cd.anchor,
        extentOffset: off,
      );
      return;
    }
    if (_blockDrag != null) {
      r.setDropIndicator(r.dropIndexAt(local.dy));
      return;
    }
    final panning = _diagramPan;
    if (panning != null) {
      r.panPreviewBy(panning, d.delta);
      return;
    }
    final resize = _colResize;
    if (resize != null) {
      final weights = [...resize.weights];
      final sum = weights.fold<double>(0, (a, b) => a + b);
      final pxPerWeight = resize.avail / sum;
      final dxPx = local.dx - resize.startX;
      if (resize.col >= weights.length) {
        // The table's right edge drags the OVERALL width: the whole table
        // shrinks/grows (columns keep their proportions).
        final startPx = resize.avail * resize.frac.clamp(0.15, 1.0);
        final minPx = (40.0 * weights.length).clamp(60.0, resize.avail);
        final newPx = (startPx + dxPx).clamp(minPx, resize.avail);
        _controller.previewTableWidth(resize.node, newPx / resize.avail);
        return;
      }
      final left = resize.col - 1;
      final right = resize.col;
      final leftPx = weights[left] * pxPerWeight;
      final rightPx = weights[right] * pxPerWeight;
      final newLeftPx = (leftPx + dxPx).clamp(40.0, leftPx + rightPx - 40.0);
      final newRightPx = leftPx + rightPx - newLeftPx;
      weights[left] = newLeftPx / pxPerWeight;
      weights[right] = newRightPx / pxPerWeight;
      _controller.previewTableColumnWidths(resize.node, weights);
      return;
    }
    if (_imageResize != null) {
      final w = r.imageWidthFor(_imageResize!, local.dx);
      _imageResizeWidth = w;
      _controller.previewImageWidth(_imageResize!, w);
      return;
    }
    if (_scrollbarDrag != null) {
      r.setCodeScrollByTrackX(_scrollbarDrag!, local.dx);
      return;
    }
    final anchor = _dragAnchor;
    if (anchor == null) return;
    final p = r.positionAt(local);
    _controller.setSelection(DocSelection(anchor: anchor, focus: p));
    // Auto-scroll the page vertically when the drag nears the viewport edge so
    // the selection can extend beyond what's currently visible.
    _lastDragGlobal = d.globalPosition;
    _autoScrollTimer ??= Timer.periodic(
      const Duration(milliseconds: 16),
      (_) => _autoScrollTick(),
    );
    // Auto-scroll a code block horizontally when selecting near its edges;
    // the closer to the edge, the faster (keeps up with the drag).
    if (_controller.focusedNode?.isCode ?? false) {
      const zone = 64.0;
      if (local.dx > r.size.width - zone) {
        r.scrollCodeAt(local, 24 + (local.dx - (r.size.width - zone)));
      } else if (local.dx < zone) {
        r.scrollCodeAt(local, -(24 + (zone - local.dx)));
      }
    }
  }

  void _onPanEnd(DragEndDetails d) {
    _diagramPan = null;
    _panDownDiagram = null;
    _cellDrag = null; // end any in-cell text drag-selection
    _areaDrag = null; // the area SELECTION stays; only the drag ends
    if (_blockDrag != null) {
      final r = _render;
      final from = _blockDrag!;
      _blockDrag = null;
      final to = r?.dropIndex;
      r?.setDropIndicator(null);
      if (to != null) {
        _controller.moveBlock(from, to);
      }
      return;
    }
    if (_imageResize != null) {
      if (_imageResizeWidth != null) {
        _controller.setImageWidth(_imageResize!, _imageResizeWidth!);
      }
      _imageResize = null;
      _imageResizeWidth = null;
      return;
    }
    final resize = _colResize;
    if (resize != null) {
      // Persist the final geometry once (preview was local-only).
      if (resize.col >= resize.weights.length) {
        _controller.setTableWidth(
          resize.node,
          _render?.tableWidthFraction(resize.node) ?? resize.frac,
        );
      } else {
        _controller.setTableColumnWidths(
          resize.node,
          _render?.tableWeights(resize.node) ?? resize.weights,
        );
      }
      _colResize = null;
      return;
    }
    _dragAnchor = null;
    _scrollbarDrag = null;
    _stopAutoScroll();
    _syncImeFromSelection();
  }

  void _stopAutoScroll() {
    _autoScrollTimer?.cancel();
    _autoScrollTimer = null;
    _lastDragGlobal = null;
  }

  /// One auto-scroll step: while the held pointer is within the edge zone of the
  /// surrounding scroll view, scroll the page (faster nearer the edge) and keep
  /// extending the selection to the content now under the pointer.
  void _autoScrollTick() {
    final scrollable = Scrollable.maybeOf(context);
    final global = _lastDragGlobal;
    final anchor = _dragAnchor;
    final r = _render;
    if (scrollable == null || global == null || anchor == null || r == null) {
      return;
    }
    final box = scrollable.context.findRenderObject() as RenderBox?;
    if (box == null) return;
    final top = box.localToGlobal(Offset.zero).dy;
    final bottom = top + box.size.height;
    const zone = 90.0; // edge band that triggers scrolling
    const maxStep = 32.0; // px per ~16ms tick at the very edge (~2000px/s)

    double delta = 0;
    if (global.dy < top + zone) {
      delta = -((top + zone - global.dy) / zone).clamp(0.0, 1.0) * maxStep;
    } else if (global.dy > bottom - zone) {
      delta = ((global.dy - (bottom - zone)) / zone).clamp(0.0, 1.0) * maxStep;
    }
    if (delta == 0) return;

    final pos = scrollable.position;
    final target = (pos.pixels + delta).clamp(
      pos.minScrollExtent,
      pos.maxScrollExtent,
    );
    if (target == pos.pixels) return;
    pos.jumpTo(target);
    final p = r.positionAt(r.globalToLocal(global));
    _controller.setSelection(DocSelection(anchor: anchor, focus: p));
  }

  // ---------------------------------------------------------------------------
  // `[[` page-link picker
  // ---------------------------------------------------------------------------

  List<PageLinkTarget> _filteredPages() {
    final all = widget.pageLinks?.call() ?? const <PageLinkTarget>[];
    final q = _pageQuery.toLowerCase();
    final list = q.isEmpty
        ? all
        : [
            for (final p in all)
              if (p.title.toLowerCase().contains(q)) p,
          ];
    return list.take(8).toList();
  }

  /// Recompute the page-link session from the caret: open the picker when an
  /// unclosed `[[` token precedes the caret, otherwise close it.
  void _refreshPageLink() {
    if (widget.pageLinks == null) return;
    final sel = _controller.selection;
    final node = _controller.focusedNode;
    if (sel == null ||
        node == null ||
        !sel.isCollapsed ||
        node.kind == 'code_block' ||
        node.kind == 'table') {
      _closePageLink();
      return;
    }
    final caret = sel.focus.offset;
    if (caret > node.text.length) {
      _closePageLink();
      return;
    }
    final before = node.text.substring(0, caret);
    final open = before.lastIndexOf('[[');
    if (open < 0) {
      _closePageLink();
      return;
    }
    final query = before.substring(open + 2);
    if (query.contains('[') || query.contains(']') || query.contains('\n')) {
      _closePageLink();
      return;
    }
    _pageStart = open;
    _pageQuery = query;
    final items = _filteredPages();
    if (items.isEmpty) {
      _closePageLink();
      return;
    }
    _pageIndex = _pageIndex.clamp(0, items.length - 1);
    if (_pageEntry == null) {
      _pageEntry = OverlayEntry(builder: _buildPageLinkOverlay);
      Overlay.of(context).insert(_pageEntry!);
    } else {
      _pageEntry!.markNeedsBuild();
    }
  }

  void _closePageLink() {
    _pageEntry?.remove();
    _pageEntry = null;
    _pageQuery = '';
    _pageIndex = 0;
  }

  void _applyPageLink(PageLinkTarget p) {
    final caret = _controller.selection?.focus.offset ?? _pageStart;
    _controller.insertPageLink(
      _pageStart,
      caret,
      p.title,
      '$_pageScheme${p.id}',
    );
    _closePageLink();
    _syncImeFromSelection(force: true);
  }

  Widget _buildPageLinkOverlay(BuildContext context) {
    final r = _render;
    final sel = _controller.selection;
    if (r == null || sel == null) return const SizedBox.shrink();
    final rect = r.caretRectFor(sel.focus);
    if (rect == null) return const SizedBox.shrink();
    final origin = r.localToGlobal(rect.bottomLeft);
    final screen = MediaQuery.of(context).size;
    final items = _filteredPages();
    final left = origin.dx.clamp(8.0, screen.width - 296);
    final top = (origin.dy + 6).clamp(8.0, screen.height - 296);
    return Positioned(
      left: left,
      top: top,
      child: ExcludeFocus(
        child: Material(
          elevation: 8,
          borderRadius: BorderRadius.circular(8),
          child: Container(
            width: 288,
            constraints: const BoxConstraints(maxHeight: 288),
            padding: const EdgeInsets.symmetric(vertical: 6),
            child: ListView.builder(
              shrinkWrap: true,
              padding: EdgeInsets.zero,
              itemCount: items.length,
              itemBuilder: (context, i) {
                final p = items[i];
                final active = i == _pageIndex;
                return InkWell(
                  onTap: () => _applyPageLink(p),
                  child: Container(
                    color: active ? const Color(0x142563EB) : null,
                    padding: const EdgeInsets.symmetric(
                      horizontal: 12,
                      vertical: 8,
                    ),
                    child: Row(
                      children: [
                        const Icon(
                          Icons.description_outlined,
                          size: 18,
                          color: Color(0xFF475569),
                        ),
                        const SizedBox(width: 10),
                        Expanded(
                          child: Text(
                            p.title.isEmpty ? 'Untitled' : p.title,
                            overflow: TextOverflow.ellipsis,
                            style: const TextStyle(
                              fontSize: 14,
                              color: Color(0xFF0F172A),
                            ),
                          ),
                        ),
                      ],
                    ),
                  ),
                );
              },
            ),
          ),
        ),
      ),
    );
  }

  // ---------------------------------------------------------------------------
  // Slash menu
  // ---------------------------------------------------------------------------

  List<_SlashOption> get _slashMenu => [
    if (widget.onAiStream != null) _aiSlashOption,
    ..._slashOptions,
  ];

  List<_SlashOption> _filteredSlash() {
    final q = _slashQuery.toLowerCase();
    if (q.isEmpty) return _slashMenu;
    return _slashMenu.where((o) => o.label.toLowerCase().contains(q)).toList();
  }

  /// Recompute the slash session from the caret: open the menu when a `/` token
  /// precedes the caret, otherwise close it.
  void _refreshSlash() {
    final sel = _controller.selection;
    final node = _controller.focusedNode;
    if (sel == null || node == null || !sel.isCollapsed) {
      _closeSlash();
      return;
    }
    final caret = sel.focus.offset;
    if (caret > node.text.length) {
      _closeSlash();
      return;
    }
    final before = node.text.substring(0, caret);
    final slash = before.lastIndexOf('/');
    if (slash < 0) {
      _closeSlash();
      return;
    }
    if (slash > 0) {
      final prev = before.substring(slash - 1, slash);
      if (prev != ' ' && prev != '\n') {
        _closeSlash();
        return;
      }
    }
    // Spaces are allowed inside the query — menu labels contain them ("Math
    // formula"); the no-match close below ends the session once the text
    // stops looking like a command. A leading space ("/ …") is prose, not a
    // command, so it still dismisses.
    final query = before.substring(slash + 1);
    if (query.startsWith(' ')) {
      _closeSlash();
      return;
    }
    _slashStart = slash;
    _slashQuery = query;
    final items = _filteredSlash();
    if (items.isEmpty) {
      _closeSlash();
      return;
    }
    _slashIndex = _slashIndex.clamp(0, items.length - 1);
    if (_slashEntry == null) {
      _slashEntry = OverlayEntry(builder: _buildSlashOverlay);
      Overlay.of(context).insert(_slashEntry!);
    } else {
      _slashEntry!.markNeedsBuild();
    }
  }

  void _closeSlash() {
    _slashEntry?.remove();
    _slashEntry = null;
    _slashQuery = '';
    _slashIndex = 0;
  }

  void _applySlash(_SlashOption opt) {
    final caret = _controller.selection?.focus.offset ?? _slashStart;
    if (opt.kind == _aiSlashKind) {
      // Clear the "/..." text back to an empty paragraph, then run the AI flow.
      _controller.applySlashCommand(_slashStart, caret, 'paragraph', {});
      _closeSlash();
      _syncImeFromSelection(force: true);
      _runInlineAi();
      return;
    }
    if (opt.kind == 'divider') {
      // Clear the "/..." text, then insert an atomic divider with a trailing
      // paragraph for the caret.
      _controller.applySlashCommand(_slashStart, caret, 'paragraph', {});
      _controller.insertDivider();
      _closeSlash();
      _syncImeFromSelection(force: true);
      return;
    }
    if (opt.kind == 'image') {
      // Clear the "/..." text, then pick + upload a file and insert the block.
      _controller.applySlashCommand(_slashStart, caret, 'paragraph', {});
      _closeSlash();
      _syncImeFromSelection(force: true);
      _pickAndInsertImage();
      return;
    }
    final data = opt.kind == 'table'
        ? TableData.empty().toBlockData()
        : opt.data;
    // The converted block's index: the caret parks on a trailing paragraph
    // when the target kind is atomic, so remember where the block itself is.
    final converted = _controller.selection?.focus.node;
    _controller.applySlashCommand(_slashStart, caret, opt.kind, data);
    _closeSlash();
    _syncImeFromSelection(force: true);
    if (opt.kind == 'math_block' && converted != null) {
      // Straight into source editing — an empty formula shows nothing.
      final node = converted < _controller.nodes.length
          ? _controller.nodes[converted]
          : null;
      if (node != null && node.kind == 'math_block') {
        _editMathBlock(node);
      }
    }
  }

  /// Edit a math block's LaTeX source in a dialog.
  Future<void> _editMathBlock(EditorNode node) async {
    final controller = TextEditingController(text: node.text);
    final source = await showDialog<String>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Math formula', style: TextStyle(fontSize: 16)),
        content: SizedBox(
          width: 420,
          child: TextField(
            controller: controller,
            autofocus: true,
            maxLines: 4,
            minLines: 1,
            style: const TextStyle(fontFamily: kMonoFont, fontSize: 14),
            decoration: const InputDecoration(
              hintText: r'E = mc^2',
              isDense: true,
            ),
          ),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context),
            child: const Text('Cancel'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, controller.text),
            child: const Text('OK'),
          ),
        ],
      ),
    );
    if (source != null) {
      _controller.setBlockText(node.id, _stripMathDelimiters(source));
    }
  }

  /// Edit a typeset inline formula's source — same dialog as the block form,
  /// but writes back to the `[start, end)` run of node [i]. Empty source
  /// deletes the formula.
  Future<void> _editInlineMath(int i, int start, int end, String source) async {
    final controller = TextEditingController(text: source);
    final edited = await showDialog<String>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Math formula', style: TextStyle(fontSize: 16)),
        content: SizedBox(
          width: 420,
          child: TextField(
            controller: controller,
            autofocus: true,
            maxLines: 4,
            minLines: 1,
            style: const TextStyle(fontFamily: kMonoFont, fontSize: 14),
            decoration: const InputDecoration(
              hintText: r'\eta = 2 \times \frac{N-1}{N}',
              isDense: true,
            ),
          ),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context),
            child: const Text('Cancel'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, controller.text),
            child: const Text('OK'),
          ),
        ],
      ),
    );
    controller.dispose();
    if (edited == null) return;
    // Re-read the run: the dialog was modal, but stay defensive.
    if (i < 0 || i >= _controller.nodes.length) return;
    _controller.setInlineMathSource(
      i,
      start,
      end,
      _stripMathDelimiters(edited).trim(),
    );
    _syncImeFromSelection(force: true);
  }

  /// LaTeX sources are stored bare; users habitually paste `$$…$$`-wrapped
  /// formulas (the app's own convention) which the typesetter can't parse.
  static String _stripMathDelimiters(String source) {
    var src = source.trim();
    final m =
        RegExp(r'^\$\$(.*)\$\$$', dotAll: true).firstMatch(src) ??
        RegExp(r'^\\\[(.*)\\\]$', dotAll: true).firstMatch(src) ??
        RegExp(r'^\\\((.*)\\\)$', dotAll: true).firstMatch(src) ??
        RegExp(r'^\$([^$]+)\$$', dotAll: true).firstMatch(src);
    if (m != null) src = m.group(1)!.trim();
    return src;
  }

  /// Right-click on an image → context menu (copy / download / delete).
  void _onSecondaryTapDown(TapDownDetails d) {
    final r = _render;
    if (r == null) return;
    final local = r.globalToLocal(d.globalPosition);
    final node = r.imageAt(local);
    if (node != null) {
      if (!widget.canEdit) return;
      _focus.requestFocus();
      _controller.collapseTo(DocPosition(node, 0)); // select the block
      _showImageMenu(node, d.globalPosition);
      return;
    }
    // Table cells have their own editing surface (and the cell field shows the
    // native text menu) — no body text menu over them.
    if (r.tableCellAt(local) != null) return;
    _showTextContextMenu(local, d.globalPosition);
  }

  /// The body text context menu: copy/cut on the current selection, paste, and
  /// paste-as-plain-text. Right-clicking INSIDE a ranged selection keeps it
  /// (copy targets it); right-clicking elsewhere moves the caret there first,
  /// so a paste lands where clicked (standard editor behavior).
  Future<void> _showTextContextMenu(Offset local, Offset globalPosition) async {
    final r = _render;
    if (r == null) return;
    final sel = _controller.selection;
    final p = r.positionAt(local);
    var insideSelection = false;
    if (sel != null && !sel.isCollapsed) {
      final s = sel.start, e = sel.end;
      final afterStart =
          p.node > s.node || (p.node == s.node && p.offset >= s.offset);
      final beforeEnd =
          p.node < e.node || (p.node == e.node && p.offset <= e.offset);
      insideSelection = afterStart && beforeEnd;
    }
    _focus.requestFocus();
    if (!insideSelection && widget.canEdit) {
      _controller.setSelection(DocSelection.collapsed(p));
      _syncImeFromSelection(force: true);
    }
    final canEdit = widget.canEdit;
    if (!insideSelection && !canEdit) return; // read-only + nothing to copy
    final choice = await _showSmallMenu(globalPosition, [
      if (insideSelection) ('copy', '复制'),
      if (insideSelection && canEdit) ('cut', '剪切'),
      if (canEdit) ('paste', '粘贴'),
      if (canEdit) ('pastePlain', '粘贴为纯文本'),
    ]);
    if (choice == null || !mounted) return;
    switch (choice) {
      case 'copy':
        _copySelection();
      case 'cut':
        _cutSelection();
      case 'paste':
        _focus.requestFocus();
        await _pasteFromClipboard(requireFocus: false);
      case 'pastePlain':
        _focus.requestFocus();
        await _pastePlainText();
    }
  }

  Future<void> _showImageMenu(int node, Offset globalPosition) async {
    final overlay =
        Overlay.of(context).context.findRenderObject() as RenderBox?;
    if (overlay == null) return;
    final data = _controller.nodes[node].data;
    final isExternal =
        data['file_id'] == null &&
        (data['url'] as String?)?.startsWith('http') == true;
    final choice = await showMenu<String>(
      context: context,
      position: RelativeRect.fromLTRB(
        globalPosition.dx,
        globalPosition.dy,
        overlay.size.width - globalPosition.dx,
        overlay.size.height - globalPosition.dy,
      ),
      items: [
        // Say where the image LIVES. This used to be implied by the mere
        // presence of the re-host entry — you had to already know that to read
        // it. Storage is the norm; an external link is the one that can rot,
        // so it names the host you're depending on.
        PopupMenuItem(
          enabled: false,
          height: 32,
          child: Text(
            isExternal
                ? '外部链接 · ${_urlHost(data['url'] as String? ?? '')}'
                // Naming it "public" matters: the blob url is unauthenticated
                // by design (the UUID is the capability) so copied images keep
                // rendering elsewhere. Copying the link IS sharing the image.
                : '已存储到 Mica · 链接公开可访问',
            style: TextStyle(fontSize: 12, color: EditorTheme.muted),
          ),
        ),
        const PopupMenuDivider(height: 1),
        const PopupMenuItem(value: 'expand', child: Text('全屏查看')),
        const PopupMenuItem(value: 'edit', child: Text('编辑图片…')),
        // Alignment is deliberately NOT here — the hover toolbar over the
        // picture already owns it.
        const PopupMenuDivider(height: 1),
        if (isExternal)
          const PopupMenuItem(value: 'rehost', child: Text('转存到 Mica 存储')),
        // Both forms have a link worth copying. A stored image's is its Mica
        // blob url: stable, never-expiring, and PUBLIC — the file_id UUID is
        // the capability, which is what lets a copied image still render in
        // Typora/a browser. Copy/export already emit it; there was just no way
        // to get at it deliberately.
        if (_imageLinkOf(data) != null) ...[
          const PopupMenuItem(value: 'copyLink', child: Text('复制图片链接')),
          const PopupMenuItem(value: 'openLink', child: Text('在浏览器中打开')),
        ],
        const PopupMenuItem(value: 'copy', child: Text('复制图片')),
        const PopupMenuItem(value: 'cut', child: Text('剪切图片')),
        // Reading a bitmap off the clipboard is a desktop-only pull: on web the
        // clipboard only arrives through the DOM paste event, so the facade
        // returns null and this could never do anything but fail.
        if (!kIsWeb && widget.onUploadImage != null)
          const PopupMenuItem(value: 'paste', child: Text('粘贴图片(替换)')),
        const PopupMenuItem(value: 'download', child: Text('下载')),
        const PopupMenuItem(value: 'delete', child: Text('删除')),
      ],
    );
    if (choice == null || !mounted) return;
    switch (choice) {
      case 'expand':
        _openImageViewer(node);
      case 'edit':
        await _showImageEditDialog(node);
      case 'rehost':
        await _rehostImage(node);
      case 'copyLink':
        final link = _imageLinkOf(data);
        if (link != null) {
          await Clipboard.setData(ClipboardData(text: link));
          if (mounted) {
            _toast(isExternal ? '已复制原始链接' : '已复制图片链接(公开可访问)');
          }
        }
      case 'openLink':
        final link = _imageLinkOf(data);
        if (link != null) openUrl(link);
      case 'copy':
        await _copyImage(node);
      case 'cut':
        await _cutImage(node);
      case 'paste':
        await _pasteImageOver(node);
      case 'download':
        await _downloadImage(node);
      case 'delete':
        _controller.deleteNode(node);
        _syncImeFromSelection(force: true);
    }
  }

  /// Cut = copy, then delete — but ONLY if the copy actually landed. Deleting
  /// after a failed clipboard write would destroy the image with nothing to
  /// paste back.
  Future<void> _cutImage(int node) async {
    if (!await _copyImage(node)) return;
    if (!mounted || node < 0 || node >= _controller.nodes.length) return;
    _controller.deleteNode(node);
    _syncImeFromSelection(force: true);
  }

  /// Replace this image with the bitmap on the clipboard (upload it first, so
  /// the block ends up on our storage like any pasted screenshot).
  Future<void> _pasteImageOver(int node) async {
    final upload = widget.onUploadImage;
    if (upload == null || node < 0 || node >= _controller.nodes.length) return;
    final id = _controller.nodes[node].id;
    final bytes = await readClipboardImage();
    if (!mounted) return;
    if (bytes == null || bytes.isEmpty) {
      _toast('剪贴板里没有图片');
      return;
    }
    final result = await upload(bytes, 'pasted-image.png', 'image/png');
    if (!mounted) return;
    if (result == null) {
      _toast('上传失败');
      return;
    }
    _primeImage(result.fileId, bytes);
    _controller.setImageSource(id, fileId: result.fileId, name: result.name);
  }

  /// Image properties + replace. Shows where the image actually lives — its
  /// full link, whichever kind — because that is the thing you cannot tell by
  /// looking at the canvas, and it decides whether the doc can rot.
  Future<void> _showImageEditDialog(int node) async {
    if (node < 0 || node >= _controller.nodes.length) return;
    final id = _controller.nodes[node].id;
    final data = _controller.nodes[node].data;
    final external =
        data['file_id'] == null &&
        (data['url'] as String?)?.startsWith('http') == true;
    final link = _imageLinkOf(data) ?? '(链接尚未解析)';
    // Returns the url to replace with; '' = handled already (a file upload) or
    // nothing typed; null = cancelled.
    final url = await showDialog<String>(
      context: context,
      builder: (_) => _ImageEditDialog(
        link: link,
        external: external,
        reHost: widget.reHostImages,
        onUploadReplace: widget.onUploadImage == null
            ? null
            : () => _replaceImageFromFile(id),
      ),
    );
    if (url == null || url.isEmpty || !mounted) return;
    if (!url.startsWith('http://') && !url.startsWith('https://')) {
      _toast('请填写 http(s) 开头的图片链接');
      return;
    }
    // Show it immediately on its url, then run the same server-then-client
    // re-host ladder the paste path uses — so "replace by link" obeys the
    // re-host setting instead of quietly inventing its own rule.
    _controller.setImageUrl(id, url);
    if (!widget.reHostImages) return;
    if (!_rehostPending.add(url)) return;
    await _rehostOne(id, url);
    if (mounted) setState(() => _rehostPending.remove(url));
  }

  /// Pick a local file and point the image block at the uploaded copy.
  Future<bool> _replaceImageFromFile(String nodeId) async {
    final upload = widget.onUploadImage;
    if (upload == null) return false;
    final picked = await pickImage();
    if (picked == null) return false;
    final result = await upload(picked.bytes, picked.name, picked.mime);
    if (result == null || !mounted) {
      if (mounted) _toast('上传失败');
      return false;
    }
    _primeImage(result.fileId, picked.bytes);
    _controller.setImageSource(
      nodeId,
      fileId: result.fileId,
      name: result.name,
    );
    return true;
  }

  /// The host of a url, for showing which site an image depends on.
  static String _urlHost(String url) {
    final h = Uri.tryParse(url)?.host ?? '';
    return h.isEmpty ? url : h;
  }

  /// A viewable link for an image block: the external url, or our own blob url
  /// for a stored one (resolved eagerly into [_imageUrlCache] for copy/export).
  /// Null when a stored image's link hasn't resolved yet (offline / local-only
  /// workspace) — the menu then just omits the link entries rather than
  /// offering one that copies an empty string.
  String? _imageLinkOf(Map<String, dynamic> data) {
    final url = data['url'] as String?;
    if (url != null && url.startsWith('http')) return url;
    final fileId = data['file_id'] as String?;
    return fileId == null ? null : _imageUrlCache[fileId];
  }

  /// Re-host one external image into Mica storage (right-click action). Uses
  /// the same server-then-client ladder as the automatic pass, so it works
  /// even when this server can't reach the host.
  Future<void> _rehostImage(int node) async {
    if (node < 0 || node >= _controller.nodes.length) return;
    final url = _controller.nodes[node].data['url'] as String?;
    if (url == null) return;
    final id = _controller.nodes[node].id;
    if (!_rehostPending.add(url)) return;
    final ok = await _rehostOne(id, url);
    if (!mounted) return;
    setState(() => _rehostPending.remove(url));
    if (!ok) _toast('转存失败:这台服务器和本机都取不到该图片');
  }

  /// Bytes + filename for an image node (re-fetched via the host loader).
  Future<({Uint8List bytes, String name, String mime})?> _imageData(
    int node,
  ) async {
    if (node < 0 || node >= _controller.nodes.length) return null;
    final data = _controller.nodes[node].data;
    final key = (data['file_id'] ?? data['url']) as String?;
    if (key == null) return null;
    final bytes = await widget.onLoadImageBytes?.call(key);
    if (bytes == null) return null;
    final name = (data['name'] as String?)?.trim();
    final fallback = 'image.${_extFromName(name) ?? 'png'}';
    return (
      bytes: bytes,
      name: (name == null || name.isEmpty) ? fallback : name,
      mime: _mimeFromName(name),
    );
  }

  /// True when the bytes actually reached the clipboard — [_cutImage] hangs the
  /// delete on this answer.
  Future<bool> _copyImage(int node) async {
    final data = await _imageData(node);
    if (data == null || !mounted) {
      _toast('图片读取失败');
      return false;
    }
    final ok = await copyImageToClipboard(data.bytes, data.mime);
    if (!mounted) return ok;
    _toast(ok ? '图片已复制' : '复制失败 —— 可改用“下载”');
    return ok;
  }

  Future<void> _downloadImage(int node) async {
    final data = await _imageData(node);
    if (data == null || !mounted) {
      _toast('Could not load the image');
      return;
    }
    downloadImage(data.bytes, data.name, data.mime);
  }

  void _toast(String message) {
    final messenger = ScaffoldMessenger.maybeOf(context);
    messenger?.showSnackBar(
      SnackBar(content: Text(message), duration: const Duration(seconds: 2)),
    );
  }

  String? _extFromName(String? name) {
    if (name == null) return null;
    final dot = name.lastIndexOf('.');
    if (dot < 0 || dot == name.length - 1) return null;
    return name.substring(dot + 1).toLowerCase();
  }

  String _mimeFromName(String? name) {
    switch (_extFromName(name)) {
      case 'jpg':
      case 'jpeg':
        return 'image/jpeg';
      case 'gif':
        return 'image/gif';
      case 'webp':
        return 'image/webp';
      case 'svg':
        return 'image/svg+xml';
      case 'bmp':
        return 'image/bmp';
      default:
        return 'image/png';
    }
  }

  void _onImageAction(int node, String action) {
    switch (action) {
      case 'expand':
        _openImageViewer(node);
      case 'left':
      case 'center':
      case 'right':
        _controller.setImageAlign(node, action);
      case 'delete':
        _controller.deleteNode(node);
        _syncImeFromSelection(force: true);
    }
  }

  /// Show a fullscreen, zoom/pan viewer for the image (its decoded bytes are
  /// already on the canvas, so reuse them).
  void _openImageViewer(int node) {
    if (node < 0 || node >= _controller.nodes.length) return;
    final data = _controller.nodes[node].data;
    final key = (data['file_id'] ?? data['url']) as String?;
    if (key == null || _imageCache[key] == null) return;
    showDialog<void>(
      context: context,
      barrierColor: const Color(0xCC000000),
      builder: (context) => Dialog(
        backgroundColor: Colors.transparent,
        insetPadding: const EdgeInsets.all(24),
        child: Stack(
          children: [
            InteractiveViewer(
              maxScale: 6,
              // An animated image keeps playing in here: the tick fires once
              // per frame, and RawImage clones whatever it is handed — so the
              // editor stays free to dispose the frame it has moved on from.
              child: Center(
                child: ValueListenableBuilder<int>(
                  valueListenable: _imageFrameTick,
                  builder: (context, _, child) {
                    final frame = _imageCache[key];
                    return frame == null
                        ? const SizedBox.shrink()
                        : RawImage(image: frame);
                  },
                ),
              ),
            ),
            Positioned(
              top: 0,
              right: 0,
              child: IconButton(
                icon: const Icon(Icons.close, color: Colors.white),
                onPressed: () => Navigator.of(context).pop(),
              ),
            ),
          ],
        ),
      ),
    );
  }

  /// Pick an image file, upload it, and insert an image block at the caret.
  Future<void> _pickAndInsertImage() async {
    final upload = widget.onUploadImage;
    if (upload == null) return;
    final picked = await pickImage();
    if (picked == null || !mounted) return;
    final result = await upload(picked.bytes, picked.name, picked.mime);
    if (result == null || !mounted) return;
    // Decode straightaway so the image paints without a resolve round-trip.
    _primeImage(result.fileId, picked.bytes);
    _controller.insertImage(fileId: result.fileId, name: result.name);
    _syncImeFromSelection(force: true);
  }

  /// Upload a pasted bitmap (screenshot / copied image) and insert it.
  Future<void> _handlePasteImage(
    Uint8List bytes,
    String mime,
    String name,
  ) async {
    final upload = widget.onUploadImage;
    if (upload == null || !mounted || !_focus.hasFocus || !widget.canEdit)
      return;
    final result = await upload(bytes, name, mime);
    if (result == null || !mounted) return;
    _primeImage(result.fileId, bytes);
    _controller.insertImage(fileId: result.fileId, name: result.name);
    _syncImeFromSelection(force: true);
  }

  /// Seed the canvas image cache with freshly-uploaded bytes (skip the fetch).
  void _primeImage(String fileId, Uint8List bytes) {
    if (_imageCache.containsKey(fileId) || _imageLoading.contains(fileId)) {
      return;
    }
    _decodeInto(fileId, bytes).catchError((_) {
      if (mounted) setState(() => _imageLoading.remove(fileId));
    });
  }

  Future<void> _runInlineAi() async {
    final stream = widget.onAiStream;
    if (stream == null) return;
    final markdown = await showDialog<String>(
      context: context,
      builder: (context) => _InlineAiDialog(onStream: stream),
    );
    if (markdown == null || markdown.trim().isEmpty) return;
    _controller.insertBlocksAfterFocus(markdownToBlocks(markdown));
    _rehostExternalImages();
    _syncImeFromSelection(force: true);
  }

  Widget _buildSlashOverlay(BuildContext context) {
    final r = _render;
    final sel = _controller.selection;
    if (r == null || sel == null) return const SizedBox.shrink();
    final rect = r.caretRectFor(sel.focus);
    if (rect == null) return const SizedBox.shrink();
    final origin = r.localToGlobal(rect.bottomLeft);
    final screen = MediaQuery.of(context).size;
    final items = _filteredSlash();
    final left = origin.dx.clamp(8.0, screen.width - 256);
    final top = (origin.dy + 6).clamp(8.0, screen.height - 296);
    return Positioned(
      left: left,
      top: top,
      // Never let the menu take keyboard focus away from the editor (a blur
      // would dismiss the menu). Taps still work — they are pointer-based.
      child: ExcludeFocus(
        child: Material(
          elevation: 8,
          borderRadius: BorderRadius.circular(8),
          child: Container(
            width: 248,
            constraints: const BoxConstraints(maxHeight: 288),
            padding: const EdgeInsets.symmetric(vertical: 6),
            child: ListView.builder(
              shrinkWrap: true,
              padding: EdgeInsets.zero,
              itemCount: items.length,
              itemBuilder: (context, i) {
                final o = items[i];
                final active = i == _slashIndex;
                return InkWell(
                  onTap: () => _applySlash(o),
                  child: Container(
                    color: active ? const Color(0x142563EB) : null,
                    padding: const EdgeInsets.symmetric(
                      horizontal: 12,
                      vertical: 8,
                    ),
                    child: Row(
                      children: [
                        Icon(o.icon, size: 18, color: const Color(0xFF475569)),
                        const SizedBox(width: 10),
                        Text(
                          o.label,
                          style: const TextStyle(
                            fontSize: 14,
                            color: Color(0xFF0F172A),
                          ),
                        ),
                      ],
                    ),
                  ),
                );
              },
            ),
          ),
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    return Listener(
      onPointerSignal: _onPointerSignal,
      // Capture the down timestamp (binding clock) for multi-tap counting; the
      // GestureDetector's onTapDown does not expose it.
      onPointerDown: (e) => _downStamp = e.timeStamp,
      child: MouseRegion(
        cursor: widget.canEdit ? _cursor : MouseCursor.defer,
        onHover: _onHover,
        onExit: (_) {
          _render?.setHover(null);
          if (!_pointerOverLinkBar) _scheduleLinkBarHide();
        },
        child: Focus(
          focusNode: _focus,
          onKeyEvent: _onKey,
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTapDown: _onTapDown,
            onSecondaryTapDown: _onSecondaryTapDown,
            onPanDown: _onPanDown,
            onPanStart: _onPanStart,
            onPanUpdate: _onPanUpdate,
            onPanEnd: _onPanEnd,
            child: Stack(
              clipBehavior: Clip.hardEdge,
              children: [
                DocumentSurface(
                  key: _surfaceKey,
                  nodes: _controller.nodes,
                  selection: _controller.selection,
                  showCaret: _focus.hasFocus && widget.canEdit,
                  caretOn: _caretOn,
                  appearance: widget.appearance,
                  images: _imageCache,
                  imageErrors: _imageErrors,
                  previewImages: _previews.images,
                  previewBaselines: _previews.baselines,
                  onRequestPreview: _previews.request,
                  onRequestImage: _requestImage,
                  onImagePainted: _onImagePainted,
                  remoteCursors: widget.remoteCursors,
                ),
                // Far off-screen: painted (capturable) but never visible.
                Positioned(
                  left: -100000,
                  top: 0,
                  child: _previews.offstageHost(),
                ),
                if (_findOpen)
                  Positioned(top: 6, right: 6, child: _buildFindBar()),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// An entry in the slash (`/`) insert menu: a label, an icon, and the block
/// kind (+ optional data) the focused block is converted to.
class _SlashOption {
  const _SlashOption(this.label, this.icon, this.kind, [this.data = const {}]);

  final String label;
  final IconData icon;
  final String kind;
  final Map<String, dynamic> data;
}

/// Special slash-menu kind that triggers the AI flow instead of a block convert.
const String _aiSlashKind = '__ai__';
const _SlashOption _aiSlashOption = _SlashOption(
  'Ask AI',
  Icons.auto_awesome,
  _aiSlashKind,
);

/// In-editor "Ask AI": streams the response live, then returns the accumulated
/// Markdown (via Navigator.pop) for the editor to insert as blocks.
class _InlineAiDialog extends StatefulWidget {
  const _InlineAiDialog({required this.onStream});

  final Stream<String> Function(String prompt, {String? system}) onStream;

  @override
  State<_InlineAiDialog> createState() => _InlineAiDialogState();
}

class _InlineAiDialogState extends State<_InlineAiDialog> {
  final _prompt = TextEditingController();
  final _scroll = ScrollController();
  final StringBuffer _buffer = StringBuffer();
  StreamSubscription<String>? _sub;
  bool _streaming = false;
  bool _done = false;
  String? _error;

  @override
  void dispose() {
    _sub?.cancel();
    _prompt.dispose();
    _scroll.dispose();
    super.dispose();
  }

  void _generate() {
    final prompt = _prompt.text.trim();
    if (prompt.isEmpty) return;
    setState(() {
      _streaming = true;
      _done = false;
      _error = null;
      _buffer.clear();
    });
    _sub = widget
        .onStream(prompt)
        .listen(
          (delta) {
            setState(() => _buffer.write(delta));
            WidgetsBinding.instance.addPostFrameCallback((_) {
              if (_scroll.hasClients) {
                _scroll.jumpTo(_scroll.position.maxScrollExtent);
              }
            });
          },
          onError: (Object error) {
            if (mounted) {
              setState(() {
                _streaming = false;
                _error = error.toString();
              });
            }
          },
          onDone: () {
            if (mounted) {
              setState(() {
                _streaming = false;
                _done = true;
              });
            }
          },
        );
  }

  @override
  Widget build(BuildContext context) {
    final hasOutput = _buffer.isNotEmpty;
    return AlertDialog(
      title: const Row(
        children: [
          Icon(Icons.auto_awesome, size: 20, color: Color(0xFF7C3AED)),
          SizedBox(width: 8),
          Text('Ask AI'),
        ],
      ),
      content: SizedBox(
        width: 480,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            TextField(
              controller: _prompt,
              autofocus: true,
              minLines: 2,
              maxLines: 4,
              enabled: !_streaming,
              decoration: const InputDecoration(
                hintText: 'Describe what to write…',
                border: OutlineInputBorder(),
              ),
            ),
            if (hasOutput || _streaming) ...[
              const SizedBox(height: 12),
              Container(
                height: 200,
                width: double.infinity,
                padding: const EdgeInsets.all(12),
                decoration: BoxDecoration(
                  color: const Color(0xFFF8FAFC),
                  borderRadius: BorderRadius.circular(8),
                  border: Border.all(color: const Color(0xFFE2E8F0)),
                ),
                child: SingleChildScrollView(
                  controller: _scroll,
                  child: Text(
                    _buffer.isEmpty ? '…' : _buffer.toString(),
                    style: const TextStyle(fontSize: 13, height: 1.5),
                  ),
                ),
              ),
              const SizedBox(height: 6),
              if (_streaming)
                const Row(
                  children: [
                    SizedBox(
                      width: 14,
                      height: 14,
                      child: CircularProgressIndicator(strokeWidth: 2),
                    ),
                    SizedBox(width: 8),
                    Text('Generating…'),
                  ],
                ),
            ],
            if (_error != null) ...[
              const SizedBox(height: 12),
              Container(
                width: double.infinity,
                padding: const EdgeInsets.all(10),
                decoration: BoxDecoration(
                  color: const Color(0x14DC2626),
                  borderRadius: BorderRadius.circular(6),
                ),
                child: Text(
                  _error!,
                  style: const TextStyle(
                    color: Color(0xFFB91C1C),
                    fontSize: 13,
                  ),
                ),
              ),
            ],
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: _streaming ? null : () => Navigator.of(context).pop(),
          child: const Text('Cancel'),
        ),
        if (!_done)
          FilledButton.icon(
            onPressed: _streaming ? null : _generate,
            icon: const Icon(Icons.auto_awesome, size: 18),
            label: Text(hasOutput ? 'Regenerate' : 'Generate'),
          )
        else ...[
          TextButton.icon(
            onPressed: _generate,
            icon: const Icon(Icons.refresh, size: 18),
            label: const Text('Regenerate'),
          ),
          FilledButton.icon(
            onPressed: () => Navigator.of(context).pop(_buffer.toString()),
            icon: const Icon(Icons.check, size: 18),
            label: const Text('Insert'),
          ),
        ],
      ],
    );
  }
}

const List<_SlashOption> _slashOptions = [
  _SlashOption('Text', Icons.notes, 'paragraph'),
  _SlashOption('Heading 1', Icons.title, 'heading', {'level': 1}),
  _SlashOption('Heading 2', Icons.title, 'heading', {'level': 2}),
  _SlashOption('Heading 3', Icons.title, 'heading', {'level': 3}),
  _SlashOption('Bulleted list', Icons.format_list_bulleted, 'bulleted_list'),
  _SlashOption('Numbered list', Icons.format_list_numbered, 'numbered_list'),
  _SlashOption('To-do', Icons.check_box_outlined, 'todo', {'checked': false}),
  _SlashOption('Quote', Icons.format_quote, 'quote'),
  _SlashOption('Code', Icons.code, 'code_block'),
  _SlashOption('Math formula', Icons.functions, 'math_block'),
  _SlashOption('Table', Icons.grid_on, 'table'),
  _SlashOption('Divider', Icons.horizontal_rule, 'divider'),
  _SlashOption('Image', Icons.image_outlined, 'image'),
];

/// Code-block language picker: a small anchored panel with a search box —
/// the language list is long, so filtering beats scrolling. ↑/↓ move the
/// highlight, Enter picks, Esc dismisses; pops with the chosen language.
class _LanguagePicker extends StatefulWidget {
  const _LanguagePicker({required this.anchor, required this.current});

  final Offset anchor;

  /// The block's own setting — `auto` when it has none. Not the *resolved*
  /// language: the point is to show whether the block is pinned or detecting.
  final String current;

  @override
  State<_LanguagePicker> createState() => _LanguagePickerState();
}

class _LanguagePickerState extends State<_LanguagePicker> {
  final TextEditingController _query = TextEditingController();
  final ScrollController _scroll = ScrollController();
  int _index = 0;

  List<String> get _filtered {
    final q = _query.text.trim().toLowerCase();
    // Alphabetical so the list is scannable; `auto` (content detection) stays
    // pinned on top as the smart default.
    final rest = kCodeLanguages.where((l) => l != 'auto').toList()..sort();
    final all = <String>['auto', ...rest];
    if (q.isEmpty) return all;
    return [
      for (final l in all)
        if (l.toLowerCase().contains(q)) l,
    ];
  }

  @override
  void dispose() {
    _query.dispose();
    _scroll.dispose();
    super.dispose();
  }

  void _pick(String language) => Navigator.of(context).pop(language);

  KeyEventResult _onKey(FocusNode node, KeyEvent event) {
    if (event is! KeyDownEvent) return KeyEventResult.ignored;
    final items = _filtered;
    if (event.logicalKey == LogicalKeyboardKey.escape) {
      Navigator.of(context).pop();
      return KeyEventResult.handled;
    }
    if (event.logicalKey == LogicalKeyboardKey.arrowDown && items.isNotEmpty) {
      setState(() => _index = (_index + 1) % items.length);
      _reveal(items.length);
      return KeyEventResult.handled;
    }
    if (event.logicalKey == LogicalKeyboardKey.arrowUp && items.isNotEmpty) {
      setState(() => _index = (_index - 1 + items.length) % items.length);
      _reveal(items.length);
      return KeyEventResult.handled;
    }
    if (event.logicalKey == LogicalKeyboardKey.enter && items.isNotEmpty) {
      _pick(items[_index.clamp(0, items.length - 1)]);
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  void _reveal(int count) {
    if (!_scroll.hasClients || count == 0) return;
    const itemH = 34.0;
    final target = (_index * itemH).clamp(
      0.0,
      _scroll.position.maxScrollExtent,
    );
    final top = _scroll.offset;
    final bottom = top + 240 - itemH;
    if (target < top || target > bottom) {
      _scroll.jumpTo(
        (target - 100).clamp(0.0, _scroll.position.maxScrollExtent),
      );
    }
  }

  @override
  Widget build(BuildContext context) {
    final screen = MediaQuery.of(context).size;
    const w = 240.0;
    const h = 312.0;
    final left = widget.anchor.dx.clamp(8.0, screen.width - w - 8);
    final top = widget.anchor.dy.clamp(8.0, screen.height - h - 8);
    final items = _filtered;
    final active = _index.clamp(0, items.isEmpty ? 0 : items.length - 1);

    return Stack(
      children: [
        Positioned(
          left: left,
          top: top,
          child: Focus(
            onKeyEvent: _onKey,
            child: Material(
              elevation: 8,
              borderRadius: BorderRadius.circular(8),
              child: Container(
                width: w,
                height: h,
                padding: const EdgeInsets.all(8),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    TextField(
                      controller: _query,
                      autofocus: true,
                      style: const TextStyle(fontSize: 13),
                      decoration: const InputDecoration(
                        hintText: 'Search language…',
                        prefixIcon: Icon(Icons.search, size: 16),
                        prefixIconConstraints: BoxConstraints(minWidth: 28),
                        isDense: true,
                        border: OutlineInputBorder(),
                        contentPadding: EdgeInsets.symmetric(
                          horizontal: 8,
                          vertical: 8,
                        ),
                      ),
                      onChanged: (_) => setState(() => _index = 0),
                    ),
                    const SizedBox(height: 6),
                    Expanded(
                      child: items.isEmpty
                          ? const Center(
                              child: Text(
                                'No matches',
                                style: TextStyle(
                                  fontSize: 13,
                                  color: Color(0xFF94A3B8),
                                ),
                              ),
                            )
                          : ListView.builder(
                              controller: _scroll,
                              itemExtent: 34,
                              itemCount: items.length,
                              itemBuilder: (context, i) => InkWell(
                                onTap: () => _pick(items[i]),
                                child: Container(
                                  color: i == active
                                      ? const Color(0x142563EB)
                                      : null,
                                  padding: const EdgeInsets.symmetric(
                                    horizontal: 10,
                                  ),
                                  alignment: Alignment.centerLeft,
                                  child: Row(
                                    children: [
                                      // Which one is this block actually on?
                                      // Without the tick the list gave no
                                      // feedback at all, so a block pinned to
                                      // `python` looked exactly like one on
                                      // `auto` that had detected python.
                                      SizedBox(
                                        width: 20,
                                        child: items[i] == widget.current
                                            ? const Icon(
                                                Icons.check,
                                                size: 14,
                                                color: Color(0xFF2563EB),
                                              )
                                            : null,
                                      ),
                                      Text(
                                        items[i],
                                        style: TextStyle(
                                          fontSize: 13,
                                          fontWeight: items[i] == widget.current
                                              ? FontWeight.w600
                                              : FontWeight.normal,
                                        ),
                                      ),
                                    ],
                                  ),
                                ),
                              ),
                            ),
                    ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ],
    );
  }
}

/// Image properties + replace. Owns its own [TextEditingController]: creating it
/// outside and disposing after `showDialog` returns is a trap — the route's exit
/// animation is still building the TextField, and it throws "used after being
/// disposed".
///
/// Pops the url to replace with; `''` when a file upload already handled the
/// replacement (or nothing was typed), null on cancel.
class _ImageEditDialog extends StatefulWidget {
  const _ImageEditDialog({
    required this.link,
    required this.external,
    required this.reHost,
    this.onUploadReplace,
  });

  final String link;
  final bool external;

  /// Mirrors the app's "re-host pasted images" setting, so the dialog can say
  /// what a pasted link will actually do instead of leaving you to guess.
  final bool reHost;
  final Future<bool> Function()? onUploadReplace;

  @override
  State<_ImageEditDialog> createState() => _ImageEditDialogState();
}

class _ImageEditDialogState extends State<_ImageEditDialog> {
  final _url = TextEditingController();
  var _busy = false;

  @override
  void dispose() {
    _url.dispose();
    super.dispose();
  }

  Future<void> _upload() async {
    final pick = widget.onUploadReplace;
    if (pick == null || _busy) return;
    setState(() => _busy = true);
    final ok = await pick();
    if (!mounted) return;
    setState(() => _busy = false);
    if (ok) Navigator.pop(context, '');
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: const Text('编辑图片'),
      content: SizedBox(
        width: 520,
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                widget.external ? '外部链接 · 原站失效后图片会丢失' : '已存储到 Mica · 链接公开可访问',
                style: TextStyle(fontSize: 12, color: EditorTheme.muted),
              ),
              const SizedBox(height: 6),
              Container(
                width: double.infinity,
                padding: const EdgeInsets.all(8),
                decoration: BoxDecoration(
                  color: EditorTheme.codeBg,
                  borderRadius: BorderRadius.circular(6),
                ),
                child: SelectableText(
                  widget.link,
                  maxLines: 3,
                  style: const TextStyle(fontFamily: kMonoFont, fontSize: 12),
                ),
              ),
              Align(
                alignment: Alignment.centerRight,
                child: TextButton.icon(
                  icon: const Icon(Icons.copy, size: 16),
                  label: const Text('复制链接'),
                  onPressed: () =>
                      Clipboard.setData(ClipboardData(text: widget.link)),
                ),
              ),
              const Divider(),
              const Text('替换图片', style: TextStyle(fontWeight: FontWeight.w600)),
              const SizedBox(height: 8),
              OutlinedButton.icon(
                icon: _busy
                    ? const SizedBox(
                        width: 16,
                        height: 16,
                        child: CircularProgressIndicator(strokeWidth: 2),
                      )
                    : const Icon(Icons.upload_file, size: 18),
                label: const Text('上传本地图片'),
                onPressed: widget.onUploadReplace == null || _busy
                    ? null
                    : _upload,
              ),
              const SizedBox(height: 10),
              TextField(
                controller: _url,
                autofocus: true,
                decoration: const InputDecoration(
                  labelText: '或粘贴图片链接',
                  hintText: 'https://…',
                  isDense: true,
                ),
                onSubmitted: (v) => Navigator.pop(context, v.trim()),
              ),
              const SizedBox(height: 6),
              Text(
                widget.reHost
                    ? '已启用自动转存:链接会被转存到 Mica 存储(取不到则保留原链接)'
                    : '自动转存已关闭:将直接保留原链接',
                style: TextStyle(fontSize: 11, color: EditorTheme.faint),
              ),
            ],
          ),
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context),
          child: const Text('取消'),
        ),
        FilledButton(
          onPressed: () => Navigator.pop(context, _url.text.trim()),
          child: const Text('替换'),
        ),
      ],
    );
  }
}
