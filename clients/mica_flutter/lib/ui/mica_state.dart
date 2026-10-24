import 'package:flutter/widgets.dart';

/// State updates invoked by UI extensions or collaborating backend objects.
/// Keep Flutter's lifecycle checks and scheduling in the owning State.
abstract class MicaState<W extends StatefulWidget> extends State<W> {
  void refresh(VoidCallback action) => setState(action);
}
