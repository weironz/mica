import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../editor/render.dart';

/// The document title shares prose preferences but has its own visual hierarchy.
/// Keep editing/saving in the host; this field only owns presentation and keys.
class PageTitleField extends StatelessWidget {
  const PageTitleField({
    required this.controller,
    required this.focusNode,
    required this.appearance,
    required this.hintText,
    required this.onChanged,
    required this.onEnterBody,
    required this.onArrowDown,
    this.readOnly = false,
    super.key,
  });

  final TextEditingController controller;
  final FocusNode focusNode;
  final EditorAppearance appearance;
  final String hintText;
  final ValueChanged<String> onChanged;
  final VoidCallback onEnterBody;
  final VoidCallback onArrowDown;
  final bool readOnly;

  void _enterBody() {
    final composing = controller.value.composing;
    if (readOnly || (composing.isValid && !composing.isCollapsed)) return;
    onEnterBody();
  }

  @override
  Widget build(BuildContext context) => LayoutBuilder(
    builder: (context, constraints) => Focus(
      canRequestFocus: false,
      skipTraversal: true,
      onKeyEvent: (_, event) {
        if (event is! KeyDownEvent || readOnly) {
          return KeyEventResult.ignored;
        }
        // Enter confirms a live preedit. It must reach the text input engine,
        // rather than moving focus and committing a partial Chinese title.
        final composing = controller.value.composing;
        if (composing.isValid && !composing.isCollapsed) {
          return KeyEventResult.ignored;
        }
        if (event.logicalKey == LogicalKeyboardKey.arrowDown) {
          onArrowDown();
          return KeyEventResult.handled;
        }
        if (event.logicalKey == LogicalKeyboardKey.enter ||
            event.logicalKey == LogicalKeyboardKey.numpadEnter) {
          _enterBody();
          return KeyEventResult.handled;
        }
        return KeyEventResult.ignored;
      },
      child: TextField(
        controller: controller,
        focusNode: focusNode,
        readOnly: readOnly,
        maxLines: null,
        keyboardType: TextInputType.text,
        textInputAction: TextInputAction.next,
        style: appearance.applyTo(
          TextStyle(
            fontSize: constraints.maxWidth < 600 ? 30 : 34,
            fontWeight: FontWeight.w600,
            height: 1.3,
            color: appearance.tokens.text.primary,
          ),
          isCode: false,
        ),
        onChanged: onChanged,
        onEditingComplete: readOnly ? null : _enterBody,
        decoration: InputDecoration(
          hintText: hintText,
          hintStyle: TextStyle(color: appearance.tokens.text.faint),
          isDense: true,
          contentPadding: const EdgeInsets.symmetric(vertical: 12),
          border: InputBorder.none,
          enabledBorder: InputBorder.none,
          focusedBorder: InputBorder.none,
          filled: false,
        ),
      ),
    ),
  );
}
