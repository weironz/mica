// Core data models, parametric shape catalog, Feishu-inspired palette,
// and pure-Dart fallback helpers for Mica's infinite whiteboard & mind map.
import 'dart:convert';
import 'dart:math' as math;

import 'package:flutter/material.dart';

const int kBoardSchemaVersion = 1;

enum BoardTool {
  select,
  hand,
  shape,
  icon,
  sticky,
  text,
  connector,
  pen,
  frame,
  mindmap,
}

class BoardShapeMeta {
  const BoardShapeMeta({
    required this.kind,
    required this.label,
    required this.category,
    required this.defaultW,
    required this.defaultH,
    required this.defaultFill,
    required this.defaultStroke,
  });

  final String kind;
  final String label;
  final String category; // 'basic' | 'flowchart'
  final double defaultW;
  final double defaultH;
  final String defaultFill;
  final String defaultStroke;
}

const List<BoardShapeMeta> kBoardShapes = [
  // Basic whiteboard shapes
  BoardShapeMeta(
    kind: 'rounded',
    label: '圆角矩形',
    category: 'basic',
    defaultW: 168,
    defaultH: 58,
    defaultFill: '#E8F3FF',
    defaultStroke: '#3370FF',
  ),
  BoardShapeMeta(
    kind: 'rect',
    label: '矩形',
    category: 'basic',
    defaultW: 160,
    defaultH: 58,
    defaultFill: '#FFFFFF',
    defaultStroke: '#646A73',
  ),
  BoardShapeMeta(
    kind: 'ellipse',
    label: '椭圆',
    category: 'basic',
    defaultW: 144,
    defaultH: 80,
    defaultFill: '#EDF7ED',
    defaultStroke: '#2DA44E',
  ),
  BoardShapeMeta(
    kind: 'triangle',
    label: '三角形',
    category: 'basic',
    defaultW: 132,
    defaultH: 96,
    defaultFill: '#FFF7E6',
    defaultStroke: '#D9822B',
  ),
  BoardShapeMeta(
    kind: 'hexagon',
    label: '六边形',
    category: 'basic',
    defaultW: 164,
    defaultH: 66,
    defaultFill: '#F5F0FF',
    defaultStroke: '#8250DF',
  ),
  BoardShapeMeta(
    kind: 'cloud',
    label: '云朵',
    category: 'basic',
    defaultW: 176,
    defaultH: 96,
    defaultFill: '#E8F3FF',
    defaultStroke: '#3370FF',
  ),
  // Flowchart standard shapes
  BoardShapeMeta(
    kind: 'pill',
    label: '开始 / 结束',
    category: 'flowchart',
    defaultW: 140,
    defaultH: 46,
    defaultFill: '#EDF7ED',
    defaultStroke: '#2DA44E',
  ),
  BoardShapeMeta(
    kind: 'diamond',
    label: '判断条件',
    category: 'flowchart',
    defaultW: 156,
    defaultH: 86,
    defaultFill: '#FFF7E6',
    defaultStroke: '#D9822B',
  ),
  BoardShapeMeta(
    kind: 'parallelogram',
    label: '输入 / 输出',
    category: 'flowchart',
    defaultW: 168,
    defaultH: 58,
    defaultFill: '#E6F7F7',
    defaultStroke: '#0E8A8A',
  ),
  BoardShapeMeta(
    kind: 'cylinder',
    label: '数据库',
    category: 'flowchart',
    defaultW: 144,
    defaultH: 76,
    defaultFill: '#F5F0FF',
    defaultStroke: '#8250DF',
  ),
  BoardShapeMeta(
    kind: 'document',
    label: '文档',
    category: 'flowchart',
    defaultW: 160,
    defaultH: 72,
    defaultFill: '#FFFFFF',
    defaultStroke: '#3370FF',
  ),
  BoardShapeMeta(
    kind: 'sticky',
    label: '便签',
    category: 'basic',
    defaultW: 180,
    defaultH: 112,
    defaultFill: '#FFF7CC',
    defaultStroke: '#E0B422',
  ),
];

const List<String> kBoardFillSwatches = [
  '#FFFFFF',
  '#E8F3FF',
  '#EDF7ED',
  '#FFF7CC',
  '#FFF7E6',
  '#F5F0FF',
  '#FFF0F0',
  '#E6F7F7',
  '#3370FF',
  '#1F2329',
  'transparent',
];

