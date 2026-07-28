/// Empty states, failure cards and status toasts — the three shapes every
/// "nothing here yet" and "this broke" moment in the app is built from.
///
/// The rule this file exists to enforce (design `19 空状态与故障态`): **every
/// empty state and every failure state says what happened and gives one next
/// step.** Never the bare words 「出错了」. That is encoded in the API rather
/// than left to discipline — each component requires a concrete title *and* a
/// concrete body, so "something went wrong" cannot be the whole message: the
/// caller has a second slot it must fill with the actual situation. Where an
/// action exists it is exactly one, and it carries a label; there is no
/// unlabelled "OK" that leaves the user where they started.
///
/// **All copy comes in from the caller, already localized** (`context.l10n.…`).
/// Nothing in this file holds a user-facing string — that keeps the arb files
/// the single source of wording and keeps this file purely layout.
library;

import 'package:flutter/material.dart';

import 'theme_tokens.dart';

// ── Palette ──────────────────────────────────────────────────────────────────
// Greys/accent are EditorTheme's, so a status surface and the canvas next to it
// can't drift apart. Only the tints that exist nowhere else are local.

/// Container border — cards and the outlined secondary button.
Color _border(BuildContext context) => MicaTheme.of(context).border.normal;

/// Accent button: accent ink on a pale wash with a slightly deeper edge. The
/// edge has no role of its own — it is the accent at low opacity, which is what
/// the design's #DBEAFE was over white, and which tints rather than glows on a
/// dark page.
Color _accentWash(BuildContext context) => MicaTheme.of(context).accent.wash;
Color _accentBorder(BuildContext context) =>
    MicaTheme.of(context).accent.primary.withValues(alpha: 0.22);

/// Failure tile tints, per severity.
Color _warnTile(BuildContext context) =>
    MicaTheme.of(context).status.warningWash;
Color _warnIcon(BuildContext context) => MicaTheme.of(context).status.warning;
Color _errorTile(BuildContext context) =>
    MicaTheme.of(context).status.dangerWash;
Color _errorIcon(BuildContext context) => MicaTheme.of(context).status.danger;

/// Toast: near-black pill, pale-blue action ink.
///
/// The pill is the one surface that does NOT invert. It is already dark, and in
/// dark mode a #0F172A pill on a #0F1419 page would simply disappear — so there
/// it lifts to the hover surface instead, which keeps it a dark pill while
/// staying visibly above the page. The pale inks below read on either.
Color _toastBg(BuildContext context) {
  final tokens = MicaTheme.of(context);
  return tokens.dark ? tokens.surface.hover : const Color(0xFF0F172A);
}

const Color _toastAction = Color(0xFF93C5FD);
Color _toastIconNeutral(BuildContext context) =>
    MicaTheme.of(context).border.strong;
const Color _toastIconAlert = Color(0xFFFCA5A5);

/// 12.5 is off the 12/13/14/16/20/26/34 ladder on purpose: it is the design's
/// measured value for the small centred body, where 13 crowds the 1.75 leading.
/// Nothing else here leaves the ladder (the design's 13.5 buttons are snapped
/// to 13).
const double _bodySize = 12.5;

// ── Empty state ──────────────────────────────────────────────────────────────

/// A centred "nothing here yet" state: tinted icon tile, title, body, and at
/// most one action.
///
/// [body] is required because the title alone is a label, not an explanation —
/// 「暂无页面」 tells the user nothing they cannot already see. The body is where
/// "what happened" lives, and [actionLabel] is the single next step. Pass no
/// action when the next step isn't a button (e.g. "open a page from the tree"):
/// then nothing is rendered rather than a dead control.
///
/// Copy ([title], [body], [actionLabel]) must arrive localized.
class MicaEmptyState extends StatelessWidget {
  const MicaEmptyState({
    required this.icon,
    required this.title,
    required this.body,
    this.actionLabel,
    this.onAction,
    super.key,
  }) : assert(
         (actionLabel == null) == (onAction == null),
         'An action needs both a label and a callback. A labelled button with no '
         'callback is the button-shaped 「出错了」: visible, and no next step.',
       );

