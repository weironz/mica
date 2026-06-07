// P2-M3: web stub — local offline is desktop-only (no native FFI on web).
//
// The Settings UI already hides the local-offline option on web; this stub keeps
// `main.dart` compiling for web without pulling the native bridge into the
// bundle. Every method is unavailable.
typedef ViewData = ({
  String id,
  String? parentId,
  String objectId,
  String name,
  String position,
  bool trashed,
});

typedef DocData = ({String rootBlockId, List<Map<String, dynamic>> blocks});

class LocalOffline {
  bool get available => false;

  Future<void> open() async =>
      throw UnsupportedError('local offline is not available on web');

  List<ViewData> listViews() => const [];

  void saveView(ViewData v) {}

  void purgeView(String viewId, String objectId) {}

  ({String docId, String rootBlockId, List<Map<String, dynamic>> blocks})
      newDoc() => throw UnsupportedError('local offline is not available on web');

  DocData? openDoc(String docId) => null;

  Future<void> applyOps(List<Map<String, dynamic>> ops) async {}

  void flush() {}
}