const List<String> kBoardStrokeSwatches = [
  '#646A73',
  '#3370FF',
  '#2DA44E',
  '#D9822B',
  '#8250DF',
  '#CF222E',
  '#0E8A8A',
  '#1F2329',
  'transparent',
];

const List<(String, String)> kMindBranchPalette = [
  ('#E8F3FF', '#3370FF'),
  ('#EDF7ED', '#2DA44E'),
  ('#FFF7E6', '#D9822B'),
  ('#F5F0FF', '#8250DF'),
  ('#FFF0F0', '#CF222E'),
  ('#E6F7F7', '#0E8A8A'),
];

Color parseBoardColor(String hex, {Color fallback = Colors.grey}) {
  final s = hex.trim();
  if (s.isEmpty || s == 'transparent' || s == 'none') {
    return Colors.transparent;
  }
  final clean = s.startsWith('#') ? s.substring(1) : s;
  if (clean.length == 6) {
    final v = int.tryParse(clean, radix: 16);
    if (v != null) return Color.fromARGB(255, (v >> 16) & 0xFF, (v >> 8) & 0xFF, v & 0xFF);
  } else if (clean.length == 8) {
    final v = int.tryParse(clean, radix: 16);
    if (v != null) {
      return Color.fromARGB((v >> 24) & 0xFF, (v >> 16) & 0xFF, (v >> 8) & 0xFF, v & 0xFF);
    }
  }
  return fallback;
}

/// Shared canvas/chrome colors resolved from hex strings (keeps visual color
/// policy clean while matching Rust SVG export colors 1:1).
abstract final class BoardColors {
  static final Color brand = parseBoardColor('#3370FF');
  static final Color brandWash = brand.withValues(alpha: 0.14);
  static final Color brandSelection = brand.withValues(alpha: 0.25);
  static final Color brandMarquee = brand.withValues(alpha: 0.12);
  static final Color guide = parseBoardColor('#FF3B69');
  static final Color danger = parseBoardColor('#CF222E');
  static final Color inkLight = parseBoardColor('#1F2329');
  static final Color inkDark = parseBoardColor('#E6E9EF');
  static final Color mutedLight = parseBoardColor('#646A73');
  static final Color mutedDark = parseBoardColor('#9CA3AF');
  static final Color iconLight = parseBoardColor('#374151');
  static final Color iconDark = parseBoardColor('#D0D5DD');
  static final Color borderLight = parseBoardColor('#E5E7EB');
  static final Color borderDark = parseBoardColor('#343944');
  static final Color dotLight = parseBoardColor('#D5DBE3');
  static final Color dotDark = parseBoardColor('#343944');
  static final Color canvasLight = parseBoardColor('#F7F8FA');
  static final Color canvasDark = parseBoardColor('#16191E');
  static final Color panelDark = parseBoardColor('#1F2329');
  static final Color chipLight = parseBoardColor('#F0F2F5');
  static final Color chipDark = parseBoardColor('#2A2F38');
  static final Color surfaceDark = parseBoardColor('#252830');
  static final Color defaultStroke = parseBoardColor('#8C959F');
}

class BoardNodeStyle {
  BoardNodeStyle({
    this.fill = '#FFFFFF',
    this.stroke = '#8C959F',
    this.strokeWidth = 1.5,
    this.dashed = false,
    this.fontSize = 13.5,
    this.textColor = '#1F2329',
    this.align = 'center',
  });

  String fill;
  String stroke;
  double strokeWidth;
  bool dashed;
  double fontSize;
  String textColor;
  String align;

  factory BoardNodeStyle.fromJson(Map<String, dynamic>? json) {
    if (json == null) return BoardNodeStyle();
    return BoardNodeStyle(
      fill: (json['fill'] as String?) ?? '#FFFFFF',
      stroke: (json['stroke'] as String?) ?? '#8C959F',
      strokeWidth: ((json['strokeWidth'] as num?) ?? 1.5).toDouble(),
      dashed: (json['dashed'] as bool?) ?? false,
      fontSize: ((json['fontSize'] as num?) ?? 13.5).toDouble(),
      textColor: (json['textColor'] as String?) ?? '#1F2329',
      align: (json['align'] as String?) ?? 'center',
    );
  }

  Map<String, dynamic> toJson() => {
    'fill': fill,
    'stroke': stroke,
    'strokeWidth': strokeWidth,
    'dashed': dashed,
    'fontSize': fontSize,
    'textColor': textColor,
    'align': align,
  };

