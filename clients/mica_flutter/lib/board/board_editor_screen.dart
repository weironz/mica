// Fullscreen Feishu-Board-inspired interactive Whiteboard + Mind Map + Flowchart
// editor screen. Loaded via Dart `deferred as board` (0 B Web cold-start cost).
import 'dart:convert';
import 'dart:ui' as ui;

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'board_bridge.dart';
import 'board_chrome.dart';
import 'board_controller.dart';
import 'board_file.dart';
import 'board_icons.dart';
import 'board_models.dart';
import 'board_painter.dart';

/// Open the fullscreen interactive board editor and return the updated compact
/// JSON string when the user clicks Done/Close (or null if unchanged).
Future<String?> showBoardEditorDialog(
  BuildContext context, {
  required String initialJson,
  bool readOnly = false,
}) {
  return showGeneralDialog<String>(
    context: context,
    barrierDismissible: false,
    barrierLabel: 'Board Editor',
    barrierColor: Colors.black54,
    transitionDuration: const Duration(milliseconds: 160),
    pageBuilder: (ctx, _, _) => BoardEditorScreen(
      initialJson: initialJson,
      readOnly: readOnly,
    ),
  );
}

class BoardEditorScreen extends StatefulWidget {
  const BoardEditorScreen({
    super.key,
    required this.initialJson,
    this.readOnly = false,
  });

  final String initialJson;
  final bool readOnly;

  @override
  State<BoardEditorScreen> createState() => _BoardEditorScreenState();
}

class _BoardEditorScreenState extends State<BoardEditorScreen> {
  late final BoardController _ctrl;
  final FocusNode _canvasFocus = FocusNode();
  final TextEditingController _editController = TextEditingController();
  final FocusNode _editFocus = FocusNode();
  final TextEditingController _titleController = TextEditingController();

  String? _boundEditNodeId;
  ui.Picture? _staticPicture;
  int _staticPictureGraphRev = -1;
  bool _staticPictureDark = false;

  bool _showShapePanel = false;
  bool _showIconPanel = false;
  bool _spacePressed = false;
  bool _panning = false;
  Offset? _marqueeStartWorld;
  Offset _lastPointer = Offset.zero;
  DateTime? _lastTapTime;
  String? _lastTapNodeId;
  Size _viewportSize = const Size(1280, 800);
  bool _didInitialFit = false;

  @override
  void initState() {
    super.initState();
    final snap =
        BoardSnapshot.tryParseJson(widget.initialJson) ??
        BoardSnapshot.starter('whiteboard');
    _ctrl = BoardController(snap);
    _titleController.text = snap.title;
    _ctrl.addListener(_onCtrlChanged);
    _editController.addListener(_onEditTextChanged);
    // Mandatory spike-verified rule: invalidate cached TextPainters & ui.Picture
    // when async Web CanvasKit CJK font slices arrive.
    PaintingBinding.instance.systemFonts.addListener(_onSystemFontsChanged);
    _refreshRustRoutes();
  }

  @override
  void dispose() {
    PaintingBinding.instance.systemFonts.removeListener(_onSystemFontsChanged);
    _ctrl.removeListener(_onCtrlChanged);
    _editController.removeListener(_onEditTextChanged);
    _ctrl.dispose();
    _canvasFocus.dispose();
    _editController.dispose();
    _editFocus.dispose();
    _titleController.dispose();
    _staticPicture?.dispose();
    super.dispose();
  }

  void _onSystemFontsChanged() {
    _ctrl.invalidateTextCaches();
  }

  void _onCtrlChanged() {
    final editId = _ctrl.editingNodeId;
    if (editId != _boundEditNodeId) {
      _boundEditNodeId = editId;
      if (editId != null) {
        final node = _ctrl.nodeById[editId];
        if (node != null) {
          _editController.value = TextEditingValue(
            text: node.text,
            selection: TextSelection(
              baseOffset: 0,
              extentOffset: node.text.length,
            ),
          );
          WidgetsBinding.instance.addPostFrameCallback((_) {
            if (mounted && _ctrl.editingNodeId == editId) {
              _editFocus.requestFocus();
            }
          });
        }
      } else {
        _editFocus.unfocus();
        _canvasFocus.requestFocus();
      }
    }
    if (mounted) setState(() {});
  }

  void _onEditTextChanged() {
    final id = _ctrl.editingNodeId;
    if (id == null) return;
    _ctrl.updateEditingNodeText(id, _editController.text);
  }

