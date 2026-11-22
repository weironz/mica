// Interactive whiteboard & mind map controller:
// - Undo / Redo (100-step coalesced history)
// - Multi-select + rubber-band marquee + Copy / Paste / Duplicate / Delete
// - Dragging with live smart alignment guides (+-6 px snapping)
// - 8-direction resize handles
// - 4-port connector drawing + curved/orthogonal/straight routing
// - Mind map keyboard workflow (Tab child, Enter sibling, Shift+Tab promote, collapse)
// - Freehand ink pen strokes
// - Spatial grid + TextPainter cache + systemFonts invalidation + 2-layer versioning
import 'dart:math' as math;

import 'package:flutter/material.dart';

import 'board_models.dart';

enum ResizeHandle { nw, n, ne, e, se, s, sw, w }

class AlignmentGuide {
  const AlignmentGuide({
    required this.vertical,
    required this.coord,
    required this.start,
    required this.end,
  });

  final bool vertical;
  final double coord;
  final double start;
  final double end;
}

class PendingConnector {
  PendingConnector({
    required this.fromNodeId,
    required this.fromAnchor,
    required this.fromWorld,
    required this.currentWorld,
    this.targetNodeId,
    this.targetAnchor,
  });

  final String fromNodeId;
  final String fromAnchor;
  final Offset fromWorld;
  Offset currentWorld;
  String? targetNodeId;
  String? targetAnchor;
}

class BoardController extends ChangeNotifier {
  BoardController(BoardSnapshot initial) : _snapshot = initial.deepCopy() {
    _rebuildIndices();
  }

  static const double nodePaddingH = 10.0;
  static const double nodePaddingV = 8.0;
  static const double _snapThreshold = 6.0;
  static const int _maxHistory = 100;

  BoardSnapshot _snapshot;
  BoardSnapshot get snapshot => _snapshot;

  final Map<String, BoardNode> nodeById = {};
  final Map<String, List<BoardEdge>> edgesByNode = {};
  final Map<String, List<Offset>> routedEdgeCache = {};
  final Map<String, _CachedText> _textCache = {};

  // History stacks (JSON strings for memory-compact deep snapshots).
  final List<BoardSnapshot> _undoStack = [];
  final List<BoardSnapshot> _redoStack = [];
  bool get canUndo => _undoStack.isNotEmpty;
  bool get canRedo => _redoStack.isNotEmpty;

  // Clipboard (in-memory for Ctrl+C/V inside board).
  List<BoardNode> _clipboardNodes = [];
  List<BoardEdge> _clipboardEdges = [];

  // Tool & creation defaults.
  BoardTool tool = BoardTool.select;
  String activeShapeKind = 'rounded';
  String activeIconId = 'server';
  String activeRouting = 'orthogonal';
  String activePenColor = '#3370FF';
  double activePenWidth = 2.6;

  // Viewport transform.
  double zoom = 1.0;
  Offset pan = const Offset(80, 60);

  // Selection & active gesture state.
  final Set<String> selectedNodeIds = {};
  String? selectedEdgeId;
  String? hoveredNodeId;
  String? editingNodeId;
  String? editingEdgeId;

  bool isDraggingNodes = false;
  ResizeHandle? activeResizeHandle;
  String? resizingNodeId;
  Rect? marqueeWorldRect;
  PendingConnector? pendingConnector;
  BoardStroke? activeStroke;
  List<AlignmentGuide> activeGuides = const [];

  BoardSnapshot? _gestureBeforeSnapshot;
  final Map<String, Offset> _dragStartPositions = {};
  Offset _dragTotalDelta = Offset.zero;
  int _idCounter = 1000;

  // Two-layer RepaintBoundary monotonic versions + direct paint signals so
  // high-frequency mouse moves repaint ONLY the CustomPaint RenderObject
  // without triggering full widget-tree setState() rebuilds.
  final _PaintSignal _staticPaintSignal = _PaintSignal();
  final _PaintSignal _activePaintSignal = _PaintSignal();
  Listenable get staticPaintSignal => _staticPaintSignal;
  Listenable get activePaintSignal => _activePaintSignal;
  int graphRevision = 0;
  int viewportRevision = 0;
  int activeRevision = 0;

  @override
  void dispose() {
    _staticPaintSignal.dispose();
    _activePaintSignal.dispose();
    super.dispose();
  }

  void _emitActiveOnly() {
    activeRevision++;
    _activePaintSignal.bump();
  }

  void _emitCanvas({bool staticDirty = true, bool rebuildChrome = false}) {
    if (staticDirty) _staticPaintSignal.bump();
    activeRevision++;
    _activePaintSignal.bump();
    if (rebuildChrome) notifyListeners();
  }

  // Counters for tests & telemetry.
  int staticPaintCount = 0;
  int pictureRebuildCount = 0;
  int activePaintCount = 0;

  String _nextId(String prefix) {
    _idCounter++;
    var candidate = '$prefix$_idCounter';
    while (nodeById.containsKey(candidate) ||
        _snapshot.edges.any((e) => e.id == candidate)) {
      _idCounter++;
      candidate = '$prefix$_idCounter';
    }
    return candidate;
  }

  void _rebuildIndices() {
    nodeById.clear();
    edgesByNode.clear();
    for (final n in _snapshot.nodes) {
      nodeById[n.id] = n;
    }
    for (final e in _snapshot.edges) {
      (edgesByNode[e.fromId] ??= []).add(e);
      (edgesByNode[e.toId] ??= []).add(e);
    }
  }

  void _pushUndo([BoardSnapshot? preState]) {
    _undoStack.add((preState ?? _snapshot).deepCopy());
    if (_undoStack.length > _maxHistory) {
      _undoStack.removeAt(0);
    }
    _redoStack.clear();
  }