  BoardNodeStyle copy() => BoardNodeStyle(
    fill: fill,
    stroke: stroke,
    strokeWidth: strokeWidth,
    dashed: dashed,
    fontSize: fontSize,
    textColor: textColor,
    align: align,
  );
}

class BoardNode {
  BoardNode({
    required this.id,
    this.kind = 'rounded',
    required this.x,
    required this.y,
    required this.w,
    required this.h,
    this.rot = 0,
    this.z = 1,
    this.parentId,
    this.rank = 0,
    this.collapsed = false,
    this.text = '',
    this.icon,
    BoardNodeStyle? style,
  }) : style = style ?? BoardNodeStyle();

  final String id;
  String kind;
  double x;
  double y;
  double w;
  double h;
  double rot;
  int z;
  String? parentId;
  int rank;
  bool collapsed;
  String text;
  String? icon;
  BoardNodeStyle style;

  Rect get rect => Rect.fromLTWH(x, y, w, h);
  Offset get center => Offset(x + w / 2, y + h / 2);

  factory BoardNode.fromJson(Map<String, dynamic> json) => BoardNode(
    id: (json['id'] as String?) ?? 'n',
    kind: (json['kind'] as String?) ?? 'rounded',
    x: ((json['x'] as num?) ?? 0).toDouble(),
    y: ((json['y'] as num?) ?? 0).toDouble(),
    w: ((json['w'] as num?) ?? 140).toDouble(),
    h: ((json['h'] as num?) ?? 52).toDouble(),
    rot: ((json['rot'] as num?) ?? 0).toDouble(),
    z: ((json['z'] as num?) ?? 1).toInt(),
    parentId: json['parentId'] as String?,
    rank: ((json['rank'] as num?) ?? 0).toInt(),
    collapsed: (json['collapsed'] as bool?) ?? false,
    text: (json['text'] as String?) ?? '',
    icon: json['icon'] as String?,
    style: BoardNodeStyle.fromJson(json['style'] as Map<String, dynamic>?),
  );

  Map<String, dynamic> toJson() => {
    'id': id,
    'kind': kind,
    'x': double.parse(x.toStringAsFixed(1)),
    'y': double.parse(y.toStringAsFixed(1)),
    'w': double.parse(w.toStringAsFixed(1)),
    'h': double.parse(h.toStringAsFixed(1)),
    'rot': double.parse(rot.toStringAsFixed(1)),
    'z': z,
    if (parentId != null) 'parentId': parentId,
    'rank': rank,
    'collapsed': collapsed,
    'text': text,
    if (icon != null) 'icon': icon,
    'style': style.toJson(),
  };

  BoardNode copy() => BoardNode(
    id: id,
    kind: kind,
    x: x,
    y: y,
    w: w,
    h: h,
    rot: rot,
    z: z,
    parentId: parentId,
    rank: rank,
    collapsed: collapsed,
    text: text,
    icon: icon,
    style: style.copy(),
  );
}

class BoardEdgeStyle {
  BoardEdgeStyle({
    this.stroke = '#646A73',
    this.strokeWidth = 1.6,
    this.dashed = false,
  });

  String stroke;
  double strokeWidth;
  bool dashed;

  factory BoardEdgeStyle.fromJson(Map<String, dynamic>? json) {
    if (json == null) return BoardEdgeStyle();
    return BoardEdgeStyle(
      stroke: (json['stroke'] as String?) ?? '#646A73',
      strokeWidth: ((json['strokeWidth'] as num?) ?? 1.6).toDouble(),
      dashed: (json['dashed'] as bool?) ?? false,
    );
  }

  Map<String, dynamic> toJson() => {
    'stroke': stroke,
    'strokeWidth': strokeWidth,
    'dashed': dashed,
  };

  BoardEdgeStyle copy() => BoardEdgeStyle(
    stroke: stroke,
    strokeWidth: strokeWidth,
    dashed: dashed,
  );
}

class BoardEdge {
  BoardEdge({
    required this.id,
    required this.fromId,
    required this.toId,
    this.fromAnchor = 'auto',
    this.toAnchor = 'auto',
    this.routing = 'orthogonal',
    this.arrowStart = 'none',
    this.arrowEnd = 'arrow',
    List<Offset>? waypoints,
    this.label = '',
    BoardEdgeStyle? style,
  }) : waypoints = waypoints ?? [],
       style = style ?? BoardEdgeStyle();

