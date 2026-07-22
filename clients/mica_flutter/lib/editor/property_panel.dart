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
class PropertyPanel extends StatefulWidget {
  const PropertyPanel({
    super.key,
    required this.frontMatter,
    required this.canEdit,
    required this.onCommit,
  });

  /// The raw inner front matter (no `---` fences) from the document root.
  final String frontMatter;
  final bool canEdit;

  /// Persist a new raw front-matter string (writes the root block's data).
  final Future<void> Function(String frontMatter) onCommit;

  @override
  State<PropertyPanel> createState() => _PropertyPanelState();
}

class _PropertyPanelState extends State<PropertyPanel> {
  late String _fm = widget.frontMatter;
  bool _addingKey = false;

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
          for (final p in props)
            _PropertyRow(
              key: ValueKey(p.key),
              property: p,
              canEdit: widget.canEdit,
              onChanged: (v) => _setValue(p.key, v),
              onRemove: () => _remove(p.key),
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
      ),
    );
  }
}

/// One property: a fixed-width key label + a type-appropriate value editor + a
/// remove affordance (edit mode only).
class _PropertyRow extends StatelessWidget {
  const _PropertyRow({
    super.key,
    required this.property,
    required this.canEdit,
    required this.onChanged,
    required this.onRemove,
  });

  final Property property;
  final bool canEdit;
  final ValueChanged<PropertyValue> onChanged;
  final VoidCallback onRemove;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 2),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SizedBox(
            width: 120,
            child: Padding(
              padding: const EdgeInsets.only(top: 6, right: 8),
              child: Text(
                property.key,
                style: const TextStyle(
                  color: EditorTheme.muted,
                  fontSize: 13,
                  fontWeight: FontWeight.w500,
                ),
                overflow: TextOverflow.ellipsis,
              ),
            ),
          ),
          Expanded(
            child: _ValueEditor(
              value: property.value,
              canEdit: canEdit,
              onChanged: onChanged,
            ),
          ),
          if (canEdit)
            IconButton(
              icon: const Icon(Icons.close, size: 14, color: EditorTheme.faint),
              splashRadius: 14,
              constraints: const BoxConstraints(minWidth: 28, minHeight: 28),
              padding: EdgeInsets.zero,
              tooltip: MaterialLocalizations.of(context).deleteButtonTooltip,
              onPressed: onRemove,
            ),
        ],
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
  });

  final PropertyValue value;
  final bool canEdit;
  final ValueChanged<PropertyValue> onChanged;

  @override
  Widget build(BuildContext context) {
    switch (value) {
      case PropCheckbox(:final value):
        return Align(
          alignment: Alignment.centerLeft,
          child: Checkbox(
            value: value,
            visualDensity: VisualDensity.compact,
            onChanged:
                canEdit ? (v) => onChanged(PropCheckbox(v ?? false)) : null,
          ),
        );
      case PropList(:final items):
        return _TagList(
          items: items,
          canEdit: canEdit,
          onChanged: (next) => onChanged(PropList(next)),
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

  static String _numText(double n) =>
      n == n.truncateToDouble() && n.abs() < 1e15
          ? n.toInt().toString()
          : n.toString();
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
        contentPadding: EdgeInsets.symmetric(vertical: 6),
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
  });

  final List<String> items;
  final bool canEdit;
  final ValueChanged<List<String>> onChanged;

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
      padding: const EdgeInsets.only(top: 3),
      child: Wrap(
        spacing: 6,
        runSpacing: 4,
        crossAxisAlignment: WrapCrossAlignment.center,
        children: [
          for (var i = 0; i < widget.items.length; i++)
            _Chip(
              label: widget.items[i],
              onDeleted: widget.canEdit ? () => _removeAt(i) : null,
            ),
          if (widget.canEdit)
            SizedBox(
              width: 96,
              child: TextField(
                controller: _add,
                style: const TextStyle(color: EditorTheme.text, fontSize: 13),
                decoration: InputDecoration(
                  isDense: true,
                  contentPadding: const EdgeInsets.symmetric(vertical: 4),
                  border: InputBorder.none,
                  hintText: context.l10n.propertyTagAdd,
                  hintStyle:
                      const TextStyle(color: EditorTheme.faint, fontSize: 13),
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
  const _Chip({required this.label, this.onDeleted});

  final String label;
  final VoidCallback? onDeleted;

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
        borderRadius: BorderRadius.circular(10),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          Text(
            label,
            style: const TextStyle(color: EditorTheme.text, fontSize: 12),
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
      padding: const EdgeInsets.only(top: 2),
      child: SizedBox(
        width: 160,
        child: TextField(
          controller: _c,
          focusNode: _focus,
          style: const TextStyle(color: EditorTheme.text, fontSize: 13),
          decoration: InputDecoration(
            isDense: true,
            contentPadding: const EdgeInsets.symmetric(vertical: 6),
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
      padding: const EdgeInsets.only(top: 2),
      child: InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(4),
        child: Padding(
          padding: const EdgeInsets.symmetric(vertical: 4, horizontal: 2),
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
