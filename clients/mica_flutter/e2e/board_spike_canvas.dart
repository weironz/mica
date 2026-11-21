// Minimal technical spike for Option C (infinite whiteboard + mind map):
// 1. A/B rendering benchmark on 500 text-bearing nodes + ~460 connectors:
//    - naive: single-layer CustomPainter, per-frame TextPainter layout, no culling
//    - optimized: cached TextPainters + vector ui.Picture static layer behind
//      RepaintBoundary + dynamic drag/selection overlay + spatial grid hit-test
// 2. Zoomed-canvas inline CJK IME editing at arbitrary zoom (0.35x - 2.5x)
//    with zero wrap jump and live auto-growing node height.
import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/scheduler.dart';
import 'package:flutter/services.dart';

enum BoardRenderMode { optimized, naive }

enum SpikeNodeKind { rect, sticky, mindRoot, mindBranch }

class SpikeNode {
  SpikeNode({
    required this.id,
    required this.x,
    required this.y,
    required this.width,
    required this.minHeight,
    required this.height,
    required this.text,
    required this.fillColor,
    required this.borderColor,
    required this.kind,
  });

  final String id;
  double x;
  double y;
  double width;
  final double minHeight;
  double height;
  String text;
  final Color fillColor;
  final Color borderColor;
  final SpikeNodeKind kind;

  Rect get rect => Rect.fromLTWH(x, y, width, height);
  Offset get center => Offset(x + width / 2, y + height / 2);
}

class SpikeEdge {
  const SpikeEdge({
    required this.id,
    required this.fromId,
    required this.toId,
    required this.curved,
  });

  final String id;
  final String fromId;
  final String toId;
  final bool curved;
}

class SpikeFrameSample {
  const SpikeFrameSample({
    required this.buildUs,
    required this.rasterUs,
    required this.totalUs,
  });

  final int buildUs;
  final int rasterUs;
  final int totalUs;

  Map<String, int> toJson() => {
    'buildUs': buildUs,
    'rasterUs': rasterUs,
    'totalUs': totalUs,
  };
}

/// Controller exposed to both UI controls and the Playwright harness.
class BoardSpikeController extends ChangeNotifier {
  BoardSpikeController({int nodeCount = 500}) {
    _generateScene(nodeCount);
  }

  static const double nodePaddingH = 10.0;
  static const double nodePaddingV = 8.0;
  static const double baseFontSize = 13.0;
  static const double baseLineHeight = 1.35;
  static const TextStyle nodeTextStyle = TextStyle(
    fontSize: baseFontSize,
    height: baseLineHeight,
    color: Color(0xFF1F2329),
    fontFamilyFallback: ['Microsoft YaHei', 'PingFang SC', 'Noto Sans SC'],
  );

  final List<SpikeNode> nodes = [];
  final Map<String, SpikeNode> nodeById = {};
  final List<SpikeEdge> edges = [];
  final Map<String, List<SpikeEdge>> edgesByNode = {};

  BoardRenderMode mode = BoardRenderMode.optimized;
  double zoom = 1.0;
  Offset pan = const Offset(160, 120);

  String? selectedNodeId;
  String? draggingNodeId;
  String? editingNodeId;

  // Counters for automated verification of layer isolation.
  int staticPaintCount = 0;
  int pictureRebuildCount = 0;
  int activePaintCount = 0;
  int naivePaintCount = 0;

  // Monotonic revisions for RepaintBoundary / Picture invalidation.
  int graphRevision = 0;
  int viewportRevision = 0;
  int activeRevision = 0;

  // Cached TextPainters keyed by node id (only used in optimized mode).
  final Map<String, _CachedTextLayout> _textCache = {};

  // Uniform spatial grid for O(1) hit-testing.
  static const double _gridCellSize = 320.0;
  final Map<int, List<SpikeNode>> _grid = {};

  // Recent Flutter FrameTimings.
  final List<SpikeFrameSample> frameSamples = [];

  void recordFrameTiming(FrameTiming timing) {
    frameSamples.add(
      SpikeFrameSample(
        buildUs: timing.buildDuration.inMicroseconds,
        rasterUs: timing.rasterDuration.inMicroseconds,
        totalUs: timing.totalSpan.inMicroseconds,
      ),
    );
    if (frameSamples.length > 600) {
      frameSamples.removeRange(0, frameSamples.length - 600);
    }
  }

  void clearFrameSamples() {
    frameSamples.clear();
  }