  /// Key on the action button, so hosts (and tests) can assert its presence.
  static const Key actionKey = Key('mica.emptyState.action');

  final IconData icon;
  final String title;
  final String body;

  /// The single next step. Both null (no action) or both set.
  final String? actionLabel;
  final VoidCallback? onAction;

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 26, vertical: 40),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Container(
              width: 46,
              height: 46,
              decoration: BoxDecoration(
                color: MicaTheme.of(context).surface.sunken,
                borderRadius: BorderRadius.circular(13),
              ),
              alignment: Alignment.center,
              child: Icon(
                icon,
                size: 21,
                color: MicaTheme.of(context).text.faint,
              ),
            ),
            const SizedBox(height: 11),
            Text(
              title,
              textAlign: TextAlign.center,
              style: TextStyle(
                fontSize: 14,
                fontWeight: FontWeight.w600,
                color: MicaTheme.of(context).text.primary,
              ),
            ),
            const SizedBox(height: 9),
            Text(
              body,
              textAlign: TextAlign.center,
              style: TextStyle(
                fontSize: _bodySize,
                height: 1.75,
                color: MicaTheme.of(context).text.faint,
              ),
            ),
            if (actionLabel != null) ...[
              const SizedBox(height: 13),
              _AccentButton(
                key: actionKey,
                label: actionLabel!,
                onPressed: onAction!,
              ),
            ],
          ],
        ),
      ),
    );
  }
}

// ── Failure card ─────────────────────────────────────────────────────────────

/// How bad it is — and nothing more. The severity picks the tile tint and the
/// default icon; it never changes the copy, because "how bad" is not "what
/// happened".
enum MicaFailureSeverity {
  /// Degraded but the user's work is intact and usable (amber).
  warning,

  /// The operation failed outright (red).
  error,
}

/// The 440-wide failure card: a tinted glyph tile, a title, an explanation, and
/// one or two buttons in the bottom-right.
///
/// The shape is deliberately rigid. [body] is required and is where the design's
/// contract is met — it must say what happened, what is *not* affected, and what
/// the buttons will do (the reference copy reads "…your other pages are
/// unaffected"). There is exactly one primary action; [secondaryLabel] is for an
/// escape hatch that is genuinely different (open version history, open the
/// releases page), not for a "Cancel" that only closes the card.
///
/// Copy must arrive localized.
class MicaFailureCard extends StatelessWidget {
  const MicaFailureCard({
    required this.title,
    required this.body,
    required this.primaryLabel,
    required this.onPrimary,
    this.severity = MicaFailureSeverity.error,
    this.icon,
    this.secondaryLabel,
    this.onSecondary,
    super.key,
  }) : assert(
         (secondaryLabel == null) == (onSecondary == null),
         'The secondary action needs both a label and a callback.',
       );

  /// The design pins this card's width; exposed so a dialog can size to it.
  static const double width = 440;

  static const Key tileKey = Key('mica.failureCard.tile');
  static const Key primaryKey = Key('mica.failureCard.primary');
  static const Key secondaryKey = Key('mica.failureCard.secondary');

  final String title;
  final String body;
  final String primaryLabel;
  final VoidCallback onPrimary;
  final MicaFailureSeverity severity;

  /// Defaults to a per-severity glyph; pass one when the specific failure has a
  /// better one (a broken-link glyph for "can't reach the server", say).
  final IconData? icon;

  final String? secondaryLabel;
  final VoidCallback? onSecondary;

  Color _tileColor(BuildContext context) =>
      severity == MicaFailureSeverity.warning
      ? _warnTile(context)
      : _errorTile(context);