  void undo() {
    if (_undoStack.isEmpty) return;
    editingNodeId = null;
    editingEdgeId = null;
    _redoStack.add(_snapshot.deepCopy());
    _snapshot = _undoStack.removeLast();
    _textCache.clear();
    routedEdgeCache.clear();
    selectedNodeIds.retainWhere(
      (id) => _snapshot.nodes.any((n) => n.id == id),
    );
    selectedEdgeId = null;
    _rebuildIndices();
    graphRevision++;
    activeRevision++;
    notifyListeners();
  }

  void redo() {
    if (_redoStack.isEmpty) return;
    editingNodeId = null;
    editingEdgeId = null;
    _undoStack.add(_snapshot.deepCopy());
    _snapshot = _redoStack.removeLast();
    _textCache.clear();
    routedEdgeCache.clear();
    selectedNodeIds.retainWhere(
      (id) => _snapshot.nodes.any((n) => n.id == id),
    );
    selectedEdgeId = null;
    _rebuildIndices();
    graphRevision++;
    activeRevision++;
    notifyListeners();
  }

  void replaceSnapshot(BoardSnapshot next, {bool recordUndo = true}) {
    if (recordUndo) _pushUndo();
    editingNodeId = null;
    editingEdgeId = null;
    selectedNodeIds.clear();
    selectedEdgeId = null;
    _snapshot = next.deepCopy();
    _textCache.clear();
    routedEdgeCache.clear();
    _rebuildIndices();
    graphRevision++;
    activeRevision++;
    notifyListeners();
  }

  void setTitle(String title) {
    final trimmed = title.trim();
    if (trimmed.isEmpty || trimmed == _snapshot.title) return;
    _pushUndo();
    _snapshot.title = trimmed;
    notifyListeners();
  }

  void setTool(BoardTool next) {
    if (tool == next) return;
    tool = next;
    activeRevision++;
    notifyListeners();
  }

  void setViewport(double nextZoom, Offset nextPan) {
    final clamped = nextZoom.clamp(0.2, 3.0);
    if (clamped == zoom && nextPan == pan) return;
    final zoomChanged = (clamped - zoom).abs() > 1e-4;
    zoom = clamped;
    pan = nextPan;
    viewportRevision++;
    // Pure panning repaints canvas directly with 0 widget rebuilds unless
    // zoom % badge or zoomed inline editor overlay needs repositioning.
    _emitCanvas(
      staticDirty: true,
      rebuildChrome: zoomChanged || editingNodeId != null,
    );
  }

  void zoomAround(double factor, Offset focalScreen) {
    final nextZoom = (zoom * factor).clamp(0.2, 3.0);
    if (nextZoom == zoom) return;
    final worldFocal = screenToWorld(focalScreen);
    setViewport(nextZoom, focalScreen - worldFocal * nextZoom);
  }

  void fitToContent(Size viewportSize) {
    if (viewportSize.width < 40 || viewportSize.height < 40) return;
    final hidden = collapsedDescendants(_snapshot);
    final visible = _snapshot.nodes.where((n) => !hidden.contains(n.id)).toList();
    if (visible.isEmpty && _snapshot.strokes.isEmpty) {
      setViewport(1.0, Offset(viewportSize.width * 0.2, viewportSize.height * 0.2));
      return;
    }
    var minX = double.infinity;
    var minY = double.infinity;
    var maxX = double.negativeInfinity;
    var maxY = double.negativeInfinity;
    for (final n in visible) {
      minX = math.min(minX, n.x);
      minY = math.min(minY, n.y);
      maxX = math.max(maxX, n.x + n.w);
      maxY = math.max(maxY, n.y + n.h);
    }
    for (final s in _snapshot.strokes) {
      for (final p in s.points) {
        minX = math.min(minX, p.dx);
        minY = math.min(minY, p.dy);
        maxX = math.max(maxX, p.dx);
        maxY = math.max(maxY, p.dy);
      }
    }
    final contentW = math.max(120.0, maxX - minX);
    final contentH = math.max(80.0, maxY - minY);
    final availW = math.max(160.0, viewportSize.width - 160);
    final availH = math.max(120.0, viewportSize.height - 140);
    final nextZoom = math.min(availW / contentW, availH / contentH).clamp(0.3, 1.35);
    final cx = (minX + maxX) / 2;
    final cy = (minY + maxY) / 2;
    final nextPan = Offset(
      viewportSize.width / 2 - cx * nextZoom,
      viewportSize.height / 2 - cy * nextZoom,
    );
    setViewport(nextZoom, nextPan);
  }

  Offset screenToWorld(Offset screen) => (screen - pan) / zoom;
  Offset worldToScreen(Offset world) => world * zoom + pan;

  void invalidateTextCaches() {
    _textCache.clear();
    graphRevision++;
    activeRevision++;
    notifyListeners();
  }

  TextPainter getCachedTextPainter(BoardNode n, {Color? overrideColor}) {
    final maxW = math.max(24.0, n.w - nodePaddingH * 2);
    final align = switch (n.style.align) {
      'left' => TextAlign.left,
      'right' => TextAlign.right,
      _ => TextAlign.center,
    };
    final color = overrideColor ?? parseBoardColor(n.style.textColor, fallback: BoardColors.inkLight);
    final key = '${n.id}|${n.text}|${maxW.toStringAsFixed(1)}|${n.style.fontSize}|${n.style.align}|${color.toARGB32()}|${n.kind == 'mind_root'}';
    final cached = _textCache[n.id];
    if (cached != null && cached.key == key) return cached.painter;
    final tp = TextPainter(
      text: TextSpan(
        text: n.text,
        style: TextStyle(
          fontSize: n.style.fontSize,
          height: 1.36,
          fontWeight: n.kind == 'mind_root' ? FontWeight.w600 : FontWeight.w400,
          color: color,
          fontFamilyFallback: const ['Microsoft YaHei', 'PingFang SC', 'Noto Sans SC'],
        ),
      ),
      textAlign: align,
      textDirection: TextDirection.ltr,
    )..layout(maxWidth: maxW);
    _textCache[n.id] = _CachedText(key: key, painter: tp);
    return tp;
  }