  void invalidateTextCaches() {
    _textCache.clear();
    for (final node in nodes) {
      node.height = measureNodeHeight(node);
    }
    _rebuildGrid();
    graphRevision++;
    activeRevision++;
    notifyListeners();
  }

  void resetCounters() {
    staticPaintCount = 0;
    pictureRebuildCount = 0;
    activePaintCount = 0;
    naivePaintCount = 0;
    frameSamples.clear();
  }

  void setMode(BoardRenderMode next) {
    if (mode == next) return;
    mode = next;
    graphRevision++;
    activeRevision++;
    notifyListeners();
  }

  void setViewport(double nextZoom, Offset nextPan) {
    final clamped = nextZoom.clamp(0.25, 3.0);
    if (clamped == zoom && nextPan == pan) return;
    zoom = clamped;
    pan = nextPan;
    viewportRevision++;
    activeRevision++;
    notifyListeners();
  }

  void zoomAround(double factor, Offset focalScreenPoint) {
    final nextZoom = (zoom * factor).clamp(0.25, 3.0);
    if (nextZoom == zoom) return;
    final worldFocal = screenToWorld(focalScreenPoint);
    final nextPan = focalScreenPoint - worldFocal * nextZoom;
    setViewport(nextZoom, nextPan);
  }

  Offset screenToWorld(Offset screen) => (screen - pan) / zoom;

  Offset worldToScreen(Offset world) => world * zoom + pan;

  Rect nodeScreenRect(SpikeNode node) {
    final tl = worldToScreen(Offset(node.x, node.y));
    return Rect.fromLTWH(tl.dx, tl.dy, node.width * zoom, node.height * zoom);
  }

  SpikeNode? hitTestWorld(Offset worldPoint) {
    final cx = (worldPoint.dx / _gridCellSize).floor();
    final cy = (worldPoint.dy / _gridCellSize).floor();
    final bucket = _grid[_cellKey(cx, cy)];
    if (bucket != null) {
      for (var i = bucket.length - 1; i >= 0; i--) {
        final node = bucket[i];
        if (node.rect.contains(worldPoint)) return node;
      }
    }
    return null;
  }

  void beginDrag(String nodeId) {
    selectedNodeId = nodeId;
    draggingNodeId = nodeId;
    // Exclude dragged node from static layer once at drag start.
    graphRevision++;
    activeRevision++;
    notifyListeners();
  }

  void updateDrag(Offset worldDelta) {
    final id = draggingNodeId;
    if (id == null) return;
    final node = nodeById[id];
    if (node == null) return;
    node.x += worldDelta.dx;
    node.y += worldDelta.dy;
    // ONLY bump activeRevision: in optimized mode, the static RepaintBoundary
    // does NOT repaint at all during drag!
    activeRevision++;
    if (mode == BoardRenderMode.naive) {
      graphRevision++;
    }
    notifyListeners();
  }

  void endDrag() {
    final id = draggingNodeId;
    if (id == null) return;
    draggingNodeId = null;
    _rebuildGrid();
    graphRevision++;
    activeRevision++;
    notifyListeners();
  }

  void startEditing(String nodeId) {
    if (!nodeById.containsKey(nodeId)) return;
    selectedNodeId = nodeId;
    editingNodeId = nodeId;
    graphRevision++;
    activeRevision++;
    notifyListeners();
  }

  void updateEditingText(String nodeId, String nextText) {
    final node = nodeById[nodeId];
    if (node == null) return;
    if (node.text == nextText) return;
    node.text = nextText;
    _textCache.remove(nodeId);
    final measured = measureNodeHeight(node);
    if ((measured - node.height).abs() > 0.5) {
      node.height = measured;
    }
    activeRevision++;
    if (mode == BoardRenderMode.naive) {
      graphRevision++;
    }
    notifyListeners();
  }

  void finishEditing() {
    if (editingNodeId == null) return;
    editingNodeId = null;
    _rebuildGrid();
    graphRevision++;
    activeRevision++;
    notifyListeners();
  }

  double measureNodeHeight(SpikeNode node) {
    final tp = getCachedTextPainter(node);
    return math.max(node.minHeight, tp.height + nodePaddingV * 2);
  }

