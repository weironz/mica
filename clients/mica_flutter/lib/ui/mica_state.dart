import 'package:flutter/widgets.dart';

export '../session/request_owner.dart';

/// State updates invoked by UI extensions or collaborating backend objects.
/// Keep Flutter's lifecycle checks and scheduling in the owning State.
abstract class MicaState<W extends StatefulWidget> extends State<W> {
  void setUi(VoidCallback action) => setState(action);
}
