import 'package:flutter/material.dart';

import '../l10n/locale_controller.dart';
import 'properties.dart';
import 'render.dart' show EditorTheme;

/// Page-properties panel — a lazy structured view over the document's YAML front
/// matter (the SOLE authority). Renders each property with a type-appropriate
/// editor; every edit recomputes the raw front-matter string via the surgical
/// `upsertProperty` / `removeProperty` helpers (untouched keys stay byte-exact)
/// and commits it through [onCommit], which persists it onto the root block's
/// `data['front_matter']`. See docs/page-properties.md.
///
/// Layout follows AFFiNE's doc-info style: compact rows (type icon + narrow name
/// + inline value), collapsible so a page with many properties doesn't push the
/// body down — the header stays a one-line summary until expanded.
class PropertyPanel extends StatefulWidget {
  const PropertyPanel({
    super.key,
    required this.frontMatter,
    required this.canEdit,
    required this.onCommit,
    this.onOpenTag,
  });

  /// The raw inner front matter (no `---` fences) from the document root.
  final String frontMatter;
  final bool canEdit;

  /// Persist a new raw front-matter string (writes the root block's data).
  final Future<void> Function(String frontMatter) onCommit;

  /// Open workspace search for a list/tag value (click-to-find). Null disables
  /// the click affordance (e.g. the read-only / no-search contexts).
  final void Function(String value)? onOpenTag;

  @override
  State<PropertyPanel> createState() => _PropertyPanelState();
}

class _PropertyPanelState extends State<PropertyPanel> {
  late String _fm = widget.frontMatter;
  bool _addingKey = false;
  bool _collapsed = false;

  @override
  void didUpdateWidget(PropertyPanel old) {
    super.didUpdateWidget(old);
    // A remote change or reload replaced the authority — adopt it. Local edits
    // already set `_fm` and echo the same string back through the parent, so
    // this only fires for genuinely different incoming content.
    if (widget.frontMatter != old.frontMatter && widget.frontMatter != _fm) {
      _fm = widget.frontMatter;
    }
  }

  void _commit(String next) {
    setState(() => _fm = next);
    widget.onCommit(next);
  }

  void _setValue(String key, PropertyValue value) =>
      _commit(upsertProperty(_fm, key, value));

  void _remove(String key) => _commit(removeProperty(_fm, key));

  void _addKey(String key) {
    setState(() => _addingKey = false);
    final k = key.trim();
    if (k.isEmpty) return;
    // A new key starts as empty text; typing a value infers its type.
    _setValue(k, const PropText(''));
  }

  @override
  Widget build(BuildContext context) {
    final props = parseProperties(_fm);
    // Nothing to show and nothing to add → take no vertical space at all.
    if (props.isEmpty && !widget.canEdit) return const SizedBox.shrink();

    return Padding(
      padding: const EdgeInsets.only(
        left: EditorTheme.gutter,
        top: 2,
        bottom: 6,
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          if (props.isNotEmpty)
            _CollapseHeader(
              collapsed: _collapsed,
              props: props,
              onToggle: () => setState(() => _collapsed = !_collapsed),
            ),
          if (props.isEmpty || !_collapsed) ...[
            for (final p in props)
              _PropertyRow(
                key: ValueKey(p.key),
                property: p,
                canEdit: widget.canEdit,
                onChanged: (v) => _setValue(p.key, v),
                onRemove: () => _remove(p.key),
                onOpenTag: widget.onOpenTag,
              ),
            if (widget.canEdit)
              _addingKey
                  ? _KeyField(
                      onSubmit: _addKey,
                      onCancel: () => setState(() => _addingKey = false),
                    )
                  : _AddPropertyButton(
                      onTap: () => setState(() => _addingKey = true),
                    ),
          ],
        ],
      ),
    );
  }
}

/// One-line collapse control. Expanded → just a down-chevron. Collapsed → a
/// right-chevron plus a faint summary of the property values, so you can see
/// what's there without expanding.
class _CollapseHeader extends StatelessWidget {
  const _CollapseHeader({
    required this.collapsed,
    required this.props,
    required this.onToggle,
  });

  final bool collapsed;
  final List<Property> props;
  final VoidCallback onToggle;