  Future<void> _refreshRustRoutes() async {
    final hasOrtho = _ctrl.snapshot.edges.any((e) => e.routing == 'orthogonal');
    if (!hasOrtho) return;
    final json = _ctrl.snapshot.toCompactJson();
    final raw = await boardRouteEdgesJson(json);
    if (!mounted || raw == null) return;
    try {
      final decoded = jsonDecode(raw) as Map<String, dynamic>;
      final routes = <String, List<Offset>>{};
      for (final entry in decoded.entries) {
        final list = entry.value as List;
        routes[entry.key] = [
          for (final p in list)
            Offset((p[0] as num).toDouble(), (p[1] as num).toDouble()),
        ];
      }
      _ctrl.applyRoutedEdges(routes);
    } catch (_) {}
  }

  ui.Picture _ensureStaticPicture(bool dark) {
    if (_staticPicture != null &&
        _staticPictureGraphRev == _ctrl.graphRevision &&
        _staticPictureDark == dark) {
      return _staticPicture!;
    }
    _staticPicture?.dispose();
    _staticPicture = recordStaticBoardPicture(_ctrl, dark: dark);
    _staticPictureGraphRev = _ctrl.graphRevision;
    _staticPictureDark = dark;
    return _staticPicture!;
  }

  // --- Pointer Events ---

  void _onPointerDown(PointerDownEvent event) {
    if (_showShapePanel || _showIconPanel) {
      setState(() {
        _showShapePanel = false;
        _showIconPanel = false;
      });
    }
    _lastPointer = event.localPosition;
    final world = _ctrl.screenToWorld(event.localPosition);

    // Middle mouse button or Hand tool or Spacebar = pan.
    if (event.buttons == kMiddleMouseButton ||
        _ctrl.tool == BoardTool.hand ||
        _spacePressed ||
        widget.readOnly) {
      _panning = true;
      return;
    }
    if (event.buttons != kPrimaryMouseButton) return;

    // Close active inline editor when clicking elsewhere.
    if (_ctrl.editingNodeId != null) {
      final editing = _ctrl.nodeById[_ctrl.editingNodeId!];
      if (editing != null && editing.rect.contains(world)) {
        return;
      }
      _ctrl.finishEditing();
      _refreshRustRoutes();
    }

    // Creation tools.
    if (_ctrl.tool == BoardTool.pen) {
      _ctrl.beginPenStroke(world);
      return;
    }
    if (_ctrl.tool == BoardTool.shape) {
      final node = _ctrl.addNodeAt(world, kind: _ctrl.activeShapeKind);
      _ctrl.startEditingNode(node.id);
      return;
    }
    if (_ctrl.tool == BoardTool.icon) {
      final meta = kBoardTechIcons.firstWhere(
        (m) => m.id == _ctrl.activeIconId,
        orElse: () => kBoardTechIcons.first,
      );
      final node = _ctrl.addNodeAt(
        world,
        kind: 'icon_card',
        icon: meta.id,
        text: meta.label,
        fill: meta.defaultFill,
        stroke: meta.defaultStroke,
      );
      _ctrl.startEditingNode(node.id);
      return;
    }
    if (_ctrl.tool == BoardTool.sticky) {
      final node = _ctrl.addNodeAt(world, kind: 'sticky');
      _ctrl.startEditingNode(node.id);
      return;
    }
    if (_ctrl.tool == BoardTool.text) {
      final node = _ctrl.addNodeAt(world, kind: 'text');
      _ctrl.startEditingNode(node.id);
      return;
    }
    if (_ctrl.tool == BoardTool.frame) {
      _ctrl.addNodeAt(world, kind: 'frame');
      return;
    }
    if (_ctrl.tool == BoardTool.mindmap) {
      final root = _ctrl.addNodeAt(world, kind: 'mind_root');
      _ctrl.addMindChild(root.id);
      _ctrl.addMindChild(root.id);
      _ctrl.selectNode(root.id);
      return;
    }

    // 1. Mind-map collapse badge click.
    final collapseId = _ctrl.hitTestCollapseBadge(world);
    if (collapseId != null) {
      _ctrl.toggleCollapse(collapseId);
      return;
    }

    // 2. Connector port drag (works in both Select & Connector modes).
    final portHit = _ctrl.hitTestAnchorPort(world);
    if (portHit != null) {
      _ctrl.beginConnectorDrag(portHit.$1, portHit.$2, world);
      return;
    }

    // 3. Resize handle drag.
    final resizeHit = _ctrl.hitTestResizeHandle(world);
    if (resizeHit != null) {
      _ctrl.beginResize(resizeHit.$1, resizeHit.$2);
      return;
    }

    // 4. Hit node.
    final nodeHit = _ctrl.hitTestNode(world);
    if (nodeHit != null) {
      if (_ctrl.tool == BoardTool.connector) {
        _ctrl.beginConnectorDrag(nodeHit.id, 'r', world);
        return;
      }
      final now = DateTime.now();
      final isDouble =
          _lastTapNodeId == nodeHit.id &&
          _lastTapTime != null &&
          now.difference(_lastTapTime!).inMilliseconds < 420;
      _lastTapNodeId = nodeHit.id;
      _lastTapTime = now;

      if (isDouble) {
        _ctrl.startEditingNode(nodeHit.id);
        return;
      }
      final shift = HardwareKeyboard.instance.isShiftPressed;
      _ctrl.beginNodeDrag(nodeHit.id, additive: shift);
      return;
    }

    // 5. Hit edge.
    final edgeHit = _ctrl.hitTestEdge(world);
    if (edgeHit != null) {
      final now = DateTime.now();
      final isDouble =
          _lastTapNodeId == edgeHit.id &&
          _lastTapTime != null &&
          now.difference(_lastTapTime!).inMilliseconds < 420;
      _lastTapNodeId = edgeHit.id;
      _lastTapTime = now;
      _ctrl.selectEdge(edgeHit.id);
      if (isDouble) {
        _promptEditEdgeLabel(edgeHit);
      }
      return;
    }

    // 6. Empty canvas -> rubber-band marquee selection.
    _ctrl.clearSelection();
    _marqueeStartWorld = world;
  }

