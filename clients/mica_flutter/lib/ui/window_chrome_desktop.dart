import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:window_manager/window_manager.dart';

bool get hasCustomWindowChrome =>
    !kIsWeb && defaultTargetPlatform == TargetPlatform.windows;

/// Use the plugin's caption so close still follows the existing tray/quit policy.
class DesktopCaption extends StatelessWidget {
  const DesktopCaption({super.key});

  @override
  Widget build(BuildContext context) => !hasCustomWindowChrome
      ? const SizedBox.shrink()
      : SizedBox(
          width: 154,
          height: 34,
          child: WindowCaption(
            brightness: Theme.of(context).brightness,
            backgroundColor: Colors.transparent,
          ),
        );
}

class DesktopDragRegion extends StatelessWidget {
  const DesktopDragRegion({super.key, required this.child});
  final Widget child;

  @override
  Widget build(BuildContext context) =>
      hasCustomWindowChrome ? DragToMoveArea(child: child) : child;
}