  // --- Hit Testing ---

  BoardNode? hitTestNode(Offset world) {
    final hidden = collapsedDescendants(_snapshot);
    final ordered = _snapshot.nodes
        .where((n) => !hidden.contains(n.id))
        .toList()
      ..sort((a, b) {
        final af = a.kind == 'frame' ? 0 : 1;
        final bf = b.kind == 'frame' ? 0 : 1;
        if (af != bf) return af.compareTo(bf);
        return a.z.compareTo(b.z);
      });
    for (var i = ordered.length - 1; i >= 0; i--) {
      final n = ordered[i];
      if (n.kind == 'frame') {
        // Hit frame only on its title bar or 12px border so clicks inside hit children.
        final outer = n.rect.inflate(6 / zoom);
        final inner = n.rect.deflate(14 / zoom);
        final titleBar = Rect.fromLTWH(n.x, n.y, n.w, 30);
        if (titleBar.contains(world) || (outer.contains(world) && !inner.contains(world))) {
          return n;
        }
      } else if (n.rect.contains(world)) {
        return n;
      }
    }
    return null;
  }

  String? hitTestCollapseBadge(Offset world) {
    final hidden = collapsedDescendants(_snapshot);
    final hasChildren = <String>{};
    for (final n in _snapshot.nodes) {
      if (n.parentId != null) hasChildren.add(n.parentId!);
    }
    final hitRadius = 11.0 / zoom;
    for (final n in _snapshot.nodes) {
      if (hidden.contains(n.id) || !hasChildren.contains(n.id)) continue;
      if (n.kind != 'mind_root' && n.kind != 'mind_node') continue;
      final badgeCenter = Offset(n.x + n.w + 10, n.y + n.h / 2);
      if ((world - badgeCenter).distance <= hitRadius) {
        return n.id;
      }
    }
    return null;
  }

  (String, String)? hitTestAnchorPort(Offset world) {
    final candidates = <String>{
      ...selectedNodeIds,
      ?hoveredNodeId,
    };
    final hitRadius = 9.0 / zoom;
    for (final id in candidates) {
      final n = nodeById[id];
      if (n == null || n.kind == 'frame') continue;
      for (final side in const ['t', 'r', 'b', 'l']) {
        final pt = nodeAnchorOffset(n, side);
        if ((world - pt).distance <= hitRadius) {
          return (id, side);
        }
      }
    }
    return null;
  }

  (String, ResizeHandle)? hitTestResizeHandle(Offset world) {
    if (selectedNodeIds.length != 1) return null;
    final n = nodeById[selectedNodeIds.first];
    if (n == null) return null;
    final r = n.rect;
    final tol = 8.0 / zoom;
    final handles = <(ResizeHandle, Offset)>[
      (ResizeHandle.nw, r.topLeft),
      (ResizeHandle.n, r.topCenter),
      (ResizeHandle.ne, r.topRight),
      (ResizeHandle.e, r.centerRight),
      (ResizeHandle.se, r.bottomRight),
      (ResizeHandle.s, r.bottomCenter),
      (ResizeHandle.sw, r.bottomLeft),
      (ResizeHandle.w, r.centerLeft),
    ];
    for (final (h, pt) in handles) {
      if ((world - pt).distance <= tol) return (n.id, h);
    }
    return null;
  }

  BoardEdge? hitTestEdge(Offset world) {
    final hidden = collapsedDescendants(_snapshot);
    final tol = 8.0 / zoom;
    for (final e in _snapshot.edges.reversed) {
      if (hidden.contains(e.fromId) || hidden.contains(e.toId)) continue;
      final from = nodeById[e.fromId];
      final to = nodeById[e.toId];
      if (from == null || to == null) continue;
      final pts = edgeDisplayPoints(e);
      if (pts.length < 2) continue;
      for (var i = 0; i < pts.length - 1; i++) {
        if (_distToSegment(world, pts[i], pts[i + 1]) <= tol) {
          return e;
        }
      }
    }
    return null;
  }

  List<Offset> edgeDisplayPoints(BoardEdge e) {
    final cached = routedEdgeCache[e.id];
    if (cached != null && cached.length >= 2) return cached;
    final from = nodeById[e.fromId];
    final to = nodeById[e.toId];
    if (from == null || to == null) return const [];
    final (start, nf, end, nt) = resolveEdgeEndpoints(e, from, to);
    if (e.routing == 'straight') {
      return [start, ...e.waypoints, end];
    }
    if (e.routing == 'curved' && e.waypoints.isEmpty) {
      final dist = ((end - start).distance * 0.42).clamp(28.0, 160.0);
      final c1 = start + nf * dist;
      final c2 = end + nt * dist;
      // Sample cubic for hit-testing; painter draws exact cubic.
      return [
        for (var i = 0; i <= 8; i++) _cubicAt(start, c1, c2, end, i / 8.0),
      ];
    }
    if (e.waypoints.isNotEmpty) {
      return [start, ...e.waypoints, end];
    }
    const stub = 18.0;
    final sOut = start + nf * stub;
    final eOut = end + nt * stub;
    final horizStart = nf.dx.abs() > 0.5;
    final horizEnd = nt.dx.abs() > 0.5;
    if (horizStart && horizEnd) {
      final mx = (sOut.dx + eOut.dx) / 2;
      return [start, sOut, Offset(mx, sOut.dy), Offset(mx, eOut.dy), eOut, end];
    }
    if (!horizStart && !horizEnd) {
      final my = (sOut.dy + eOut.dy) / 2;
      return [start, sOut, Offset(sOut.dx, my), Offset(eOut.dx, my), eOut, end];
    }
    if (horizStart) {
      return [start, sOut, Offset(eOut.dx, sOut.dy), eOut, end];
    }
    return [start, sOut, Offset(sOut.dx, eOut.dy), eOut, end];
  }

