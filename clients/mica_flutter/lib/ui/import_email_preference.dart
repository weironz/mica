import 'package:flutter/material.dart';

import '../l10n/locale_controller.dart';

/// Account-owned preference, loaded from the server rather than device prefs.
class ImportEmailPreference extends StatefulWidget {
  const ImportEmailPreference({
    super.key,
    required this.load,
    required this.save,
  });
  final Future<bool> Function() load;
  final Future<void> Function(bool enabled) save;

  @override
  State<ImportEmailPreference> createState() => _ImportEmailPreferenceState();
}

class _ImportEmailPreferenceState extends State<ImportEmailPreference> {
  bool? _enabled;
  bool _busy = false;
  bool _failedLoad = false;
  bool _failedSave = false;
  int _request = 0;

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    final request = ++_request;
    setState(() {
      _busy = true;
      _failedLoad = false;
    });
    try {
      final enabled = await widget.load();
      if (!mounted || request != _request) return;
      setState(() => _enabled = enabled);
    } catch (_) {
      if (!mounted || request != _request) return;
      setState(() => _failedLoad = true);
    } finally {
      if (mounted && request == _request) setState(() => _busy = false);
    }
  }

  Future<void> _save(bool enabled) async {
    final request = ++_request;
    setState(() {
      _busy = true;
      _failedSave = false;
    });
    try {
      await widget.save(enabled);
      if (!mounted || request != _request) return;
      setState(() => _enabled = enabled);
    } catch (_) {
      if (!mounted || request != _request) return;
      setState(() => _failedSave = true);
    } finally {
      if (mounted && request == _request) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        SwitchListTile(
          contentPadding: EdgeInsets.zero,
          secondary: _busy
              ? const SizedBox(
                  width: 18,
                  height: 18,
                  child: CircularProgressIndicator(strokeWidth: 2),
                )
              : null,
          title: Text(l10n.importEmailTitle),
          subtitle: Text(l10n.importEmailHelp),
          value: _enabled ?? false,
          onChanged: _busy || _enabled == null ? null : _save,
        ),
        if (_failedLoad)
          Row(
            children: [
              Expanded(child: Text(l10n.importEmailLoadError)),
              TextButton(
                onPressed: _busy ? null : _load,
                child: Text(l10n.commonRetry),
              ),
            ],
          ),
        if (_failedSave) Text(l10n.importEmailSaveError),
      ],
    );
  }
}
