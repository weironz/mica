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

/// How many workspace name matches the search panel shows at once.
///
/// Small on purpose: they sit above the page results and do not scroll, so an
/// uncapped run of similarly-named workspaces would push the page half off the
/// panel. Five is enough to disambiguate a typed name; past that, type more.
const int kMaxWorkspaceHits = 5;

/// Workspaces whose NAME contains [query], case-insensitively, capped at [limit].
///
/// Client-side because the whole list is already in memory: switching among
/// fifty workspaces should not cost a round trip, and no endpoint answers it.
/// Input order is preserved — that is the user's own workspace order, and
/// re-ranking it by match position would reshuffle a list they arranged.
///
/// An empty or whitespace query matches NOTHING rather than everything: the
/// panel shows recents before anything is typed, and a full workspace dump
/// would bury them.
List<({String id, String name})> matchingWorkspaces({
  required List<({String id, String name})> workspaces,
  required String query,
  int limit = kMaxWorkspaceHits,
}) {
  final needle = query.trim().toLowerCase();
  if (needle.isEmpty || limit <= 0) return const [];
  final hits = <({String id, String name})>[];
  for (final w in workspaces) {
    if (!w.name.toLowerCase().contains(needle)) continue;
    hits.add(w);
    if (hits.length == limit) break;
  }
  return hits;
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

/// The trail to show for a hit that lives in ANOTHER workspace: that
/// workspace's name, alone.
///
/// The folder chain is deliberately absent — the client only holds the tree of
/// workspaces it has visited, and a trail invented from the CURRENT tree is the
/// 2026-08-28 bug this function exists to fix: every cross-workspace hit wore
/// the current workspace's name. Unknown id (not in [workspaces]) says nothing
/// at all; a blank beats a wrong label, because the label is the one thing
/// telling the user "this hit is not from here".
List<String> workspaceTrailFor(
  String workspaceId,
  List<({String id, String name})> workspaces,
) {
  for (final w in workspaces) {
    if (w.id == workspaceId) return [w.name];
  }
  return const [];
}

/// Search EVERY workspace, preferring the open one — and fall back to the
/// current-workspace search only when there is no global search to use.
///
/// [searchEverywhere] is null in the local (offline) world, which has exactly
/// one workspace, and on a server too old for `GET /search` the call throws;
/// both land on [searchHere]. Note the direction: the narrow search is the
/// FALLBACK now, not the default.
///
/// **Why this replaced "current first, widen only when empty" (2026-08-28).**
/// That rule looks reasonable and fails badly: searching "claude" from a
/// workspace holding two passing mentions returned exactly those two, while
/// fifty pages actually NAMED after it sat one workspace away — unreachable,
/// because two results is not zero results, so the widening never fired. A
/// gate on emptiness lets any weak local hit veto every strong remote one.
///
/// The scope checkbox that preceded both (removed the same day) existed
/// because cross-workspace search cost N workspaces' worth of body reads.
/// Server-side FTS M2 made the wide scan the same price as the narrow one, so
/// there is nothing left to make the user pay for — and asking someone to tick
/// a box to see obviously better answers is interaction cost covering for a
/// design flaw. Relevance is ranked server-side (`prefer_workspace`): a
/// foreign name match still beats a local body mention, and among equals the
/// local one leads.
Future<List<T>> searchScopeFor<T>({
  required Future<List<T>> Function() searchHere,
  required Future<List<T>> Function()? searchEverywhere,
  void Function(Object error)? onGlobalError,
}) async {
  if (searchEverywhere == null) return searchHere();
  try {
    return await searchEverywhere();
  } catch (e) {
    // An old server (no `GET /search`, pre-0.13.18 — observed live) or a blip:
    // answer from the workspace we CAN search rather than reporting the whole
    // query as failed. Only `searchHere` throwing reaches the caller.
    onGlobalError?.call(e);
    return searchHere();
  }
}

/// The workspace a hit lives in when that is NOT the one on screen — the one
/// question both click paths have to ask, and the one they used to answer
/// separately.
///
/// A null [hitWorkspaceId] means "the workspace we searched" (a server older
/// than the field does not send it), so it is never elsewhere. A null
/// [currentWorkspaceId] means no workspace is open, and nothing can be
/// elsewhere than nowhere — treating that as "elsewhere" would send a click
/// into a workspace switch with no destination.
///
/// Reported 2026-08-28: clicking a FOLDER from another workspace did nothing.
/// Pages asked this question and switched first; folders never asked it at all,
/// so the reveal was parked against the tree on screen — which will never
/// contain that folder. Both paths now go through here, which is the point of
/// it being a function rather than two `!=` expressions.
/// Returns the OTHER workspace's id, or null when the hit belongs to the one on
/// screen. Returning the id rather than a bool is deliberate: every caller then
/// needs it, and a bool would leave them re-deriving it behind a `!` that the
/// compiler cannot check.
String? elsewhereWorkspace(String? hitWorkspaceId, String? currentWorkspaceId) =>
    (hitWorkspaceId != null &&
        currentWorkspaceId != null &&
        hitWorkspaceId != currentWorkspaceId)
    ? hitWorkspaceId
    : null;
