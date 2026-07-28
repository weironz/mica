// The right-hand half of the desktop sign-in screen: which world, then how.
//
// Replaces `world_picker.dart`, which listed every world inline with a delete
// button on each row — about 200px of always-open configuration above the form.
// It worked, but it read as a settings panel bolted onto a sign-in screen, and
// it was not what design 01 draws.
//
// The design's shape, which this follows:
//   ┌ 云端账户 │ 本地模式 ┐        tabs — the KIND of world
//   │ ● 已连接到服务器   ⌄ │        just the current one, address in mono
//   │   mica.example.com   │        chevron opens the list to manage the rest
//   └──────────────────────┘
//   登录到 Mica  + the form        (cloud tab)
//   在此设备上使用 + 开始使用       (local tab)
//
// Two things in the mockup are deliberately NOT copied:
//   * A pencil "edit" action per server. We can add and remove, not edit; a
//     pencil that does nothing is one more control that lies.
//   * The three capability cards under the local tab. The brand half on the LEFT
//     already lists three capabilities — the same screen would say it twice.
//
// The connection dot is EARNED, not painted: `probeHealth` is asked, and until it
// answers the row says so. The mockup's green 「已连接到服务器」 would otherwise be
// a claim made before anything was contacted — and "wrong URL / server down" is
// the most common first-run failure there is, so guessing here is the one place
// it really costs.
//
// **All copy arrives already localized** (the `status_kit.dart` contract).

import 'package:flutter/material.dart';

import '../editor/model.dart' show kMonoFont;
import '../server_list.dart' show serverLabel;

/// The origin string meaning "the on-device world", matching the store's own
/// `origin` column.
const String kLocalOrigin = 'local';

/// Whether the server behind the current row answered.
enum ServerReach { checking, reachable, unreachable }

/// Finished strings for [SignInPane].
class SignInPaneStrings {
  const SignInPaneStrings({
    required this.cloudTab,
    required this.localTab,
    required this.connected,
    required this.unreachable,
    required this.checking,
    required this.serversLabel,
    required this.addServer,
    required this.removeServer,
    required this.localTitle,
    required this.localBody,
    required this.localAction,
  });

  final String cloudTab;
  final String localTab;

  /// Status line beside the dot: 「已连接到服务器」 / 「连不上服务器」 / 「正在检测…」.
  final String connected;
  final String unreachable;
  final String checking;

  /// Section label inside the expanded server list.
  final String serversLabel;

  final String addServer;
  final String removeServer;

  final String localTitle;
  final String localBody;
  final String localAction;
}

/// Tabs + server row + either the credentials form or the local-mode panel.
class SignInPane extends StatefulWidget {
  const SignInPane({
    required this.strings,
    required this.origins,
    required this.active,
    required this.authForm,
    required this.onSelect,
    required this.onEnterLocal,
    this.onAdd,
    this.onRemove,
    this.probeHealth,
    super.key,
  });

  final SignInPaneStrings strings;

  /// Configured server origins in the user's order. [kLocalOrigin] is not one.
  final List<String> origins;

  /// The world in effect — one of [origins], or [kLocalOrigin].
  final String active;

  /// The credentials form, shown under the cloud tab.
  final Widget authForm;

  /// Point the app at this server. Stays on this screen: you still have to sign
  /// in to it.
  final void Function(String origin) onSelect;

  /// Enter 本地模式 — which also leaves this screen, because there is nothing
  /// left to sign in to.
  final VoidCallback onEnterLocal;

  /// Null hides the row instead of showing a dead one.
  final VoidCallback? onAdd;
  final void Function(String origin)? onRemove;

  /// Does this origin answer? Null means do not claim anything about the
  /// connection — an unearned green dot is worse than no dot.
  final Future<bool> Function(String origin)? probeHealth;

  @override
  State<SignInPane> createState() => _SignInPaneState();
}

class _SignInPaneState extends State<SignInPane> {
  /// Which tab. Starts on the one matching the world already in effect, so the
  /// screen opens describing where you actually are.
  late bool _cloud = widget.active != kLocalOrigin;
  bool _serversOpen = false;

  ServerReach? _reach;
  String? _probed;

  @override
  void initState() {
    super.initState();
    _probe();
  }

  @override
  void didUpdateWidget(SignInPane old) {
    super.didUpdateWidget(old);
    if (old.active != widget.active) _probe();
  }

