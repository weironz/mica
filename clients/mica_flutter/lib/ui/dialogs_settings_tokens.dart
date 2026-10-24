part of '../main.dart';

// API-token tab: create/reveal/revoke. Cloud only.
extension _SettingsTokensTab on _SettingsDialogState {
  Future<void> _showTokenSecret(Map<String, dynamic> created) {
    final token = created['token'] as String? ?? '';
    return showDialog<void>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: Text(context.l10n.tokenCreated),
        content: SizedBox(
          width: 420,
          // Green, and only here: this is the one moment the secret exists in the
          // UI, and the panel has to look different from every other grey box so
          // 「立即复制」 is not read as boilerplate.
          child: Container(
            decoration: BoxDecoration(
              color: MicaTheme.of(context).status.successWash,
              border: Border.all(
                color: MicaTheme.of(
                  context,
                ).status.success.withValues(alpha: 0.45),
                width: 1.5,
              ),
              borderRadius: BorderRadius.circular(14),
            ),
            padding: const EdgeInsets.all(14),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Row(
                  children: [
                    Icon(
                      Icons.check_circle_outline,
                      size: 16,
                      color: MicaTheme.of(context).status.success,
                    ),
                    const SizedBox(width: 7),
                    Expanded(
                      child: Text(
                        context.l10n.tokenCopyNow,
                        style: TextStyle(
                          fontSize: 12.5,
                          height: 1.5,
                          color: MicaTheme.of(context).status.success,
                        ),
                      ),
                    ),
                  ],
                ),
                const SizedBox(height: 11),
                Container(
                  width: double.infinity,
                  padding: const EdgeInsets.symmetric(
                    horizontal: 11,
                    vertical: 10,
                  ),
                  decoration: BoxDecoration(
                    color: MicaTheme.of(context).surface.base,
                    border: Border.all(
                      color: MicaTheme.of(
                        context,
                      ).status.success.withValues(alpha: 0.3),
                    ),
                    borderRadius: BorderRadius.circular(8),
                  ),
                  // Monospace via kMonoFont, not 'monospace': that family name
                  // does not resolve on web (model.dart). Selectable so the
                  // secret can still be taken by hand if the clipboard is
                  // locked down.
                  child: SelectableText(
                    token,
                    maxLines: 1,
                    style: const TextStyle(fontSize: 13, fontFamily: kMonoFont),
                  ),
                ),
              ],
            ),
          ),
        ),
        actions: [
          // The secret is unrecoverable, so "did that work?" is a question
          // worth answering — but the button answers it in place. A snackbar
          // here had to crawl out from under the dialog to be seen at all.
          InlineCopyButton(
            label: context.l10n.commonCopy,
            tooltip: context.l10n.commonCopy,
            size: 18,
            onCopy: () async {
              await Clipboard.setData(ClipboardData(text: token));
              return true;
            },
          ),
          FilledButton(
            onPressed: () => Navigator.of(ctx).pop(),
            child: Text(context.l10n.commonDone),
          ),
        ],
      ),
    );
  }

  /// Whether this token's `expires_at` is already in the past.
  ///
  /// An unparseable or absent value reads as NOT expired: guessing "expired"
  /// would dim a live token and invite someone to revoke a key their backups
  /// still depend on.
  bool _tokenExpired(dynamic expiresAt) {
    if (expiresAt == null) return false;
    final at = DateTime.tryParse(expiresAt.toString());
    if (at == null) return false;
    return at.isBefore(DateTime.now());
  }

  /// Read-only vs read-write, as a badge.
  ///
  /// The meta line used to print the server's own scope list — `read` /
  /// `read, write` — straight at the user: English, comma-joined, and in a
  /// Chinese UI. What a person needs from it is one bit, and write access is the
  /// half worth colouring.
  Widget _scopeBadge(BuildContext context, dynamic scopes) {
    final list =
        (scopes as List<dynamic>?)?.map((e) => '$e').toList() ?? const [];
    final canWrite = list.contains('write');
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 7, vertical: 1),
      decoration: BoxDecoration(
        color: canWrite
            ? MicaTheme.of(context).status.warningWash
            : MicaTheme.of(context).surface.hover,
        borderRadius: BorderRadius.circular(5),
      ),
      child: Text(
        canWrite ? context.l10n.tokenScopeWrite : context.l10n.tokenScopeRead,
        style: TextStyle(
          fontSize: 11,
          fontWeight: FontWeight.w500,
          color: canWrite
              ? MicaTheme.of(context).status.warning
              : MicaTheme.of(context).text.muted,
        ),
      ),
    );
  }

  /// Not static: a null timestamp means "no expiry", and that word has to be
  /// localized — it shipped as the English literal `never`, so the Chinese UI
  /// read 「过期 never」 while `tokenNever` sat unused behind the input hint.
  String _shortTime(dynamic value) {
    if (value == null) return context.l10n.tokenNever;
    final s = value.toString();
    return s.length >= 10 ? s.substring(0, 10) : s;
  }

  /// Revocation takes effect on the server the instant it returns: every cron
  /// job, script and CLI holding this token starts getting 401s. It used to
  /// fire straight off the icon button, so one mis-click silently broke someone
  /// else's backup with no way to put the secret back.
  Future<bool> _confirmRevoke(String label) {
    final l10n = context.l10n;
    return showDestructiveConfirm(
      context,
      title: l10n.tokenRevokeConfirmTitle(label),
      body: l10n.tokenRevokeConfirmBody,
      confirmLabel: l10n.tokenRevoke,
      cancelLabel: l10n.commonCancel,
    );
  }

  List<Widget> _tokensSection(BuildContext context) {
    final onLoad = widget.onLoadTokens;
    final onCreate = widget.onCreateToken;
    final onRevoke = widget.onRevokeToken;
    if (onLoad == null || onCreate == null || onRevoke == null) {
      return const [];
    }

    // Lazy-load the list the first time the tab is shown.
    if (!_tokensLoaded) {
      _tokensLoaded = true;
      onLoad()
          .then((list) {
            if (mounted) setUi(() => _tokens = list);
          })
          .catchError((Object e) {
            if (mounted) setUi(() => _tokensError = e.toString());
          });
    }

    Future<void> refresh() async {
      try {
        final list = await onLoad();
        if (mounted) {
          setUi(() {
            _tokens = list;
            _tokensError = null;
          });
        }
      } catch (e) {
        if (mounted) setUi(() => _tokensError = e.toString());
      }
    }

    return [
      const SizedBox(height: 4),
      Text(
        context.l10n.tokenDescription,
        style: Theme.of(context).textTheme.bodySmall?.copyWith(
          color: MicaTheme.of(context).text.muted,
        ),
      ),
      const SizedBox(height: 16),
      TextField(
        controller: _tokenName,
        decoration: InputDecoration(
          labelText: context.l10n.tokenName,
          hintText: context.l10n.tokenNameHint,
          border: const OutlineInputBorder(),
          isDense: true,
        ),
      ),
      const SizedBox(height: 10),
      Row(
        children: [
          Switch(
            value: _tokenWrite,
            onChanged: _tokenBusy ? null : (v) => setUi(() => _tokenWrite = v),
          ),
          Text(context.l10n.tokenWriteAccess),
          const Spacer(),
          SizedBox(
            width: 160,
            child: DropdownButtonFormField<int?>(
              initialValue: _tokenExpiryDays,
              isDense: true,
              decoration: InputDecoration(
                labelText: context.l10n.tokenExpires,
                border: const OutlineInputBorder(),
                isDense: true,
              ),
              // `never` stays first and stays the default: it is what shipped,
              // and quietly starting to expire tokens people already treat as
              // permanent would break their cron jobs on a timer.
              items: [
                DropdownMenuItem(
                  value: null,
                  child: Text(context.l10n.tokenNever),
                ),
                for (final d in const [30, 90, 365])
                  DropdownMenuItem(
                    value: d,
                    child: Text(context.l10n.tokenExpiryDays(d)),
                  ),
              ],
              onChanged: _tokenBusy
                  ? null
                  : (v) => setUi(() => _tokenExpiryDays = v),
            ),
          ),
        ],
      ),
      const SizedBox(height: 10),
      Align(
        alignment: Alignment.centerLeft,
        child: FilledButton.icon(
          icon: const Icon(Icons.add, size: 18),
          label: Text(context.l10n.tokenCreate),
          onPressed: _tokenBusy
              ? null
              : () async {
                  final name = _tokenName.text.trim();
                  if (name.isEmpty) {
                    setUi(() => _tokensError = context.l10n.tokenNameRequired);
                    return;
                  }
                  setUi(() {
                    _tokenBusy = true;
                    _tokensError = null;
                  });
                  try {
                    final scopes = _tokenWrite
                        ? <String>['read', 'write']
                        : <String>['read'];
                    final days = _tokenExpiryDays;
                    final created = await onCreate(name, scopes, days);
                    _tokenName.clear();

                    if (mounted) {
                      setUi(() {
                        _tokenWrite = false;
                        _tokenBusy = false;
                      });
                      await _showTokenSecret(created);
                    }
                    await refresh();
                  } catch (e) {
                    if (mounted) {
                      setUi(() {
                        _tokensError = e.toString();
                        _tokenBusy = false;
                      });
                    }
                  }
                },
        ),
      ),
      if (_tokensError != null) ...[
        const SizedBox(height: 12),
        ErrorBanner(_tokensError!),
      ],
      const SizedBox(height: 18),
      const Divider(height: 1),
      const SizedBox(height: 8),
      if (_tokens == null)
        const Padding(
          padding: EdgeInsets.all(16),
          child: Center(child: CircularProgressIndicator()),
        )
      else if (_tokens!.isEmpty)
        // A title with no body and no next step was the shipped state. No action
        // button: the next step is the form directly above this, so a button
        // would just point at itself.
        EmptyState(
          icon: Icons.key_outlined,
          title: context.l10n.tokenNone,
          detail: context.l10n.tokenEmptyBody,
        )
      else
        for (final t in _tokens!)
          Opacity(
            // An expired token still lists (you may want to revoke it), but it
            // can't do anything — reading as live is the misleading part.
            opacity: _tokenExpired(t['expires_at']) ? 0.62 : 1,
            child: ListTile(
              contentPadding: EdgeInsets.zero,
              dense: true,
              title: Row(
                children: [
                  Flexible(
                    child: Text(
                      t['name'] as String? ?? context.l10n.tokenUnnamed,
                      overflow: TextOverflow.ellipsis,
                    ),
                  ),
                  const SizedBox(width: 8),
                  _scopeBadge(context, t['scopes']),
                ],
              ),
              subtitle: Text(
                context.l10n.tokenMeta(
                  _shortTime(t['last_used_at']),
                  _shortTime(t['expires_at']),
                ),
              ),
              trailing: IconButton(
                icon: const Icon(Icons.delete_outline, size: 20),
                tooltip: context.l10n.tokenRevoke,
                onPressed: _tokenBusy
                    ? null
                    : () async {
                        final label =
                            t['name'] as String? ?? context.l10n.tokenUnnamed;
                        if (!await _confirmRevoke(label)) return;
                        setUi(() => _tokenBusy = true);
                        try {
                          await onRevoke(t['id'] as String);
                          await refresh();
                        } catch (e) {
                          if (mounted) setUi(() => _tokensError = e.toString());
                        } finally {
                          if (mounted) setUi(() => _tokenBusy = false);
                        }
                      },
              ),
            ),
          ),
    ];
  }
}