  void _onPointerMove(PointerMoveEvent event) {
    final delta = event.localPosition - _lastPointer;
    _lastPointer = event.localPosition;
    final world = _ctrl.screenToWorld(event.localPosition);

    if (_panning) {
      _ctrl.setViewport(_ctrl.zoom, _ctrl.pan + delta);
      return;
    }
    if (_ctrl.activeStroke != null) {
      _ctrl.updatePenStroke(world);
      return;
    }
    if (_ctrl.pendingConnector != null) {
      _ctrl.updateConnectorDrag(world);
      return;
    }
    if (_ctrl.resizingNodeId != null) {
      _ctrl.updateResize(delta / _ctrl.zoom);
      return;
    }
    if (_ctrl.isDraggingNodes) {
      _ctrl.updateNodeDrag(
        delta / _ctrl.zoom,
        snap: !HardwareKeyboard.instance.isAltPressed,
      );
      return;
    }
    if (_marqueeStartWorld != null) {
      _ctrl.updateMarquee(_marqueeStartWorld!, world);
    }
  }

  void _onPointerUp(PointerEvent event) {
    if (_panning) _panning = false;
    if (_ctrl.activeStroke != null) _ctrl.endPenStroke();
    if (_ctrl.pendingConnector != null) {
      _ctrl.endConnectorDrag();
      _refreshRustRoutes();
    }
    if (_ctrl.resizingNodeId != null) {
      _ctrl.endResize();
      _refreshRustRoutes();
    }
    if (_ctrl.isDraggingNodes) {
      _ctrl.endNodeDrag();
      _refreshRustRoutes();
    }
    if (_marqueeStartWorld != null) {
      _marqueeStartWorld = null;
      _ctrl.endMarquee();
    }
  }

  void _onPointerHover(PointerHoverEvent event) {
    if (_ctrl.isDraggingNodes || _panning) return;
    final world = _ctrl.screenToWorld(event.localPosition);
    final hit = _ctrl.hitTestNode(world);
    _ctrl.setHoveredNode(hit?.id);
  }

  void _onPointerSignal(PointerSignalEvent event) {
    if (event is PointerScrollEvent) {
      final factor = event.scrollDelta.dy < 0 ? 1.12 : 1 / 1.12;
      _ctrl.zoomAround(factor, event.localPosition);
    }
  }

  // --- Keyboard Shortcuts ---

