// Feishu-Board-inspired floating UI chrome:
// - Left vertical tool dock (Select, Hand, 12-Shape picker, 20-Icon Tech Library,
//   Sticky, Text, Connector, Freehand Pen, Frame, Mind Map)
// - Contextual floating inspector for selected node(s) or edge
// - Bottom-right zoom / fit-to-content / shortcut hint pill
import 'package:flutter/material.dart';

import 'board_controller.dart';
import 'board_icons.dart';
import 'board_models.dart';

class BoardLeftDock extends StatelessWidget {
  const BoardLeftDock({
    super.key,
    required this.controller,
    required this.dark,
    required this.showShapePanel,
    required this.showIconPanel,
    required this.onToggleShapePanel,
    required this.onToggleIconPanel,
  });

  final BoardController controller;
  final bool dark;
  final bool showShapePanel;
  final bool showIconPanel;
  final VoidCallback onToggleShapePanel;
  final VoidCallback onToggleIconPanel;

  @override
  Widget build(BuildContext context) {
    final bg = dark ? BoardColors.panelDark : Colors.white;
    final border = dark ? BoardColors.borderDark : BoardColors.borderLight;

    return Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        Material(
          elevation: 4,
          shadowColor: Colors.black.withValues(alpha: 0.12),
          borderRadius: BorderRadius.circular(12),
          color: bg,
          child: Container(
            padding: const EdgeInsets.symmetric(vertical: 8, horizontal: 6),
            decoration: BoxDecoration(
              borderRadius: BorderRadius.circular(12),
              border: Border.all(color: border),
            ),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                _DockButton(
                  icon: Icons.near_me_outlined,
                  tooltip: '选择 / 框选 (V)',
                  active: controller.tool == BoardTool.select && !showShapePanel && !showIconPanel,
                  dark: dark,
                  onTap: () => controller.setTool(BoardTool.select),
                ),
                _DockButton(
                  icon: Icons.pan_tool_outlined,
                  tooltip: '抓手平移 (H / 按住空格)',
                  active: controller.tool == BoardTool.hand,
                  dark: dark,
                  onTap: () => controller.setTool(BoardTool.hand),
                ),
                _dockDivider(border),
                _DockButton(
                  icon: Icons.category_outlined,
                  tooltip: '图形库（白板 / 流程图）',
                  active: controller.tool == BoardTool.shape || showShapePanel,
                  dark: dark,
                  onTap: onToggleShapePanel,
                ),
                _DockButton(
                  icon: Icons.dns_outlined,
                  tooltip: '架构图标库（20 个技术组件）',
                  active: controller.tool == BoardTool.icon || showIconPanel,
                  dark: dark,
                  onTap: onToggleIconPanel,
                ),
                _DockButton(
                  icon: Icons.sticky_note_2_outlined,
                  tooltip: '便签 (S)',
                  active: controller.tool == BoardTool.sticky,
                  dark: dark,
                  onTap: () => controller.setTool(BoardTool.sticky),
                ),
                _DockButton(
                  icon: Icons.text_fields,
                  tooltip: '文本 (T)',
                  active: controller.tool == BoardTool.text,
                  dark: dark,
                  onTap: () => controller.setTool(BoardTool.text),
                ),
                _DockButton(
                  icon: Icons.timeline,
                  tooltip: '连线 / 箭头 (L)',
                  active: controller.tool == BoardTool.connector,
                  dark: dark,
                  onTap: () => controller.setTool(BoardTool.connector),
                ),
                _DockButton(
                  icon: Icons.gesture,
                  tooltip: '自由画笔 (P)',
                  active: controller.tool == BoardTool.pen,
                  dark: dark,
                  onTap: () => controller.setTool(BoardTool.pen),
                ),
                _DockButton(
                  icon: Icons.crop_free,
                  tooltip: '分组容器 Frame (F)',
                  active: controller.tool == BoardTool.frame,
                  dark: dark,
                  onTap: () => controller.setTool(BoardTool.frame),
                ),
                _dockDivider(border),
                _DockButton(
                  icon: Icons.account_tree_outlined,
                  tooltip: '插入思维导图 (M)',
                  active: controller.tool == BoardTool.mindmap,
                  dark: dark,
                  onTap: () => controller.setTool(BoardTool.mindmap),
                ),
              ],
            ),
          ),
        ),
        if (showShapePanel) ...[
          const SizedBox(width: 8),
          _ShapeLibraryPanel(
            controller: controller,
            dark: dark,
            onPicked: onToggleShapePanel,
          ),
        ],
        if (showIconPanel) ...[
          const SizedBox(width: 8),
          _TechIconLibraryPanel(
            controller: controller,
            dark: dark,
            onPicked: onToggleIconPanel,
          ),
        ],
      ],
    );
  }

  Widget _dockDivider(Color color) => Container(
    width: 24,
    height: 1,
    margin: const EdgeInsets.symmetric(vertical: 5),
    color: color,
  );
}

