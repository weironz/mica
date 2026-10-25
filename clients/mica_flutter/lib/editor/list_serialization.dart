import 'model.dart';

typedef ListSerialization = ({int? number, int extraIndent});

/// Mirrors Rust export_md's flat list runs, including container children.
List<ListSerialization> markdownListLayout(List<EditorNode> nodes) {
  final runs = <({String tag, int next, int width})?>[];
  final result = <ListSerialization>[];
  var quote = 0;
  for (final node in nodes) {
    final rawLi = node.data['li'];
    final li = rawLi is int ? rawLi.clamp(0, kMaxNestingDepth) : null;
    if (li == null &&
        (node.quoteDepth != quote ||
            node.data.containsKey('qbreak') ||
            !node.isListKind)) {
      runs.clear();
    }
    if (li == null) quote = node.quoteDepth;
    int? number;
    if (node.isListKind && li == null) {
      final level = node.indent;
      if (runs.length > level + 1) runs.removeRange(level + 1, runs.length);
      while (runs.length <= level) {
        runs.add(null);
      }
      final tag = node.kind == 'numbered_list' ? 'ol' : 'ul';
      final previous = runs[level];
      final starts =
          node.data.containsKey('start') || node.data.containsKey('marker');
      final start = node.data['start'];
      final n = previous != null && previous.tag == tag && !starts
          ? previous.next
          : start is int && start >= 0
          ? start
          : 1;
      final width = tag == 'ol' ? (n.toString().length + 2).clamp(4, 22) : 4;
      runs[level] = (tag: tag, next: n + 1, width: width);
      if (tag == 'ol') number = n <= 999999999 ? n : 1;
    }
    final ancestors = li == null ? node.indent : li + 1;
    final extra = runs
        .take(ancestors)
        .fold<int>(0, (sum, run) => sum + (run == null ? 0 : run.width - 4));
    result.add((number: number, extraIndent: extra));
  }
  return result;
}