  KeyEventResult _handleCanvasKey(FocusNode node, KeyEvent event) {
    if (event.logicalKey == LogicalKeyboardKey.space) {
      if (event is KeyDownEvent) {
        _spacePressed = true;
      } else if (event is KeyUpEvent) {
        _spacePressed = false;
      }
      return KeyEventResult.handled;
    }
    if (event is! KeyDownEvent) return KeyEventResult.ignored;
    if (_ctrl.editingNodeId != null) return KeyEventResult.ignored;

    final ctrlOrCmd =
        HardwareKeyboard.instance.isControlPressed ||
        HardwareKeyboard.instance.isMetaPressed;
    final shift = HardwareKeyboard.instance.isShiftPressed;
    final key = event.logicalKey;

    if (ctrlOrCmd) {
      if (key == LogicalKeyboardKey.keyZ) {
        if (shift) {
          _ctrl.redo();
        } else {
          _ctrl.undo();
        }
        return KeyEventResult.handled;
      }
      if (key == LogicalKeyboardKey.keyY) {
        _ctrl.redo();
        return KeyEventResult.handled;
      }
      if (key == LogicalKeyboardKey.keyC) {
        _ctrl.copySelection();
        return KeyEventResult.handled;
      }
      if (key == LogicalKeyboardKey.keyV) {
        _ctrl.pasteSelection();
        return KeyEventResult.handled;
      }
      if (key == LogicalKeyboardKey.keyD) {
        _ctrl.duplicateSelection();
        return KeyEventResult.handled;
      }
      if (key == LogicalKeyboardKey.keyA) {
        _ctrl.selectAll();
        return KeyEventResult.handled;
      }
      return KeyEventResult.ignored;
    }

    if (widget.readOnly) {
      if (key == LogicalKeyboardKey.escape) {
        _closeAndSave();
        return KeyEventResult.handled;
      }
      return KeyEventResult.ignored;
    }

    // Mind map Tab / Enter keyboard workflow when a single node is selected.
    if (key == LogicalKeyboardKey.tab && _ctrl.selectedNodeIds.length == 1) {
      final child = _ctrl.addMindChild(_ctrl.selectedNodeIds.first);
      if (child != null) _ctrl.startEditingNode(child.id);
      return KeyEventResult.handled;
    }
    if (key == LogicalKeyboardKey.enter && _ctrl.selectedNodeIds.length == 1) {
      final sel = _ctrl.nodeById[_ctrl.selectedNodeIds.first];
      if (sel != null && (sel.kind == 'mind_root' || sel.kind == 'mind_node')) {
        final sib = _ctrl.addMindSibling(sel.id);
        if (sib != null) _ctrl.startEditingNode(sib.id);
      } else if (sel != null) {
        _ctrl.startEditingNode(sel.id);
      }
      return KeyEventResult.handled;
    }
    if (key == LogicalKeyboardKey.delete ||
        key == LogicalKeyboardKey.backspace) {
      _ctrl.deleteSelection();
      return KeyEventResult.handled;
    }
    if (key == LogicalKeyboardKey.escape) {
      if (_ctrl.selectedNodeIds.isNotEmpty || _ctrl.selectedEdgeId != null) {
        _ctrl.clearSelection();
      } else if (_ctrl.tool != BoardTool.select) {
        _ctrl.setTool(BoardTool.select);
      } else {
        _closeAndSave();
      }
      return KeyEventResult.handled;
    }

    // Single-key tool shortcuts.
    switch (key) {
      case LogicalKeyboardKey.keyV:
        _ctrl.setTool(BoardTool.select);
        return KeyEventResult.handled;
      case LogicalKeyboardKey.keyH:
        _ctrl.setTool(BoardTool.hand);
        return KeyEventResult.handled;
      case LogicalKeyboardKey.keyS:
        _ctrl.setTool(BoardTool.sticky);
        return KeyEventResult.handled;
      case LogicalKeyboardKey.keyT:
        _ctrl.setTool(BoardTool.text);
        return KeyEventResult.handled;
      case LogicalKeyboardKey.keyL:
        _ctrl.setTool(BoardTool.connector);
        return KeyEventResult.handled;
      case LogicalKeyboardKey.keyP:
        _ctrl.setTool(BoardTool.pen);
        return KeyEventResult.handled;
      case LogicalKeyboardKey.keyF:
        _ctrl.setTool(BoardTool.frame);
        return KeyEventResult.handled;
      case LogicalKeyboardKey.keyM:
        _ctrl.setTool(BoardTool.mindmap);
        return KeyEventResult.handled;
      default:
        return KeyEventResult.ignored;
    }
  }

  // --- Import / Export Actions ---

  Future<void> _importDrawioOrJson() async {
    final picked = await pickBoardImportFile();
    if (picked == null || !mounted) return;
    final bytes = picked.bytes;
    final name = picked.name.toLowerCase();
    if (name.endsWith('.json')) {
      final snap = BoardSnapshot.tryParseJson(utf8.decode(bytes));
      if (snap != null) {
        _ctrl.replaceSnapshot(snap);
        _titleController.text = snap.title;
        _ctrl.fitToContent(_viewportSize);
        _snack('已导入 Mica 画板（${snap.nodes.length} 节点）');
      } else {
        _snack('JSON 格式无法识别为 Mica 画板');
      }
      return;
    }

    try {
      final importedJson = await boardImportDrawio(bytes);
      if (!mounted) return;
      if (importedJson == null) {
        _snack('当前环境未加载 Rust 画板引擎，无法解析 draw.io');
        return;
      }
      final snap = BoardSnapshot.tryParseJson(importedJson);
      if (snap == null) {
        _snack('导入失败：无法解析画板结构');
        return;
      }
      _ctrl.replaceSnapshot(snap);
      _titleController.text = snap.title;
      _ctrl.fitToContent(_viewportSize);
      await _refreshRustRoutes();
      _snack('成功导入 draw.io（${snap.nodes.length} 个图形，${snap.edges.length} 条连线）');
    } catch (e) {
      if (mounted) _snack('导入 draw.io 失败：$e');
    }
  }

