/// Stable parent-before-child order for workspace migration. This stays
/// independent of the shell and of native ViewData/FFI so both platforms can
/// use and test the same traversal without a State or a backend.
List<T> orderViewsParentFirst<T>(
  List<T> views, {
  required String Function(T view) idOf,
  required String? Function(T view) parentOf,
}) {
  final byParent = <String?, List<T>>{};
  for (final view in views) {
    (byParent[parentOf(view)] ??= []).add(view);
  }
  final ids = {for (final view in views) idOf(view)};
  final out = <T>[];
  void emit(String? parent) {
    for (final view in byParent[parent] ?? <T>[]) {
      out.add(view);
      emit(idOf(view));
    }
  }

  emit(null);
  // Preserve orphan roots and their descendants rather than dropping content.
  for (final view in views) {
    final parent = parentOf(view);
    if (parent != null && !ids.contains(parent)) {
      out.add(view);
      emit(idOf(view));
    }
  }
  return out;
}