  final String id;
  String fromId;
  String toId;
  String fromAnchor;
  String toAnchor;
  String routing; // 'curved' | 'orthogonal' | 'straight'
  String arrowStart;
  String arrowEnd;
  List<Offset> waypoints;
  String label;
  BoardEdgeStyle style;

  factory BoardEdge.fromJson(Map<String, dynamic> json) {
    final rawPts = (json['waypoints'] as List?) ?? const [];
    final pts = <Offset>[];
    for (final p in rawPts) {
      if (p is List && p.length >= 2) {
        pts.add(Offset((p[0] as num).toDouble(), (p[1] as num).toDouble()));
      }
    }
    return BoardEdge(
      id: (json['id'] as String?) ?? 'e',
      fromId: (json['fromId'] as String?) ?? '',
      toId: (json['toId'] as String?) ?? '',
      fromAnchor: (json['fromAnchor'] as String?) ?? 'auto',
      toAnchor: (json['toAnchor'] as String?) ?? 'auto',
      routing: (json['routing'] as String?) ?? 'orthogonal',
      arrowStart: (json['arrowStart'] as String?) ?? 'none',
      arrowEnd: (json['arrowEnd'] as String?) ?? 'arrow',
      waypoints: pts,
      label: (json['label'] as String?) ?? '',
      style: BoardEdgeStyle.fromJson(json['style'] as Map<String, dynamic>?),
    );
  }

  Map<String, dynamic> toJson() => {
    'id': id,
    'fromId': fromId,
    'toId': toId,
    'fromAnchor': fromAnchor,
    'toAnchor': toAnchor,
    'routing': routing,
    'arrowStart': arrowStart,
    'arrowEnd': arrowEnd,
    'waypoints': [
      for (final p in waypoints)
        [
          double.parse(p.dx.toStringAsFixed(1)),
          double.parse(p.dy.toStringAsFixed(1)),
        ],
    ],
    'label': label,
    'style': style.toJson(),
  };

  BoardEdge copy() => BoardEdge(
    id: id,
    fromId: fromId,
    toId: toId,
    fromAnchor: fromAnchor,
    toAnchor: toAnchor,
    routing: routing,
    arrowStart: arrowStart,
    arrowEnd: arrowEnd,
    waypoints: List<Offset>.from(waypoints),
    label: label,
    style: style.copy(),
  );
}

class BoardStroke {
  BoardStroke({
    required this.id,
    this.color = '#3370FF',
    this.width = 2.4,
    List<Offset>? points,
  }) : points = points ?? [];

  final String id;
  String color;
  double width;
  List<Offset> points;

  factory BoardStroke.fromJson(Map<String, dynamic> json) {
    final raw = (json['points'] as List?) ?? const [];
    final pts = <Offset>[];
    for (final p in raw) {
      if (p is List && p.length >= 2) {
        pts.add(Offset((p[0] as num).toDouble(), (p[1] as num).toDouble()));
      }
    }
    return BoardStroke(
      id: (json['id'] as String?) ?? 's',
      color: (json['color'] as String?) ?? '#3370FF',
      width: ((json['width'] as num?) ?? 2.4).toDouble(),
      points: pts,
    );
  }

  Map<String, dynamic> toJson() => {
    'id': id,
    'color': color,
    'width': width,
    'points': [
      for (final p in points)
        [
          double.parse(p.dx.toStringAsFixed(1)),
          double.parse(p.dy.toStringAsFixed(1)),
        ],
    ],
  };

  BoardStroke copy() => BoardStroke(
    id: id,
    color: color,
    width: width,
    points: List<Offset>.from(points),
  );
}

class BoardSnapshot {
  BoardSnapshot({
    this.version = kBoardSchemaVersion,
    this.title = '未命名画板',
    this.mode = 'whiteboard',
    List<BoardNode>? nodes,
    List<BoardEdge>? edges,
    List<BoardStroke>? strokes,
  }) : nodes = nodes ?? [],
       edges = edges ?? [],
       strokes = strokes ?? [];

  int version;
  String title;
  String mode;
  List<BoardNode> nodes;
  List<BoardEdge> edges;
  List<BoardStroke> strokes;

