// Where the platform's hidden text input sits over the editor (gh#171).
library;

import 'dart:math' as math;

import 'package:flutter/rendering.dart';

/// The editable box, its transform and the caret rect (relative to that box)
/// to report to the platform text input, for an editor of [editorSize] whose
/// global transform is [editorTransform] and a [caret] in editor coordinates.
///
/// On the web the box is the caret itself, not the whole editor. Flutter's
/// iOS Safari text strategy lets that hidden input take pointer events, so an
/// editor-sized one swallowed touch drags: the browser placed its own caret in
/// the invisible text and reported it back, and the selection jumped instead
/// of following the finger. Desktop keeps the editor-sized box its native IMEs
/// have always been given.
({Size size, Matrix4 transform, Rect caret}) imeGeometry(
  Size editorSize,
  Matrix4 editorTransform,
  Rect caret, {
  required bool web,
}) {
  final at = web ? caret.topLeft : Offset.zero;
  return (
    size: web ? Size(math.max(caret.width, 1), caret.height) : editorSize,
    transform: editorTransform.clone()..translateByDouble(at.dx, at.dy, 0, 1),
    caret: caret.shift(-at),
  );
}
