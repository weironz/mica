import 'package:flutter/foundation.dart';

import '../api/models.dart';
import 'rename.dart' show renamedTo;

/// The inline-rename (create-then-name) flow's state, extracted from
/// `_WorkspaceViewState` (god-class wrap-up; docs/code-optimization/
/// OPTIMIZATION_PLAN_ROUND3.md §7). One nullable id and three transitions —
/// deliberately small: the NAME VALIDATION lives in `renamedTo` (ui/rename.dart,
/// already tested) and the dialogs stay in the State.
///
/// `_renamingViewId` only ever drives the create-then-name flow: a centered
/// dialog handles every other rename (a wide inline field overflows on long
/// names and there is no room to widen a row), but a JUST-CREATED row is empty,
/// cannot overflow, and a dialog per create is heavy.
class InlineRenameController extends ChangeNotifier {
  /// The sidebar row whose name is being typed, or null when none is.
  String? renamingId;

  bool isRenaming(String id) => renamingId == id;

  /// A row was just created: its row goes straight into inline-rename so the
  /// user just types the name.
  void begin(String id) {
    renamingId = id;
    notifyListeners();
  }

  /// Esc / blur with nothing to save. A no-op when nothing is renaming (commit
  /// and blur can both fire for the same field).
  void cancel() {
    if (renamingId == null) return;
    renamingId = null;
    notifyListeners();
  }

  /// The name to save for [view], or null when nothing should be sent —
  /// blank means "changed my mind", unchanged means the field was merely
  /// visited (`renamedTo` pins both). Always ends the editing state.
  String? commit(DocumentView view, String typedName) {
    if (renamingId != null) {
      renamingId = null;
      notifyListeners();
    }
    return renamedTo(typedName, view.name);
  }
}