  /// Ask the current server whether it is there.
  ///
  /// The answer is dropped unless it is still about the origin we asked about, so
  /// switching servers quickly cannot show the previous one's verdict.
  void _probe() {
    final probe = widget.probeHealth;
    final origin = widget.active;
    if (probe == null || origin == kLocalOrigin) {
      setState(() {
        _reach = null;
        _probed = null;
      });
      return;
    }
    setState(() {
      _reach = ServerReach.checking;
      _probed = origin;
    });
    probe(origin)
        .then((ok) {
          if (!mounted || _probed != origin) return;
          setState(
            () => _reach = ok ? ServerReach.reachable : ServerReach.unreachable,
          );
        })
        .catchError((_) {
          if (!mounted || _probed != origin) return;
          setState(() => _reach = ServerReach.unreachable);
        });
  }

  @override
  Widget build(BuildContext context) {
    final hasServer = widget.origins.isNotEmpty;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _tabs(),
        const SizedBox(height: 24),
        if (_cloud) ...[
          // No server configured yet is a real first-run state: adding one is the
          // only thing to offer, and the form would have nothing to sign in to.
          if (hasServer) _serverRow(context) else _addRow(boxed: true),
          const SizedBox(height: 24),
          if (hasServer) widget.authForm,
        ] else
          _localPanel(),
      ],
    );
  }

  Widget _tabs() {
    Widget tab(
      String label,
      IconData icon,
      bool selected,
      VoidCallback onTap,
    ) => Expanded(
      child: InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(9),
        child: Container(
          height: 38,
          alignment: Alignment.center,
          decoration: BoxDecoration(
            color: selected ? Colors.white : null,
            borderRadius: BorderRadius.circular(9),
            border: selected
                ? Border.all(color: const Color(0xFFE2E8F0))
                : null,
          ),
          child: Row(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              Icon(
                icon,
                size: 15,
                color: selected
                    ? const Color(0xFF2563EB)
                    : const Color(0xFF64748B),
              ),
              const SizedBox(width: 7),
              Text(
                label,
                style: TextStyle(
                  fontSize: 13,
                  fontWeight: selected ? FontWeight.w600 : FontWeight.w400,
                  color: selected
                      ? const Color(0xFF0F172A)
                      : const Color(0xFF64748B),
                ),
              ),
            ],
          ),
        ),
      ),
    );

    return DecoratedBox(
      decoration: BoxDecoration(
        color: const Color(0xFFF1F5F9),
        borderRadius: BorderRadius.circular(11),
      ),
      child: Padding(
        padding: const EdgeInsets.all(3),
        child: Row(
          children: [
            tab(widget.strings.cloudTab, Icons.cloud_outlined, _cloud, () {
              // Re-probe on coming back to this tab. The verdict is a moment in
              // time, and the server may well have been started since — a stale
              // 「连不上服务器」 is the same lie as an unearned green dot, just
              // pointing the other way. (Caught on a real machine: started the
              // backend after opening this screen, and the row kept saying it
              // was unreachable.)
              final wasLocal = !_cloud;
              setState(() => _cloud = true);
              if (wasLocal) _probe();
            }),
            tab(
              widget.strings.localTab,
              Icons.computer_outlined,
              !_cloud,
              () => setState(() => _cloud = false),
            ),
          ],
        ),
      ),
    );
  }

  static const _green = Color(0xFF22C55E);

  ({Color dot, String text}) get _status => switch (_reach) {
    ServerReach.reachable => (dot: _green, text: widget.strings.connected),
    ServerReach.unreachable => (
      dot: const Color(0xFFDC2626),
      text: widget.strings.unreachable,
    ),
    ServerReach.checking => (
      dot: const Color(0xFFCBD5E1),
      text: widget.strings.checking,
    ),
    // No probe wired: show the server, claim nothing about the connection.
    null => (dot: const Color(0xFFCBD5E1), text: ''),
  };

  Widget _serverRow(BuildContext context) {
    final status = _status;
    // In 本地模式 the "current server" is the first configured one — the row is
    // showing what you would sign in to, not where you are.
    final showing = widget.active == kLocalOrigin
        ? widget.origins.first
        : widget.active;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        InkWell(
          onTap: () => setState(() => _serversOpen = !_serversOpen),
          borderRadius: BorderRadius.circular(12),
          child: Container(
            padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 11),
            decoration: BoxDecoration(
              color: const Color(0xFFF8FAFC),
              borderRadius: BorderRadius.circular(12),
              border: Border.all(color: const Color(0xFFE5E9F0)),
            ),
            child: Row(
              children: [
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      if (status.text.isNotEmpty)
                        Row(
                          children: [
                            Container(
                              width: 6,
                              height: 6,
                              decoration: BoxDecoration(
                                color: status.dot,
                                shape: BoxShape.circle,
                              ),
                            ),
                            const SizedBox(width: 6),
                            Text(
                              status.text,
                              style: TextStyle(
                                fontSize: 12,
                                fontWeight: FontWeight.w600,
                                color: status.dot == _green
                                    ? const Color(0xFF16A34A)
                                    : const Color(0xFF64748B),
                              ),
                            ),
                          ],
                        ),
                      Text(
                        serverLabel(showing),
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: const TextStyle(
                          fontFamily: kMonoFont,
                          fontSize: 12.5,
                          color: Color(0xFF334155),
                        ),
                      ),
                    ],
                  ),
                ),
                AnimatedRotation(
                  turns: _serversOpen ? 0.5 : 0,
                  duration: const Duration(milliseconds: 180),
                  child: const Icon(
                    Icons.expand_more,
                    size: 18,
                    color: Color(0xFF64748B),
                  ),
                ),
              ],
            ),
          ),
        ),
        // Expands in place rather than floating: this pane already scrolls, and
        // an overlay would need its own dismiss handling to avoid a menu you
        // cannot get rid of.
        if (_serversOpen) ...[
          const SizedBox(height: 8),
          DecoratedBox(
            decoration: BoxDecoration(
              color: Colors.white,
              borderRadius: BorderRadius.circular(12),
              border: Border.all(color: const Color(0xFFE5E9F0)),
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Padding(
                  padding: const EdgeInsets.fromLTRB(14, 10, 14, 8),
                  child: Text(
                    widget.strings.serversLabel,
                    style: const TextStyle(
                      fontSize: 11,
                      fontWeight: FontWeight.w600,
                      letterSpacing: 0.6,
                      color: Color(0xFF9AA4AF),
                    ),
                  ),
                ),
                for (final origin in widget.origins) _listRow(origin),
                if (widget.onAdd != null)
                  DecoratedBox(
                    decoration: const BoxDecoration(
                      border: Border(top: BorderSide(color: Color(0xFFF1F5F9))),
                    ),
                    child: _addRow(),
                  ),
              ],
            ),
          ),
        ],
      ],
    );
  }

  Widget _listRow(String origin) {
    final selected = origin == widget.active;
    return InkWell(
      onTap: () {
        setState(() => _serversOpen = false);
        widget.onSelect(origin);
      },
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 9),
        child: Row(
          children: [
            SizedBox(
              width: 18,
              child: selected
                  ? const Icon(Icons.check, size: 15, color: Color(0xFF2563EB))
                  : null,
            ),
            const SizedBox(width: 6),
            Expanded(
              child: Text(
                serverLabel(origin),
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: const TextStyle(
                  fontFamily: kMonoFont,
                  fontSize: 12.5,
                  color: Color(0xFF24292F),
                ),
              ),
            ),
            if (widget.onRemove != null)
              IconButton(
                onPressed: () {
                  setState(() => _serversOpen = false);
                  widget.onRemove!(origin);
                },
                icon: const Icon(Icons.delete_outline, size: 16),
                color: const Color(0xFFDC2626),
                tooltip: widget.strings.removeServer,
                visualDensity: VisualDensity.compact,
                padding: EdgeInsets.zero,
                constraints: const BoxConstraints(minWidth: 28, minHeight: 28),
              ),
          ],
        ),
      ),
    );
  }

  Widget _addRow({bool boxed = false}) {
    final row = InkWell(
      onTap: widget.onAdd,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 11),
        child: Row(
          children: [
            const Icon(Icons.add, size: 16, color: Color(0xFF2563EB)),
            const SizedBox(width: 9),
            Text(
              widget.strings.addServer,
              style: const TextStyle(
                fontSize: 13,
                fontWeight: FontWeight.w600,
                color: Color(0xFF2563EB),
              ),
            ),
          ],
        ),
      ),
    );
    if (!boxed) return row;
    return DecoratedBox(
      decoration: BoxDecoration(
        color: const Color(0xFFF8FAFC),
        borderRadius: BorderRadius.circular(12),
        border: Border.all(color: const Color(0xFFE5E9F0)),
      ),
      child: row,
    );
  }

  Widget _localPanel() {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text(
          widget.strings.localTitle,
          style: const TextStyle(
            fontSize: 26,
            fontWeight: FontWeight.w700,
            letterSpacing: -0.5,
            color: Color(0xFF0F172A),
          ),
        ),
        const SizedBox(height: 8),
        Text(
          widget.strings.localBody,
          style: const TextStyle(
            fontSize: 14,
            height: 1.6,
            color: Color(0xFF64748B),
          ),
        ),
        const SizedBox(height: 24),
        SizedBox(
          height: 46,
          child: FilledButton(
            onPressed: widget.onEnterLocal,
            child: Text(widget.strings.localAction),
          ),
        ),
      ],
    );
  }
}