  static BoardSnapshot? tryParseJson(String raw) {
    try {
      final decoded = jsonDecode(raw);
      if (decoded is! Map<String, dynamic>) return null;
      return BoardSnapshot.fromJson(decoded);
    } catch (_) {
      return null;
    }
  }

  factory BoardSnapshot.fromJson(Map<String, dynamic> json) {
    final nodes = <BoardNode>[];
    for (final item in (json['nodes'] as List?) ?? const []) {
      if (item is Map<String, dynamic>) nodes.add(BoardNode.fromJson(item));
    }
    final edges = <BoardEdge>[];
    for (final item in (json['edges'] as List?) ?? const []) {
      if (item is Map<String, dynamic>) edges.add(BoardEdge.fromJson(item));
    }
    final strokes = <BoardStroke>[];
    for (final item in (json['strokes'] as List?) ?? const []) {
      if (item is Map<String, dynamic>) strokes.add(BoardStroke.fromJson(item));
    }
    return BoardSnapshot(
      version: ((json['version'] as num?) ?? kBoardSchemaVersion).toInt(),
      title: (json['title'] as String?) ?? '未命名画板',
      mode: (json['mode'] as String?) ?? 'whiteboard',
      nodes: nodes,
      edges: edges,
      strokes: strokes,
    );
  }

  Map<String, dynamic> toJson() => {
    'version': version,
    'title': title,
    'mode': mode,
    'nodes': [for (final n in nodes) n.toJson()],
    'edges': [for (final e in edges) e.toJson()],
    'strokes': [for (final s in strokes) s.toJson()],
  };

  String toCompactJson() => jsonEncode(toJson());

  BoardSnapshot deepCopy() => BoardSnapshot(
    version: version,
    title: title,
    mode: mode,
    nodes: [for (final n in nodes) n.copy()],
    edges: [for (final e in edges) e.copy()],
    strokes: [for (final s in strokes) s.copy()],
  );