  // --- Creation Helpers ---

  BoardNode addNodeAt(
    Offset worldCenter, {
    required String kind,
    String? icon,
    String? text,
    double? width,
    double? height,
    String? fill,
    String? stroke,
  }) {
    _pushUndo();
    final meta = kBoardShapes.firstWhere(
      (s) => s.kind == kind,
      orElse: () => kBoardShapes.first,
    );
    final w = width ?? (kind == 'frame' ? 380.0 : kind == 'text' ? 180.0 : kind == 'icon_card' ? 128.0 : meta.defaultW);
    final h = height ?? (kind == 'frame' ? 240.0 : kind == 'text' ? 40.0 : kind == 'icon_card' ? 86.0 : meta.defaultH);
    final maxZ = _snapshot.nodes.fold<int>(0, (m, n) => math.max(m, n.z));
    final node = BoardNode(
      id: _nextId('n'),
      kind: kind,
      x: worldCenter.dx - w / 2,
      y: worldCenter.dy - h / 2,
      w: w,
      h: h,
      z: kind == 'frame' ? 0 : maxZ + 1,
      text: text ??
          (kind == 'frame'
              ? '分区 Frame'
              : kind == 'text'
                  ? '输入文字'
                  : kind == 'sticky'
                      ? '便签内容'
                      : kind == 'mind_root'
                          ? '中心主题'
                          : meta.label),
      icon: icon,
      style: BoardNodeStyle(
        fill: fill ??
            (kind == 'text'
                ? 'transparent'
                : kind == 'frame'
                    ? '#F7F8FA'
                    : kind == 'mind_root'
                        ? '#3370FF'
                        : meta.defaultFill),
        stroke: stroke ??
            (kind == 'text'
                ? 'transparent'
                : kind == 'frame'
                    ? '#C2C8D0'
                    : kind == 'mind_root'
                        ? '#245BDB'
                        : meta.defaultStroke),
        dashed: kind == 'frame',
        textColor: kind == 'mind_root' ? '#FFFFFF' : '#1F2329',
        align: kind == 'sticky' || kind == 'text' || kind == 'frame' ? 'left' : 'center',
      ),
    );
    _snapshot.nodes.add(node);
    _rebuildIndices();
    selectedNodeIds
      ..clear()
      ..add(node.id);
    selectedEdgeId = null;
    tool = BoardTool.select;
    graphRevision++;
    activeRevision++;
    notifyListeners();
    return node;
  }

  // --- Selection, Dragging with Smart Guides, Resizing, Marquee, Pen ---

  void selectNode(String id, {bool additive = false}) {
    selectedEdgeId = null;
    if (additive) {
      if (!selectedNodeIds.remove(id)) selectedNodeIds.add(id);
    } else {
      selectedNodeIds
        ..clear()
        ..add(id);
    }
    activeRevision++;
    notifyListeners();
  }

  void selectEdge(String id) {
    selectedNodeIds.clear();
    selectedEdgeId = id;
    activeRevision++;
    notifyListeners();
  }

  void clearSelection() {
    if (selectedNodeIds.isEmpty && selectedEdgeId == null) return;
    selectedNodeIds.clear();
    selectedEdgeId = null;
    activeRevision++;
    notifyListeners();
  }

  void selectAll() {
    final hidden = collapsedDescendants(_snapshot);
    selectedNodeIds
      ..clear()
      ..addAll(_snapshot.nodes.where((n) => !hidden.contains(n.id)).map((n) => n.id));
    selectedEdgeId = null;
    activeRevision++;
    notifyListeners();
  }

  void setHoveredNode(String? id) {
    if (hoveredNodeId == id) return;
    hoveredNodeId = id;
    _emitActiveOnly();
  }

  void beginNodeDrag(String hitId, {bool additive = false}) {
    if (!selectedNodeIds.contains(hitId)) {
      selectNode(hitId, additive: additive);
    }
    _gestureBeforeSnapshot = _snapshot.deepCopy();
    _dragStartPositions.clear();
    for (final id in selectedNodeIds) {
      final n = nodeById[id];
      if (n != null) _dragStartPositions[id] = Offset(n.x, n.y);
    }
    _dragTotalDelta = Offset.zero;
    isDraggingNodes = true;
    activeGuides = const [];
    // Exclude dragged nodes from static picture ONCE at drag start.
    graphRevision++;
    _emitCanvas(staticDirty: true, rebuildChrome: true);
  }

