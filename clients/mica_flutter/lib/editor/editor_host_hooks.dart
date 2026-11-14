import 'package:flutter/foundation.dart';

/// Lets a host ask the editor to scroll a block into view.
class EditorScrollHook {
  void Function(String blockId)? _scroll;

  void bindScroll(void Function(String blockId) scroll) => _scroll = scroll;

  void scrollToBlock(String blockId) => _scroll?.call(blockId);
}

/// Lets the host open the editor's in-page find bar when focus is elsewhere.
class EditorFindHook {
  void Function()? _open;

  void bindOpen(VoidCallback open) => _open = open;

  void unbindOpen(VoidCallback open) {
    if (_open == open) _open = null;
  }

  void open() => _open?.call();
}

/// Publishes the focused block kind and heading level for the host toolbar.
class EditorActiveBlockHook extends ChangeNotifier {
  String? _kind;
  int? _level;
  String? get kind => _kind;
  int? get level => _level;

  void publish(String? kind, int? level) {
    if (kind == _kind && level == _level) return;
    _kind = kind;
    _level = level;
    notifyListeners();
  }
}
