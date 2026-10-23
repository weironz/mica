// The document WebSocket URI builder and the protocol version it announces.
// Extracted from api/sync_client.dart when the presence client was folded into
// CloudSyncSession (2026-10): the URI seam survived the client that carried it.

/// The WS sync protocol this client speaks — mirrors `WS_PROTOCOL_VERSION` in
/// `crates/api-server/src/routes/ws.rs`, and must be bumped with it.
///
/// Announcing it lets the server refuse a client too old to be handled safely
/// (`client_too_old`) instead of accepting the socket and failing in ways
/// neither side can explain. The server's floor is 0 today, so declaring this
/// costs nothing now and is the whole point later: a desktop install that never
/// updates is exactly the client that will still be connecting when the old
/// fallbacks are gone.
const int kSyncProtocolVersion = 1;

Uri documentSocketUri(
  Uri base,
  String workspaceId,
  String documentId,
  String token,
) {
  final scheme = base.scheme == 'https' ? 'wss' : 'ws';
  return base.replace(
    scheme: scheme,
    path: '/ws/workspaces/$workspaceId/documents/$documentId',
    queryParameters: {'token': token, 'v': '$kSyncProtocolVersion'},
  );
}