  @override
  Widget build(BuildContext context) {
    return InkWell(
      onTap: onToggle,
      borderRadius: BorderRadius.circular(4),
      child: Padding(
        padding: const EdgeInsets.symmetric(vertical: 2),
        child: Row(
          children: [
            Icon(
              collapsed ? Icons.chevron_right : Icons.expand_more,
              size: 16,
              color: EditorTheme.faint,
            ),
            const SizedBox(width: 2),
            if (collapsed)
              Expanded(
                child: Text(
                  _summary(props),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: const TextStyle(color: EditorTheme.faint, fontSize: 12),
                ),
              )
            else
              Text(
                '${props.length}',
                style: const TextStyle(color: EditorTheme.faint, fontSize: 12),
              ),
          ],
        ),
      ),
    );
  }

  static String _summary(List<Property> props) {
    final bits = <String>[];
    for (final p in props) {
      final v = switch (p.value) {
        PropText(:final value) => value,
        PropNumber(:final value) => _numText(value),
        PropCheckbox(:final value) => value ? '✓' : '✗',
        PropDate(:final value) => value,
        PropList(:final items) => items.join(', '),
      };
      bits.add(v.isEmpty ? p.key : '${p.key}: $v');
    }
    return bits.join('  ·  ');
  }
}

String _numText(double n) =>
    n == n.truncateToDouble() && n.abs() < 1e15
        ? n.toInt().toString()
        : n.toString();

IconData _typeIcon(PropertyValue v) => switch (v) {
      PropText() => Icons.subject,
      PropNumber() => Icons.tag,
      PropCheckbox() => Icons.check_box_outlined,
      PropDate() => Icons.calendar_today_outlined,
      PropList() => Icons.sell_outlined,
    };

/// One property: a small type icon + a narrow key label + a type-appropriate
/// value editor. The remove × is revealed on hover (edit mode only) to keep the
/// row uncluttered.
class _PropertyRow extends StatefulWidget {
  const _PropertyRow({
    super.key,
    required this.property,
    required this.canEdit,
    required this.onChanged,
    required this.onRemove,
    this.onOpenTag,
  });

  final Property property;
  final bool canEdit;
  final ValueChanged<PropertyValue> onChanged;
  final VoidCallback onRemove;
  final void Function(String value)? onOpenTag;

  @override
  State<_PropertyRow> createState() => _PropertyRowState();
}

class _PropertyRowState extends State<_PropertyRow> {
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    return MouseRegion(
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: Padding(
        padding: const EdgeInsets.symmetric(vertical: 1),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Padding(
              padding: const EdgeInsets.only(top: 5),
              child: Icon(
                _typeIcon(widget.property.value),
                size: 14,
                color: EditorTheme.faint,
              ),
            ),
            const SizedBox(width: 6),
            Padding(
              padding: const EdgeInsets.only(top: 3),
              child: SizedBox(
                width: 84,
                child: Text(
                  widget.property.key,
                  style: const TextStyle(
                    color: EditorTheme.muted,
                    fontSize: 13,
                  ),
                  overflow: TextOverflow.ellipsis,
                ),
              ),
            ),
            const SizedBox(width: 8),
            Expanded(
              child: _ValueEditor(
                value: widget.property.value,
                canEdit: widget.canEdit,
                onChanged: widget.onChanged,
                onOpenTag: widget.onOpenTag,
              ),
            ),
            // Remove × — reserved space always (no layout jump), shown on hover.
            SizedBox(
              width: 22,
              child: (widget.canEdit && _hover)
                  ? IconButton(
                      icon: const Icon(Icons.close,
                          size: 13, color: EditorTheme.faint),
                      splashRadius: 12,
                      visualDensity: VisualDensity.compact,
                      constraints:
                          const BoxConstraints(minWidth: 22, minHeight: 22),
                      padding: EdgeInsets.zero,
                      tooltip:
                          MaterialLocalizations.of(context).deleteButtonTooltip,
                      onPressed: widget.onRemove,
                    )
                  : null,
            ),
          ],
        ),
      ),
    );
  }
}

