import 'package:flutter/material.dart';

import '../widgets/mica_logo.dart';
import 'theme_tokens.dart';
import 'window_chrome.dart';

/// One shared top edge for branding, navigation, document tabs and OS controls.
class ShellTitleBar extends StatelessWidget {
  const ShellTitleBar({
    super.key,
    required this.navigationWidth,
    required this.toggleLabel,
    required this.onToggle,
    this.tabs,
  });

  final double navigationWidth;
  final String toggleLabel;
  final VoidCallback onToggle;
  final Widget? tabs;

  @override
  Widget build(BuildContext context) => Material(
    color: MicaTheme.of(context).surface.sunken,
    child: SizedBox(
      key: const ValueKey('shell-title-bar'),
      height: 34,
      child: Row(
        children: [
          SizedBox(
            width: navigationWidth,
            child: Row(
              children: [
                const SizedBox(width: 12),
                const DesktopDragRegion(child: MicaLogo(size: 22)),
                const SizedBox(width: 6),
                IconButton(
                  tooltip: toggleLabel,
                  onPressed: onToggle,
                  padding: EdgeInsets.zero,
                  style: IconButton.styleFrom(
                    minimumSize: const Size(30, 30),
                    maximumSize: const Size(30, 30),
                    tapTargetSize: MaterialTapTargetSize.shrinkWrap,
                  ),
                  constraints: const BoxConstraints.tightFor(
                    width: 30,
                    height: 30,
                  ),
                  icon: const Icon(Icons.view_sidebar_outlined, size: 19),
                ),
                const Expanded(
                  child: DesktopDragRegion(child: SizedBox.expand()),
                ),
              ],
            ),
          ),
          Expanded(
            child: tabs ?? const DesktopDragRegion(child: SizedBox.expand()),
          ),
          const DesktopCaption(),
        ],
      ),
    ),
  );
}
