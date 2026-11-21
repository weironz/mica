// Lightweight built-in technical architecture icon library (20 vector icons
// painted directly onto Flutter Canvas, mirroring Rust `tech_icon_svg_inner`).
import 'package:flutter/material.dart';

class BoardTechIconMeta {
  const BoardTechIconMeta({
    required this.id,
    required this.label,
    required this.group,
    required this.defaultFill,
    required this.defaultStroke,
  });

  final String id;
  final String label;
  final String group; // '计算' | '数据' | '网络' | '终端' | '安全与AI'
  final String defaultFill;
  final String defaultStroke;
}

const List<BoardTechIconMeta> kBoardTechIcons = [
  BoardTechIconMeta(
    id: 'server',
    label: '服务器',
    group: '计算',
    defaultFill: '#E8F3FF',
    defaultStroke: '#3370FF',
  ),
  BoardTechIconMeta(
    id: 'container',
    label: '容器 / K8s',
    group: '计算',
    defaultFill: '#E8F3FF',
    defaultStroke: '#3370FF',
  ),
  BoardTechIconMeta(
    id: 'api',
    label: 'API 服务',
    group: '计算',
    defaultFill: '#EDF7ED',
    defaultStroke: '#2DA44E',
  ),
  BoardTechIconMeta(
    id: 'worker',
    label: '后台任务',
    group: '计算',
    defaultFill: '#EDF7ED',
    defaultStroke: '#2DA44E',
  ),
  BoardTechIconMeta(
    id: 'database',
    label: '数据库',
    group: '数据',
    defaultFill: '#F5F0FF',
    defaultStroke: '#8250DF',
  ),
  BoardTechIconMeta(
    id: 'cache',
    label: '缓存 Redis',
    group: '数据',
    defaultFill: '#FFF0F0',
    defaultStroke: '#CF222E',
  ),
  BoardTechIconMeta(
    id: 'queue',
    label: '消息队列',
    group: '数据',
    defaultFill: '#FFF7E6',
    defaultStroke: '#D9822B',
  ),
  BoardTechIconMeta(
    id: 'storage',
    label: '对象存储',
    group: '数据',
    defaultFill: '#F5F0FF',
    defaultStroke: '#8250DF',
  ),
  BoardTechIconMeta(
    id: 'gateway',
    label: '网关',
    group: '网络',
    defaultFill: '#E6F7F7',
    defaultStroke: '#0E8A8A',
  ),
  BoardTechIconMeta(
    id: 'lb',
    label: '负载均衡',
    group: '网络',
    defaultFill: '#E6F7F7',
    defaultStroke: '#0E8A8A',
  ),
  BoardTechIconMeta(
    id: 'cdn',
    label: 'CDN 节点',
    group: '网络',
    defaultFill: '#E8F3FF',
    defaultStroke: '#3370FF',
  ),
  BoardTechIconMeta(
    id: 'cloud',
    label: '云服务',
    group: '网络',
    defaultFill: '#E8F3FF',
    defaultStroke: '#3370FF',
  ),
  BoardTechIconMeta(
    id: 'browser',
    label: 'Web 端',
    group: '终端',
    defaultFill: '#FFFFFF',
    defaultStroke: '#646A73',
  ),
  BoardTechIconMeta(
    id: 'mobile',
    label: '移动端',
    group: '终端',
    defaultFill: '#FFFFFF',
    defaultStroke: '#646A73',
  ),
  BoardTechIconMeta(
    id: 'user',
    label: '用户',
    group: '终端',
    defaultFill: '#FFF7E6',
    defaultStroke: '#D9822B',
  ),
  BoardTechIconMeta(
    id: 'doc',
    label: '文档',
    group: '终端',
    defaultFill: '#FFFFFF',
    defaultStroke: '#3370FF',
  ),
  BoardTechIconMeta(
    id: 'firewall',
    label: '防火墙',
    group: '安全与AI',
    defaultFill: '#FFF0F0',
    defaultStroke: '#CF222E',
  ),
  BoardTechIconMeta(
    id: 'lock',
    label: '鉴权加密',
    group: '安全与AI',
    defaultFill: '#FFF7E6',
    defaultStroke: '#D9822B',
  ),
  BoardTechIconMeta(
    id: 'monitor',
    label: '监控告警',
    group: '安全与AI',
    defaultFill: '#EDF7ED',
    defaultStroke: '#2DA44E',
  ),
  BoardTechIconMeta(
    id: 'ai',
    label: 'AI / LLM',
    group: '安全与AI',
    defaultFill: '#F5F0FF',
    defaultStroke: '#8250DF',
  ),
];

