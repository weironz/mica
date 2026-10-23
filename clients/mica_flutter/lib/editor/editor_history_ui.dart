part of 'editor.dart';

extension _EditorHistoryUi on _MicaEditorState {
  Future<void> _applyEditorHistory({required bool redo}) async {
    if (!mounted || !widget.canEdit || _controller.isApplyingHistory) return;
    _closeSlash();
    _closePageLink();
    _closeCellEditor();
    _focus.requestFocus();
    await (redo ? _controller.redo() : _controller.undo());
    if (!mounted) return;
    _focus.requestFocus();
    // The engine history callback is async. Sending the old value before it
    // finishes lets the next IME character replace the undone text wholesale.
    _syncImeFromSelection(force: true);
  }

  KeyEventResult _keyDuringHistory(KeyEvent event) {
    final key = event.logicalKey;
    final hw = HardwareKeyboard.instance;
    if (hw.isControlPressed ||
        hw.isMetaPressed ||
        hw.isAltPressed ||
        {
          LogicalKeyboardKey.enter,
          LogicalKeyboardKey.numpadEnter,
          LogicalKeyboardKey.tab,
          LogicalKeyboardKey.escape,
          LogicalKeyboardKey.arrowUp,
          LogicalKeyboardKey.arrowDown,
          LogicalKeyboardKey.arrowLeft,
          LogicalKeyboardKey.arrowRight,
          LogicalKeyboardKey.home,
          LogicalKeyboardKey.end,
          LogicalKeyboardKey.pageUp,
          LogicalKeyboardKey.pageDown,
        }.contains(key)) {
      return KeyEventResult.handled;
    }
    final selection = _controller.selection;
    if (selection == null || selection.isMultiNode) {
      return KeyEventResult.handled;
    }
    final node = _controller.focusedNode;
    if (selection.isCollapsed &&
        ((key == LogicalKeyboardKey.backspace && selection.focus.offset == 0) ||
            (key == LogicalKeyboardKey.delete &&
                selection.focus.offset == node?.text.length))) {
      return KeyEventResult.handled;
    }
    // Ordinary IME text and in-block deletions are kept as a draft and rebased
    // onto the canonical result by EditorController when history completes.
    return _passToTextInput;
  }
}
