import 'model.dart';

/// Captures selection coordinates before reconcile mutates the live nodes.
class ReconcileSelection {
  ReconcileSelection(List<EditorNode> nodes, this._selection)
    : _previous = [for (final node in nodes) (id: node.id, text: node.text)];

  final DocSelection? _selection;
  final List<({String id, String text})> _previous;

  DocSelection? remap(List<EditorNode> next) {
    final selection = _selection;
    if (selection == null || next.isEmpty || _previous.isEmpty) return null;
    final indices = {for (var i = 0; i < next.length; i++) next[i].id: i};

    final offsetMaps = <int, List<_TextEdit>>{};

    DocPosition remapPosition(DocPosition position) {
      final oldIndex = position.node.clamp(0, _previous.length - 1);
      final old = _previous[oldIndex];
      final index = indices[old.id];
      if (index != null) {
        return DocPosition(
          index,
          _remapOffset(
            offsetMaps.putIfAbsent(
              oldIndex,
              () => _textEdits(old.text, next[index].text),
            ),
            position.offset.clamp(0, old.text.length),
          ),
        );
      }
      // Follow old document neighbors rather than a numeric index: remote
      // insertions and moves may have changed every index around the deletion.
      for (var i = oldIndex + 1; i < _previous.length; i++) {
        final successor = indices[_previous[i].id];
        if (successor != null) return DocPosition(successor, 0);
      }
      for (var i = oldIndex - 1; i >= 0; i--) {
        final predecessor = indices[_previous[i].id];
        if (predecessor != null) {
          return DocPosition(predecessor, next[predecessor].text.length);
        }
      }
      // A complete replacement leaves no stable block identity to follow.
      return DocPosition(oldIndex.clamp(0, next.length - 1), 0);
    }

    return DocSelection(
      anchor: remapPosition(selection.anchor),
      focus: remapPosition(selection.focus),
    );
  }
}

typedef _TextEdit = ({int start, int oldEnd, int newEnd});

int _remapOffset(List<_TextEdit> edits, int offset) {
  var delta = 0;
  for (final edit in edits) {
    if (offset < edit.start) break;
    // Right affinity: an insertion exactly at the caret goes before the caret.
    if (offset >= edit.oldEnd) {
      delta = edit.newEnd - edit.oldEnd;
    } else {
      // Deleted/replaced text maps to the start of this replacement.
      return edit.start + delta;
    }
  }
  return offset + delta;
}

/// Infer edits in UTF-16 units, matching Flutter selection offsets. Common
/// prefix/suffix trimming also preserves the existing alignment of repeated text.
List<_TextEdit> _textEdits(String before, String after) {
  if (before == after) return const [];
  var start = 0;
  while (start < before.length &&
      start < after.length &&
      before.codeUnitAt(start) == after.codeUnitAt(start)) {
    start++;
  }
  var oldEnd = before.length;
  var newEnd = after.length;
  while (oldEnd > start &&
      newEnd > start &&
      before.codeUnitAt(oldEnd - 1) == after.codeUnitAt(newEnd - 1)) {
    oldEnd--;
    newEnd--;
  }
  final fallback = [(start: start, oldEnd: oldEnd, newEnd: newEnd)];
  if (start == oldEnd || start == newEnd) return fallback;

  // This runs on the editor hot path. Bound both the Myers trace (O(D²)
  // storage, at most 129² integers) and frontier/snake work. Huge replacements
  // fall back to one replacement; unchanged interior text may then collapse.
  const maxDistance = 128;
  const maxWork = 200000;
  final oldLength = oldEnd - start;
  final newLength = newEnd - start;
  if ((oldLength - newLength).abs() > maxDistance) return fallback;
  final trace = <List<int>>[];
  var work = 0;
  for (var distance = 0; distance <= maxDistance; distance++) {
    final row = List<int>.filled(2 * distance + 1, -1);
    final previous = distance == 0 ? const <int>[] : trace.last;
    for (var k = -distance; k <= distance; k += 2) {
      if (++work > maxWork) return fallback;
      // Ties prefer deletion, making ambiguous repeated-text alignment stable.
      final insert =
          k == -distance ||
          (k != distance &&
              previous[k + distance - 2] < previous[k + distance]);
      var x = distance == 0
          ? 0
          : insert
          ? previous[k + distance]
          : previous[k + distance - 2] + 1;
      var y = x - k;
      while (x < oldLength && y < newLength) {
        if (++work > maxWork) return fallback;
        if (before.codeUnitAt(start + x) != after.codeUnitAt(start + y)) break;
        x++;
        y++;
      }
      row[k + distance] = x;
      if (x == oldLength && y == newLength) {
        trace.add(row);
        // Recover matching runs, then coalesce adjacent insert/delete steps
        // into replacements so their interior continues to map to the start.
        final matches = <({int oldStart, int newStart, int length})>[];
        for (var d = distance; d > 0; d--) {
          final previous = trace[d - 1];
          final k = x - y;
          final insert =
              k == -d || (k != d && previous[k + d - 2] < previous[k + d]);
          final previousK = insert ? k + 1 : k - 1;
          final previousX = previous[previousK + d - 1];
          final previousY = previousX - previousK;
          final snakeX = previousX + (insert ? 0 : 1);
          final snakeY = previousY + (insert ? 1 : 0);
          if (x > snakeX) {
            matches.add((
              oldStart: snakeX,
              newStart: snakeY,
              length: x - snakeX,
            ));
          }
          x = previousX;
          y = previousY;
        }
        if (x > 0) matches.add((oldStart: 0, newStart: 0, length: x));
        final edits = <_TextEdit>[];
        var oldCursor = 0;
        var newCursor = 0;
        for (final match in matches.reversed) {
          if (oldCursor != match.oldStart || newCursor != match.newStart) {
            edits.add((
              start: start + oldCursor,
              oldEnd: start + match.oldStart,
              newEnd: start + match.newStart,
            ));
          }
          oldCursor = match.oldStart + match.length;
          newCursor = match.newStart + match.length;
        }
        if (oldCursor != oldLength || newCursor != newLength) {
          edits.add((start: start + oldCursor, oldEnd: oldEnd, newEnd: newEnd));
        }
        return edits;
      }
    }
    trace.add(row);
  }
  return fallback;
}