  Color _iconColor(BuildContext context) =>
      severity == MicaFailureSeverity.warning
      ? _warnIcon(context)
      : _errorIcon(context);

  IconData get _icon =>
      icon ??
      (severity == MicaFailureSeverity.warning
          ? Icons.warning_amber_rounded
          : Icons.error_outline);

  @override
  Widget build(BuildContext context) {
    return Container(
      width: width,
      padding: const EdgeInsets.all(24),
      decoration: BoxDecoration(
        color: MicaTheme.of(context).surface.base,
        border: Border.all(color: _border(context)),
        borderRadius: BorderRadius.circular(16),
        // A wide, far-offset, tightly-spread shadow: the card should look lifted
        // off the page without a visible grey halo around its edge.
        boxShadow: const [
          BoxShadow(
            color: Color(0x730F172A),
            offset: Offset(0, 30),
            blurRadius: 45,
            spreadRadius: -30,
          ),
        ],
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        mainAxisSize: MainAxisSize.min,
        children: [
          Row(
            children: [
              Container(
                key: tileKey,
                width: 34,
                height: 34,
                decoration: BoxDecoration(
                  color: _tileColor(context),
                  borderRadius: BorderRadius.circular(10),
                ),
                alignment: Alignment.center,
                child: Icon(_icon, size: 17, color: _iconColor(context)),
              ),
              const SizedBox(width: 10),
              Expanded(
                child: Text(
                  title,
                  style: TextStyle(
                    fontSize: 16,
                    fontWeight: FontWeight.w700,
                    color: MicaTheme.of(context).text.primary,
                  ),
                ),
              ),
            ],
          ),
          const SizedBox(height: 10),
          Text(
            body,
            style: TextStyle(
              fontSize: 13,
              height: 1.75,
              color: MicaTheme.of(context).text.muted,
            ),
          ),
          const SizedBox(height: 18),
          // Wrap, not Row: the card's width is fixed by the design but the
          // button labels are localized, and a pair of long ones must fall onto
          // a second line rather than throw an overflow — a failure card that
          // itself fails to lay out is the worst possible place for that.
          // double.infinity: the Column hands its children loose constraints, so
          // without this the Wrap shrinks to the buttons and `alignment: end`
          // has nothing to push against.
          SizedBox(
            width: double.infinity,
            child: Wrap(
              alignment: WrapAlignment.end,
              spacing: 10,
              runSpacing: 10,
              children: [
                if (secondaryLabel != null)
                  _OutlinedButton(
                    key: secondaryKey,
                    label: secondaryLabel!,
                    onPressed: onSecondary!,
                  ),
                _PrimaryButton(
                  key: primaryKey,
                  label: primaryLabel,
                  onPressed: onPrimary,
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

// ── Status toast ─────────────────────────────────────────────────────────────

/// Why the toast is up — which decides whether it may carry a button, and who
/// is allowed to take it down.
enum MicaToastKind {
  /// The app is stuck until a human chooses (session expired, sync paused).
  /// Carries exactly one action: the thing that unsticks it.
  needsDecision,

  /// The app is handling it and will recover on its own (offline, reconnecting).
  /// Carries **no** action, and must not auto-dismiss.
  ///
  /// Both halves matter. A button here would either do nothing ("Retry" while a
  /// retry is already in flight) or claim to fix something the user cannot fix
  /// — and a timer would hide a condition that is *still true*, which is how
  /// people end up typing into an offline app believing it is saving to the
  /// cloud. Remove the toast when the condition clears, not when a clock says
  /// so.
  selfHealing,
}

/// The dark status pill from the design, pinned bottom-right by
/// [MicaStatusToastHost].
///
/// [message] must name the state and, for self-healing states, its consequence
/// — "offline · edits are saved locally, they'll sync when you're back" rather
/// than "offline". With no button to carry the next step, the sentence is the
/// next step.
///
/// This widget never dismisses itself; it has no timer. The host owns the list
/// (see [MicaToastKind.selfHealing] for why that is not an oversight).
///
/// Copy must arrive localized.
class MicaStatusToast extends StatelessWidget {
  const MicaStatusToast({
    required this.kind,
    required this.icon,
    required this.message,
    this.actionLabel,
    this.onAction,
    this.iconColor,
    super.key,
  }) : assert(
         kind == MicaToastKind.needsDecision
             ? (actionLabel != null && onAction != null)
             : (actionLabel == null && onAction == null),
         'needsDecision carries exactly one labelled action (the thing that '
         'unsticks the app); selfHealing carries none — a button on a state the '
         'app is already recovering from can only lie about what it does.',
       );

  static const Key actionKey = Key('mica.statusToast.action');

  /// Widest the pill grows before its message wraps; it hugs shorter copy.
  static const double maxWidth = 420;

  final MicaToastKind kind;
  final IconData icon;
  final String message;
  final String? actionLabel;
  final VoidCallback? onAction;

  /// Defaults to neutral grey for self-healing (the app is coping, so the pill
  /// should not shout) and a warm alert tone when a decision is wanted.
  final Color? iconColor;

  @override
  Widget build(BuildContext context) {
    final tint =
        iconColor ??
        (kind == MicaToastKind.selfHealing
            ? _toastIconNeutral(context)
            : _toastIconAlert);
    return ConstrainedBox(
      constraints: const BoxConstraints(maxWidth: maxWidth),
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 13),
        decoration: BoxDecoration(
          color: _toastBg(context),
          borderRadius: BorderRadius.circular(11),
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Padding(
              padding: const EdgeInsets.only(top: 1),
              child: Icon(icon, size: 16, color: tint),
            ),
            const SizedBox(width: 11),
            Flexible(
              child: Text(
                message,
                style: const TextStyle(
                  fontSize: 13,
                  height: 1.5,
                  color: Colors.white,
                ),
              ),
            ),
            if (actionLabel != null) ...[
              const SizedBox(width: 11),
              _TextAction(
                key: actionKey,
                label: actionLabel!,
                onPressed: onAction!,
              ),
            ],
          ],
        ),
      ),
    );
  }
}

/// Stacks status toasts in the bottom-right corner, newest last, 12px apart.
///
/// One fixed corner on purpose: a status pill that moves is a status pill the
/// user has to hunt for. Drop this into a `Stack` over the app shell (e.g.
/// `Positioned.fill`) and hand it the toasts that are *currently true* — it
/// renders exactly those, and removing one is the host's decision, never a
/// timer's.
class MicaStatusToastHost extends StatelessWidget {
  const MicaStatusToastHost({
    required this.toasts,
    this.padding = const EdgeInsets.all(20),
    super.key,
  });

  final List<MicaStatusToast> toasts;

  /// Inset from the window edge; widen it to clear a floating bar.
  final EdgeInsets padding;

  @override
  Widget build(BuildContext context) {
    if (toasts.isEmpty) return const SizedBox.shrink();
    return Align(
      alignment: Alignment.bottomRight,
      child: Padding(
        padding: padding,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.end,
          children: [
            for (var i = 0; i < toasts.length; i++) ...[
              if (i > 0) const SizedBox(height: 12),
              toasts[i],
            ],
          ],
        ),
      ),
    );
  }
}

// ── Progress ─────────────────────────────────────────────────────────────────

/// A *determinate* progress row for long work whose size the server reports.
///
/// Determinate is the whole point. An indeterminate spinner and a wedged job
/// look exactly alike, and the one thing someone waiting on a 96-page import
/// wants to know is whether it is still moving. The import job had been
/// reporting `done`/`total` all along — the client parsed both and then dropped
/// them, leaving a bare spinner turning for minutes.
///
/// A caller with no real total should render nothing here rather than a bar that
/// pretends to know; hence the assert instead of a quiet fallback to
/// indeterminate.
class MicaProgressRow extends StatelessWidget {
  MicaProgressRow({
    required this.label,
    required this.done,
    required this.total,
    super.key,
  }) : assert(total > 0, 'a determinate bar needs a real total');

  /// Already-localized description, counts included — wording lives in the arb
  /// files, this file stays layout (see the library doc).
  final String label;

  final int done;
  final int total;

  @override
  Widget build(BuildContext context) {
    // Clamped because `done > total` is reachable: the planner counts pages and
    // the worker can emit extra units, and a bar overshooting its track reads as
    // a rendering bug rather than as progress.
    final fraction = (done / total).clamp(0.0, 1.0);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(
          label,
          style: TextStyle(
            fontSize: _bodySize,
            color: MicaTheme.of(context).text.muted,
          ),
        ),
        const SizedBox(height: 6),
        ClipRRect(
          borderRadius: BorderRadius.circular(3),
          child: SizedBox(
            height: 6,
            child: LinearProgressIndicator(
              value: fraction,
              backgroundColor: MicaTheme.of(context).border.subtle,
              valueColor: AlwaysStoppedAnimation(
                MicaTheme.of(context).accent.primary,
              ),
            ),
          ),
        ),
      ],
    );
  }
}

// ── Buttons ──────────────────────────────────────────────────────────────────
// Hand-rolled rather than Material's: the design's paddings and 8px radius are
// specific, and Material's defaults (44px min height, ripple, letter-spacing)
// fight them. No Material ancestor is required either, so these work inside a
// bare overlay.

class _AccentButton extends StatelessWidget {
  const _AccentButton({
    required this.label,
    required this.onPressed,
    super.key,
  });

  final String label;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) {
    return _Tappable(
      onPressed: onPressed,
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 7),
        decoration: BoxDecoration(
          color: _accentWash(context),
          border: Border.all(color: _accentBorder(context)),
          borderRadius: BorderRadius.circular(8),
        ),
        child: Text(
          label,
          style: TextStyle(
            fontSize: 13,
            fontWeight: FontWeight.w600,
            color: MicaTheme.of(context).accent.primary,
          ),
        ),
      ),
    );
  }
}