  void updateNodeDrag(Offset worldDelta, {bool snap = true}) {
    if (!isDraggingNodes || selectedNodeIds.isEmpty) return;
    _dragTotalDelta += worldDelta;
    for (final id in selectedNodeIds) {
      final n = nodeById[id];
      final origin = _dragStartPositions[id];
      if (n != null && origin != null) {
        n.x = origin.dx + _dragTotalDelta.dx;
        n.y = origin.dy + _dragTotalDelta.dy;
      }
    }
    // Smart alignment snapping on raw unsnapped coordinates so slow mouse
    // movement never gets trapped on an alignment guide.
    if (snap && selectedNodeIds.length == 1) {
      _applySmartSnap(nodeById[selectedNodeIds.first]!);
    } else {
      activeGuides = const [];
    }
    // Clear cached routes for incident edges only.
    for (final id in selectedNodeIds) {
      for (final e in edgesByNode[id] ?? const <BoardEdge>[]) {
        routedEdgeCache.remove(e.id);
      }
    }
    // ONLY repaint active overlay RenderObject: 0 static repaints, 0 widget rebuilds!
    _emitActiveOnly();
  }

  void _applySmartSnap(BoardNode moving) {
    final guides = <AlignmentGuide>[];
    final hidden = collapsedDescendants(_snapshot);
    final threshold = (_snapThreshold / zoom).clamp(2.5, _snapThreshold);
    double? snapDx;
    double? snapDy;
    var bestDx = threshold;
    var bestDy = threshold;
    AlignmentGuide? vGuide;
    AlignmentGuide? hGuide;

    final mXs = [moving.x, moving.x + moving.w / 2, moving.x + moving.w];
    final mYs = [moving.y, moving.y + moving.h / 2, moving.y + moving.h];

    for (final other in _snapshot.nodes) {
      if (other.id == moving.id || hidden.contains(other.id)) continue;
      // Match corresponding edges only (left↔left, center↔center, right↔right;
      // top↔top, center↔center, bottom↔bottom) to avoid bumpy false snaps.
      final oXs = [other.x, other.x + other.w / 2, other.x + other.w];
      for (var i = 0; i < 3; i++) {
        final d = (oXs[i] - mXs[i]).abs();
        if (d < bestDx) {
          bestDx = d;
          snapDx = oXs[i] - mXs[i];
          vGuide = AlignmentGuide(
            vertical: true,
            coord: oXs[i],
            start: math.min(moving.y, other.y) - 16,
            end: math.max(moving.y + moving.h, other.y + other.h) + 16,
          );
        }
      }
      final oYs = [other.y, other.y + other.h / 2, other.y + other.h];
      for (var i = 0; i < 3; i++) {
        final d = (oYs[i] - mYs[i]).abs();
        if (d < bestDy) {
          bestDy = d;
          snapDy = oYs[i] - mYs[i];
          hGuide = AlignmentGuide(
            vertical: false,
            coord: oYs[i],
            start: math.min(moving.x, other.x) - 16,
            end: math.max(moving.x + moving.w, other.x + other.w) + 16,
          );
        }
      }
    }

    if (snapDx != null) {
      moving.x += snapDx;
      if (vGuide != null) guides.add(vGuide);
    }
    if (snapDy != null) {
      moving.y += snapDy;
      if (hGuide != null) guides.add(hGuide);
    }
    activeGuides = guides;
  }

  void endNodeDrag() {
    if (!isDraggingNodes) return;
    isDraggingNodes = false;
    activeGuides = const [];
    final moved = _dragTotalDelta.distance > 0.5;
    _dragStartPositions.clear();
    _dragTotalDelta = Offset.zero;
    if (_gestureBeforeSnapshot != null) {
      if (moved) _pushUndo(_gestureBeforeSnapshot);
      _gestureBeforeSnapshot = null;
    }
    graphRevision++;
    _emitCanvas(staticDirty: true, rebuildChrome: true);
  }

  void beginResize(String nodeId, ResizeHandle handle) {
    _gestureBeforeSnapshot = _snapshot.deepCopy();
    resizingNodeId = nodeId;
    activeResizeHandle = handle;
    graphRevision++;
    activeRevision++;
    notifyListeners();
  }

  void updateResize(Offset worldDelta) {
    final id = resizingNodeId;
    final h = activeResizeHandle;
    if (id == null || h == null) return;
    final n = nodeById[id];
    if (n == null) return;
    const minW = 56.0;
    const minH = 34.0;
    switch (h) {
      case ResizeHandle.e:
        n.w = math.max(minW, n.w + worldDelta.dx);
      case ResizeHandle.s:
        n.h = math.max(minH, n.h + worldDelta.dy);
      case ResizeHandle.se:
        n.w = math.max(minW, n.w + worldDelta.dx);
        n.h = math.max(minH, n.h + worldDelta.dy);
      case ResizeHandle.w:
        final nw = math.max(minW, n.w - worldDelta.dx);
        n.x += n.w - nw;
        n.w = nw;
      case ResizeHandle.n:
        final nh = math.max(minH, n.h - worldDelta.dy);
        n.y += n.h - nh;
        n.h = nh;
      case ResizeHandle.nw:
        final nw = math.max(minW, n.w - worldDelta.dx);
        final nh = math.max(minH, n.h - worldDelta.dy);
        n.x += n.w - nw;
        n.y += n.h - nh;
        n.w = nw;
        n.h = nh;
      case ResizeHandle.ne:
        final nh = math.max(minH, n.h - worldDelta.dy);
        n.y += n.h - nh;
        n.h = nh;
        n.w = math.max(minW, n.w + worldDelta.dx);
      case ResizeHandle.sw:
        final nw = math.max(minW, n.w - worldDelta.dx);
        n.x += n.w - nw;
        n.w = nw;
        n.h = math.max(minH, n.h + worldDelta.dy);
    }
    _textCache.remove(id);
    for (final e in edgesByNode[id] ?? const <BoardEdge>[]) {
      routedEdgeCache.remove(e.id);
    }
    _emitActiveOnly();
  }