  Future<void> _importMarkdownOutline() async {
    final tc = TextEditingController(
      text: '# 中心主题\n- 核心模块\n  - 数据模型\n  - 渲染引擎\n- 交付验收\n  - 自动化测试\n',
    );
    final md = await showDialog<String>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: const Text('从 Markdown 大纲生成思维导图'),
        content: SizedBox(
          width: 480,
          child: TextField(
            controller: tc,
            maxLines: 10,
            decoration: const InputDecoration(
              hintText: '粘贴 # 标题 或 - 缩进列表...',
              border: OutlineInputBorder(),
            ),
          ),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(ctx),
            child: const Text('取消'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(ctx, tc.text),
            child: const Text('生成导图'),
          ),
        ],
      ),
    );
    tc.dispose();
    if (md == null || md.trim().isEmpty || !mounted) return;
    final json = await boardMarkdownToMindmap(md, title: _ctrl.snapshot.title);
    if (!mounted) return;
    final snap = json != null ? BoardSnapshot.tryParseJson(json) : null;
    if (snap != null) {
      _ctrl.replaceSnapshot(snap);
      _titleController.text = snap.title;
      _ctrl.fitToContent(_viewportSize);
      _snack('已由 Markdown 大纲生成思维导图（${snap.nodes.length} 节点）');
    } else {
      _snack('生成失败');
    }
  }

  Future<void> _exportSvg() async {
    final svg = await boardRenderSvg(_ctrl.snapshot.toCompactJson());
    if (!mounted) return;
    if (svg == null) {
      _snack('当前环境未就绪 SVG 导出引擎');
      return;
    }
    final saved = await saveBoardTextFile(
      suggestedName: '${_ctrl.snapshot.title}.svg',
      extension: 'svg',
      content: svg,
    );
    if (!mounted) return;
    if (saved != null) {
      _snack('已导出 SVG：$saved');
      return;
    }
    await Clipboard.setData(ClipboardData(text: svg));
    if (mounted) _snack('已复制 SVG 源码到剪贴板');
  }

  Future<void> _exportMarkdownOutline() async {
    final md = await boardMindmapToMarkdown(_ctrl.snapshot.toCompactJson());
    if (!mounted) return;
    if (md == null || md.isEmpty) {
      _snack('没有可导出的节点文本');
      return;
    }
    await Clipboard.setData(ClipboardData(text: md));
    if (mounted) _snack('已复制 Markdown 大纲到剪贴板');
  }

  Future<void> _exportJson() async {
    final json = _ctrl.snapshot.toCompactJson();
    final saved = await saveBoardTextFile(
      suggestedName: '${_ctrl.snapshot.title}.mica-board.json',
      extension: 'json',
      content: json,
    );
    if (!mounted) return;
    if (saved != null) {
      _snack('已导出 JSON：$saved');
      return;
    }
    await Clipboard.setData(ClipboardData(text: json));
    if (mounted) _snack('已复制画板 JSON 到剪贴板');
  }

  Future<void> _promptEditEdgeLabel(BoardEdge edge) async {
    final tc = TextEditingController(text: edge.label);
    final next = await showDialog<String>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: const Text('编辑连线文字'),
        content: TextField(
          controller: tc,
          autofocus: true,
          decoration: const InputDecoration(hintText: '例如：是 / 否 / HTTP 请求'),
          onSubmitted: (v) => Navigator.pop(ctx, v),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(ctx),
            child: const Text('取消'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(ctx, tc.text),
            child: const Text('确定'),
          ),
        ],
      ),
    );
    tc.dispose();
    if (next != null && mounted) {
      _ctrl.setEdgeLabel(edge.id, next);
    }
  }

  void _snack(String msg) {
    ScaffoldMessenger.maybeOf(context)?.showSnackBar(
      SnackBar(
        content: Text(msg),
        duration: const Duration(seconds: 2),
        behavior: SnackBarBehavior.floating,
      ),
    );
  }

  void _closeAndSave() {
    _ctrl.finishEditing();
    _ctrl.setTitle(_titleController.text);
    Navigator.of(context).pop(_ctrl.snapshot.toCompactJson());
  }

  @override
  Widget build(BuildContext context) {
    final dark = Theme.of(context).brightness == Brightness.dark;
    final canvasBg = dark ? BoardColors.canvasDark : BoardColors.canvasLight;
    final barBg = dark ? BoardColors.panelDark : Colors.white;
    final barBorder = dark ? BoardColors.borderDark : BoardColors.borderLight;
    final textPrimary = dark ? BoardColors.inkDark : BoardColors.inkLight;
    final textMuted = dark ? BoardColors.mutedDark : BoardColors.mutedLight;
    final editNode = _ctrl.editingNodeId == null
        ? null
        : _ctrl.nodeById[_ctrl.editingNodeId!];

    return Scaffold(
      backgroundColor: canvasBg,
      body: Focus(
        focusNode: _canvasFocus,
        autofocus: true,
        onKeyEvent: _handleCanvasKey,
        child: Column(
          children: [
            // Top bar
            Container(
              height: 50,
              padding: const EdgeInsets.symmetric(horizontal: 14),
              decoration: BoxDecoration(
                color: barBg,
                border: Border(bottom: BorderSide(color: barBorder)),
              ),
              child: Row(
                children: [
                  Icon(
                    _ctrl.snapshot.mode == 'mindmap'
                        ? Icons.account_tree_outlined
                        : Icons.draw_outlined,
                    size: 19,
                    color: BoardColors.brand,
                  ),
                  const SizedBox(width: 8),
                  SizedBox(
                    width: 220,
                    child: TextField(
                      controller: _titleController,
                      readOnly: widget.readOnly,
                      style: TextStyle(
                        fontSize: 14,
                        fontWeight: FontWeight.w600,
                        color: textPrimary,
                      ),
                      decoration: const InputDecoration(
                        isDense: true,
                        border: InputBorder.none,
                        hintText: '未命名画板',
                      ),
                      onSubmitted: _ctrl.setTitle,
                    ),
                  ),
                  const SizedBox(width: 8),
                  if (!widget.readOnly) ...[
                    IconButton(
                      icon: const Icon(Icons.undo, size: 18),
                      tooltip: '撤销 (Ctrl+Z)',
                      onPressed: _ctrl.canUndo ? _ctrl.undo : null,
                    ),
                    IconButton(
                      icon: const Icon(Icons.redo, size: 18),
                      tooltip: '重做 (Ctrl+Shift+Z)',
                      onPressed: _ctrl.canRedo ? _ctrl.redo : null,
                    ),
                    const SizedBox(width: 4),
                    TextButton.icon(
                      icon: const Icon(Icons.auto_fix_high, size: 16),
                      label: const Text('整理导图', style: TextStyle(fontSize: 13)),
                      onPressed: _ctrl.relayoutMindmap,
                    ),
                    PopupMenuButton<String>(
                      tooltip: '导入',
                      onSelected: (v) {
                        if (v == 'drawio') _importDrawioOrJson();
                        if (v == 'md') _importMarkdownOutline();
                      },
                      itemBuilder: (_) => const [
                        PopupMenuItem(
                          value: 'drawio',
                          child: Text('导入 draw.io (.drawio/.xml/.svg/.png) 或 JSON'),
                        ),
                        PopupMenuItem(
                          value: 'md',
                          child: Text('从 Markdown 大纲生成思维导图'),
                        ),
                      ],
                      child: Padding(
                        padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
                        child: Row(
                          mainAxisSize: MainAxisSize.min,
                          children: [
                            Icon(Icons.file_upload_outlined, size: 16, color: textPrimary),
                            const SizedBox(width: 4),
                            Text('导入', style: TextStyle(fontSize: 13, color: textPrimary)),
                          ],
                        ),
                      ),
                    ),
                  ],
                  PopupMenuButton<String>(
                    tooltip: '导出',
                    onSelected: (v) {
                      if (v == 'svg') _exportSvg();
                      if (v == 'md') _exportMarkdownOutline();
                      if (v == 'json') _exportJson();
                    },
                    itemBuilder: (_) => const [
                      PopupMenuItem(value: 'svg', child: Text('导出高清矢量 SVG')),
                      PopupMenuItem(value: 'md', child: Text('复制为 Markdown 大纲')),
                      PopupMenuItem(value: 'json', child: Text('导出 .mica-board.json')),
                    ],
                    child: Padding(
                      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
                      child: Row(
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          Icon(Icons.file_download_outlined, size: 16, color: textPrimary),
                          const SizedBox(width: 4),
                          Text('导出', style: TextStyle(fontSize: 13, color: textPrimary)),
                        ],
                      ),
                    ),
                  ),
                  const Spacer(),
                  Text(
                    '${_ctrl.snapshot.nodes.length} 图形 · ${_ctrl.snapshot.edges.length} 连线',
                    style: TextStyle(
                      fontSize: 12,
                      color: textMuted,
                    ),
                  ),
                  const SizedBox(width: 12),
                  FilledButton(
                    onPressed: _closeAndSave,
                    style: FilledButton.styleFrom(
                      backgroundColor: BoardColors.brand,
                      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
                    ),
                    child: const Text('完成', style: TextStyle(fontSize: 13)),
                  ),
                ],
              ),
            ),

            // Main infinite canvas area
            Expanded(
              child: LayoutBuilder(
                builder: (context, constraints) {
                  _viewportSize = constraints.biggest;
                  if (!_didInitialFit) {
                    _didInitialFit = true;
                    WidgetsBinding.instance.addPostFrameCallback((_) {
                      if (mounted) _ctrl.fitToContent(_viewportSize);
                    });
                  }
                  return Stack(
                    children: [
                      // Two-layer RepaintBoundary Canvas
                      Positioned.fill(
                        child: MouseRegion(
                          cursor: _panning || _ctrl.tool == BoardTool.hand || _spacePressed
                              ? SystemMouseCursors.grab
                              : (_ctrl.tool == BoardTool.select
                                  ? SystemMouseCursors.basic
                                  : SystemMouseCursors.precise),
                          onHover: _onPointerHover,
                          child: Listener(
                            behavior: HitTestBehavior.opaque,
                            onPointerDown: _onPointerDown,
                            onPointerMove: _onPointerMove,
                            onPointerUp: _onPointerUp,
                            onPointerCancel: _onPointerUp,
                            onPointerSignal: _onPointerSignal,
                            child: Stack(
                              children: [
                                Positioned.fill(
                                  child: RepaintBoundary(
                                    child: CustomPaint(
                                      painter: StaticBoardPainter(
                                        controller: _ctrl,
                                        pictureProvider: () => _ensureStaticPicture(dark),
                                        graphRev: _ctrl.graphRevision,
                                        viewportRev: _ctrl.viewportRevision,
                                        dark: dark,
                                      ),
                                    ),
                                  ),
                                ),
                                Positioned.fill(
                                  child: RepaintBoundary(
                                    child: CustomPaint(
                                      painter: ActiveBoardPainter(
                                        controller: _ctrl,
                                        activeRev: _ctrl.activeRevision,
                                        dark: dark,
                                      ),
                                    ),
                                  ),
                                ),
                              ],
                            ),
                          ),
                        ),
                      ),

                      // Zoomed-canvas CJK IME inline editor (spike-verified 0.0px drift)
                      if (editNode != null)
                        Positioned(
                          left: editNode.x * _ctrl.zoom + _ctrl.pan.dx,
                          top: editNode.y * _ctrl.zoom + _ctrl.pan.dy,
                          child: Transform.scale(
                            scale: _ctrl.zoom,
                            alignment: Alignment.topLeft,
                            child: Focus(
                              onKeyEvent: (_, ev) {
                                if (ev is KeyDownEvent) {
                                  if (ev.logicalKey == LogicalKeyboardKey.escape) {
                                    _ctrl.finishEditing();
                                    _refreshRustRoutes();
                                    return KeyEventResult.handled;
                                  }
                                  if (ev.logicalKey == LogicalKeyboardKey.enter &&
                                      (HardwareKeyboard.instance.isControlPressed ||
                                          HardwareKeyboard.instance.isMetaPressed)) {
                                    _ctrl.finishEditing();
                                    _refreshRustRoutes();
                                    return KeyEventResult.handled;
                                  }
                                }
                                return KeyEventResult.ignored;
                              },
                              child: Container(
                                width: editNode.w,
                                constraints: BoxConstraints(minHeight: editNode.h),
                                padding: EdgeInsets.fromLTRB(
                                  BoardController.nodePaddingH,
                                  BoardController.nodePaddingV +
                                      (editNode.icon != null ? 30.0 : 0.0),
                                  BoardController.nodePaddingH,
                                  BoardController.nodePaddingV,
                                ),
                                decoration: BoxDecoration(
                                  color: parseBoardColor(
                                    (dark && editNode.style.fill.toUpperCase() == '#FFFFFF')
                                        ? '#252830'
                                        : editNode.style.fill,
                                    fallback: dark ? BoardColors.surfaceDark : Colors.white,
                                  ),
                                  borderRadius: BorderRadius.circular(8),
                                  border: Border.all(
                                    color: BoardColors.brand,
                                    width: 2,
                                  ),
                                ),
                                child: EditableText(
                                  controller: _editController,
                                  focusNode: _editFocus,
                                  textAlign: switch (editNode.style.align) {
                                    'left' => TextAlign.left,
                                    'right' => TextAlign.right,
                                    _ => TextAlign.center,
                                  },
                                  style: TextStyle(
                                    fontSize: editNode.style.fontSize,
                                    height: 1.36,
                                    fontWeight: editNode.kind == 'mind_root'
                                        ? FontWeight.w600
                                        : FontWeight.w400,
                                    color: parseBoardColor(
                                      (dark &&
                                              editNode.style.textColor.toUpperCase() ==
                                                  '#1F2329' &&
                                              (editNode.style.fill.toUpperCase() ==
                                                      '#FFFFFF' ||
                                                  editNode.style.fill == 'transparent'))
                                          ? '#E6E9EF'
                                          : editNode.style.textColor,
                                      fallback: textPrimary,
                                    ),
                                    fontFamilyFallback: const [
                                      'Microsoft YaHei',
                                      'PingFang SC',
                                      'Noto Sans SC',
                                    ],
                                  ),
                                  cursorColor: BoardColors.brand,
                                  backgroundCursorColor: BoardColors.brandWash,
                                  selectionColor: BoardColors.brandSelection,
                                  maxLines: null,
                                  keyboardType: TextInputType.multiline,
                                  textInputAction: TextInputAction.newline,
                                ),
                              ),
                            ),
                          ),
                        ),

                      // Left vertical Feishu-style tool dock
                      if (!widget.readOnly)
                        Positioned(
                          left: 14,
                          top: 16,
                          child: BoardLeftDock(
                            controller: _ctrl,
                            dark: dark,
                            showShapePanel: _showShapePanel,
                            showIconPanel: _showIconPanel,
                            onToggleShapePanel: () => setState(() {
                              _showShapePanel = !_showShapePanel;
                              _showIconPanel = false;
                            }),
                            onToggleIconPanel: () => setState(() {
                              _showIconPanel = !_showIconPanel;
                              _showShapePanel = false;
                            }),
                          ),
                        ),

                      // Top-center contextual floating inspector (hidden while dragging)
                      if (!widget.readOnly &&
                          _ctrl.editingNodeId == null &&
                          !_ctrl.isDraggingNodes &&
                          _ctrl.resizingNodeId == null &&
                          _ctrl.pendingConnector == null)
                        Positioned(
                          top: 14,
                          left: 72,
                          right: 72,
                          child: Center(
                            child: BoardSelectionInspector(
                              controller: _ctrl,
                              dark: dark,
                              onEditEdgeLabel: () {
                                final e = _ctrl.snapshot.edges
                                    .where((x) => x.id == _ctrl.selectedEdgeId)
                                    .firstOrNull;
                                if (e != null) _promptEditEdgeLabel(e);
                              },
                            ),
                          ),
                        ),

                      // Bottom-right zoom & shortcut pill
                      Positioned(
                        right: 16,
                        bottom: 16,
                        child: Material(
                          elevation: 3,
                          borderRadius: BorderRadius.circular(10),
                          color: barBg,
                          child: Container(
                            padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 4),
                            decoration: BoxDecoration(
                              borderRadius: BorderRadius.circular(10),
                              border: Border.all(color: barBorder),
                            ),
                            child: Row(
                              mainAxisSize: MainAxisSize.min,
                              children: [
                                IconButton(
                                  icon: const Icon(Icons.remove, size: 16),
                                  tooltip: '缩小',
                                  constraints: const BoxConstraints.tightFor(width: 28, height: 28),
                                  padding: EdgeInsets.zero,
                                  onPressed: () => _ctrl.zoomAround(
                                    1 / 1.15,
                                    Offset(_viewportSize.width / 2, _viewportSize.height / 2),
                                  ),
                                ),
                                InkWell(
                                  onTap: () => _ctrl.setViewport(1.0, _ctrl.pan),
                                  borderRadius: BorderRadius.circular(4),
                                  child: Padding(
                                    padding: const EdgeInsets.symmetric(horizontal: 6),
                                    child: Text(
                                      '${(_ctrl.zoom * 100).round()}%',
                                      style: TextStyle(fontSize: 12, color: textPrimary),
                                    ),
                                  ),
                                ),
                                IconButton(
                                  icon: const Icon(Icons.add, size: 16),
                                  tooltip: '放大',
                                  constraints: const BoxConstraints.tightFor(width: 28, height: 28),
                                  padding: EdgeInsets.zero,
                                  onPressed: () => _ctrl.zoomAround(
                                    1.15,
                                    Offset(_viewportSize.width / 2, _viewportSize.height / 2),
                                  ),
                                ),
                                IconButton(
                                  icon: const Icon(Icons.center_focus_strong, size: 16),
                                  tooltip: '自适应视图',
                                  constraints: const BoxConstraints.tightFor(width: 28, height: 28),
                                  padding: EdgeInsets.zero,
                                  onPressed: () => _ctrl.fitToContent(_viewportSize),
                                ),
                              ],
                            ),
                          ),
                        ),
                      ),

                      // Subtle bottom-left shortcut hint
                      Positioned(
                        left: 16,
                        bottom: 16,
                        child: Container(
                          padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 5),
                          decoration: BoxDecoration(
                            color: barBg.withValues(alpha: 0.9),
                            borderRadius: BorderRadius.circular(8),
                            border: Border.all(color: barBorder),
                          ),
                          child: Text(
                            '双击编辑文字 · 导图 Tab 加子节点 / Enter 加同级 · 空格+拖拽平移 · 滚轮缩放',
                            style: TextStyle(
                              fontSize: 11,
                              color: textMuted,
                            ),
                          ),
                        ),
                      ),
                    ],
                  );
                },
              ),
            ),
          ],
        ),
      ),
    );
  }
}