class _DockButton extends StatelessWidget {
  const _DockButton({
    required this.icon,
    required this.tooltip,
    required this.active,
    required this.dark,
    required this.onTap,
  });

  final IconData icon;
  final String tooltip;
  final bool active;
  final bool dark;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return Tooltip(
      message: tooltip,
      waitDuration: const Duration(milliseconds: 300),
      child: InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(8),
        child: Container(
          width: 36,
          height: 36,
          margin: const EdgeInsets.symmetric(vertical: 2),
          decoration: BoxDecoration(
            color: active
                ? BoardColors.brand.withValues(alpha: dark ? 0.28 : 0.12)
                : Colors.transparent,
            borderRadius: BorderRadius.circular(8),
          ),
          child: Icon(
            icon,
            size: 19,
            color: active
                ? BoardColors.brand
                : (dark ? BoardColors.iconDark : BoardColors.iconLight),
          ),
        ),
      ),
    );
  }
}

class _ShapeLibraryPanel extends StatelessWidget {
  const _ShapeLibraryPanel({
    required this.controller,
    required this.dark,
    required this.onPicked,
  });

  final BoardController controller;
  final bool dark;
  final VoidCallback onPicked;

  @override
  Widget build(BuildContext context) {
    final bg = dark ? BoardColors.panelDark : Colors.white;
    final border = dark ? BoardColors.borderDark : BoardColors.borderLight;
    final subtle = dark ? BoardColors.mutedDark : BoardColors.mutedLight;
    final basic = kBoardShapes.where((s) => s.category == 'basic').toList();
    final flow = kBoardShapes.where((s) => s.category == 'flowchart').toList();

    Widget buildGrid(List<BoardShapeMeta> items) => Wrap(
      spacing: 6,
      runSpacing: 6,
      children: [
        for (final meta in items)
          InkWell(
            onTap: () {
              controller.activeShapeKind = meta.kind;
              controller.setTool(BoardTool.shape);
              onPicked();
            },
            borderRadius: BorderRadius.circular(8),
            child: Container(
              width: 82,
              padding: const EdgeInsets.symmetric(vertical: 8, horizontal: 6),
              decoration: BoxDecoration(
                color: controller.activeShapeKind == meta.kind
                    ? BoardColors.brand.withValues(alpha: 0.12)
                    : (dark ? BoardColors.chipDark : BoardColors.canvasLight),
                borderRadius: BorderRadius.circular(8),
                border: Border.all(
                  color: controller.activeShapeKind == meta.kind
                      ? BoardColors.brand
                      : Colors.transparent,
                ),
              ),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                children: [
                  SizedBox(
                    width: 34,
                    height: 22,
                    child: CustomPaint(
                      painter: _MiniShapePainter(
                        kind: meta.kind,
                        stroke: parseBoardColor(meta.defaultStroke),
                        fill: parseBoardColor(meta.defaultFill),
                      ),
                    ),
                  ),
                  const SizedBox(height: 4),
                  Text(
                    meta.label,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      fontSize: 11,
                      color: dark ? BoardColors.inkDark : BoardColors.inkLight,
                    ),
                  ),
                ],
              ),
            ),
          ),
      ],
    );

    return Material(
      elevation: 6,
      borderRadius: BorderRadius.circular(12),
      color: bg,
      child: Container(
        width: 280,
        padding: const EdgeInsets.all(12),
        decoration: BoxDecoration(
          borderRadius: BorderRadius.circular(12),
          border: Border.all(color: border),
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          mainAxisSize: MainAxisSize.min,
          children: [
            Text('基础图形（选后点击画布放置）', style: TextStyle(fontSize: 11, color: subtle)),
            const SizedBox(height: 6),
            buildGrid(basic),
            const SizedBox(height: 10),
            Text('流程图标准图形', style: TextStyle(fontSize: 11, color: subtle)),
            const SizedBox(height: 6),
            buildGrid(flow),
          ],
        ),
      ),
    );
  }
}

