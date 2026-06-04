import 'dart:async';
import 'dart:ui' as ui;

import 'package:flutter/rendering.dart' show RenderRepaintBoundary;
import 'package:flutter_math_fork/flutter_math.dart';

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
import 'model.dart';
import 'open_url.dart';
import 'pick_image.dart';
import 'render.dart';
import 'rich_paste.dart';
import 'table.dart';

export 'controller.dart' show DocOp, ApplyOps;
export 'markdown.dart' show markdownToBlocks, BlockSpec;
export 'model.dart' show EditorNode;
export 'render.dart' show EditorAppearance;

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
}

class MicaEditor extends StatefulWidget {
  const MicaEditor({
    required this.rootBlockId,
    required this.nodes,
    required this.version,
    required this.canEdit,
    required this.onApplyOperations,
    this.onAiStream,
    this.onUploadImage,
    this.onImportImageUrl,
    this.onLoadImageBytes,
    this.onResolveImageUrls,
    this.reHostImages = true,
    this.focusNode,
    this.scrollHook,
    this.commandHook,
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

  /// Streams Markdown from a prompt for the in-editor "Ask AI" command (deltas
  /// shown live). When null, the AI slash entry is hidden.
  final Stream<String> Function(String prompt, {String? system})? onAiStream;

  /// Upload image bytes, returning the new `(file_id, name)`. When null, image
  /// insertion is disabled.
  final Future<({String fileId, String name})?> Function(
    Uint8List bytes,
    String fileName,
    String mimeType,
  )? onUploadImage;

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
  TextEditingValue _lastSentIme = TextEditingValue.empty;

  Timer? _blink;
  bool _caretOn = true;
  DocPosition? _dragAnchor;
  int? _scrollbarDrag; // code-block index whose scrollbar is being dragged
  int? _blockDrag; // block index being moved via its gutter drag handle

  // Math rasterization: formulas render in an offstage flutter_math_fork
  // widget, get captured as ui.Image post-frame, and paint like images.
  final Map<String, ui.Image> _mathCache = {};
  final Set<String> _mathPending = {};
  final Map<String, GlobalKey> _mathKeys = {};

  void _requestMath(String source) {
    if (_mathCache.containsKey(source) || _mathPending.contains(source)) {
      return;
    }
    // Called from the render object's layout — defer the rebuild.
    _mathPending.add(source);
    _mathKeys[source] = GlobalKey();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      setState(() {});
      WidgetsBinding.instance.addPostFrameCallback((_) => _captureMath());
    });
  }

  Future<void> _captureMath() async {
    if (_mathPending.isEmpty) return;
    final captured = <String>[];
    for (final source in _mathPending.toList()) {
      final boundary = _mathKeys[source]?.currentContext?.findRenderObject();
      if (boundary is! RenderRepaintBoundary || boundary.debugNeedsPaint) {
        continue; // not laid out yet — retry next frame
      }
      try {
        final img =
            await boundary.toImage(pixelRatio: EditorTheme.mathPixelRatio);
        _mathCache[source] = img;
        captured.add(source);
      } catch (_) {
        captured.add(source); // give up on this source; keep showing text
      }
    }
    if (captured.isNotEmpty && mounted) {
      setState(() {
        for (final s in captured) {
          _mathPending.remove(s);
          _mathKeys.remove(s);
        }
      });
    }
    if (_mathPending.isNotEmpty && mounted) {
      WidgetsBinding.instance.addPostFrameCallback((_) => _captureMath());
    }
  }

