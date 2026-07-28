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
    required this.form,
    this.onClose,
    super.key,
  });

  /// The brand panel (design 01's dark left side).
  final Widget hero;

  /// The credentials form.
  final Widget form;

  /// Non-null on desktop, where signing in is OPTIONAL — the app is usable in
  /// 本地模式 without an account, so this screen must be leaveable. Null on web,
  /// where there is nothing behind it to go back to.
  final VoidCallback? onClose;

  @override
  Widget build(BuildContext context) {
    final formPane = Stack(
      children: [
        Positioned.fill(
          child: ColoredBox(
            color: Colors.white,
            child: Center(
              child: SingleChildScrollView(
                padding: const EdgeInsets.symmetric(
                  horizontal: 28,
                  vertical: 32,
                ),
                child: form,
              ),
            ),
          ),
        ),
        if (onClose != null)
          Positioned(
            top: 8,
            right: 8,
            child: IconButton(
              onPressed: onClose,
              icon: const Icon(Icons.close),
              tooltip: MaterialLocalizations.of(context).closeButtonTooltip,
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
