// The sign-in screen, one composition for both platforms.
//
// Design 01 is a split screen: dark brand panel on the LEFT, the form on the
// RIGHT. What shipped had them mirrored (form left, near-white brand right), and
// desktop had no split screen at all — it collected the same credentials in a
// small AlertDialog, so the first thing a desktop user saw looked nothing like
// the web app.
//
// This widget takes the two halves as children and knows only how to place them,
// which is why it can be tested and why both platforms can share it: the web
// entry screen and the desktop sign-in route pass the same pair.

import 'package:flutter/material.dart';
import 'theme_tokens.dart';

/// Below this width the brand panel is dropped instead of squeezed.
///
/// A narrow desktop window would otherwise leave a form too tight to type in
/// beside a hero too narrow to read — two compromised halves instead of one
/// usable form.
const double kSignInSplitMinWidth = 900;

/// Width of the form column on the split layout.
const double kSignInFormWidth = 420;

/// Brand half + form half.
class SignInScreen extends StatelessWidget {
  const SignInScreen({
    required this.hero,
    required this.pane,
    this.onClose,
    this.closeLabel,
    super.key,
  });

  /// The brand panel (design 01's dark left side).
  final Widget hero;

  /// Everything on the right: on web just the credentials form, on desktop the
  /// world tabs + server row + that form (see `sign_in_pane.dart`). Passed in as
  /// one widget so this file stays pure layout.
  final Widget pane;

  /// Non-null on desktop, where signing in is OPTIONAL — the app is usable in
  /// 本地模式 without an account, so this screen must be leaveable. Null on web,
  /// where there is nothing behind it to go back to.
  final VoidCallback? onClose;

  /// Label for the close affordance. Null keeps the bare × (web's gate, where
  /// there is nothing behind this screen to describe going back to).
  final String? closeLabel;

  @override
  Widget build(BuildContext context) {
    final formPane = Stack(
      children: [
        Positioned.fill(
          child: ColoredBox(
            color: MicaTheme.of(context).surface.base,
            child: Center(
              child: SingleChildScrollView(
                padding: const EdgeInsets.symmetric(
                  horizontal: 28,
                  vertical: 32,
                ),
                child: pane,
              ),
            ),
          ),
        ),
        if (onClose != null)
          Positioned(
            top: 8,
            right: 8,
            // A LABELLED button when the caller supplies a label.
            //
            // This screen is opaque and fills the window, so a signed-in user
            // who opened it to switch worlds sees what looks like a logged-out
            // app — the session and every open page are still there, directly
            // behind it. Reported 2026-08-12: 「原来是页面被登录页遮挡住了，
            // 不过这个好难注意到」. A bare × in the corner is not a way back
            // that anyone finds; a word is.
            child: closeLabel == null
                ? IconButton(
                    onPressed: onClose,
                    icon: const Icon(Icons.close),
                    tooltip: MaterialLocalizations.of(
                      context,
                    ).closeButtonTooltip,
                  )
                : TextButton.icon(
                    onPressed: onClose,
                    icon: const Icon(Icons.close, size: 18),
                    label: Text(closeLabel!),
                  ),
          ),
      ],
    );

    return LayoutBuilder(
      builder: (context, constraints) {
        if (constraints.maxWidth < kSignInSplitMinWidth) {
          return formPane;
        }
        return Row(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            // Brand first: design 01 puts it on the left, and sign_in_hero.dart
            // used to be the only place that said so while the wiring did the
            // opposite.
            Expanded(child: hero),
            SizedBox(width: kSignInFormWidth, child: formPane),
          ],
        );
      },
    );
  }
}
