import 'package:flutter/material.dart';
import 'theme_tokens.dart';
import 'typography.dart';

/// One option in an existing single-selection flow. Keyboard focus and the
/// selected/disabled semantics remain native Material/InkWell behavior.
class MicaSegmentButton extends StatelessWidget {
  const MicaSegmentButton({
    required this.label,
    required this.selected,
    required this.onSelected,
    super.key,
  });
  final Widget label;
  final bool selected;
  final ValueChanged<bool>? onSelected;
  @override
  Widget build(BuildContext context) {
    final t = MicaTheme.of(context);
    final enabled = onSelected != null;
    return Semantics(
      button: true,
      selected: selected,
      enabled: enabled,
      child: Material(
        color: selected
            ? (t.dark ? t.surface.overlay : t.surface.base)
            : Colors.transparent,
        animationDuration: Duration.zero,
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(6),
          side: selected ? BorderSide(color: t.border.normal) : BorderSide.none,
        ),
        child: InkWell(
          borderRadius: BorderRadius.circular(6),
          onTap: enabled ? () => onSelected!(!selected) : null,
          child: ConstrainedBox(
            constraints: const BoxConstraints(minHeight: 32),
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
              child: DefaultTextStyle.merge(
                style: MicaTypography.body.copyWith(
                  fontWeight: selected ? FontWeight.w500 : FontWeight.w400,
                  color: enabled
                      ? (selected ? t.text.primary : t.text.muted)
                      : t.text.ghost,
                ),
                child: label,
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class MicaSegmentGroup extends StatelessWidget {
  const MicaSegmentGroup({required this.children, super.key});
  final List<Widget> children;
  @override
  Widget build(BuildContext context) => Align(
    alignment: AlignmentDirectional.centerStart,
    heightFactor: 1,
    child: DecoratedBox(
      decoration: BoxDecoration(
        color: MicaTheme.of(context).surface.sunken,
        borderRadius: BorderRadius.circular(6),
      ),
      child: Padding(
        padding: const EdgeInsets.all(2),
        child: Wrap(spacing: 2, runSpacing: 2, children: children),
      ),
    ),
  );
}
