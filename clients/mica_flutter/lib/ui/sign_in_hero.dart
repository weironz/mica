// The brand half of the sign-in screen (design 01): mark, name, one-line pitch,
// a short capability list, and a build badge.
//
// A standalone library so main.dart keeps one line of wiring, and so the layout
// can be tested — the sign-in screen itself lives inside main.dart's `build` and
// cannot be constructed by a test.
//
// **All copy arrives already localized** (same contract as `status_kit.dart`).

import 'package:flutter/material.dart';

import '../editor/render.dart' show EditorTheme;
import '../widgets/mica_logo.dart';

/// Copy for [SignInHero]. Every string is the caller's, so wording stays in the
/// arb files.
class SignInHeroStrings {
  const SignInHeroStrings({
    required this.tagline,
    required this.pitch,
    required this.features,
    required this.badge,
  });

  /// The product line, e.g. 「本地优先的协作知识库」.
  final String tagline;

  /// The paragraph under it.
  final String pitch;

  /// Short capability lines. Each must describe something that actually exists —
  /// this is the first screen a new user reads, and the easiest place in the whole
  /// app to promise something the product does not do.
  final List<String> features;

  /// Quiet footer, e.g. 「CRDT SYNC · OFFLINE-FIRST · v0.13.0」.
  final String badge;
}

/// The left half of the sign-in screen.
///
/// Replaces an `EmptyState` that said only 「登录后即可打开你的工作区」 — true, but
/// it told a first-time visitor nothing about what they were signing in to.
class SignInHero extends StatelessWidget {
  const SignInHero({required this.strings, super.key});

  final SignInHeroStrings strings;

  @override
  Widget build(BuildContext context) {
    return Container(
      color: const Color(0xFFF8FAFC),
      alignment: Alignment.center,
      child: SingleChildScrollView(
        padding: const EdgeInsets.symmetric(horizontal: 48, vertical: 40),
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 460),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Row(
                children: [
                  const MicaLogo(size: 34),
                  const SizedBox(width: 11),
                  const Text(
                    'Mica',
                    style: TextStyle(
                      fontSize: 26,
                      fontWeight: FontWeight.w700,
                      color: EditorTheme.text,
                    ),
                  ),
                ],
              ),
              const SizedBox(height: 26),
              Text(
                strings.tagline,
                style: const TextStyle(
                  fontSize: 34,
                  height: 1.3,
                  fontWeight: FontWeight.w700,
                  color: EditorTheme.text,
                ),
              ),
              const SizedBox(height: 14),
              Text(
                strings.pitch,
                style: const TextStyle(
                  fontSize: 14,
                  height: 1.75,
                  color: EditorTheme.muted,
                ),
              ),
              const SizedBox(height: 26),
              for (final f in strings.features)
                Padding(
                  padding: const EdgeInsets.only(bottom: 12),
                  child: Row(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Container(
                        width: 22,
                        height: 22,
                        decoration: BoxDecoration(
                          color: const Color(0xFFEFF6FF),
                          borderRadius: BorderRadius.circular(6),
                        ),
                        alignment: Alignment.center,
                        child: const Icon(
                          Icons.check,
                          size: 13,
                          color: EditorTheme.caret,
                        ),
                      ),
                      const SizedBox(width: 10),
                      Expanded(
                        child: Text(
                          f,
                          style: const TextStyle(
                            fontSize: 13,
                            height: 1.6,
                            color: EditorTheme.text,
                          ),
                        ),
                      ),
                    ],
                  ),
                ),
              const SizedBox(height: 14),
              Text(
                strings.badge,
                style: const TextStyle(
                  fontSize: 11,
                  letterSpacing: 0.8,
                  color: EditorTheme.faint,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
