// Pure logic behind the search dialog: which result is selected, and where the
// query matched inside a snippet.
//
// The dialog is `part of main.dart` and cannot be constructed by a test, so the
// two things that are easy to get subtly wrong live here instead.

/// One run of snippet text, flagged if it is part of a query match.
typedef HighlightRun = ({String text, bool hit});

/// Split [text] into runs, marking every occurrence of [query].
///
/// **Case-insensitive on purpose**: the server matches with `ILIKE`, so a
/// case-sensitive highlighter would return real hits with nothing highlighted
/// inside them — the row would read as a false positive from the search engine.
///
/// Matching is plain substring search, never a regex: a query of `50%` or `a.b`
/// or `(` has to match literally, and building a pattern out of user input is how
/// you get either a crash or a wrong match. (The server escapes LIKE
/// metacharacters for the same reason.)
List<HighlightRun> highlightRuns(String text, String query) {
  final needle = query.trim();
  if (text.isEmpty) return const [];
  if (needle.isEmpty) return [(text: text, hit: false)];

  final haystack = text.toLowerCase();
  final lowerNeedle = needle.toLowerCase();
  final runs = <HighlightRun>[];
  var cursor = 0;
  while (true) {
    final at = haystack.indexOf(lowerNeedle, cursor);
    if (at < 0) break;
    if (at > cursor) {
      runs.add((text: text.substring(cursor, at), hit: false));
    }
    // Slice out of the ORIGINAL text so a highlighted run keeps the casing the
    // document actually has.
    runs.add((text: text.substring(at, at + needle.length), hit: true));
    cursor = at + needle.length;
  }
  if (cursor < text.length) {
    runs.add((text: text.substring(cursor), hit: false));
  }
  return runs;
}

/// The next selected index when the user presses ↓ ([delta] 1) or ↑ ([delta] -1).
///
/// Wraps at both ends: with a short result list, walking off the bottom and
/// landing back on the first row is what every picker does, and stopping dead at
/// the edge reads as a stuck key. Returns -1 for an empty list — there is nothing
/// to select, and 0 would point at a row that isn't there.
int moveSelection({
  required int current,
  required int count,
  required int delta,
}) {
  if (count <= 0) return -1;
  if (current < 0) {
    // Nothing selected yet: ↓ takes the first row, ↑ takes the last.
    return delta > 0 ? 0 : count - 1;
  }
  final next = (current + delta) % count;
  return next < 0 ? next + count : next;
}