  TextPainter getCachedTextPainter(SpikeNode node) {
    final maxW = math.max(24.0, node.width - nodePaddingH * 2);
    final existing = _textCache[node.id];
    if (existing != null &&
        existing.text == node.text &&
        existing.maxWidth == maxW) {
      return existing.painter;
    }
    final tp = TextPainter(
      text: TextSpan(text: node.text, style: nodeTextStyle),
      textDirection: TextDirection.ltr,
    )..layout(maxWidth: maxW);
    _textCache[node.id] = _CachedTextLayout(
      text: node.text,
      maxWidth: maxW,
      painter: tp,
    );
    return tp;
  }

  void _generateScene(int targetCount) {
    nodes.clear();
    nodeById.clear();
    edges.clear();
    edgesByNode.clear();
    _textCache.clear();

    void addNode(SpikeNode n) {
      n.height = measureNodeHeight(n);
      nodes.add(n);
      nodeById[n.id] = n;
    }

    void addEdge(String fromId, String toId, {bool curved = true}) {
      final edge = SpikeEdge(
        id: 'e${edges.length}',
        fromId: fromId,
        toId: toId,
        curved: curved,
      );
      edges.add(edge);
      (edgesByNode[fromId] ??= []).add(edge);
      (edgesByNode[toId] ??= []).add(edge);
    }

    // Hero cluster at origin so initial viewport (0..1000, 0..700) is crisp & rich.
    addNode(
      SpikeNode(
        id: 'n0',
        x: 120,
        y: 180,
        width: 220,
        minHeight: 68,
        height: 68,
        text: 'Mica 白板技术探针\n双击测试缩放态中文 IME',
        fillColor: const Color(0xFFE8F3FF),
        borderColor: const Color(0xFF3370FF),
        kind: SpikeNodeKind.mindRoot,
      ),
    );

    const heroLabels = [
      'Rust 数据面\nYrs CRDT + 空间索引',
      '双层重绘隔离\nui.Picture + Overlay',
      '缩放中文 IME\n候选框坐标校验',
      'Markdown 兼容\nSVG 静态导出 + 回导',
      'WS 双通道\nPresence 与 CRDT 分离',
    ];
    for (var i = 0; i < heroLabels.length; i++) {
      final id = 'n${i + 1}';
      addNode(
        SpikeNode(
          id: id,
          x: 430,
          y: 40.0 + i * 96.0,
          width: 210,
          minHeight: 60,
          height: 60,
          text: heroLabels[i],
          fillColor: i.isEven
              ? const Color(0xFFFFFFFF)
              : const Color(0xFFFFF7E6),
          borderColor: i.isEven
              ? const Color(0xFF8C959F)
              : const Color(0xFFD9822B),
          kind: i.isEven ? SpikeNodeKind.mindBranch : SpikeNodeKind.sticky,
        ),
      );
      addEdge('n0', id, curved: true);
    }

    // Generate remaining clusters (mind maps + architecture grids) up to targetCount.
    const palette = [
      (Color(0xFFFFFFFF), Color(0xFF8C959F), SpikeNodeKind.rect),
      (Color(0xFFFFF8D6), Color(0xFFD4A72C), SpikeNodeKind.sticky),
      (Color(0xFFEDF7ED), Color(0xFF2DA44E), SpikeNodeKind.mindBranch),
      (Color(0xFFF5F0FF), Color(0xFF8250DF), SpikeNodeKind.rect),
      (Color(0xFFFFF0F0), Color(0xFFCF222E), SpikeNodeKind.sticky),
    ];
    const sampleTopics = [
      '文档同步状态机',
      '离线 SQLite 快照',
      '正交折线避障',
      '对齐吸附参考线',
      '节点分组与层级',
      'Web CanvasKit 渲染',
      '协同光标广播',
      '思维导图自动布局',
      'AI / MCP 图谱读写',
      'Brotli 分包体积',
    ];

    var idx = nodes.length;
    var cluster = 0;
    while (idx < targetCount) {
      final col = cluster % 6;
      final row = cluster ~/ 6;
      final baseX = -600.0 + col * 720.0;
      final baseY = 580.0 + row * 480.0;

      final rootId = 'n$idx';
      addNode(
        SpikeNode(
          id: rootId,
          x: baseX,
          y: baseY + 140,
          width: 176,
          minHeight: 54,
          height: 54,
          text: '模块 #$cluster\n${sampleTopics[cluster % sampleTopics.length]}',
          fillColor: const Color(0xFFE8F3FF),
          borderColor: const Color(0xFF3370FF),
          kind: SpikeNodeKind.mindRoot,
        ),
      );
      idx++;

      // 4 branches per cluster, each with up to 3 leaves = 17 nodes/cluster.
      for (var b = 0; b < 4 && idx < targetCount; b++) {
        final branchId = 'n$idx';
        final style = palette[(cluster + b) % palette.length];
        addNode(
          SpikeNode(
            id: branchId,
            x: baseX + 230,
            y: baseY + b * 105.0,
            width: 170,
            minHeight: 52,
            height: 52,
            text:
                '分支 $cluster.$b: ${sampleTopics[(cluster + b) % sampleTopics.length]}',
            fillColor: style.$1,
            borderColor: style.$2,
            kind: style.$3,
          ),
        );
        addEdge(rootId, branchId, curved: b.isEven);
        idx++;

        for (var l = 0; l < 2 && idx < targetCount; l++) {
          final leafId = 'n$idx';
          final lStyle = palette[(cluster + b + l + 1) % palette.length];
          addNode(
            SpikeNode(
              id: leafId,
              x: baseX + 455,
              y: baseY + b * 105.0 + l * 48.0 - 10,
              width: 165,
              minHeight: 40,
              height: 40,
              text: '节点 #$idx · Leaf $b-$l',
              fillColor: lStyle.$1,
              borderColor: lStyle.$2,
              kind: lStyle.$3,
            ),
          );
          addEdge(branchId, leafId, curved: true);
          idx++;
        }
      }
      cluster++;
    }

    _rebuildGrid();
    graphRevision++;
    activeRevision++;
  }

