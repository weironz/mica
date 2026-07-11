// Desktop/mobile variant of the P4-2 platform doc-store factory: the web
// IndexedDB store does not exist here — the caller uses the FFI-backed
// LocalOffline store instead.
import 'cloud_doc_store.dart';

Future<CloudDocStore?> openWebDocStore(String origin, String docId) async =>
    null;

/// Desktop reads its doc mirrors synchronously through LocalOffline (FFI);
/// this async web-mirror reader never has anything here.
Future<({String rootBlockId, List<Map<String, dynamic>> blocks})?>
    openWebDocMirror(String origin, String docId) async => null;
