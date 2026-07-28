// Text controllers that live exactly as long as the dialog they belong to.
//
// Four dialogs in this app created their `TextEditingController`s next to the
// `showDialog` call and disposed them the moment the future resolved:
//
//     final controller = TextEditingController();
//     final value = await showDialog(...);
//     controller.dispose();          // ← too early
//
// That future completes on `Navigator.pop`, while the route's exit transition
// still has the `TextField` mounted for another frame or two. Any rebuild inside
// that window re-runs `InputDecorator`'s animation, which re-listens to the
// controller through a merged listenable — on a disposed `ChangeNotifier`. The
// app then died with "A TextEditingController was used after being disposed",
// followed by a red `_dependents.isEmpty` screen.
//
// It was reliably reachable, not a race: signing in and adding a server both
// call `setState` on the app state right after the dialog pops, which rebuilds
// the whole tree — the leaving route included. (Adding a server with an EMPTY
// field never crashed, because that path returns before the setState. That is
// why it first looked intermittent.)
//
// The fix is ownership, not timing: put the controllers inside the route's own
// subtree, where `State.dispose` runs when the route is really gone. Nothing has
// to guess how long an animation takes.

import 'package:flutter/material.dart';

/// Owns [count] text controllers for the lifetime of this subtree and hands them
/// to [builder].
///
/// Use it INSIDE a `showDialog` builder, wrapping the dialog:
///
/// ```dart
/// showDialog<String>(
///   context: context,
///   builder: (context) => DialogTextControllers(
///     count: 1,
///     builder: (context, c) => AlertDialog(
///       content: TextField(controller: c[0]),
///       actions: [ ... Navigator.pop(context, c[0].text) ... ],
///     ),
///   ),
/// );
/// ```
///
/// The caller keeps no reference and disposes nothing — which is the point: the
/// only correct moment to dispose is one this widget can see and the caller
/// cannot.
class DialogTextControllers extends StatefulWidget {
  const DialogTextControllers({
    super.key,
    required this.count,
    required this.builder,
    this.initialTexts,
  });

  /// How many fields the dialog has.
  final int count;

  /// Optional starting text per field; a shorter list leaves the rest empty.
  final List<String>? initialTexts;

  final Widget Function(
    BuildContext context,
    List<TextEditingController> fields,
  )
  builder;

  @override
  State<DialogTextControllers> createState() => _DialogTextControllersState();
}

class _DialogTextControllersState extends State<DialogTextControllers> {
  late final List<TextEditingController> _fields = [
    for (var i = 0; i < widget.count; i++)
      TextEditingController(
        text: (widget.initialTexts != null && i < widget.initialTexts!.length)
            ? widget.initialTexts![i]
            : '',
      ),
  ];

  @override
  void dispose() {
    for (final field in _fields) {
      field.dispose();
    }
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => widget.builder(context, _fields);
}
