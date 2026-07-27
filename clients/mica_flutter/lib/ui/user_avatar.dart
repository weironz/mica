// The one circle that stands for a person.
//
// Everywhere a user appears — the account tile, the member list, the settings
// account panel — the same widget draws them, so "has a picture" and "has only
// a letter" cannot look like two different products.

import 'package:flutter/material.dart';

/// A user's profile picture, falling back to their initial.
///
/// [url] null means the user has no picture (see `avatar_url.dart`); the initial
/// is then not a placeholder for a failed load but the real, intended rendering.
///
/// A load that fails lands in exactly the same place. That is deliberate: an
/// avatar is decoration, and a broken-image glyph in the corner of the sidebar
/// would look like something is wrong with the app when the only thing wrong is
/// that a picture did not arrive.
class UserAvatar extends StatelessWidget {
  const UserAvatar({
    super.key,
    required this.url,
    required this.fallback,
    this.radius = 16,
    this.background = const Color(0xFFE2E8F0),
    this.foreground = const Color(0xFF334155),
  });

  final String? url;

  /// What to draw without a picture — usually one initial.
  final String fallback;

  final double radius;
  final Color background;
  final Color foreground;

  @override
  Widget build(BuildContext context) {
    return CircleAvatar(
      radius: radius,
      backgroundColor: background,
      // foregroundImage, not backgroundImage: it paints OVER the child, so the
      // initial is already in place underneath and a slow or failed load never
      // leaves an empty circle.
      foregroundImage: url == null ? null : NetworkImage(url!),
      onForegroundImageError: url == null ? null : (_, _) {},
      child: Text(
        fallback,
        style: TextStyle(
          fontSize: radius * 0.875,
          fontWeight: FontWeight.w600,
          color: foreground,
        ),
      ),
    );
  }
}