  void _rebuildGrid() {
    _grid.clear();
    for (final node in nodes) {
      final minCx = (node.x / _gridCellSize).floor();
      final maxCx = ((node.x + node.width) / _gridCellSize).floor();
      final minCy = (node.y / _gridCellSize).floor();
      final maxCy = ((node.y + node.height) / _gridCellSize).floor();
      for (var cx = minCx; cx <= maxCx; cx++) {
        for (var cy = minCy; cy <= maxCy; cy++) {
          (_grid[_cellKey(cx, cy)] ??= []).add(node);
        }
      }
    }
  }

  int _cellKey(int cx, int cy) => (cx + 100000) * 200000 + (cy + 100000);
}

class _CachedTextLayout {
  const _CachedTextLayout({
    required this.text,
    required this.maxWidth,
    required this.painter,
  });

  final String text;
  final double maxWidth;
  final TextPainter painter;
}

class BoardSpikeCanvas extends StatefulWidget {
  const BoardSpikeCanvas({super.key, required this.controller});

  final BoardSpikeController controller;

  @override
  State<BoardSpikeCanvas> createState() => _BoardSpikeCanvasState();
}

class _BoardSpikeCanvasState extends State<BoardSpikeCanvas> {
  final TextEditingController _editController = TextEditingController();
  final FocusNode _editFocus = FocusNode();
  String? _boundEditNodeId;

  // Picture cache for optimized static layer.
  ui.Picture? _staticPicture;
  int _staticPictureGraphRev = -1;

  // Drag state.
  bool _panning = false;
  Offset _lastPointer = Offset.zero;
  DateTime? _lastTapTime;
  String? _lastTapNodeId;

  BoardSpikeController get _ctrl => widget.controller;

  @override
  void initState() {
    super.initState();
    _ctrl.addListener(_onControllerChanged);
    _editController.addListener(_onEditTextChanged);
    SchedulerBinding.instance.addTimingsCallback(_onTimings);
    // CanvasKit downloads CJK fallback font slices asynchronously after first
    // layout; invalidate cached TextPainters & ui.Picture when slices arrive.
    PaintingBinding.instance.systemFonts.addListener(_onSystemFontsChanged);
  }

  @override
  void dispose() {
    PaintingBinding.instance.systemFonts.removeListener(_onSystemFontsChanged);
    SchedulerBinding.instance.removeTimingsCallback(_onTimings);
    _ctrl.removeListener(_onControllerChanged);
    _editController.removeListener(_onEditTextChanged);
    _editController.dispose();
    _editFocus.dispose();
    _staticPicture?.dispose();
    super.dispose();
  }

  void _onSystemFontsChanged() {
    _ctrl.invalidateTextCaches();
  }

  void _onTimings(List<FrameTiming> timings) {
    for (final t in timings) {
      _ctrl.recordFrameTiming(t);
    }
  }