/// Dispatches to a checkbox, a tag/list chip editor, or a scalar text field by
/// the value's type.
class _ValueEditor extends StatelessWidget {
  const _ValueEditor({
    required this.value,
    required this.canEdit,
    required this.onChanged,
    this.onOpenTag,
  });

  final PropertyValue value;
  final bool canEdit;
  final ValueChanged<PropertyValue> onChanged;
  final void Function(String value)? onOpenTag;

  @override
  Widget build(BuildContext context) {
    switch (value) {
      case PropCheckbox(:final value):
        return Align(
          alignment: Alignment.centerLeft,
          child: SizedBox(
            height: 24,
            width: 24,
            child: Checkbox(
              value: value,
              visualDensity: VisualDensity.compact,
              materialTapTargetSize: MaterialTapTargetSize.shrinkWrap,
              onChanged:
                  canEdit ? (v) => onChanged(PropCheckbox(v ?? false)) : null,
            ),
          ),
        );
      case PropList(:final items):
        return _TagList(
          items: items,
          canEdit: canEdit,
          onChanged: (next) => onChanged(PropList(next)),
          onOpenTag: onOpenTag,
        );
      case PropText(:final value):
        return _ScalarField(text: value, canEdit: canEdit, onChanged: onChanged);
      case PropNumber(:final value):
        return _ScalarField(
          text: _numText(value),
          canEdit: canEdit,
          onChanged: onChanged,
        );
      case PropDate(:final value):
        return _ScalarField(text: value, canEdit: canEdit, onChanged: onChanged);
    }
  }
}

/// A text field for a scalar property. Commits on submit / focus loss, and
/// re-infers the type from the raw text (typing `true`/`42`/`2026-01-01` flips
/// the property's type, matching how front-matter values are typed by shape).
class _ScalarField extends StatefulWidget {
  const _ScalarField({
    required this.text,
    required this.canEdit,
    required this.onChanged,
  });

  final String text;
  final bool canEdit;
  final ValueChanged<PropertyValue> onChanged;

  @override
  State<_ScalarField> createState() => _ScalarFieldState();
}

class _ScalarFieldState extends State<_ScalarField> {
  late final TextEditingController _c = TextEditingController(text: widget.text);
  final FocusNode _focus = FocusNode();

  @override
  void initState() {
    super.initState();
    _focus.addListener(() {
      if (!_focus.hasFocus) _commit();
    });
  }

  @override
  void didUpdateWidget(_ScalarField old) {
    super.didUpdateWidget(old);
    // Adopt an external (remote) change only while the user isn't editing.
    if (widget.text != old.text && !_focus.hasFocus && _c.text != widget.text) {
      _c.text = widget.text;
    }
  }

  void _commit() {
    if (_c.text == widget.text) return;
    widget.onChanged(inferValue(_c.text));
  }

  @override
  void dispose() {
    _c.dispose();
    _focus.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return TextField(
      controller: _c,
      focusNode: _focus,
      enabled: widget.canEdit,
      style: const TextStyle(color: EditorTheme.text, fontSize: 13),
      decoration: const InputDecoration(
        isDense: true,
        isCollapsed: true,
        contentPadding: EdgeInsets.symmetric(vertical: 4),
        border: InputBorder.none,
        hintText: '—',
        hintStyle: TextStyle(color: EditorTheme.faint, fontSize: 13),
      ),
      onSubmitted: (_) => _commit(),
    );
  }
}

/// A list/tags editor: chips (each removable) plus an inline add field.
class _TagList extends StatefulWidget {
  const _TagList({
    required this.items,
    required this.canEdit,
    required this.onChanged,
    this.onOpenTag,
  });

  final List<String> items;
  final bool canEdit;
  final ValueChanged<List<String>> onChanged;
  final void Function(String value)? onOpenTag;

  @override
  State<_TagList> createState() => _TagListState();
}

class _TagListState extends State<_TagList> {
  final TextEditingController _add = TextEditingController();

  void _submitAdd(String raw) {
    final t = raw.trim();
    _add.clear();
    if (t.isEmpty) return;
    widget.onChanged([...widget.items, t]);
  }

  void _removeAt(int i) {
    final next = [...widget.items]..removeAt(i);
    widget.onChanged(next);
  }