  void endResize() {
    if (resizingNodeId == null) return;
    resizingNodeId = null;
    activeResizeHandle = null;
    if (_gestureBeforeSnapshot != null) {
      _pushUndo(_gestureBeforeSnapshot);
      _gestureBeforeSnapshot = null;
    }
    graphRevision++;
    activeRevision++;
    notifyListeners();
  }

  void updateMarquee(Offset startWorld, Offset currentWorld) {
    marqueeWorldRect = Rect.fromPoints(startWorld, currentWorld);
    final hidden = collapsedDescendants(_snapshot);
    selectedNodeIds.clear();
    selectedEdgeId = null;
    for (final n in _snapshot.nodes) {
      if (hidden.contains(n.id)) continue;
      if (marqueeWorldRect!.overlaps(n.rect)) {
        selectedNodeIds.add(n.id);
      }
    }
    _emitActiveOnly();
  }

  void endMarquee() {
    if (marqueeWorldRect == null) return;
    marqueeWorldRect = null;
    _emitCanvas(staticDirty: false, rebuildChrome: true);
  }

  void beginConnectorDrag(String fromId, String fromAnchor, Offset worldPt) {
    final from = nodeById[fromId];
    if (from == null) return;
    pendingConnector = PendingConnector(
      fromNodeId: fromId,
      fromAnchor: fromAnchor,
      fromWorld: nodeAnchorOffset(from, fromAnchor),
      currentWorld: worldPt,
    );
    _emitCanvas(staticDirty: false, rebuildChrome: true);
  }

  void updateConnectorDrag(Offset worldPt) {
    final pc = pendingConnector;
    if (pc == null) return;
    pc.currentWorld = worldPt;
    final hit = hitTestNode(worldPt);
    if (hit != null && hit.id != pc.fromNodeId && hit.kind != 'frame') {
      pc.targetNodeId = hit.id;
      // Pick closest anchor port on target node.
      var bestSide = 'l';
      var bestDist = double.infinity;
      for (final side in const ['t', 'r', 'b', 'l']) {
        final d = (nodeAnchorOffset(hit, side) - worldPt).distance;
        if (d < bestDist) {
          bestDist = d;
          bestSide = side;
        }
      }
      pc.targetAnchor = bestSide;
      pc.currentWorld = nodeAnchorOffset(hit, bestSide);
    } else {
      pc.targetNodeId = null;
      pc.targetAnchor = null;
    }
    _emitActiveOnly();
  }

  void endConnectorDrag() {
    final pc = pendingConnector;
    if (pc == null) return;
    pendingConnector = null;
    if (pc.targetNodeId != null) {
      _pushUndo();
      final edge = BoardEdge(
        id: _nextId('e'),
        fromId: pc.fromNodeId,
        toId: pc.targetNodeId!,
        fromAnchor: pc.fromAnchor,
        toAnchor: pc.targetAnchor ?? 'auto',
        routing: activeRouting,
      );
      _snapshot.edges.add(edge);
      _rebuildIndices();
      selectedNodeIds.clear();
      selectedEdgeId = edge.id;
      graphRevision++;
    }
    activeRevision++;
    notifyListeners();
  }

  void beginPenStroke(Offset worldPt) {
    _gestureBeforeSnapshot = _snapshot.deepCopy();
    activeStroke = BoardStroke(
      id: _nextId('s'),
      color: activePenColor,
      width: activePenWidth,
      points: [worldPt],
    );
    activeRevision++;
    notifyListeners();
  }

  void updatePenStroke(Offset worldPt) {
    final st = activeStroke;
    if (st == null) return;
    if (st.points.isEmpty || (st.points.last - worldPt).distance >= 3.0) {
      st.points.add(worldPt);
      _emitActiveOnly();
    }
  }

  void endPenStroke() {
    final st = activeStroke;
    if (st == null) return;
    activeStroke = null;
    if (st.points.length >= 2) {
      _pushUndo(_gestureBeforeSnapshot);
      _snapshot.strokes.add(st);
      graphRevision++;
    }
    _gestureBeforeSnapshot = null;
    activeRevision++;
    notifyListeners();
  }

  // --- Inline Text Editing (Node & Edge) ---

  void startEditingNode(String nodeId) {
    if (!nodeById.containsKey(nodeId)) return;
    editingEdgeId = null;
    editingNodeId = nodeId;
    selectedNodeIds
      ..clear()
      ..add(nodeId);
    selectedEdgeId = null;
    _gestureBeforeSnapshot = _snapshot.deepCopy();
    graphRevision++;
    activeRevision++;
    notifyListeners();
  }

  void updateEditingNodeText(String nodeId, String nextText) {
    final n = nodeById[nodeId];
    if (n == null || n.text == nextText) return;
    n.text = nextText;
    _textCache.remove(nodeId);
    final tp = getCachedTextPainter(n);
    final minH = n.kind == 'mind_root' ? 48.0 : n.kind == 'mind_node' ? 40.0 : n.kind == 'icon_card' ? 82.0 : 44.0;
    final iconExtra = n.icon != null ? 30.0 : 0.0;
    final neededH = math.max(minH, tp.height + nodePaddingV * 2 + iconExtra);
    if (neededH > n.h + 1.0) {
      n.h = neededH;
    }
    for (final e in edgesByNode[nodeId] ?? const <BoardEdge>[]) {
      routedEdgeCache.remove(e.id);
    }
    activeRevision++;
    notifyListeners();
  }