  void _onControllerChanged() {
    final editId = _ctrl.editingNodeId;
    if (editId != _boundEditNodeId) {
      _boundEditNodeId = editId;
      if (editId != null) {
        final node = _ctrl.nodeById[editId];
        if (node != null) {
          _editController.value = TextEditingValue(
            text: node.text,
            selection: TextSelection.collapsed(offset: node.text.length),
          );
          WidgetsBinding.instance.addPostFrameCallback((_) {
            if (mounted && _ctrl.editingNodeId == editId) {
              _editFocus.requestFocus();
            }
          });
        }
      } else {
        _editFocus.unfocus();
      }
    }
    if (mounted) setState(() {});
  }

  void _onEditTextChanged() {
    final editId = _ctrl.editingNodeId;
    if (editId == null) return;
    _ctrl.updateEditingText(editId, _editController.text);
  }

  ui.Picture _ensureStaticPicture() {
    if (_staticPicture != null &&
        _staticPictureGraphRev == _ctrl.graphRevision) {
      return _staticPicture!;
    }
    _staticPicture?.dispose();
    final recorder = ui.PictureRecorder();
    final canvas = Canvas(recorder);
    final dynamicId = _ctrl.draggingNodeId ?? _ctrl.editingNodeId;

    // Draw all static edges first (excluding incident edges of dragged/edited node).
    for (final edge in _ctrl.edges) {
      if (dynamicId != null &&
          (edge.fromId == dynamicId || edge.toId == dynamicId)) {
        continue;
      }
      final a = _ctrl.nodeById[edge.fromId];
      final b = _ctrl.nodeById[edge.toId];
      if (a != null && b != null) {
        _paintEdge(canvas, a, b, edge.curved, highlighted: false);
      }
    }

    // Draw all static nodes using cached TextPainters.
    for (final node in _ctrl.nodes) {
      if (node.id == dynamicId) continue;
      _paintNode(
        canvas,
        node,
        _ctrl.getCachedTextPainter(node),
        selected: node.id == _ctrl.selectedNodeId,
        hideText: false,
      );
    }

    _staticPicture = recorder.endRecording();
    _staticPictureGraphRev = _ctrl.graphRevision;
    _ctrl.pictureRebuildCount++;
    return _staticPicture!;
  }

  void _handlePointerDown(PointerDownEvent event) {
    if (event.buttons != kPrimaryMouseButton) return;
    final world = _ctrl.screenToWorld(event.localPosition);
    final hit = _ctrl.hitTestWorld(world);
    _lastPointer = event.localPosition;

    if (hit == null) {
      if (_ctrl.editingNodeId != null) {
        _ctrl.finishEditing();
      }
      _ctrl.selectedNodeId = null;
      _ctrl.activeRevision++;
      _panning = true;
      setState(() {});
      return;
    }

    if (_ctrl.editingNodeId != null && _ctrl.editingNodeId != hit.id) {
      _ctrl.finishEditing();
    }

    final now = DateTime.now();
    final isDoubleTap =
        _lastTapNodeId == hit.id &&
        _lastTapTime != null &&
        now.difference(_lastTapTime!).inMilliseconds < 420;
    _lastTapTime = now;
    _lastTapNodeId = hit.id;

    if (isDoubleTap) {
      _panning = false;
      _ctrl.startEditing(hit.id);
      return;
    }

    if (_ctrl.editingNodeId == hit.id) {
      return;
    }

    _panning = false;
    _ctrl.beginDrag(hit.id);
  }

  void _handlePointerMove(PointerMoveEvent event) {
    final delta = event.localPosition - _lastPointer;
    _lastPointer = event.localPosition;
    if (_panning) {
      _ctrl.setViewport(_ctrl.zoom, _ctrl.pan + delta);
    } else if (_ctrl.draggingNodeId != null) {
      _ctrl.updateDrag(delta / _ctrl.zoom);
    }
  }

  void _handlePointerUp(PointerEvent event) {
    if (_panning) {
      _panning = false;
    }
    if (_ctrl.draggingNodeId != null) {
      _ctrl.endDrag();
    }
  }

  void _handlePointerSignal(PointerSignalEvent event) {
    if (event is PointerScrollEvent) {
      final factor = event.scrollDelta.dy < 0 ? 1.12 : 1 / 1.12;
      _ctrl.zoomAround(factor, event.localPosition);
    }
  }

