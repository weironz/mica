import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../doc_tab.dart';
import '../l10n/locale_controller.dart';
import '../widgets/mica_logo.dart';
import 'about_dialog.dart';
import 'theme_tokens.dart';
import 'typography.dart';
import 'update_controls.dart';

/// A small application menu, backed by the same actions as the resident UI.
/// No editing commands here yet: those must target the pre-menu editor focus.
class ApplicationMenu extends StatefulWidget {
  const ApplicationMenu({
    super.key,
    required this.onSettings,
    required this.onToggleSidebar,
    required this.onToggleSidePanel,
    required this.sidebarVisible,
    required this.sidePanelVisible,
    required this.updates,
    this.onNewPage,
    this.onNewTab,
    this.onCloseTab,
    this.onHistory,
    this.tabs = const [],
    this.activeTabIndex = 0,
  });

  final VoidCallback onSettings, onToggleSidebar, onToggleSidePanel;
  final VoidCallback? onNewPage, onNewTab, onCloseTab;
  final void Function(bool forward)? onHistory;
  final bool sidebarVisible, sidePanelVisible;
  final DesktopUpdateController updates;
  final List<DocTab> tabs;
  final int activeTabIndex;

  @override
  State<ApplicationMenu> createState() => _ApplicationMenuState();
}

class _ApplicationMenuState extends State<ApplicationMenu> {
  final _menu = MenuController();

  SingleActivator? _shortcut(LogicalKeyboardKey key, {bool desktop = false}) {
    if (desktop && kIsWeb) return null;
    final mac = defaultTargetPlatform == TargetPlatform.macOS;
    return SingleActivator(key, control: !mac, meta: mac);
  }

  Widget _item(
    String label,
    VoidCallback? action, {
    MenuSerializableShortcut? shortcut,
    bool? checked,
  }) => MenuItemButton(
    onPressed: action == null
        ? null
        : () {
            _menu.close();
            action();
          },
    shortcut: shortcut,
    leadingIcon: checked == null
        ? null
        : SizedBox(
            width: 18,
            child: checked ? const Icon(Icons.check, size: 18) : null,
          ),
    style: MenuItemButton.styleFrom(
      minimumSize: const Size(200, 34),
      textStyle: MicaTypography.body,
    ),
    child: Text(label),
  );

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.updates,
    builder: (context, _) {
      final l = context.l10n;
      final tokens = MicaTheme.of(context);
      final menuStyle = MenuStyle(
        backgroundColor: WidgetStatePropertyAll(tokens.surface.overlay),
        padding: const WidgetStatePropertyAll(EdgeInsets.all(4)),
        shape: WidgetStatePropertyAll(
          RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(9),
            side: BorderSide(color: tokens.border.subtle),
          ),
        ),
      );
      Widget group(String label, List<Widget> children) => SubmenuButton(
        menuStyle: menuStyle,
        style: MenuItemButton.styleFrom(
          minimumSize: const Size(110, 34),
          textStyle: MicaTypography.body,
        ),
        menuChildren: children,
        child: Text(label),
      );
      final active = widget.activeTabIndex;
      final history = active >= 0 && active < widget.tabs.length
          ? widget.tabs[active].history
          : null;
      final updateLabel = switch (widget.updates.stage) {
        DesktopUpdateStage.available => l.updateNow,
        DesktopUpdateStage.checking => l.updateChecking,
        DesktopUpdateStage.downloading => l.updateDownloading(
          (widget.updates.progress * 100).round(),
        ),
        _ => l.updateCheck,
      };
      return MenuAnchor(
        controller: _menu,
        alignmentOffset: const Offset(0, 4),
        style: menuStyle,
        menuChildren: [
          group(l.appMenuFile, [
            _item(
              l.newPage,
              widget.onNewPage,
              shortcut: _shortcut(LogicalKeyboardKey.keyN),
            ),
            if (widget.onNewTab != null)
              _item(
                l.tabNewTooltip,
                widget.onNewTab,
                shortcut: _shortcut(LogicalKeyboardKey.keyT, desktop: true),
              ),
            if (widget.onCloseTab != null)
              _item(
                l.shortcutsTabClose,
                widget.tabs.length > 1 ? widget.onCloseTab : null,
                shortcut: _shortcut(LogicalKeyboardKey.keyW, desktop: true),
              ),
            const Divider(height: 9),
            _item(
              l.settingsTitle,
              widget.onSettings,
              shortcut: _shortcut(LogicalKeyboardKey.comma),
            ),
          ]),
          group(l.appMenuView, [
            _item(
              widget.sidebarVisible ? l.sidebarCollapse : l.sidebarExpand,
              widget.onToggleSidebar,
              checked: widget.sidebarVisible,
            ),
            _item(
              widget.sidePanelVisible
                  ? l.pageHideSidePanel
                  : l.pageShowSidePanel,
              widget.onToggleSidePanel,
              checked: widget.sidePanelVisible,
            ),
          ]),
          if (widget.onHistory != null && widget.tabs.isNotEmpty)
            group(l.appMenuHistory, [
              _item(
                l.tabBack,
                history?.canGoBack == true
                    ? () => widget.onHistory!(false)
                    : null,
              ),
              _item(
                l.tabForward,
                history?.canGoForward == true
                    ? () => widget.onHistory!(true)
                    : null,
              ),
            ]),
          group(l.appMenuHelp, [
            _item(
              l.appMenuAbout,
              () => showMicaAboutDialog(context, widget.updates),
            ),
            if (widget.updates.supported)
              _item(
                updateLabel,
                widget.updates.isBusy
                    ? null
                    : () => activateDesktopUpdate(context, widget.updates),
              ),
          ]),
        ],
        builder: (context, controller, _) => IconButton(
          tooltip: l.appMenuTitle,
          onPressed: () =>
              controller.isOpen ? controller.close() : controller.open(),
          padding: EdgeInsets.zero,
          constraints: const BoxConstraints.tightFor(width: 30, height: 30),
          style: IconButton.styleFrom(
            tapTargetSize: MaterialTapTargetSize.shrinkWrap,
          ),
          icon: const MicaLogo(size: 22),
        ),
      );
    },
  );
}
