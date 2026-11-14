import 'package:flutter/widgets.dart';

bool get hasCustomWindowChrome => false;

class DesktopCaption extends StatelessWidget {
  const DesktopCaption({super.key});
  @override
  Widget build(BuildContext context) => const SizedBox.shrink();
}

class DesktopDragRegion extends StatelessWidget {
  const DesktopDragRegion({super.key, required this.child});
  final Widget child;
  @override
  Widget build(BuildContext context) => child;
}