  @override
  Widget build(BuildContext context) {
    final editNode = _ctrl.editingNodeId == null
        ? null
        : _ctrl.nodeById[_ctrl.editingNodeId!];

    return Scaffold(
      backgroundColor: const Color(0xFFF7F8FA),
      body: Stack(
        children: [
          Positioned.fill(
            child: Listener(
              behavior: HitTestBehavior.opaque,
              onPointerDown: _handlePointerDown,
              onPointerMove: _handlePointerMove,
              onPointerUp: _handlePointerUp,
              onPointerCancel: _handlePointerUp,
              onPointerSignal: _handlePointerSignal,
              child: _ctrl.mode == BoardRenderMode.optimized
                  ? Stack(
                      children: [
                        // Layer 1: Static vector Picture behind RepaintBoundary.
                        // Does NOT repaint while dragging a node!
                        Positioned.fill(
                          child: RepaintBoundary(
                            child: CustomPaint(
                              painter: _OptimizedStaticPainter(
                                controller: _ctrl,
                                pictureProvider: _ensureStaticPicture,
                                graphRev: _ctrl.graphRevision,
                                viewportRev: _ctrl.viewportRevision,
                              ),
                            ),
                          ),
                        ),
                        // Layer 2: Active overlay (dragged node + incident edges).
                        Positioned.fill(
                          child: RepaintBoundary(
                            child: CustomPaint(
                              painter: _ActiveOverlayPainter(
                                controller: _ctrl,
                                activeRev: _ctrl.activeRevision,
                              ),
                            ),
                          ),
                        ),
                      ],
                    )
                  : CustomPaint(
                      painter: _NaiveBoardPainter(
                        controller: _ctrl,
                        rev:
                            _ctrl.graphRevision +
                            _ctrl.viewportRevision +
                            _ctrl.activeRevision,
                      ),
                    ),
            ),
          ),

          // Zoomed inline text editor overlay (screen-space positioned, world-space
          // scaled via Transform.scale so line-wrapping matches canvas 1:1).
          if (editNode != null)
            Positioned(
              left: editNode.x * _ctrl.zoom + _ctrl.pan.dx,
              top: editNode.y * _ctrl.zoom + _ctrl.pan.dy,
              child: Transform.scale(
                scale: _ctrl.zoom,
                alignment: Alignment.topLeft,
                child: Focus(
                  onKeyEvent: (_, event) {
                    if (event is KeyDownEvent &&
                        event.logicalKey == LogicalKeyboardKey.escape) {
                      _ctrl.finishEditing();
                      return KeyEventResult.handled;
                    }
                    return KeyEventResult.ignored;
                  },
                  child: Container(
                    width: editNode.width,
                    constraints: BoxConstraints(minHeight: editNode.minHeight),
                    padding: const EdgeInsets.symmetric(
                      horizontal: BoardSpikeController.nodePaddingH,
                      vertical: BoardSpikeController.nodePaddingV,
                    ),
                    decoration: BoxDecoration(
                      color: editNode.fillColor,
                      borderRadius: BorderRadius.circular(8),
                      border: Border.all(
                        color: const Color(0xFF3370FF),
                        width: 2,
                      ),
                    ),
                    child: EditableText(
                      controller: _editController,
                      focusNode: _editFocus,
                      style: BoardSpikeController.nodeTextStyle,
                      cursorColor: const Color(0xFF3370FF),
                      backgroundCursorColor: const Color(0xFFD0E3FF),
                      selectionColor: const Color(0x553370FF),
                      maxLines: null,
                      keyboardType: TextInputType.multiline,
                      textInputAction: TextInputAction.newline,
                    ),
                  ),
                ),
              ),
            ),

          // Minimal HUD bar for human exploration & visual screenshot context.
          Positioned(
            top: 12,
            left: 12,
            child: Material(
              elevation: 2,
              borderRadius: BorderRadius.circular(8),
              color: Colors.white.withValues(alpha: 0.94),
              child: Padding(
                padding: const EdgeInsets.symmetric(
                  horizontal: 10,
                  vertical: 6,
                ),
                child: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Text(
                      'Mica Board Spike · ${_ctrl.nodes.length} nodes / ${_ctrl.edges.length} edges',
                      style: const TextStyle(
                        fontSize: 12,
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                    const SizedBox(width: 10),
                    _HudChip(
                      label: 'Optimized',
                      active: _ctrl.mode == BoardRenderMode.optimized,
                      onTap: () => _ctrl.setMode(BoardRenderMode.optimized),
                    ),
                    const SizedBox(width: 4),
                    _HudChip(
                      label: 'Naive',
                      active: _ctrl.mode == BoardRenderMode.naive,
                      onTap: () => _ctrl.setMode(BoardRenderMode.naive),
                    ),
                    const SizedBox(width: 10),
                    for (final z in [0.45, 1.0, 2.0]) ...[
                      _HudChip(
                        label: '${(z * 100).round()}%',
                        active: (_ctrl.zoom - z).abs() < 0.02,
                        onTap: () => _ctrl.setViewport(z, _ctrl.pan),
                      ),
                      const SizedBox(width: 4),
                    ],
                    const SizedBox(width: 6),
                    Text(
                      'static:${_ctrl.staticPaintCount} pic:${_ctrl.pictureRebuildCount} active:${_ctrl.activePaintCount}',
                      style: const TextStyle(
                        fontSize: 11,
                        color: Color(0xFF646A73),
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}

class _HudChip extends StatelessWidget {
  const _HudChip({
    required this.label,
    required this.active,
    required this.onTap,
  });

  final String label;
  final bool active;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return InkWell(
      onTap: onTap,
      borderRadius: BorderRadius.circular(5),
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
        decoration: BoxDecoration(
          color: active ? const Color(0xFF3370FF) : const Color(0xFFF0F2F5),
          borderRadius: BorderRadius.circular(5),
        ),
        child: Text(
          label,
          style: TextStyle(
            fontSize: 11,
            fontWeight: FontWeight.w500,
            color: active ? Colors.white : const Color(0xFF1F2329),
          ),
        ),
      ),
    );
  }
}

class _OptimizedStaticPainter extends CustomPainter {
  _OptimizedStaticPainter({
    required this.controller,
    required this.pictureProvider,
    required this.graphRev,
    required this.viewportRev,
  });

  final BoardSpikeController controller;
  final ui.Picture Function() pictureProvider;
  final int graphRev;
  final int viewportRev;

  @override
  void paint(Canvas canvas, Size size) {
    controller.staticPaintCount++;
    _paintDotGrid(canvas, size, controller.zoom, controller.pan);
    final picture = pictureProvider();
    canvas.save();
    canvas.translate(controller.pan.dx, controller.pan.dy);
    canvas.scale(controller.zoom, controller.zoom);
    canvas.drawPicture(picture);
    canvas.restore();
  }

  @override
  bool shouldRepaint(covariant _OptimizedStaticPainter oldDelegate) {
    return oldDelegate.graphRev != graphRev ||
        oldDelegate.viewportRev != viewportRev;
  }
}

class _ActiveOverlayPainter extends CustomPainter {
  _ActiveOverlayPainter({required this.controller, required this.activeRev});

  final BoardSpikeController controller;
  final int activeRev;

  @override
  void paint(Canvas canvas, Size size) {
    controller.activePaintCount++;
    final dragId = controller.draggingNodeId;
    final editId = controller.editingNodeId;
    if (dragId == null && editId == null) return;

    canvas.save();
    canvas.translate(controller.pan.dx, controller.pan.dy);
    canvas.scale(controller.zoom, controller.zoom);

    if (dragId != null) {
      final node = controller.nodeById[dragId];
      if (node != null) {
        final incident = controller.edgesByNode[dragId] ?? const <SpikeEdge>[];
        for (final edge in incident) {
          final a = controller.nodeById[edge.fromId];
          final b = controller.nodeById[edge.toId];
          if (a != null && b != null) {
            _paintEdge(canvas, a, b, edge.curved, highlighted: true);
          }
        }
        _paintNode(
          canvas,
          node,
          controller.getCachedTextPainter(node),
          selected: true,
          hideText: false,
        );
      }
    } else if (editId != null) {
      // Repaint live incident edges when node height auto-grows during typing.
      final incident = controller.edgesByNode[editId] ?? const <SpikeEdge>[];
      for (final edge in incident) {
        final a = controller.nodeById[edge.fromId];
        final b = controller.nodeById[edge.toId];
        if (a != null && b != null) {
          _paintEdge(canvas, a, b, edge.curved, highlighted: true);
        }
      }
    }

    canvas.restore();
  }

  @override
  bool shouldRepaint(covariant _ActiveOverlayPainter oldDelegate) {
    return oldDelegate.activeRev != activeRev;
  }
}

/// Deliberately unoptimized baseline: single CustomPainter, lays out every
/// TextPainter from scratch on every frame with zero caching or layer split.
class _NaiveBoardPainter extends CustomPainter {
  _NaiveBoardPainter({required this.controller, required this.rev});

  final BoardSpikeController controller;
  final int rev;

  @override
  void paint(Canvas canvas, Size size) {
    controller.naivePaintCount++;
    _paintDotGrid(canvas, size, controller.zoom, controller.pan);
    canvas.save();
    canvas.translate(controller.pan.dx, controller.pan.dy);
    canvas.scale(controller.zoom, controller.zoom);

    for (final edge in controller.edges) {
      final a = controller.nodeById[edge.fromId];
      final b = controller.nodeById[edge.toId];
      if (a != null && b != null) {
        final hl =
            edge.fromId == controller.draggingNodeId ||
            edge.toId == controller.draggingNodeId;
        _paintEdge(canvas, a, b, edge.curved, highlighted: hl);
      }
    }

    for (final node in controller.nodes) {
      final maxW = math.max(
        24.0,
        node.width - BoardSpikeController.nodePaddingH * 2,
      );
      final tp = TextPainter(
        text: TextSpan(
          text: node.text,
          style: BoardSpikeController.nodeTextStyle,
        ),
        textDirection: TextDirection.ltr,
      )..layout(maxWidth: maxW);
      _paintNode(
        canvas,
        node,
        tp,
        selected: node.id == controller.selectedNodeId,
        hideText: node.id == controller.editingNodeId,
      );
    }

    canvas.restore();
  }

  @override
  bool shouldRepaint(covariant _NaiveBoardPainter oldDelegate) =>
      oldDelegate.rev != rev;
}

final Paint _dotPaint = Paint()..color = const Color(0xFFD6DBE3);
final Paint _edgePaint = Paint()
  ..color = const Color(0xFF9AA3B2)
  ..style = PaintingStyle.stroke
  ..strokeWidth = 1.6;
final Paint _edgeHighlightPaint = Paint()
  ..color = const Color(0xFF3370FF)
  ..style = PaintingStyle.stroke
  ..strokeWidth = 2.2;

void _paintDotGrid(Canvas canvas, Size size, double zoom, Offset pan) {
  final step = 28.0 * zoom;
  if (step < 10) return;
  final startX = pan.dx % step;
  final startY = pan.dy % step;
  final points = <Offset>[];
  for (var x = startX; x < size.width; x += step) {
    for (var y = startY; y < size.height; y += step) {
      points.add(Offset(x, y));
    }
  }
  _dotPaint.strokeWidth = (1.6 * zoom).clamp(1.0, 2.5);
  canvas.drawPoints(ui.PointMode.points, points, _dotPaint);
}

void _paintEdge(
  Canvas canvas,
  SpikeNode from,
  SpikeNode to,
  bool curved, {
  required bool highlighted,
}) {
  final start = Offset(from.x + from.width, from.y + from.height / 2);
  final end = Offset(to.x, to.y + to.height / 2);
  final paint = highlighted ? _edgeHighlightPaint : _edgePaint;

  final path = Path()..moveTo(start.dx, start.dy);
  if (curved) {
    final dx = ((end.dx - start.dx).abs() * 0.45).clamp(24.0, 140.0);
    path.cubicTo(
      start.dx + dx,
      start.dy,
      end.dx - dx,
      end.dy,
      end.dx,
      end.dy,
    );
  } else {
    final midX = (start.dx + end.dx) / 2;
    path
      ..lineTo(midX, start.dy)
      ..lineTo(midX, end.dy)
      ..lineTo(end.dx, end.dy);
  }
  canvas.drawPath(path, paint);
}

void _paintNode(
  Canvas canvas,
  SpikeNode node,
  TextPainter tp, {
  required bool selected,
  required bool hideText,
}) {
  final rrect = RRect.fromRectAndRadius(node.rect, const Radius.circular(8));
  final fill = Paint()..color = node.fillColor;
  canvas.drawRRect(rrect, fill);

  final border = Paint()
    ..color = selected ? const Color(0xFF3370FF) : node.borderColor
    ..style = PaintingStyle.stroke
    ..strokeWidth = selected ? 2.2 : 1.2;
  canvas.drawRRect(rrect, border);

  if (!hideText) {
    tp.paint(
      canvas,
      Offset(
        node.x + BoardSpikeController.nodePaddingH,
        node.y + BoardSpikeController.nodePaddingV,
      ),
    );
  }
}
