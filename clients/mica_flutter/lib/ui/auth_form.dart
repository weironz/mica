// The one cloud sign-in form.
//
// There were two: `SidePanel._authForm` (web's entry screen) and the AlertDialog
// inside `_promptCloudAuth` (desktop). Same three fields, same login/register
// toggle, two implementations — so a change to one silently skipped the other,
// and the two platforms drifted into looking like different products. Same class
// of problem as every other double representation in this codebase.
//
// **All copy arrives already localized** (the `status_kit.dart` contract), which
// is also what lets this be tested: nothing here reaches for AppLocalizations.

import 'package:flutter/material.dart';

import '../api/models.dart' show AuthFormValue, AuthMode;

/// Finished strings for [AuthFormCard].
class AuthFormStrings {
  const AuthFormStrings({
    required this.title,
    required this.login,
    required this.register,
    required this.email,
    required this.displayName,
    required this.password,
    required this.forgotPassword,
  });

  /// Heading above the fields.
  final String title;

  /// Labels for the two modes — also the submit button's text.
  final String login;
  final String register;

  final String email;
  final String displayName;
  final String password;

  final String forgotPassword;
}

/// Email + password (+ display name when registering), with the mode toggle.
///
/// Owns its controllers, so their lifetime is the widget's — the alternative
/// (creating them next to a `showDialog` call and disposing when it returns)
/// is what `dialog_controllers.dart` documents as a crash.
class AuthFormCard extends StatefulWidget {
  const AuthFormCard({
    required this.strings,
    required this.onSubmit,
    this.onForgotPassword,
    this.actionLabelOverride,
    this.note,
    this.isBusy = false,
    super.key,
  });

  final AuthFormStrings strings;

  /// Hand the credentials up. The caller decides what signing in means (plain
  /// sign-in, or sign-in-then-migrate) — this form only collects.
  final Future<void> Function(AuthMode mode, AuthFormValue form) onSubmit;

  /// Receives the email currently typed (possibly empty — the caller owns the
  /// "you didn't type one" message, because that is copy). Null removes the
  /// link entirely rather than showing a dead one.
  final Future<void> Function(String email)? onForgotPassword;

  /// Replaces the submit button's text. The migrate flow says 「迁移」 there, not
  /// 「登录」, because that is what the button is about to do.
  final String? actionLabelOverride;

  /// One explanatory line under the title (the migrate flow explains itself).
  final String? note;

  final bool isBusy;

  @override
  State<AuthFormCard> createState() => _AuthFormCardState();
}

class _AuthFormCardState extends State<AuthFormCard> {
  AuthMode _mode = AuthMode.login;
  final _email = TextEditingController();
  final _displayName = TextEditingController();
  final _password = TextEditingController();

  @override
  void dispose() {
    _email.dispose();
    _displayName.dispose();
    _password.dispose();
    super.dispose();
  }

  AuthFormValue get _value => AuthFormValue(
    email: _email.text.trim(),
    displayName: _displayName.text.trim(),
    password: _password.text,
  );

  @override
  Widget build(BuildContext context) {
    final s = widget.strings;
    final registering = _mode == AuthMode.register;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(s.title, style: Theme.of(context).textTheme.titleLarge),
        if (widget.note case final note?) ...[
          const SizedBox(height: 6),
          Text(
            note,
            style: Theme.of(
              context,
            ).textTheme.bodySmall?.copyWith(color: const Color(0xFF64748B)),
          ),
        ],
        const SizedBox(height: 16),
        SegmentedButton<AuthMode>(
          segments: [
            ButtonSegment(
              value: AuthMode.login,
              icon: const Icon(Icons.login),
              label: Text(s.login),
            ),
            ButtonSegment(
              value: AuthMode.register,
              icon: const Icon(Icons.person_add),
              label: Text(s.register),
            ),
          ],
          selected: {_mode},
          onSelectionChanged: widget.isBusy
              ? null
              : (selection) => setState(() => _mode = selection.single),
        ),
        const SizedBox(height: 18),
        TextField(
          controller: _email,
          enabled: !widget.isBusy,
          keyboardType: TextInputType.emailAddress,
          decoration: InputDecoration(
            labelText: s.email,
            prefixIcon: const Icon(Icons.alternate_email),
            border: const OutlineInputBorder(),
          ),
        ),
        const SizedBox(height: 12),
        // Only registering needs a name; asking for one to log in would imply it
        // is part of the credential.
        if (registering) ...[
          TextField(
            controller: _displayName,
            enabled: !widget.isBusy,
            decoration: InputDecoration(
              labelText: s.displayName,
              prefixIcon: const Icon(Icons.badge),
              border: const OutlineInputBorder(),
            ),
          ),
          const SizedBox(height: 12),
        ],
        TextField(
          controller: _password,
          enabled: !widget.isBusy,
          obscureText: true,
          onSubmitted: widget.isBusy
              ? null
              : (_) => widget.onSubmit(_mode, _value),
          decoration: InputDecoration(
            labelText: s.password,
            prefixIcon: const Icon(Icons.lock),
            border: const OutlineInputBorder(),
          ),
        ),
        const SizedBox(height: 16),
        SizedBox(
          width: double.infinity,
          child: FilledButton.icon(
            onPressed: widget.isBusy
                ? null
                : () => widget.onSubmit(_mode, _value),
            icon: Icon(registering ? Icons.person_add : Icons.login),
            label: Text(
              widget.actionLabelOverride ?? (registering ? s.register : s.login),
            ),
          ),
        ),
        // Login tab only: someone registering cannot have forgotten a password
        // yet. A reset link is emailed; the reset itself is a web page.
        if (!registering && widget.onForgotPassword != null)
          Align(
            alignment: Alignment.centerLeft,
            child: TextButton(
              onPressed: widget.isBusy
                  ? null
                  : () => widget.onForgotPassword!(_email.text.trim()),
              child: Text(s.forgotPassword),
            ),
          ),
      ],
    );
  }
}