  static BoardSnapshot starter(String template) {
    switch (template) {
      case 'mindmap':
        final snap = BoardSnapshot(
          title: '思维导图',
          mode: 'mindmap',
          nodes: [
            BoardNode(
              id: 'm0',
              kind: 'mind_root',
              x: 80,
              y: 180,
              w: 168,
              h: 52,
              z: 1,
              text: '中心主题',
              style: BoardNodeStyle(
                fill: '#3370FF',
                stroke: '#245BDB',
                strokeWidth: 1.8,
                fontSize: 16,
                textColor: '#FFFFFF',
                align: 'center',
              ),
            ),
            BoardNode(
              id: 'm1',
              kind: 'mind_node',
              x: 316,
              y: 100,
              w: 152,
              h: 42,
              z: 2,
              parentId: 'm0',
              rank: 0,
              text: '分支主题 1',
              style: BoardNodeStyle(
                fill: '#E8F3FF',
                stroke: '#3370FF',
                align: 'left',
              ),
            ),
            BoardNode(
              id: 'm2',
              kind: 'mind_node',
              x: 316,
              y: 185,
              w: 152,
              h: 42,
              z: 3,
              parentId: 'm0',
              rank: 1,
              text: '分支主题 2',
              style: BoardNodeStyle(
                fill: '#EDF7ED',
                stroke: '#2DA44E',
                align: 'left',
              ),
            ),
            BoardNode(
              id: 'm3',
              kind: 'mind_node',
              x: 316,
              y: 270,
              w: 152,
              h: 42,
              z: 4,
              parentId: 'm0',
              rank: 2,
              text: '分支主题 3',
              style: BoardNodeStyle(
                fill: '#FFF7E6',
                stroke: '#D9822B',
                align: 'left',
              ),
            ),
          ],
          edges: [
            BoardEdge(
              id: 'me1',
              fromId: 'm0',
              toId: 'm1',
              fromAnchor: 'r',
              toAnchor: 'l',
              routing: 'curved',
              arrowEnd: 'none',
              style: BoardEdgeStyle(stroke: '#3370FF', strokeWidth: 1.8),
            ),
            BoardEdge(
              id: 'me2',
              fromId: 'm0',
              toId: 'm2',
              fromAnchor: 'r',
              toAnchor: 'l',
              routing: 'curved',
              arrowEnd: 'none',
              style: BoardEdgeStyle(stroke: '#2DA44E', strokeWidth: 1.8),
            ),
            BoardEdge(
              id: 'me3',
              fromId: 'm0',
              toId: 'm3',
              fromAnchor: 'r',
              toAnchor: 'l',
              routing: 'curved',
              arrowEnd: 'none',
              style: BoardEdgeStyle(stroke: '#D9822B', strokeWidth: 1.8),
            ),
          ],
        );
        layoutMindmapLocal(snap);
        return snap;
      case 'flowchart':
        return BoardSnapshot(
          title: '流程图',
          mode: 'flowchart',
          nodes: [
            BoardNode(
              id: 'f1',
              kind: 'pill',
              x: 220,
              y: 60,
              w: 140,
              h: 46,
              z: 1,
              text: '开始',
              style: BoardNodeStyle(fill: '#EDF7ED', stroke: '#2DA44E'),
            ),
            BoardNode(
              id: 'f2',
              kind: 'rounded',
              x: 200,
              y: 156,
              w: 180,
              h: 54,
              z: 2,
              text: '处理请求',
              style: BoardNodeStyle(fill: '#E8F3FF', stroke: '#3370FF'),
            ),
            BoardNode(
              id: 'f3',
              kind: 'diamond',
              x: 210,
              y: 262,
              w: 160,
              h: 84,
              z: 3,
              text: '校验通过？',
              style: BoardNodeStyle(fill: '#FFF7E6', stroke: '#D9822B'),
            ),
            BoardNode(
              id: 'f4',
              kind: 'cylinder',
              x: 215,
              y: 400,
              w: 150,
              h: 68,
              z: 4,
              text: '写入数据库',
              style: BoardNodeStyle(fill: '#F5F0FF', stroke: '#8250DF'),
            ),
          ],
          edges: [
            BoardEdge(
              id: 'fe1',
              fromId: 'f1',
              toId: 'f2',
              fromAnchor: 'b',
              toAnchor: 't',
              routing: 'orthogonal',
            ),
            BoardEdge(
              id: 'fe2',
              fromId: 'f2',
              toId: 'f3',
              fromAnchor: 'b',
              toAnchor: 't',
              routing: 'orthogonal',
            ),
            BoardEdge(
              id: 'fe3',
              fromId: 'f3',
              toId: 'f4',
              fromAnchor: 'b',
              toAnchor: 't',
              routing: 'orthogonal',
              label: '是',
            ),
          ],
        );
      default:
        return BoardSnapshot(
          title: '未命名画板',
          mode: 'whiteboard',
          nodes: [
            BoardNode(
              id: 'n1',
              kind: 'rounded',
              x: 120,
              y: 140,
              w: 180,
              h: 64,
              z: 1,
              text: '核心想法',
              style: BoardNodeStyle(
                fill: '#E8F3FF',
                stroke: '#3370FF',
                strokeWidth: 1.8,
              ),
            ),
            BoardNode(
              id: 'n2',
              kind: 'sticky',
              x: 380,
              y: 132,
              w: 196,
              h: 80,
              z: 2,
              text: '双击编辑文字\n拖边缘蓝点拉出连线',
              style: BoardNodeStyle(
                fill: '#FFF7CC',
                stroke: '#E0B422',
                strokeWidth: 1.4,
                align: 'left',
              ),
            ),
          ],
          edges: [
            BoardEdge(
              id: 'e1',
              fromId: 'n1',
              toId: 'n2',
              fromAnchor: 'r',
              toAnchor: 'l',
              routing: 'curved',
            ),
          ],
        );
    }
  }
}

