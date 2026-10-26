import 'dart:async';

/// Who an async action is working for, carried across its awaits.
///
/// An action captures shared state's owner (server + account) when it starts,
/// awaits the network, then writes into whatever state is current when the
/// answer lands. If the user signed out, switched account or switched
/// workspace in between, that write lands in the wrong session — an old
/// account's new workspace in the new account's list, a page from another
/// workspace selected.
///
/// [runOwned] records the owner in a Zone value, which survives every await
/// inside the action; [ensureOwner] is the one line each commit point adds
/// before writing. A stale result throws [StaleResult], which the runner drops.
/// See docs/dev/async-review-checklist.md.
class StaleResult implements Exception {
  const StaleResult();
  @override
  String toString() => 'StaleResult: the request no longer owns this state';
}

const Symbol _ownerKey = #micaRequestOwner;

/// Run [body] on behalf of [owner] (null: nobody signed in).
Future<T> runOwned<T>(Object? owner, Future<T> Function() body) =>
    runZoned(body, zoneValues: {_ownerKey: _Owner(owner)});

/// Throw [StaleResult] unless the action this runs in still works for [now]
/// — the owner as it is AT THIS MOMENT — and, when given, unless
/// [workspaceId] is still the selected one ([selectedWorkspaceId]).
///
/// Outside [runOwned] it does nothing: there is no captured owner to compare.
void ensureOwner(
  Object? now, {
  String? workspaceId,
  String? selectedWorkspaceId,
}) {
  final captured = Zone.current[_ownerKey];
  if (captured is! _Owner) return;
  if (captured.value != now) throw const StaleResult();
  if (workspaceId != null && workspaceId != selectedWorkspaceId) {
    throw const StaleResult();
  }
}

/// True when the current action (if any) was started for a different owner
/// than [now] — for error paths, which must not act on the new session.
bool ownerChanged(Object? now) {
  final captured = Zone.current[_ownerKey];
  return captured is _Owner && captured.value != now;
}

/// Boxed so a captured `null` (signed out) differs from "no runOwned zone".
class _Owner {
  const _Owner(this.value);
  final Object? value;
}
