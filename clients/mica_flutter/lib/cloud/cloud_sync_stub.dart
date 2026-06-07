// P2-M4.5c: web stub for the cloud yrs sync session.
//
// The yrs CRDT session is desktop-only (it needs the native FFI). On web the app
// keeps using the op-based cloud path; this stub keeps `main.dart` compiling for
// web without pulling the native bridge into the bundle. `isReady` is always
// false, so callers fall back to the op/REST path.
//
// Mirrors the public surface of `cloud_sync.dart`.
typedef DocOp = Map<String, dynamic>;

class CloudSyncSession {
  CloudSyncSession({
    required this.uri,
    required this.clientId,
    required this.onReady,
    required this.onRemoteBlocks,
  });

  final Uri uri;
  final BigInt clientId;
  final void Function(String rootBlockId, List<Map<String, dynamic>> blocks)
  onReady;
  final void Function(List<Map<String, dynamic>> blocks) onRemoteBlocks;

  String get rootBlockId => '';
  bool get isReady => false;

  List<Map<String, dynamic>> allBlocks() => const [];
  List<Map<String, dynamic>> childBlocks() => const [];

  void connect() {}
  void applyLocalOps(List<DocOp> ops) {}
  void dispose() {}
}