  /// Offstage host that lays out pending formulas for capture.
  Widget _mathRasterHost() => Offstage(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            for (final source in _mathPending)
              RepaintBoundary(
                key: _mathKeys[source],
                child: Math.tex(
                  source,
                  textStyle:
                      const TextStyle(fontSize: 18, color: EditorTheme.text),
                  onErrorFallback: (e) => Text(
                    source,
                    style: const TextStyle(
                      fontFamily: 'monospace',
                      fontSize: 14,
                      color: Color(0xFFB91C1C),
                    ),
                  ),
                ),
              ),
          ],
        ),
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
  OverlayEntry? _markBar; // floating inline-format toolbar over a selection
  MouseCursor _cursor = SystemMouseCursors.text;

  // Decoded images keyed by file_id, painted on the canvas by RenderDocument.
  final Map<String, ui.Image> _imageCache = {};
  final Set<String> _imageErrors = {};
  final Set<String> _imageLoading = {};
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
    _registerCommandHook();
    _ensureNotEmptyDeferred();
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
      };
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
    final target = (pos.pixels + (globalY - viewTop) - 16)
        .clamp(pos.minScrollExtent, pos.maxScrollExtent);
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
    for (final img in _imageCache.values) {
      img.dispose();
    }
    _imageCache.clear();
    setRichPasteHandler(null);
    setRichImagePasteHandler(null);
    _stopAutoScroll();
    _blink?.cancel();
    _conn?.close();
    _focus.removeListener(_onFocusChange);
    // Only dispose a focus node we created; an external one is owned by the host.
    if (widget.focusNode == null) _focus.dispose();
    _controller.removeListener(_onControllerChanged);
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

  void _onControllerChanged() {
    // Repaint only. The OS input connection is the source of truth while
    // typing, so we never push editing state back from here — that is done
    // explicitly at the call sites that move the caret programmatically
    // (arrows, click, structural edits, slash apply). Pushing here would echo
    // an `updateEditingValue` back and, e.g., dismiss the slash menu.
    void apply() {
      if (!mounted) return;
      setState(() {});
      _restartBlink();
      _refreshMarkBar();
      _cacheImageUrls();
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
  bool _handleRichPaste(String markdown, String plain, bool rich) {
    if (!mounted || !_focus.hasFocus || !widget.canEdit) return false;

    // Inside a code block, paste raw text verbatim (keep newlines, stay inside).
    final node = _controller.focusedNode;
    if (node != null && node.isCode) {
      final raw = plain.isNotEmpty ? plain : markdown;
      if (raw.isEmpty) return false;
      _controller.insertTextAtCaret(raw);
      _syncImeFromSelection(force: true);
      return true;
    }

    // A bare image URL: re-host it (server-side fetch) so the link can't rot,
    // then insert an image block instead of pasting the raw link text.
    final trimmed = plain.trim();
    if (!rich && widget.onImportImageUrl != null && _looksLikeImageUrl(trimmed)) {
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

    if (markdown.trim().isEmpty) return false;
    if (rich || markdown.contains('\n')) {
      _controller.insertBlocksAfterFocus(markdownToBlocks(markdown));
      _rehostExternalImages();
      _syncImeFromSelection(force: true);
      return true;
    }
    return false;
  }

  static final RegExp _bareUrlRe =
      RegExp(r'^https?://\S+$', caseSensitive: false);

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
    if (result == null || !mounted) return;
    _controller.insertImage(fileId: result.fileId, name: result.name);
    _syncImeFromSelection(force: true);
  }

  /// Re-host any image blocks that still reference an external `url` (e.g. from
  /// pasted/AI Markdown) into our own storage, so they render on the canvas and
  /// can't rot. Runs in the background; each conversion repaints when done.
  void _rehostExternalImages() {
    if (!widget.reHostImages) return;
    final import = widget.onImportImageUrl;
    if (import == null) return;
    for (final node in [..._controller.nodes]) {
      if (node.kind != 'image') continue;
      final url = node.data['url'] as String?;
      if (node.data['file_id'] != null || url == null || !url.startsWith('http')) {
        continue;
      }
      final id = node.id;
      import(url).then((result) {
        if (result == null || !mounted) return;
        _controller.setImageSource(id, fileId: result.fileId, name: result.name);
      });
    }
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
    if (_conn != null && _conn!.attached) {
      _syncImeFromSelection();
      return;
    }
    _conn = TextInput.attach(
      this,
      const TextInputConfiguration(
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
    final rect = _render?.caretRectFor(_controller.selection!.focus);
    if (rect != null) conn.setCaretRect(rect);
  }

  // TextInputClient ----------------------------------------------------------

  @override
  TextEditingValue get currentTextEditingValue => _imeValue();

  @override
  AutofillScope? get currentAutofillScope => null;

  @override
  void updateEditingValue(TextEditingValue value) {
    final node = _controller.focusedNode;
    if (node == null) return;
    final shift = HardwareKeyboard.instance.isShiftPressed;
    final text = value.text;

    // A multi-line chunk arriving at once is a paste: parse it as Markdown into
    // structured blocks (headings, lists, code, …) instead of one literal block.
    if (!node.isCode &&
        !shift &&
        ('\n'.allMatches(text).length >= 2 || text.contains('\n\n'))) {
      _controller.replaceFocusedWithBlocks(markdownToBlocks(text));
      _rehostExternalImages();
      _lastSentIme = _imeValue();
      _syncImeFromSelection(force: true);
      return;
    }

    // A single newline in a normal block means "split here" (Enter). Code blocks
    // and Shift+Enter keep the newline as a soft break inside the node.
    if (text.contains('\n') && !node.isCode && !shift) {
      final idx = text.indexOf('\n');
      _controller.applyNewlineSplit(text.substring(0, idx), text.substring(idx + 1));
      _lastSentIme = _imeValue();
      _syncImeFromSelection(force: true);
      return;
    }

    final base = value.selection.baseOffset;
    final ext = value.selection.extentOffset;
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
  }

  @override
  void didChangeInputControl(TextInputControl? oldControl, TextInputControl? newControl) {}

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

    if (accel && key == LogicalKeyboardKey.keyC) {
      final text = _controller.selectionText(imageUrls: _imageUrlCache);
      if (text.isEmpty) return KeyEventResult.ignored;
      copyTextToClipboard(text).then((_) {
        if (mounted) _focus.requestFocus();
      });
      return KeyEventResult.handled;
    }

    if (accel && key == LogicalKeyboardKey.keyX) {
      final text = _controller.selectionText(imageUrls: _imageUrlCache);
      if (text.isEmpty) return KeyEventResult.ignored;
      copyTextToClipboard(text).then((_) {
        if (!mounted) return;
        _focus.requestFocus();
        _controller.deleteSelection();
        _syncImeFromSelection();
      });
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
      // Within-node delete is done by the OS input; keep it from the app's
      // shortcuts but let the platform text field act on it.
      return KeyEventResult.skipRemainingHandlers;
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
      return KeyEventResult.skipRemainingHandlers;
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
    if (event.character != null && event.character!.isNotEmpty) {
      return KeyEventResult.skipRemainingHandlers;
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
      final wholeNode = sel.start.node == i &&
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
    final mathIdx = r.blockAt(local);
    if (mathIdx != null &&
        mathIdx < _controller.nodes.length &&
        _controller.nodes[mathIdx].kind == 'math_block') {
      _editMathBlock(_controller.nodes[mathIdx]);
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
      _openRowMenu(rowHandle.node, rowHandle.row, d.globalPosition);
      return;
    }
    final colHandle = r.tableColHandleAt(local);
    if (colHandle != null) {
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
    final cb = r.checkboxAt(local);
    if (cb != null) {
      _controller.toggleTodo(cb);
      return;
    }
    if (local.dy > r.contentBottom) {
      _controller.appendOrFocusLast();
    } else {
      _controller.collapseTo(r.positionAt(local));
    }
    _syncImeFromSelection();
  }

  void _closeCellEditor() {
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
    final localRect = r.tableCellRect(node, row, col);
    if (localRect == null) return;
    final table = TableData.fromBlock(_controller.nodes[node].data);
    final text = (row < table.rows.length && col < table.rows[row].length)
        ? table.rows[row][col]
        : '';
    final topLeft = r.localToGlobal(localRect.topLeft);
    final controller = TextEditingController(text: text);
    final focus = FocusNode();
    var committed = false;
    late OverlayEntry entry;

    void commit() {
      if (committed) return;
      committed = true;
      _render?.editingCell = null;
      _controller.setTableCell(node, row, col, controller.text);
      controller.dispose();
      focus.removeListener(_cellFocusListener!);
      focus.dispose();
      entry.remove();
      if (identical(_cellEntry, entry)) _cellEntry = null;
      _commitCellEditor = null;
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
      if (key == LogicalKeyboardKey.arrowDown && row + 1 < rows) {
        moveTo(row + 1, col);
        return KeyEventResult.handled;
      }
      if (key == LogicalKeyboardKey.arrowUp && row > 0) {
        moveTo(row - 1, col);
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
      builder: (context) => Positioned(
        left: topLeft.dx,
        top: topLeft.dy,
        width: localRect.width,
        child: Focus(
          onKeyEvent: onKey,
          child: Container(
            constraints: BoxConstraints(minHeight: localRect.height),
            color: Colors.transparent,
            padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 7),
            child: TextField(
              controller: controller,
              focusNode: focus,
              autofocus: true,
              maxLines: null,
              cursorColor: const Color(0xFF2563EB),
              style: const TextStyle(fontSize: 15, height: 1.4),
              decoration: const InputDecoration(
                isDense: true,
                contentPadding: EdgeInsets.zero,
                border: InputBorder.none,
                isCollapsed: true,
              ),
              onSubmitted: (_) => commit(),
            ),
          ),
        ),
      ),
    );
    _cellEntry = entry;
    _render?.editingCell = (node: node, row: row, col: col);
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
    // An external url that we're about to re-host: don't CORS-fetch it (it will
    // become a file_id shortly). When re-hosting is off, fetch it best-effort.
    final isUrl = fileId.startsWith('http://') || fileId.startsWith('https://');
    if (isUrl && widget.reHostImages) return;
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
        final codec = await ui.instantiateImageCodec(bytes);
        final frame = await codec.getNextFrame();
        if (!mounted) {
          frame.image.dispose();
          return;
        }
        _imageLoading.remove(fileId);
        setState(() => _imageCache[fileId] = frame.image);
      } catch (_) {
        if (!mounted) return;
        _imageLoading.remove(fileId);
        setState(() => _imageErrors.add(fileId));
      }
    });
  }

  /// Resolve fresh download URLs for any image file_ids not yet cached, so copy
  /// can emit working Markdown links synchronously (no gesture-breaking await).
  void _cacheImageUrls() {
    final resolve = widget.onResolveImageUrls;
    if (resolve == null) return;
    final ids = <String>[];
    for (final n in _controller.nodes) {
      if (n.kind != 'image') continue;
      final id = n.data['file_id'] as String?;
      if (id != null && !_imageUrlCache.containsKey(id)) ids.add(id);
    }
    if (ids.isEmpty) return;
    resolve(ids).then((map) {
      if (mounted) _imageUrlCache.addAll(map);
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
    if (sel == null || sel.isCollapsed || sel.isMultiNode || node == null) return;
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
    // Show on any ranged selection: inline marks for a single text block, plus
    // block-type conversion (list/quote/heading/code) for any selection.
    final show = _focus.hasFocus &&
        widget.canEdit &&
        sel != null &&
        !sel.isCollapsed &&
        _render?.caretRectFor(sel.start) != null;
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

  Widget _buildMarkBar(BuildContext context) {
    final r = _render;
    final sel = _controller.selection;
    if (r == null || sel == null) return const SizedBox.shrink();
    final rect = r.caretRectFor(sel.start);
    if (rect == null) return const SizedBox.shrink();
    final origin = r.localToGlobal(rect.topLeft);
    final screen = MediaQuery.of(context).size;
    final left = origin.dx.clamp(8.0, screen.width - 420);
    final top = (origin.dy - 44).clamp(8.0, screen.height - 8);

    final singleText = !sel.isMultiNode &&
        _controller.focusedNode != null &&
        _controller.focusedNode!.kind != 'code_block' &&
        _controller.focusedNode!.kind != 'table';

    Widget markBtn(IconData icon, String type, String tip, {VoidCallback? custom}) {
      return IconButton(
        iconSize: 18,
        visualDensity: VisualDensity.compact,
        tooltip: tip,
        icon: Icon(icon, color: EditorTheme.text),
        onPressed: custom ?? () => _controller.toggleMark(type),
      );
    }

    Widget blockBtn(IconData icon, String kind, String tip,
        {Map<String, dynamic>? data}) {
      return IconButton(
        iconSize: 18,
        visualDensity: VisualDensity.compact,
        tooltip: tip,
        icon: Icon(icon, color: EditorTheme.muted),
        onPressed: () => _controller.setSelectedBlocksKind(kind, data: data),
      );
    }

    return Positioned(
      left: left,
      top: top,
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
                  markBtn(Icons.code, 'code', 'Inline code'),
                  markBtn(Icons.strikethrough_s, 'strike', 'Strikethrough'),
                  markBtn(Icons.link, 'link', 'Link', custom: _promptLink),
                  const VerticalDivider(width: 9, indent: 8, endIndent: 8),
                ],
                blockBtn(Icons.notes, 'paragraph', 'Text'),
                blockBtn(Icons.title, 'heading', 'Heading', data: {'level': 2}),
                blockBtn(Icons.format_list_bulleted, 'bulleted_list', 'Bulleted list'),
                blockBtn(Icons.format_list_numbered, 'numbered_list', 'Numbered list'),
                blockBtn(Icons.check_box_outlined, 'todo', 'To-do', data: {'checked': false}),
                blockBtn(Icons.format_quote, 'quote', 'Quote'),
                blockBtn(Icons.terminal, 'code_block', 'Code block'),
              ],
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
      ('copy', 'Copy table'),
      ('delete', 'Delete table'),
    ]);
    switch (selected) {
      case 'left':
      case 'center':
      case 'right':
        _controller.setTableAlign(node, selected!);
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
    final overlay = Overlay.of(context).context.findRenderObject() as RenderBox?;
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
      final same = _linkBar != null &&
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
    if (r.tableColBorderAt(local) != null || r.imageResizeAt(local) != null) {
      return SystemMouseCursors.resizeLeftRight;
    }
    final clickable = r.codeLanguageAt(local) != null ||
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
    if (event is! PointerScrollEvent) return;
    final r = _render;
    if (r == null) return;
    final shift = HardwareKeyboard.instance.isShiftPressed;
    final dx = event.scrollDelta.dx != 0
        ? event.scrollDelta.dx
        : (shift ? event.scrollDelta.dy : 0.0);
    if (dx == 0) return;
    r.scrollCodeAt(r.globalToLocal(event.position), dx);
  }

  Future<void> _openLanguageMenu(int nodeIndex, Offset globalPosition) async {
    final selected = await showDialog<String>(
      context: context,
      barrierColor: Colors.transparent,
      builder: (context) => _LanguagePicker(anchor: globalPosition),
    );
    if (selected != null) {
      _controller.setCodeLanguage(nodeIndex, selected);
    }
  }

  void _onPanStart(DragStartDetails d) {
    if (!widget.canEdit) return;
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
      _colResize = (
        node: colBorder.node,
        col: colBorder.col,
        startX: local.dx,
        weights: [...r.tableWeights(colBorder.node)],
        avail: r.tableAvailWidth(),
        frac: r.tableWidthFraction(colBorder.node),
      );
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
    _focus.requestFocus();
    final p = r.positionAt(local);
    _dragAnchor = p;
    _controller.setSelection(DocSelection.collapsed(p));
  }

  void _onPanUpdate(DragUpdateDetails d) {
    final r = _render;
    if (r == null) return;
    final local = r.globalToLocal(d.globalPosition);
    if (_blockDrag != null) {
      r.setDropIndicator(r.dropIndexAt(local.dy));
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
    final target = (pos.pixels + delta).clamp(pos.minScrollExtent, pos.maxScrollExtent);
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
    final query = before.substring(slash + 1);
    if (query.contains(' ')) {
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
    _controller.applySlashCommand(_slashStart, caret, opt.kind, data);
    _closeSlash();
    _syncImeFromSelection(force: true);
    if (opt.kind == 'math_block') {
      // Straight into source editing — an empty formula shows nothing.
      final node = _controller.focusedNode;
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
            style: const TextStyle(fontFamily: 'monospace', fontSize: 14),
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
      _controller.setBlockText(node.id, source.trim());
    }
  }

  /// Right-click on an image → context menu (copy / download / delete).
  void _onSecondaryTapDown(TapDownDetails d) {
    if (!widget.canEdit) return;
    final r = _render;
    if (r == null) return;
    final node = r.imageAt(r.globalToLocal(d.globalPosition));
    if (node == null) return;
    _focus.requestFocus();
    _controller.collapseTo(DocPosition(node, 0)); // select the block
    _showImageMenu(node, d.globalPosition);
  }

  Future<void> _showImageMenu(int node, Offset globalPosition) async {
    final overlay = Overlay.of(context).context.findRenderObject() as RenderBox?;
    if (overlay == null) return;
    final data = _controller.nodes[node].data;
    final isExternal =
        data['file_id'] == null && (data['url'] as String?)?.startsWith('http') == true;
    final choice = await showMenu<String>(
      context: context,
      position: RelativeRect.fromLTRB(
        globalPosition.dx,
        globalPosition.dy,
        overlay.size.width - globalPosition.dx,
        overlay.size.height - globalPosition.dy,
      ),
      items: [
        if (isExternal)
          const PopupMenuItem(value: 'rehost', child: Text('Save to Mica storage')),
        const PopupMenuItem(value: 'copy', child: Text('Copy image')),
        const PopupMenuItem(value: 'download', child: Text('Download')),
        const PopupMenuItem(value: 'delete', child: Text('Delete')),
      ],
    );
    if (choice == null || !mounted) return;
    switch (choice) {
      case 'rehost':
        await _rehostImage(node);
      case 'copy':
        await _copyImage(node);
      case 'download':
        await _downloadImage(node);
      case 'delete':
        _controller.deleteNode(node);
        _syncImeFromSelection(force: true);
    }
  }

  /// Re-host one external image into Mica storage (right-click action).
  Future<void> _rehostImage(int node) async {
    final import = widget.onImportImageUrl;
    if (import == null || node < 0 || node >= _controller.nodes.length) return;
    final url = _controller.nodes[node].data['url'] as String?;
    if (url == null) return;
    final id = _controller.nodes[node].id;
    final result = await import(url);
    if (result == null || !mounted) {
      _toast('Could not save the image');
      return;
    }
    _controller.setImageSource(id, fileId: result.fileId, name: result.name);
  }

  /// Bytes + filename for an image node (re-fetched via the host loader).
  Future<({Uint8List bytes, String name, String mime})?> _imageData(int node) async {
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

  Future<void> _copyImage(int node) async {
    final data = await _imageData(node);
    if (data == null || !mounted) {
      _toast('Could not load the image');
      return;
    }
    final ok = await copyImageToClipboard(data.bytes, data.mime);
    if (!mounted) return;
    _toast(ok ? 'Image copied' : 'Copy failed — try Download instead');
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
    final image = key == null ? null : _imageCache[key];
    if (image == null) return;
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
              child: Center(child: RawImage(image: image)),
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
  Future<void> _handlePasteImage(Uint8List bytes, String mime, String name) async {
    final upload = widget.onUploadImage;
    if (upload == null || !mounted || !_focus.hasFocus || !widget.canEdit) return;
    final result = await upload(bytes, name, mime);
    if (result == null || !mounted) return;
    _primeImage(result.fileId, bytes);
    _controller.insertImage(fileId: result.fileId, name: result.name);
    _syncImeFromSelection(force: true);
  }

  /// Seed the canvas image cache with freshly-uploaded bytes (skip the fetch).
  void _primeImage(String fileId, Uint8List bytes) {
    if (_imageCache.containsKey(fileId)) return;
    ui.instantiateImageCodec(bytes).then((codec) => codec.getNextFrame()).then((
      frame,
    ) {
      if (!mounted) {
        frame.image.dispose();
        return;
      }
      _imageErrors.remove(fileId);
      setState(() => _imageCache[fileId] = frame.image);
    }).catchError((_) {});
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
                  padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
                  child: Row(
                    children: [
                      Icon(o.icon, size: 18, color: const Color(0xFF475569)),
                      const SizedBox(width: 10),
                      Text(
                        o.label,
                        style: const TextStyle(fontSize: 14, color: Color(0xFF0F172A)),
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
            onPanStart: _onPanStart,
            onPanUpdate: _onPanUpdate,
            onPanEnd: _onPanEnd,
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
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
                  mathImages: _mathCache,
                  onRequestMath: _requestMath,
                  onRequestImage: _requestImage,
                ),
                _mathRasterHost(),
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
    _sub = widget.onStream(prompt).listen(
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
                  style: const TextStyle(color: Color(0xFFB91C1C), fontSize: 13),
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
  const _LanguagePicker({required this.anchor});

  final Offset anchor;

  @override
  State<_LanguagePicker> createState() => _LanguagePickerState();
}

class _LanguagePickerState extends State<_LanguagePicker> {
  final TextEditingController _query = TextEditingController();
  final ScrollController _scroll = ScrollController();
  int _index = 0;

  List<String> get _filtered {
    final q = _query.text.trim().toLowerCase();
    if (q.isEmpty) return kCodeLanguages;
    return [
      for (final l in kCodeLanguages)
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
    final target = (_index * itemH).clamp(0.0, _scroll.position.maxScrollExtent);
    final top = _scroll.offset;
    final bottom = top + 240 - itemH;
    if (target < top || target > bottom) {
      _scroll.jumpTo((target - 100).clamp(0.0, _scroll.position.maxScrollExtent));
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
                                  child: Text(
                                    items[i],
                                    style: const TextStyle(fontSize: 13),
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