  void finishEditing() {
    if (editingNodeId == null && editingEdgeId == null) return;
    final editedNode = editingNodeId != null ? nodeById[editingNodeId!] : null;
    editingNodeId = null;
    editingEdgeId = null;
    if (_gestureBeforeSnapshot != null) {
      _pushUndo(_gestureBeforeSnapshot);
      _gestureBeforeSnapshot = null;
    }
    if (editedNode != null &&
        (editedNode.kind == 'mind_root' || editedNode.kind == 'mind_node')) {
      layoutMindmapLocal(_snapshot);
    }
    graphRevision++;
    activeRevision++;
    notifyListeners();
  }

  void setEdgeLabel(String edgeId, String label) {
    final idx = _snapshot.edges.indexWhere((e) => e.id == edgeId);
    if (idx < 0) return;
    _pushUndo();
    _snapshot.edges[idx].label = label;
    graphRevision++;
    activeRevision++;
    notifyListeners();
  }

  // --- Mind Map Keyboard Workflow ---

  BoardNode? addMindChild(String parentId) {
    final parent = nodeById[parentId];
    if (parent == null) return null;
    _pushUndo();
    parent.collapsed = false;
    final existingKids = _snapshot.nodes
        .where((n) => n.parentId == parentId)
        .toList();
    final nextRank = existingKids.fold<int>(0, (m, n) => math.max(m, n.rank + 1));

    // Pick branch palette based on top-level branch index or parent's stroke.
    String fill;
    String stroke;
    if (parent.kind == 'mind_root' || parent.parentId == null) {
      final pair = kMindBranchPalette[existingKids.length % kMindBranchPalette.length];
      fill = pair.$1;
      stroke = pair.$2;
    } else {
      fill = '#FFFFFF';
      stroke = parent.style.stroke;
    }

    final child = BoardNode(
      id: _nextId('m'),
      kind: 'mind_node',
      x: parent.x + parent.w + 68,
      y: parent.y,
      w: 148,
      h: 40,
      z: _snapshot.nodes.length + 2,
      parentId: parentId,
      rank: nextRank,
      text: '子主题',
      style: BoardNodeStyle(
        fill: fill,
        stroke: stroke,
        strokeWidth: 1.5,
        align: 'left',
      ),
    );
    _snapshot.nodes.add(child);
    _snapshot.edges.add(
      BoardEdge(
        id: _nextId('me'),
        fromId: parentId,
        toId: child.id,
        fromAnchor: 'r',
        toAnchor: 'l',
        routing: 'curved',
        arrowEnd: 'none',
        style: BoardEdgeStyle(stroke: stroke, strokeWidth: 1.7),
      ),
    );
    _rebuildIndices();
    layoutMindmapLocal(_snapshot);
    routedEdgeCache.clear();
    selectedNodeIds
      ..clear()
      ..add(child.id);
    selectedEdgeId = null;
    graphRevision++;
    activeRevision++;
    notifyListeners();
    return child;
  }

  BoardNode? addMindSibling(String nodeId) {
    final node = nodeById[nodeId];
    if (node == null) return null;
    if (node.parentId == null || !nodeById.containsKey(node.parentId)) {
      return addMindChild(nodeId);
    }
    return addMindChild(node.parentId!);
  }

  void toggleCollapse(String nodeId) {
    final n = nodeById[nodeId];
    if (n == null) return;
    _pushUndo();
    n.collapsed = !n.collapsed;
    layoutMindmapLocal(_snapshot);
    routedEdgeCache.clear();
    graphRevision++;
    activeRevision++;
    notifyListeners();
  }

  void relayoutMindmap() {
    _pushUndo();
    layoutMindmapLocal(_snapshot);
    routedEdgeCache.clear();
    graphRevision++;
    activeRevision++;
    notifyListeners();
  }

  // --- Styling, Z-Order, Copy/Paste/Duplicate, Delete ---

  void updateSelectedNodesStyle({
    String? fill,
    String? stroke,
    bool? dashed,
    double? fontSize,
    String? textColor,
    String? kind,
  }) {
    if (selectedNodeIds.isEmpty) return;
    _pushUndo();
    for (final id in selectedNodeIds) {
      final n = nodeById[id];
      if (n == null) continue;
      if (fill != null) {
        n.style.fill = fill;
        // Auto-contrast text color for dark fills (#3370FF / #1F2329).
        if (fill == '#3370FF' || fill == '#1F2329') {
          n.style.textColor = '#FFFFFF';
        } else if (n.style.textColor == '#FFFFFF' && textColor == null) {
          n.style.textColor = '#1F2329';
        }
      }
      if (stroke != null) n.style.stroke = stroke;
      if (dashed != null) n.style.dashed = dashed;
      if (fontSize != null) n.style.fontSize = fontSize;
      if (textColor != null) n.style.textColor = textColor;
      if (kind != null) n.kind = kind;
      _textCache.remove(id);
    }
    graphRevision++;
    activeRevision++;
    notifyListeners();
  }

  void updateSelectedEdge({
    String? routing,
    String? stroke,
    bool? dashed,
    String? arrowEnd,
    String? arrowStart,
  }) {
    final id = selectedEdgeId;
    if (id == null) return;
    final idx = _snapshot.edges.indexWhere((e) => e.id == id);
    if (idx < 0) return;
    _pushUndo();
    final e = _snapshot.edges[idx];
    if (routing != null) {
      e.routing = routing;
      e.waypoints.clear();
    }
    if (stroke != null) e.style.stroke = stroke;
    if (dashed != null) e.style.dashed = dashed;
    if (arrowEnd != null) e.arrowEnd = arrowEnd;
    if (arrowStart != null) e.arrowStart = arrowStart;
    routedEdgeCache.remove(id);
    graphRevision++;
    activeRevision++;
    notifyListeners();
  }