/// Paint a crisp 24x24-coordinate tech architecture vector icon inside [rect].
void paintBoardTechIcon(Canvas canvas, String id, Rect rect, Color color) {
  canvas.save();
  canvas.translate(rect.left, rect.top);
  canvas.scale(rect.width / 24.0, rect.height / 24.0);
  final p = Paint()
    ..color = color
    ..style = PaintingStyle.stroke
    ..strokeWidth = 1.8
    ..strokeCap = StrokeCap.round
    ..strokeJoin = StrokeJoin.round;
  final fillDot = Paint()..color = color;

  switch (id) {
    case 'server':
      canvas.drawRRect(
        RRect.fromRectAndRadius(
          const Rect.fromLTWH(3, 4, 18, 6),
          const Radius.circular(1.5),
        ),
        p,
      );
      canvas.drawRRect(
        RRect.fromRectAndRadius(
          const Rect.fromLTWH(3, 14, 18, 6),
          const Radius.circular(1.5),
        ),
        p,
      );
      canvas.drawCircle(const Offset(7, 7), 1.1, fillDot);
      canvas.drawCircle(const Offset(7, 17), 1.1, fillDot);
    case 'database':
      canvas.drawOval(const Rect.fromLTWH(4.5, 2.8, 15, 5.4), p);
      canvas.drawLine(const Offset(4.5, 5.5), const Offset(4.5, 18.5), p);
      canvas.drawLine(const Offset(19.5, 5.5), const Offset(19.5, 18.5), p);
      canvas.drawArc(
        const Rect.fromLTWH(4.5, 15.8, 15, 5.4),
        0,
        3.14159,
        false,
        p,
      );
      canvas.drawArc(
        const Rect.fromLTWH(4.5, 9.3, 15, 5.4),
        0,
        3.14159,
        false,
        p,
      );
    case 'cache':
      final path = Path()
        ..moveTo(13, 2)
        ..lineTo(4, 14)
        ..lineTo(11, 14)
        ..lineTo(10, 22)
        ..lineTo(20, 10)
        ..lineTo(13, 10)
        ..close();
      canvas.drawPath(path, p);
    case 'queue':
      for (final x in [3.0, 10.0, 17.0]) {
        canvas.drawRRect(
          RRect.fromRectAndRadius(
            Rect.fromLTWH(x, 5, 4, 14),
            const Radius.circular(1),
          ),
          p,
        );
      }
    case 'gateway':
      canvas.drawCircle(const Offset(12, 12), 8, p);
      canvas.drawLine(const Offset(4, 12), const Offset(20, 12), p);
      canvas.drawOval(const Rect.fromLTWH(8, 4, 8, 16), p);
    case 'cloud':
      final path = Path()
        ..moveTo(7, 18)
        ..lineTo(17.5, 18)
        ..cubicTo(21, 18, 21.5, 11, 17.5, 10.5)
        ..cubicTo(16.5, 5.5, 9, 5.5, 7.6, 9.5)
        ..cubicTo(3.5, 10, 3.5, 18, 7, 18)
        ..close();
      canvas.drawPath(path, p);
    case 'browser':
      canvas.drawRRect(
        RRect.fromRectAndRadius(
          const Rect.fromLTWH(3, 4, 18, 16),
          const Radius.circular(2),
        ),
        p,
      );
      canvas.drawLine(const Offset(3, 9), const Offset(21, 9), p);
      canvas.drawCircle(const Offset(6.5, 6.5), 0.9, fillDot);
      canvas.drawCircle(const Offset(9.5, 6.5), 0.9, fillDot);
    case 'mobile':
      canvas.drawRRect(
        RRect.fromRectAndRadius(
          const Rect.fromLTWH(6.5, 2.5, 11, 19),
          const Radius.circular(2),
        ),
        p,
      );
      canvas.drawLine(const Offset(10.5, 18.5), const Offset(13.5, 18.5), p);
    case 'user':
      canvas.drawCircle(const Offset(12, 8), 3.5, p);
      canvas.drawArc(
        const Rect.fromLTWH(5, 14, 14, 12),
        3.14159,
        3.14159,
        false,
        p,
      );
    case 'lock':
      canvas.drawRRect(
        RRect.fromRectAndRadius(
          const Rect.fromLTWH(5, 11, 14, 10),
          const Radius.circular(2),
        ),
        p,
      );
      canvas.drawArc(
        const Rect.fromLTWH(8, 3.5, 8, 14),
        3.14159,
        3.14159,
        false,
        p,
      );
    case 'api':
      final path = Path()
        ..moveTo(8, 7)
        ..lineTo(4, 12)
        ..lineTo(8, 17)
        ..moveTo(16, 7)
        ..lineTo(20, 12)
        ..lineTo(16, 17)
        ..moveTo(13.5, 5)
        ..lineTo(10.5, 19);
      canvas.drawPath(path, p);
    case 'container':
      final hex = Path()
        ..moveTo(12, 3)
        ..lineTo(20, 7.5)
        ..lineTo(20, 16.5)
        ..lineTo(12, 21)
        ..lineTo(4, 16.5)
        ..lineTo(4, 7.5)
        ..close();
      canvas.drawPath(hex, p);
      canvas.drawLine(const Offset(12, 12), const Offset(20, 7.5), p);
      canvas.drawLine(const Offset(12, 12), const Offset(4, 7.5), p);
      canvas.drawLine(const Offset(12, 12), const Offset(12, 21), p);
    case 'storage':
      canvas.drawRRect(
        RRect.fromRectAndRadius(
          const Rect.fromLTWH(3, 5, 18, 14),
          const Radius.circular(2),
        ),
        p,
      );
      canvas.drawLine(const Offset(3, 10), const Offset(21, 10), p);
      canvas.drawLine(const Offset(8, 14.5), const Offset(11.5, 14.5), p);
    case 'ai':
      final star = Path()
        ..moveTo(12, 3)
        ..lineTo(14.2, 8.8)
        ..lineTo(20, 11)
        ..lineTo(14.2, 13.2)
        ..lineTo(12, 19)
        ..lineTo(9.8, 13.2)
        ..lineTo(4, 11)
        ..lineTo(9.8, 8.8)
        ..close();
      canvas.drawPath(star, p);
    case 'monitor':
      canvas.drawRRect(
        RRect.fromRectAndRadius(
          const Rect.fromLTWH(3, 4, 18, 12),
          const Radius.circular(2),
        ),
        p,
      );
      final line = Path()
        ..moveTo(6.5, 12)
        ..lineTo(9.5, 9)
        ..lineTo(12, 11)
        ..lineTo(15, 7)
        ..lineTo(17.5, 9);
      canvas.drawPath(line, p);
      canvas.drawLine(const Offset(8, 20), const Offset(16, 20), p);
      canvas.drawLine(const Offset(12, 16), const Offset(12, 20), p);
    case 'cdn':
      canvas.drawCircle(const Offset(12, 12), 2.8, p);
      for (final c in const [
        Offset(5, 6),
        Offset(19, 6),
        Offset(5, 18),
        Offset(19, 18),
      ]) {
        canvas.drawCircle(c, 1.8, p);
        canvas.drawLine(Offset.lerp(c, const Offset(12, 12), 0.25)!,
            Offset.lerp(c, const Offset(12, 12), 0.65)!, p);
      }
    case 'lb':
      canvas.drawCircle(const Offset(5, 12), 2.2, p);
      for (final y in [6.0, 12.0, 18.0]) {
        canvas.drawCircle(Offset(19, y), 2.0, p);
        canvas.drawLine(const Offset(7.2, 12), Offset(17, y), p);
      }
    case 'firewall':
      final shield = Path()
        ..moveTo(12, 3)
        ..lineTo(19, 6)
        ..lineTo(19, 11.5)
        ..cubicTo(19, 16, 16, 19.5, 12, 21)
        ..cubicTo(8, 19.5, 5, 16, 5, 11.5)
        ..lineTo(5, 6)
        ..close();
      canvas.drawPath(shield, p);
    case 'worker':
      canvas.drawCircle(const Offset(12, 12), 3.4, p);
      canvas.drawCircle(const Offset(12, 12), 7.2, p..strokeWidth = 1.4);
    case 'doc':
      final d = Path()
        ..moveTo(6, 3)
        ..lineTo(14, 3)
        ..lineTo(18, 7)
        ..lineTo(18, 21)
        ..lineTo(6, 21)
        ..close();
      canvas.drawPath(d, p);
      canvas.drawLine(const Offset(9, 11.5), const Offset(15, 11.5), p);
      canvas.drawLine(const Offset(9, 15.5), const Offset(15, 15.5), p);
    default:
      canvas.drawRRect(
        RRect.fromRectAndRadius(
          const Rect.fromLTWH(4, 4, 16, 16),
          const Radius.circular(3),
        ),
        p,
      );
  }
  canvas.restore();
}
