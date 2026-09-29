/// Decides which of several overlapping fetches for one WORKSPACE is allowed to
/// commit its result.
///
/// A page-tree fetch is a network round trip, and more than one can be in flight
/// for the same workspace at once — the tree-change socket bell fires while a
/// workspace switch is still loading, or two bells land back to back. Without
/// this, whichever response arrived LAST won, which is not necessarily the
/// newest one.
///
/// That is not merely a cosmetic staleness: the response carries the ETag along
/// with the tree, and the ETag is persisted. A stale ETag outlives the session —
/// the next cold start sends it, the server answers 304, and the client keeps a
/// tree the server has moved past. Reported shape: a page that exists looks
/// missing, on a tree that looks perfectly normal.
///
/// Its own class so the rule is testable without a widget tree, an HTTP client
/// or a clock — the failure it prevents needs two responses to arrive out of
/// order, which is exactly what a test can stage and production cannot be
/// relied on to.
class TreeRequestSeq {
  final Map<String, int> _latest = {};

  /// Claim a number for a request about to start. Pass the result to [isCurrent]
  /// once the response lands.
  int claim(String workspaceId) {
    final next = (_latest[workspaceId] ?? 0) + 1;
    _latest[workspaceId] = next;
    return next;
  }

  /// Whether the response to the request that claimed [seq] is still the one
  /// worth applying — i.e. nothing newer has been started for that workspace
  /// since.
  bool isCurrent(String workspaceId, int seq) => _latest[workspaceId] == seq;

  /// Forget a workspace (signed out, removed). Not required for correctness —
  /// numbers only ever grow — but it keeps the map from holding ids for
  /// workspaces this session will never touch again.
  void forget(String workspaceId) => _latest.remove(workspaceId);

  /// Drop every workspace, for a sign-out or a server switch.
  void clear() => _latest.clear();
}