  void bringSelectedToFront() {
    if (selectedNodeIds.isEmpty) return;
    _pushUndo();
    var maxZ = _snapshot.nodes.fold<int>(0, (m, n) => math.max(m, n.z));
    for (final id in selectedNodeIds) {
      final n = nodeById[id];
      if (n != null) {
        maxZ++;
        n.z = maxZ;
      }
    }
    graphRevision++;
    activeRevision++;
    notifyListeners();
  }

  void sendSelectedToBack() {
    if (selectedNodeIds.isEmpty) return;
    _pushUndo();
    var minZ = _snapshot.nodes.fold<int>(1, (m, n) => math.min(m, n.z));
    for (final id in selectedNodeIds) {
      final n = nodeById[id];
      if (n != null) {
        minZ--;
        n.z = minZ;
      }
    }
    graphRevision++;
    activeRevision++;
    notifyListeners();
  }

  void copySelection() {
    if (selectedNodeIds.isEmpty) return;
    _clipboardNodes = [
      for (final id in selectedNodeIds)
        if (nodeById[id] != null) nodeById[id]!.copy(),
    ];
    _clipboardEdges = [
      for (final e in _snapshot.edges)
        if (selectedNodeIds.contains(e.fromId) &&
            selectedNodeIds.contains(e.toId))
          e.copy(),
    ];
  }

  void pasteSelection({Offset offset = const Offset(28, 28)}) {
    if (_clipboardNodes.isEmpty) return;
    _pushUndo();
    final idMap = <String, String>{};
    final newIds = <String>[];
    var maxZ = _snapshot.nodes.fold<int>(0, (m, n) => math.max(m, n.z));
    for (final src in _clipboardNodes) {
      final nid = _nextId('n');
      idMap[src.id] = nid;
      maxZ++;
      final clone = src.copy()
        ..x += offset.dx
        ..y += offset.dy
        ..z = maxZ
        ..parentId = null;
      _snapshot.nodes.add(
        BoardNode(
          id: nid,
          kind: clone.kind,
          x: clone.x,
          y: clone.y,
          w: clone.w,
          h: clone.h,
          rot: clone.rot,
          z: clone.z,
          text: clone.text,
          icon: clone.icon,
          style: clone.style,
        ),
      );
      newIds.add(nid);
    }
    for (final srcEdge in _clipboardEdges) {
      final nf = idMap[srcEdge.fromId];
      final nt = idMap[srcEdge.toId];
      if (nf != null && nt != null) {
        _snapshot.edges.add(
          BoardEdge(
            id: _nextId('e'),
            fromId: nf,
            toId: nt,
            fromAnchor: srcEdge.fromAnchor,
            toAnchor: srcEdge.toAnchor,
            routing: srcEdge.routing,
            arrowStart: srcEdge.arrowStart,
            arrowEnd: srcEdge.arrowEnd,
            label: srcEdge.label,
            style: srcEdge.style.copy(),
          ),
        );
      }
    }
    _rebuildIndices();
    selectedNodeIds
      ..clear()
      ..addAll(newIds);
    selectedEdgeId = null;
    graphRevision++;
    activeRevision++;
    notifyListeners();
  }

  void duplicateSelection() {
    copySelection();
    pasteSelection();
  }

  void deleteSelection() {
    if (selectedNodeIds.isEmpty && selectedEdgeId == null) return;
    _pushUndo();
    if (selectedEdgeId != null) {
      _snapshot.edges.removeWhere((e) => e.id == selectedEdgeId);
      routedEdgeCache.remove(selectedEdgeId);
      selectedEdgeId = null;
    }
    if (selectedNodeIds.isNotEmpty) {
      // Also delete collapsed descendants of selected mind-map nodes.
      final toDelete = Set<String>.from(selectedNodeIds);
      var grew = true;
      while (grew) {
        grew = false;
        for (final n in _snapshot.nodes) {
          if (n.parentId != null &&
              toDelete.contains(n.parentId) &&
              toDelete.add(n.id)) {
            grew = true;
          }
        }
      }
      _snapshot.nodes.removeWhere((n) => toDelete.contains(n.id));
      _snapshot.edges.removeWhere(
        (e) => toDelete.contains(e.fromId) || toDelete.contains(e.toId),
      );
      for (final id in toDelete) {
        _textCache.remove(id);
      }
      selectedNodeIds.clear();
    }
    routedEdgeCache.clear();
    _rebuildIndices();
    graphRevision++;
    activeRevision++;
    notifyListeners();
  }

  void applyRoutedEdges(Map<String, List<Offset>> routes) {
    routedEdgeCache
      ..clear()
      ..addAll(routes);
    graphRevision++;
    activeRevision++;
    notifyListeners();
  }
}

class _PaintSignal extends ChangeNotifier {
  void bump() => notifyListeners();
}

class _CachedText {
  const _CachedText({required this.key, required this.painter});
  final String key;
  final TextPainter painter;
}

Offset _cubicAt(Offset p0, Offset p1, Offset p2, Offset p3, double t) {
  final mt = 1 - t;
  return p0 * (mt * mt * mt) +
      p1 * (3 * mt * mt * t) +
      p2 * (3 * mt * t * t) +
      p3 * (t * t * t);
}

double _distToSegment(Offset p, Offset a, Offset b) {
  final ab = b - a;
  final len2 = ab.dx * ab.dx + ab.dy * ab.dy;
  if (len2 < 1e-4) return (p - a).distance;
  final t = (((p.dx - a.dx) * ab.dx + (p.dy - a.dy) * ab.dy) / len2).clamp(0.0, 1.0);
  final proj = Offset(a.dx + ab.dx * t, a.dy + ab.dy * t);
  return (p - proj).distance;
}