class _TechIconLibraryPanel extends StatelessWidget {
  const _TechIconLibraryPanel({
    required this.controller,
    required this.dark,
    required this.onPicked,
  });

  final BoardController controller;
  final bool dark;
  final VoidCallback onPicked;

  @override
  Widget build(BuildContext context) {
    final bg = dark ? BoardColors.panelDark : Colors.white;
    final border = dark ? BoardColors.borderDark : BoardColors.borderLight;
    final subtle = dark ? BoardColors.mutedDark : BoardColors.mutedLight;

    return Material(
      elevation: 6,
      borderRadius: BorderRadius.circular(12),
      color: bg,
      child: Container(
        width: 312,
        constraints: const BoxConstraints(maxHeight: 420),
        padding: const EdgeInsets.all(12),
        decoration: BoxDecoration(
          borderRadius: BorderRadius.circular(12),
          border: Border.all(color: border),
        ),
        child: SingleChildScrollView(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            mainAxisSize: MainAxisSize.min,
            children: [
              Text('技术架构图标库（选后点击画布放置）', style: TextStyle(fontSize: 11, color: subtle)),
              const SizedBox(height: 8),
              Wrap(
                spacing: 6,
                runSpacing: 6,
                children: [
                  for (final meta in kBoardTechIcons)
                    InkWell(
                      onTap: () {
                        controller.activeIconId = meta.id;
                        controller.setTool(BoardTool.icon);
                        onPicked();
                      },
                      borderRadius: BorderRadius.circular(8),
                      child: Container(
                        width: 68,
                        padding: const EdgeInsets.symmetric(vertical: 8, horizontal: 4),
                        decoration: BoxDecoration(
                          color: controller.activeIconId == meta.id
                              ? BoardColors.brand.withValues(alpha: 0.12)
                              : (dark ? BoardColors.chipDark : BoardColors.canvasLight),
                          borderRadius: BorderRadius.circular(8),
                          border: Border.all(
                            color: controller.activeIconId == meta.id
                                ? BoardColors.brand
                                : Colors.transparent,
                          ),
                        ),
                        child: Column(
                          mainAxisSize: MainAxisSize.min,
                          children: [
                            SizedBox(
                              width: 24,
                              height: 24,
                              child: CustomPaint(
                                painter: _MiniTechIconPainter(
                                  id: meta.id,
                                  color: parseBoardColor(meta.defaultStroke),
                                ),
                              ),
                            ),
                            const SizedBox(height: 4),
                            Text(
                              meta.label,
                              maxLines: 1,
                              overflow: TextOverflow.ellipsis,
                              style: TextStyle(
                                fontSize: 11,
                                color: dark ? BoardColors.inkDark : BoardColors.inkLight,
                              ),
                            ),
                          ],
                        ),
                      ),
                    ),
                ],
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class BoardSelectionInspector extends StatelessWidget {
  const BoardSelectionInspector({
    super.key,
    required this.controller,
    required this.dark,
    required this.onEditEdgeLabel,
  });

  final BoardController controller;
  final bool dark;
  final VoidCallback onEditEdgeLabel;

  @override
  Widget build(BuildContext context) {
    final hasNodes = controller.selectedNodeIds.isNotEmpty;
    final edge = controller.selectedEdgeId == null
        ? null
        : controller.snapshot.edges
              .where((e) => e.id == controller.selectedEdgeId)
              .firstOrNull;
    if (!hasNodes && edge == null) return const SizedBox.shrink();

    final bg = dark ? BoardColors.panelDark : Colors.white;
    final border = dark ? BoardColors.borderDark : BoardColors.borderLight;

    return Material(
      elevation: 4,
      shadowColor: Colors.black.withValues(alpha: 0.12),
      borderRadius: BorderRadius.circular(10),
      color: bg,
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
        decoration: BoxDecoration(
          borderRadius: BorderRadius.circular(10),
          border: Border.all(color: border),
        ),
        child: hasNodes
            ? _buildNodeControls(context, border)
            : _buildEdgeControls(context, edge!, border),
      ),
    );
  }

  Widget _buildNodeControls(BuildContext context, Color border) {
    final first = controller.nodeById[controller.selectedNodeIds.first];
    if (first == null) return const SizedBox.shrink();
    final isMind = first.kind == 'mind_root' || first.kind == 'mind_node';

    return Wrap(
      crossAxisAlignment: WrapCrossAlignment.center,
      spacing: 6,
      children: [
        // Fill swatches
        for (final hex in kBoardFillSwatches.take(8))
          _ColorDot(
            hex: hex,
            selected: first.style.fill.toUpperCase() == hex.toUpperCase(),
            onTap: () => controller.updateSelectedNodesStyle(fill: hex),
          ),
        _vSep(border),
        // Stroke swatches
        for (final hex in kBoardStrokeSwatches.take(6))
          _ColorDot(
            hex: hex,
            outlined: true,
            selected: first.style.stroke.toUpperCase() == hex.toUpperCase(),
            onTap: () => controller.updateSelectedNodesStyle(stroke: hex),
          ),
        _vSep(border),
        _SmallAction(
          icon: first.style.dashed ? Icons.linear_scale : Icons.horizontal_rule,
          tooltip: first.style.dashed ? '切为实线边框' : '切为虚线边框',
          active: first.style.dashed,
          dark: dark,
          onTap: () => controller.updateSelectedNodesStyle(dashed: !first.style.dashed),
        ),
        _SmallAction(
          icon: Icons.text_decrease,
          tooltip: '缩小字号',
          dark: dark,
          onTap: () => controller.updateSelectedNodesStyle(
            fontSize: (first.style.fontSize - 1.5).clamp(10.0, 28.0),
          ),
        ),
        _SmallAction(
          icon: Icons.text_increase,
          tooltip: '增大字号',
          dark: dark,
          onTap: () => controller.updateSelectedNodesStyle(
            fontSize: (first.style.fontSize + 1.5).clamp(10.0, 28.0),
          ),
        ),
        if (isMind) ...[
          _vSep(border),
          _SmallAction(
            icon: Icons.subdirectory_arrow_right,
            tooltip: '添加子节点 (Tab)',
            dark: dark,
            onTap: () {
              final child = controller.addMindChild(first.id);
              if (child != null) controller.startEditingNode(child.id);
            },
          ),
          _SmallAction(
            icon: Icons.add,
            tooltip: '添加同级节点 (Enter)',
            dark: dark,
            onTap: () {
              final sib = controller.addMindSibling(first.id);
              if (sib != null) controller.startEditingNode(sib.id);
            },
          ),
        ],
        _vSep(border),
        _SmallAction(
          icon: Icons.flip_to_front,
          tooltip: '置于顶层',
          dark: dark,
          onTap: controller.bringSelectedToFront,
        ),
        _SmallAction(
          icon: Icons.flip_to_back,
          tooltip: '置于底层',
          dark: dark,
          onTap: controller.sendSelectedToBack,
        ),
        _SmallAction(
          icon: Icons.copy_outlined,
          tooltip: '克隆 (Ctrl+D)',
          dark: dark,
          onTap: controller.duplicateSelection,
        ),
        _SmallAction(
          icon: Icons.delete_outline,
          tooltip: '删除 (Delete)',
          dark: dark,
          danger: true,
          onTap: controller.deleteSelection,
        ),
      ],
    );
  }

  Widget _buildEdgeControls(BuildContext context, BoardEdge edge, Color border) {
    return Wrap(
      crossAxisAlignment: WrapCrossAlignment.center,
      spacing: 6,
      children: [
        for (final r in const [
          ('orthogonal', '折线'),
          ('curved', '曲线'),
          ('straight', '直线'),
        ])
          InkWell(
            onTap: () => controller.updateSelectedEdge(routing: r.$1),
            borderRadius: BorderRadius.circular(6),
            child: Container(
              padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
              decoration: BoxDecoration(
                color: edge.routing == r.$1
                    ? BoardColors.brand
                    : (dark ? BoardColors.chipDark : BoardColors.chipLight),
                borderRadius: BorderRadius.circular(6),
              ),
              child: Text(
                r.$2,
                style: TextStyle(
                  fontSize: 12,
                  color: edge.routing == r.$1
                      ? Colors.white
                      : (dark ? BoardColors.inkDark : BoardColors.inkLight),
                ),
              ),
            ),
          ),
        _vSep(border),
        for (final hex in kBoardStrokeSwatches.take(6))
          _ColorDot(
            hex: hex,
            selected: edge.style.stroke.toUpperCase() == hex.toUpperCase(),
            onTap: () => controller.updateSelectedEdge(stroke: hex),
          ),
        _vSep(border),
        _SmallAction(
          icon: Icons.linear_scale,
          tooltip: edge.style.dashed ? '实线' : '虚线',
          active: edge.style.dashed,
          dark: dark,
          onTap: () => controller.updateSelectedEdge(dashed: !edge.style.dashed),
        ),
        _SmallAction(
          icon: Icons.arrow_forward,
          tooltip: edge.arrowEnd == 'arrow' ? '隐藏终点箭头' : '显示终点箭头',
          active: edge.arrowEnd == 'arrow',
          dark: dark,
          onTap: () => controller.updateSelectedEdge(
            arrowEnd: edge.arrowEnd == 'arrow' ? 'none' : 'arrow',
          ),
        ),
        _SmallAction(
          icon: Icons.label_outline,
          tooltip: '编辑连线标签',
          dark: dark,
          onTap: onEditEdgeLabel,
        ),
        _SmallAction(
          icon: Icons.delete_outline,
          tooltip: '删除连线',
          dark: dark,
          danger: true,
          onTap: controller.deleteSelection,
        ),
      ],
    );
  }

  Widget _vSep(Color c) => Container(width: 1, height: 18, color: c);
}

class _ColorDot extends StatelessWidget {
  const _ColorDot({
    required this.hex,
    required this.selected,
    required this.onTap,
    this.outlined = false,
  });

  final String hex;
  final bool selected;
  final VoidCallback onTap;
  final bool outlined;

  @override
  Widget build(BuildContext context) {
    final c = parseBoardColor(hex, fallback: Colors.transparent);
    return InkWell(
      onTap: onTap,
      borderRadius: BorderRadius.circular(10),
      child: Container(
        width: 18,
        height: 18,
        decoration: BoxDecoration(
          color: outlined ? Colors.transparent : c,
          shape: BoxShape.circle,
          border: Border.all(
            color: selected
                ? BoardColors.brand
                : (outlined ? c : BoardColors.iconDark),
            width: selected ? 2.4 : (outlined ? 2.5 : 1.0),
          ),
        ),
      ),
    );
  }
}

class _SmallAction extends StatelessWidget {
  const _SmallAction({
    required this.icon,
    required this.tooltip,
    required this.dark,
    required this.onTap,
    this.active = false,
    this.danger = false,
  });

  final IconData icon;
  final String tooltip;
  final bool dark;
  final VoidCallback onTap;
  final bool active;
  final bool danger;

  @override
  Widget build(BuildContext context) {
    final color = danger
        ? BoardColors.danger
        : active
            ? BoardColors.brand
            : (dark ? BoardColors.iconDark : BoardColors.iconLight);
    return Tooltip(
      message: tooltip,
      child: InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(6),
        child: Container(
          width: 28,
          height: 28,
          decoration: BoxDecoration(
            color: active ? BoardColors.brandWash : null,
            borderRadius: BorderRadius.circular(6),
          ),
          child: Icon(icon, size: 16, color: color),
        ),
      ),
    );
  }
}

class _MiniShapePainter extends CustomPainter {
  const _MiniShapePainter({
    required this.kind,
    required this.stroke,
    required this.fill,
  });

  final String kind;
  final Color stroke;
  final Color fill;

  @override
  void paint(Canvas canvas, Size size) {
    final r = Offset.zero & size;
    final fp = Paint()..color = fill;
    final sp = Paint()
      ..color = stroke
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1.4;
    switch (kind) {
      case 'ellipse':
        canvas.drawOval(r.deflate(1), fp);
        canvas.drawOval(r.deflate(1), sp);
      case 'diamond':
        final p = Path()
          ..moveTo(r.center.dx, 1)
          ..lineTo(r.right - 1, r.center.dy)
          ..lineTo(r.center.dx, r.bottom - 1)
          ..lineTo(1, r.center.dy)
          ..close();
        canvas.drawPath(p, fp);
        canvas.drawPath(p, sp);
      case 'pill':
        final rr = RRect.fromRectAndRadius(r.deflate(1), Radius.circular(r.height / 2));
        canvas.drawRRect(rr, fp);
        canvas.drawRRect(rr, sp);
      case 'triangle':
        final p = Path()
          ..moveTo(r.center.dx, 1)
          ..lineTo(r.right - 1, r.bottom - 1)
          ..lineTo(1, r.bottom - 1)
          ..close();
        canvas.drawPath(p, fp);
        canvas.drawPath(p, sp);
      case 'parallelogram':
        final p = Path()
          ..moveTo(6, 1)
          ..lineTo(r.right - 1, 1)
          ..lineTo(r.right - 6, r.bottom - 1)
          ..lineTo(1, r.bottom - 1)
          ..close();
        canvas.drawPath(p, fp);
        canvas.drawPath(p, sp);
      case 'hexagon':
        final p = Path()
          ..moveTo(6, 1)
          ..lineTo(r.right - 6, 1)
          ..lineTo(r.right - 1, r.center.dy)
          ..lineTo(r.right - 6, r.bottom - 1)
          ..lineTo(6, r.bottom - 1)
          ..lineTo(1, r.center.dy)
          ..close();
        canvas.drawPath(p, fp);
        canvas.drawPath(p, sp);
      case 'cylinder':
        final rr = RRect.fromRectAndRadius(r.deflate(1), const Radius.circular(4));
        canvas.drawRRect(rr, fp);
        canvas.drawRRect(rr, sp);
        canvas.drawOval(Rect.fromLTWH(1, 1, r.width - 2, 7), sp);
      case 'rect':
        canvas.drawRect(r.deflate(1), fp);
        canvas.drawRect(r.deflate(1), sp);
      default:
        final rr = RRect.fromRectAndRadius(r.deflate(1), const Radius.circular(4));
        canvas.drawRRect(rr, fp);
        canvas.drawRRect(rr, sp);
    }
  }

  @override
  bool shouldRepaint(covariant _MiniShapePainter oldDelegate) =>
      oldDelegate.kind != kind;
}

class _MiniTechIconPainter extends CustomPainter {
  const _MiniTechIconPainter({required this.id, required this.color});
  final String id;
  final Color color;

  @override
  void paint(Canvas canvas, Size size) {
    paintBoardTechIcon(canvas, id, Offset.zero & size, color);
  }

  @override
  bool shouldRepaint(covariant _MiniTechIconPainter oldDelegate) =>
      oldDelegate.id != id || oldDelegate.color != color;
}