class _PrimaryButton extends StatelessWidget {
  const _PrimaryButton({
    required this.label,
    required this.onPressed,
    super.key,
  });

  final String label;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) {
    return _Tappable(
      onPressed: onPressed,
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 9),
        decoration: BoxDecoration(
          color: MicaTheme.of(context).accent.primary,
          borderRadius: BorderRadius.circular(8),
        ),
        child: Text(
          label,
          style: TextStyle(
            fontSize: 13,
            fontWeight: FontWeight.w600,
            color: MicaTheme.of(context).accent.onPrimary,
          ),
        ),
      ),
    );
  }
}

class _OutlinedButton extends StatelessWidget {
  const _OutlinedButton({
    required this.label,
    required this.onPressed,
    super.key,
  });

  final String label;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) {
    return _Tappable(
      onPressed: onPressed,
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 9),
        decoration: BoxDecoration(
          border: Border.all(color: _border(context)),
          borderRadius: BorderRadius.circular(8),
        ),
        child: Text(
          label,
          style: TextStyle(
            fontSize: 13,
            color: MicaTheme.of(context).text.muted,
          ),
        ),
      ),
    );
  }
}

/// The toast's trailing action — bare text, since the pill is already the
/// surface.
class _TextAction extends StatelessWidget {
  const _TextAction({required this.label, required this.onPressed, super.key});

  final String label;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) {
    return _Tappable(
      onPressed: onPressed,
      child: Text(
        label,
        style: const TextStyle(fontSize: 13, color: _toastAction),
      ),
    );
  }
}

class _Tappable extends StatelessWidget {
  const _Tappable({required this.child, required this.onPressed});

  final Widget child;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) {
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      child: GestureDetector(onTap: onPressed, child: child),
    );
  }
}
