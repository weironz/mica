/// The account menu entry and the detailed About control share one updater.
library;

import 'typography.dart';

import 'dart:async';

import 'package:flutter/material.dart';

import '../editor/open_url.dart';
import '../l10n/locale_controller.dart';
import '../updater_common.dart';
import '../updater_controller.dart';
import 'status_kit.dart';
import 'theme_tokens.dart';

export '../updater_controller.dart';

Future<void> _confirmUpdate(
  BuildContext context,
  DesktopUpdateController controller,
) => controller.confirmAndApply((info) async {
  if (!context.mounted) return false;
  return await showDialog<bool>(
        context: context,
        builder: (ctx) => AlertDialog(
          title: Text(ctx.l10n.updateDialogTitle(info.version)),
          content: Text(ctx.l10n.updateDialogBody),
          actions: [
            TextButton(
              onPressed: () => Navigator.of(ctx).pop(false),
              child: Text(ctx.l10n.commonCancel),
            ),
            FilledButton(
              onPressed: () => Navigator.of(ctx).pop(true),
              child: Text(ctx.l10n.updateAndRestart),
            ),
          ],
        ),
      ) ==
      true;
});

/// Keeps the shared updater checking in the background for as long as [child]
/// is mounted. It draws nothing itself: the account menu shows the result.
class UpdateWatcher extends StatefulWidget {
  const UpdateWatcher({
    super.key,
    required this.controller,
    required this.child,
  });

  final DesktopUpdateController controller;
  final Widget child;

  @override
  State<UpdateWatcher> createState() => _UpdateWatcherState();
}

class _UpdateWatcherState extends State<UpdateWatcher>
    with WidgetsBindingObserver {
  Timer? _timer;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    _scheduleChecks();
  }

  @override
  void didUpdateWidget(UpdateWatcher oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.controller != widget.controller) _scheduleChecks();
  }

  void _scheduleChecks() {
    _timer?.cancel();
    final controller = widget.controller;
    if (!controller.supported) return;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted && controller == widget.controller) {
        unawaited(controller.check());
      }
    });
    _timer = Timer.periodic(
      DesktopUpdateController.checkInterval,
      (_) => unawaited(controller.check(force: true)),
    );
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state == AppLifecycleState.resumed && widget.controller.supported) {
      unawaited(widget.controller.check());
    }
  }

  @override
  void dispose() {
    _timer?.cancel();
    WidgetsBinding.instance.removeObserver(this);
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => widget.child;
}
Future<void> activateDesktopUpdate(
  BuildContext context,
  DesktopUpdateController controller,
) async {
  if (controller.isBusy) return;
  if (controller.info == null) {
    await controller.check(force: true);
    if (!context.mounted) return;
  }
  if (controller.info != null) {
    await _confirmUpdate(context, controller);
    if (!context.mounted) return;
  }
  if (controller.stage == DesktopUpdateStage.upToDate ||
      controller.stage == DesktopUpdateStage.error) {
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(
        content: Text(
          controller.error == null
              ? context.l10n.updateUpToDate(controller.currentVersion)
              : context.l10n.updateFailed(controller.error.toString()),
        ),
      ),
    );
  }
}

/// Detailed status remains available from Settings > About.
class UpdateChecker extends StatelessWidget {
  const UpdateChecker({super.key, required this.controller});

  final DesktopUpdateController controller;

  @override
  Widget build(BuildContext context) {
    if (!controller.supported) return const SizedBox.shrink();
    return ListenableBuilder(
      listenable: controller,
      builder: (context, _) => _buildStatus(context),
    );
  }

  Widget _buildStatus(BuildContext context) {
    final check = controller.isBusy
        ? null
        : () => controller.check(force: true);
    switch (controller.stage) {
      case DesktopUpdateStage.idle:
        return Align(
          alignment: Alignment.centerLeft,
          child: OutlinedButton.icon(
            icon: const Icon(Icons.system_update_alt, size: 18),
            label: Text(context.l10n.updateCheck),
            onPressed: check,
          ),
        );
      case DesktopUpdateStage.checking:
        return Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            const SizedBox(
              width: 16,
              height: 16,
              child: CircularProgressIndicator(strokeWidth: 2),
            ),
            const SizedBox(width: 10),
            Text(context.l10n.updateChecking),
          ],
        );
      case DesktopUpdateStage.upToDate:
        return Row(
          children: [
            Icon(
              Icons.check_circle_outline,
              size: 18,
              color: MicaTheme.of(context).status.success,
            ),
            const SizedBox(width: 8),
            Text(context.l10n.updateUpToDate(controller.currentVersion)),
            const Spacer(),
            TextButton(
              onPressed: check,
              child: Text(context.l10n.updateRecheck),
            ),
          ],
        );
      case DesktopUpdateStage.available:
        final info = controller.info!;
        return Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Icon(
                  Icons.new_releases_outlined,
                  size: 18,
                  color: MicaTheme.of(context).text.primary,
                ),
                const SizedBox(width: 8),
                Expanded(
                  child: Text(
                    context.l10n.updateFound(
                      info.version,
                      controller.currentVersion,
                    ),
                  ),
                ),
              ],
            ),
            const SizedBox(height: 10),
            Align(
              alignment: Alignment.centerLeft,
              child: FilledButton.icon(
                icon: const Icon(Icons.download, size: 18),
                label: Text(context.l10n.updateNow),
                onPressed: controller.isBusy
                    ? null
                    : () => _confirmUpdate(context, controller),
              ),
            ),
          ],
        );
      case DesktopUpdateStage.downloading:
        return Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              context.l10n.updateDownloading(
                (controller.progress * 100).round(),
              ),
            ),
            const SizedBox(height: 8),
            LinearProgressIndicator(
              value: controller.progress > 0 ? controller.progress : null,
            ),
            const SizedBox(height: 6),
            Text(
              context.l10n.updateWillRestart,
              style: MicaTypography.caption.copyWith(
                color: MicaTheme.of(context).text.muted,
              ),
            ),
          ],
        );
      case DesktopUpdateStage.error:
        if (controller.integrityFailed) {
          return MicaFailureCard(
            icon: Icons.gpp_maybe_outlined,
            title: context.l10n.updaterAbortedTitle,
            body: context.l10n.updaterAbortedBody(controller.currentVersion),
            secondaryLabel: context.l10n.updaterOpenReleases,
            onSecondary: () =>
                openUrl('https://github.com/$kUpdateRepo/releases'),
            primaryLabel: context.l10n.commonRetry,
            onPrimary: () => controller.check(force: true),
          );
        }
        return Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Icon(
                  Icons.error_outline,
                  size: 18,
                  color: MicaTheme.of(context).status.danger,
                ),
                const SizedBox(width: 8),
                Expanded(
                  child: Text(
                    context.l10n.updateFailed(controller.error.toString()),
                    style: MicaTypography.small,
                  ),
                ),
              ],
            ),
            const SizedBox(height: 8),
            Align(
              alignment: Alignment.centerLeft,
              child: TextButton(
                onPressed: check,
                child: Text(context.l10n.commonRetry),
              ),
            ),
          ],
        );
    }
  }
}