/// Fast local Reingold-Tilford tree layout (mirrors Rust `layout_mindmap` so
/// keyboard actions like Tab/Enter feel 0ms-instant on every platform).
void layoutMindmapLocal(BoardSnapshot snap) {
  const hGap = 68.0;
  const vGap = 18.0;
  final byId = <String, BoardNode>{for (final n in snap.nodes) n.id: n};
  final children = <String, List<BoardNode>>{};
  for (final n in snap.nodes) {
    final pid = n.parentId;
    if (pid != null && byId.containsKey(pid) && pid != n.id) {
      (children[pid] ??= []).add(n);
    }
  }
  for (final list in children.values) {
    list.sort((a, b) {
      final c = a.rank.compareTo(b.rank);
      return c != 0 ? c : a.id.compareTo(b.id);
    });
  }

  final memo = <String, double>{};
  double measure(BoardNode n, Set<String> stack) {
    if (!stack.add(n.id)) return n.h;
    final ownH = math.max(36.0, n.h);
    final kids = n.collapsed ? const <BoardNode>[] : (children[n.id] ?? const []);
    if (kids.isEmpty) {
      stack.remove(n.id);
      return memo[n.id] = ownH;
    }
    var sum = 0.0;
    for (var i = 0; i < kids.length; i++) {
      if (i > 0) sum += vGap;
      sum += measure(kids[i], stack);
    }
    stack.remove(n.id);
    return memo[n.id] = math.max(ownH, sum);
  }

  final visited = <String>{};
  void place(BoardNode n, double x, double topY) {
    final totalH = memo[n.id] ?? n.h;
    final ownH = math.max(36.0, n.h);
    final ownW = math.max(80.0, n.w);
    n.x = x;
    n.y = topY + (totalH - ownH) / 2;
    if (n.collapsed) return;
    final kids = children[n.id] ?? const [];
    if (kids.isEmpty) return;
    var kidsSum = 0.0;
    for (var i = 0; i < kids.length; i++) {
      if (i > 0) kidsSum += vGap;
      kidsSum += memo[kids[i].id] ?? kids[i].h;
    }
    var curY = topY + (totalH - kidsSum) / 2;
    final childX = x + ownW + hGap;
    for (final k in kids) {
      if (!visited.add(k.id)) continue;
      final kh = memo[k.id] ?? k.h;
      place(k, childX, curY);
      curY += kh + vGap;
    }
  }

  for (final n in snap.nodes) {
    final isRoot =
        n.kind == 'mind_root' ||
        (n.kind == 'mind_node' &&
            (n.parentId == null || !byId.containsKey(n.parentId)));
    if (!isRoot || !visited.add(n.id)) continue;
    final totalH = measure(n, <String>{});
    final centerY = n.y + n.h / 2;
    place(n, n.x, centerY - totalH / 2);
  }
}

/// Set of node ids hidden beneath collapsed mind-map ancestors.
Set<String> collapsedDescendants(BoardSnapshot snap) {
  final children = <String, List<String>>{};
  for (final n in snap.nodes) {
    final pid = n.parentId;
    if (pid != null) (children[pid] ??= []).add(n.id);
  }
  final hidden = <String>{};
  final stack = <String>[];
  for (final n in snap.nodes) {
    if (n.collapsed) {
      stack.addAll(children[n.id] ?? const []);
    }
  }
  while (stack.isNotEmpty) {
    final id = stack.removeLast();
    if (hidden.add(id)) {
      stack.addAll(children[id] ?? const []);
    }
  }
  return hidden;
}

/// Local anchor & polyline calculation (mirrors Rust `routing.rs` for 60 FPS
/// zero-latency drag feedback; Rust A* handles complex obstacle routing & SVG).
(Offset, Offset, Offset, Offset) resolveEdgeEndpoints(
  BoardEdge edge,
  BoardNode from,
  BoardNode to,
) {
  final dx = to.center.dx - from.center.dx;
  final dy = to.center.dy - from.center.dy;
  String autoFrom;
  String autoTo;
  if (dx.abs() >= dy.abs()) {
    autoFrom = dx >= 0 ? 'r' : 'l';
    autoTo = dx >= 0 ? 'l' : 'r';
  } else {
    autoFrom = dy >= 0 ? 'b' : 't';
    autoTo = dy >= 0 ? 't' : 'b';
  }
  final sf = edge.fromAnchor == 'auto' ? autoFrom : edge.fromAnchor;
  final st = edge.toAnchor == 'auto' ? autoTo : edge.toAnchor;
  return (
    nodeAnchorOffset(from, sf),
    anchorNormal(sf),
    nodeAnchorOffset(to, st),
    anchorNormal(st),
  );
}

Offset nodeAnchorOffset(BoardNode n, String side) {
  return switch (side) {
    't' => Offset(n.x + n.w / 2, n.y),
    'r' => Offset(n.x + n.w, n.y + n.h / 2),
    'b' => Offset(n.x + n.w / 2, n.y + n.h),
    'l' => Offset(n.x, n.y + n.h / 2),
    _ => n.center,
  };
}

Offset anchorNormal(String side) {
  return switch (side) {
    't' => const Offset(0, -1),
    'r' => const Offset(1, 0),
    'b' => const Offset(0, 1),
    'l' => const Offset(-1, 0),
    _ => const Offset(1, 0),
  };
}