  @override
  void dispose() {
    _add.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(top: 2),
      child: Wrap(
        spacing: 6,
        runSpacing: 4,
        crossAxisAlignment: WrapCrossAlignment.center,
        children: [
          for (var i = 0; i < widget.items.length; i++)
            _Chip(
              label: widget.items[i],
              onDeleted: widget.canEdit ? () => _removeAt(i) : null,
              onTap: widget.onOpenTag == null
                  ? null
                  : () => widget.onOpenTag!(widget.items[i]),
            ),
          if (widget.canEdit)
            SizedBox(
              width: 84,
              child: TextField(
                controller: _add,
                style: const TextStyle(color: EditorTheme.text, fontSize: 13),
                decoration: InputDecoration(
                  isDense: true,
                  isCollapsed: true,
                  contentPadding: const EdgeInsets.symmetric(vertical: 4),
                  border: InputBorder.none,
                  hintText: context.l10n.propertyTagAdd,
                  hintStyle:
                      const TextStyle(color: EditorTheme.faint, fontSize: 12),
                ),
                onSubmitted: _submitAdd,
              ),
            ),
        ],
      ),
    );
  }
}

class _Chip extends StatelessWidget {
  const _Chip({required this.label, this.onDeleted, this.onTap});

  final String label;
  final VoidCallback? onDeleted;

  /// Tapping the chip body (not the delete ×) — used to search for the tag.
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: EdgeInsets.only(
        left: 8,
        right: onDeleted == null ? 8 : 3,
        top: 2,
        bottom: 2,
      ),
      decoration: BoxDecoration(
        color: EditorTheme.codeBg,
        borderRadius: BorderRadius.circular(6),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          MouseRegion(
            cursor:
                onTap == null ? MouseCursor.defer : SystemMouseCursors.click,
            child: GestureDetector(
              onTap: onTap,
              child: Text(
                label,
                style: const TextStyle(color: EditorTheme.text, fontSize: 12),
              ),
            ),
          ),
          if (onDeleted != null)
            GestureDetector(
              onTap: onDeleted,
              child: const Padding(
                padding: EdgeInsets.only(left: 3),
                child: Icon(Icons.close, size: 12, color: EditorTheme.faint),
              ),
            ),
        ],
      ),
    );
  }
}

/// The inline "type a new property name" field, shown after tapping "+ add".
class _KeyField extends StatefulWidget {
  const _KeyField({required this.onSubmit, required this.onCancel});

  final ValueChanged<String> onSubmit;
  final VoidCallback onCancel;

  @override
  State<_KeyField> createState() => _KeyFieldState();
}

class _KeyFieldState extends State<_KeyField> {
  final TextEditingController _c = TextEditingController();
  final FocusNode _focus = FocusNode();

  @override
  void initState() {
    super.initState();
    _focus.requestFocus();
    _focus.addListener(() {
      if (!_focus.hasFocus) widget.onCancel();
    });
  }

  @override
  void dispose() {
    _c.dispose();
    _focus.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(top: 2, left: 20),
      child: SizedBox(
        width: 160,
        child: TextField(
          controller: _c,
          focusNode: _focus,
          style: const TextStyle(color: EditorTheme.text, fontSize: 13),
          decoration: InputDecoration(
            isDense: true,
            isCollapsed: true,
            contentPadding: const EdgeInsets.symmetric(vertical: 5),
            border: InputBorder.none,
            hintText: context.l10n.propertyKeyHint,
            hintStyle: const TextStyle(color: EditorTheme.faint, fontSize: 13),
          ),
          onSubmitted: widget.onSubmit,
        ),
      ),
    );
  }
}

class _AddPropertyButton extends StatelessWidget {
  const _AddPropertyButton({required this.onTap});

  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(top: 1),
      child: InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(4),
        child: Padding(
          padding: const EdgeInsets.symmetric(vertical: 3, horizontal: 2),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              const Icon(Icons.add, size: 15, color: EditorTheme.faint),
              const SizedBox(width: 4),
              Text(
                context.l10n.propertyAdd,
                style: const TextStyle(color: EditorTheme.faint, fontSize: 13),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
