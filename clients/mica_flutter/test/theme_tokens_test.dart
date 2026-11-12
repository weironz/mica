import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/cjk_fonts.dart';
import 'package:mica_flutter/ui/theme_tokens.dart' show MicaTokens, MicaTheme;

/// Dark mode's failure mode is not "it crashes" — it is "you cannot read it".
/// So the assertions here are about legibility and about NOT redesigning light
/// mode by accident, rather than about specific hex values.
void main() {
  /// WCAG relative luminance.
  double lum(Color c) {
    double ch(double v) => v <= 0.03928
        ? v / 12.92
        : math.pow((v + 0.055) / 1.055, 2.4).toDouble();
    return 0.2126 * ch(c.r) + 0.7152 * ch(c.g) + 0.0722 * ch(c.b);
  }

  double contrast(Color fg, Color bg) {
    final a = lum(fg), b = lum(bg);
    final hi = math.max(a, b), lo = math.min(a, b);
    return (hi + 0.05) / (lo + 0.05);
  }

  group('phase 1 warm gray palette', () {
    test('the ink and code background match the approved warm gray values', () {
      // Specified by docs/plans/ui-visual-refresh.md; status/accent stay fixed.
      expect(MicaTokens.light.text.primary, const Color(0xFF37352F));
      expect(MicaTokens.light.text.muted, const Color(0xFF5F5E5B));
      expect(MicaTokens.light.text.faint, const Color(0xFF6C6B66));
      expect(MicaTokens.light.editor.codeBg, const Color(0xFFF7F6F3));
      expect(MicaTokens.light.editor.caret, const Color(0xFF2563EB));
      expect(MicaTokens.light.accent.primary, const Color(0xFF2563EB));
    });
  });

  group('body text stays readable in BOTH modes', () {
    for (final (name, t) in [
      ('light', MicaTokens.light),
      ('dark', MicaTokens.dark_),
    ]) {
      test('$name: primary ink on the page clears WCAG AA (4.5:1)', () {
        expect(
          contrast(t.text.primary, t.surface.base),
          greaterThanOrEqualTo(4.5),
          reason: '$name body text is not readable on its own background',
        );
      });

      test(
        '$name: muted ink still clears AA for large/secondary text (3:1)',
        () {
          // Muted carries meta lines and descriptions — it is allowed to be
          // quieter, not invisible.
          expect(
            contrast(t.text.muted, t.surface.base),
            greaterThanOrEqualTo(3.0),
          );
        },
      );

      test('$name: faint ink clears AA on every surface it sits on', () {
        // Faint carries timestamps, counts and key hints at body size, so it
        // gets the normal-text bar, not the large-text one — on the page AND
        // on the raised / sunken / hover fills of panels, rows and menus.
        for (final (s, bg) in [
          ('base', t.surface.base),
          ('raised', t.surface.raised),
          ('sunken', t.surface.sunken),
          ('hover', t.surface.hover),
        ]) {
          expect(
            contrast(t.text.faint, bg),
            greaterThanOrEqualTo(4.5),
            reason: '$name faint text on $s',
          );
        }
        expect(
          contrast(t.text.faint, t.surface.base),
          lessThan(contrast(t.text.muted, t.surface.base)),
          reason: 'faint stays quieter than muted',
        );
      });

      test('$name: ink on the accent fill clears AA', () {
        // The primary button's label. Asserted through accent.onPrimary, the ONE
        // name for this — the first version of this test used a second, parallel
        // token and caught the two disagreeing.
        expect(
          contrast(t.accent.onPrimary, t.accent.primary),
          greaterThanOrEqualTo(4.5),
          reason: '$name primary button label is not readable',
        );
      });

      test('$name: code sits on its own background legibly', () {
        expect(
          contrast(t.text.primary, t.editor.codeBg),
          greaterThanOrEqualTo(4.5),
        );
      });

      test('$name: every alert type has an accent', () {
        expect(t.editor.alertAccents.keys.toSet(), {
          'note',
          'tip',
          'important',
          'warning',
          'caution',
        });
      });
    }
  });

  test('phase 1 preserves accent, status, syntax and editor emphasis hues', () {
    List<int> fixed(MicaTokens t) => [
      t.accent.primary,
      t.accent.hover,
      t.accent.wash,
      t.status.danger,
      t.status.dangerWash,
      t.status.warning,
      t.status.warningWash,
      t.status.success,
      t.status.successWash,
      t.code.keyword,
      t.code.string,
      t.code.number,
      t.code.comment,
      t.code.key,
      t.code.function,
      t.editor.selection,
      t.editor.caret,
      t.editor.dropLine,
      t.editor.commentHighlight,
      t.editor.commentHighlightActive,
      ...t.editor.alertAccents.values,
    ].map((c) => c.toARGB32()).toList();
    expect(fixed(MicaTokens.light), [
      0xFF2563EB,
      0xFF1D4ED8,
      0xFFEFF6FF,
      0xFFDC2626,
      0xFFFEF2F2,
      0xFFB45309,
      0xFFFFFBEB,
      0xFF16A34A,
      0xFFF0FDF4,
      0xFF7C3AED,
      0xFF0A7D34,
      0xFFB45309,
      0xFF6B7280,
      0xFF0F766E,
      0xFF2563EB,
      0x332563EB,
      0xFF2563EB,
      0xFF2563EB,
      0x33F59E0B,
      0x59F59E0B,
      0xFF0969DA,
      0xFF1A7F37,
      0xFF8250DF,
      0xFF9A6700,
      0xFFCF222E,
    ]);
    expect(fixed(MicaTokens.dark_), [
      0xFF4C8DF6,
      0xFF6BA1F8,
      0x1F4C8DF6,
      0xFFF87171,
      0x1FF87171,
      0xFFE3B341,
      0x1FE3B341,
      0xFF56D364,
      0x1F56D364,
      0xFFC4A0FF,
      0xFF7EE787,
      0xFFE3B341,
      0xFF8B949E,
      0xFF39C5CF,
      0xFF79C0FF,
      0x4C4C8DF6,
      0xFF4C8DF6,
      0xFF4C8DF6,
      0x3DE3B341,
      0x66E3B341,
      0xFF58A6FF,
      0xFF3FB950,
      0xFFA371F7,
      0xFFD29922,
      0xFFF85149,
    ]);
  });

  group('approved contrast table and ghost separation', () {
    for (final (name, t, minimums) in [
      (
        'light',
        MicaTokens.light,
        [
          [12.26, 11.43, 10.84, 10.65],
          [6.48, 6.04, 5.73, 5.63],
          [5.34, 4.98, 4.72, 4.64],
        ],
      ),
      (
        'dark',
        MicaTokens.dark_,
        [
          [11.86, 10.99, 10.21, 9.42],
          [7.22, 6.69, 6.22, 5.74],
          [5.80, 5.37, 4.99, 4.60],
        ],
      ),
    ]) {
      test('$name: all informational inks meet the rounded spec table', () {
        final fills = [
          t.surface.base,
          t.surface.raised,
          t.surface.sunken,
          t.surface.hover,
        ];
        final inks = [t.text.primary, t.text.muted, t.text.faint];
        for (var i = 0; i < inks.length; i++) {
          for (var j = 0; j < fills.length; j++) {
            final rounded = double.parse(
              contrast(inks[i], fills[j]).toStringAsFixed(2),
            );
            expect(rounded, greaterThanOrEqualTo(minimums[i][j]));
          }
        }
        expect(
          contrast(t.text.ghost, t.surface.base),
          lessThan(contrast(t.text.faint, t.surface.base)),
        );
      });
    }
    test('ghost values and interpolation stay separate from information', () {
      expect(MicaTokens.light.text.ghost, const Color(0xFF9B9A97));
      expect(MicaTokens.dark_.text.ghost, const Color(0xFF6B6B6B));
      expect(
        MicaTokens.lerp(MicaTokens.light, MicaTokens.dark_, 0.5).text.ghost,
        Color.lerp(
          MicaTokens.light.text.ghost,
          MicaTokens.dark_.text.ghost,
          0.5,
        ),
      );
      final t = MicaTokens.light;
      final bg = Color.alphaBlend(t.editor.inlineCodeBg, t.surface.base);
      expect(contrast(t.editor.inlineCodeInk, bg), greaterThanOrEqualTo(8.5));
    });
  });

  test(
    'dark mode is not an inversion — no pure black page, no pure white ink',
    () {
      // The two mistakes every first dark theme makes.
      expect(MicaTokens.dark_.surface.base, isNot(const Color(0xFF000000)));
      expect(MicaTokens.dark_.text.primary, isNot(const Color(0xFFFFFFFF)));
      // And the page is genuinely dark, not merely "less white".
      expect(lum(MicaTokens.dark_.surface.base), lessThan(0.05));
    },
  );

  test('dark surfaces step in the right direction', () {
    // raised is nearer the viewer than base; sunken is quieter. On dark that
    // means lighter, not darker — getting this backwards is what makes a dark
    // theme look like flat mud.
    final d = MicaTokens.dark_;
    expect(lum(d.surface.raised), greaterThan(lum(d.surface.base)));
    expect(lum(d.surface.overlay), greaterThan(lum(d.surface.base)));
    final l = MicaTokens.light;
    expect(lum(l.surface.raised), lessThan(lum(l.surface.base)));
  });

  test('the washes flip kind: opaque tints on light, translucent on dark', () {
    // A light theme's near-white wash on a dark page reads as a bright hole.
    expect(MicaTokens.light.accent.wash.a, 1.0);
    expect(MicaTokens.dark_.accent.wash.a, lessThan(1.0));
    expect(MicaTokens.dark_.status.dangerWash.a, lessThan(1.0));
  });

  group('lerp', () {
    test('the endpoints are the endpoints', () {
      final at0 = MicaTokens.lerp(MicaTokens.light, MicaTokens.dark_, 0);
      final at1 = MicaTokens.lerp(MicaTokens.light, MicaTokens.dark_, 1);
      expect(at0.surface.base, MicaTokens.light.surface.base);
      expect(at1.surface.base, MicaTokens.dark_.surface.base);
      expect(at0.dark, isFalse);
      expect(at1.dark, isTrue);
    });

    test('halfway is between, and the discrete bits pick a side', () {
      final mid = MicaTokens.lerp(MicaTokens.light, MicaTokens.dark_, 0.5);
      expect(
        lum(mid.surface.base),
        lessThan(lum(MicaTokens.light.surface.base)),
      );
      expect(
        lum(mid.surface.base),
        greaterThan(lum(MicaTokens.dark_.surface.base)),
      );
      // Alert accents are keyed, not continuous: a muddy intermediate would help
      // nobody, so it snaps.
      expect(mid.editor.alertAccents, MicaTokens.dark_.editor.alertAccents);
    });
  });

  testWidgets('with no provider above it, a widget still gets light', (
    tester,
  ) async {
    // Half the tree has no provider during the migration, and every widget test
    // pumps without one. Throwing there would just mean nothing renders.
    late MicaTokens seen;
    await tester.pumpWidget(
      MaterialApp(
        home: Builder(
          builder: (context) {
            seen = MicaTheme.of(context);
            return const SizedBox();
          },
        ),
      ),
    );
    expect(seen.dark, isFalse);
  });

  testWidgets('a provider hands its tokens down', (tester) async {
    late MicaTokens seen;
    await tester.pumpWidget(
      MicaTheme(
        tokens: MicaTokens.dark_,
        child: MaterialApp(
          home: Builder(
            builder: (context) {
              seen = MicaTheme.of(context);
              return const SizedBox();
            },
          ),
        ),
      ),
    );
    expect(seen.dark, isTrue);
  });

  test('the Material theme follows the tokens, so stock widgets match', () {
    // Dialogs, text fields and buttons we do not hand-colour must not stay light
    // while everything around them goes dark.
    final dark = MicaTokens.dark_.toMaterialTheme();
    expect(dark.colorScheme.brightness, Brightness.dark);
    expect(dark.scaffoldBackgroundColor, MicaTokens.dark_.surface.base);
    expect(dark.colorScheme.onSurface, MicaTokens.dark_.text.primary);
    final light = MicaTokens.light.toMaterialTheme();
    expect(light.colorScheme.brightness, Brightness.light);
  });

  test('button labels keep the bundled CJK fallback', () {
    // Material replaces these text styles wholesale; a bare TextStyle drops
    // `fontFamilyFallback`, leaving Chinese labels on the remote fallback.
    final theme = MicaTokens.light.toMaterialTheme();
    final filled = theme.filledButtonTheme.style?.textStyle?.resolve(
      const <WidgetState>{},
    );
    final text = theme.textButtonTheme.style?.textStyle?.resolve(
      const <WidgetState>{},
    );
    expect(filled?.fontFamilyFallback, cjkFontFallback);
    expect(text?.fontFamilyFallback, cjkFontFallback);
  });

  /// Every role carries a DIFFERENT colour in the two palettes.
  ///
  /// This is the one structural risk a hand-written token layer has, and the
  /// compiler cannot see it: add a role, fill it in for light, forget dark —
  /// and it still compiles, because both fields are non-null `Color`s. The dark
  /// palette then quietly paints that role with whatever was copy-pasted,
  /// usually the light value, and nothing fails until someone looks at it.
  ///
  /// Deliberately NOT a golden image. The peers that tried pixel comparison
  /// gave it up — AppFlowy's single golden assertion sits commented out with
  /// "screenshots are different on different platform" — and a two-target app
  /// like this one (Windows native and CanvasKit) cannot share a baseline at
  /// all. An inventory assertion is stable wherever a Dart VM runs.
  group('every role is defined in both palettes', () {
    Map<String, Color> roles(MicaTokens t) => {
      'surface.base': t.surface.base,
      'surface.raised': t.surface.raised,
      'surface.sunken': t.surface.sunken,
      'surface.overlay': t.surface.overlay,
      'surface.hover': t.surface.hover,
      'text.primary': t.text.primary,
      'text.muted': t.text.muted,
      'text.faint': t.text.faint,
      'border.subtle': t.border.subtle,
      'border.normal': t.border.normal,
      'border.strong': t.border.strong,
      'accent.primary': t.accent.primary,
      'accent.wash': t.accent.wash,
      'status.success': t.status.success,
      'status.warning': t.status.warning,
      'status.danger': t.status.danger,
      'editor.caret': t.editor.caret,
      'editor.codeBg': t.editor.codeBg,
    };

    test('light and dark disagree on every one of them', () {
      final light = roles(MicaTokens.light);
      final dark = roles(MicaTokens.dark_);
      expect(
        dark.keys.toList(),
        light.keys.toList(),
        reason: 'the two palettes must list the same roles',
      );

      final identical = <String>[
        for (final entry in light.entries)
          if (dark[entry.key] == entry.value) entry.key,
      ];
      expect(
        identical,
        isEmpty,
        reason:
            'these roles carry the SAME colour in light and dark, which is '
            'exactly what forgetting to fill one in looks like: $identical',
      );
    });

    test('no role is left fully transparent', () {
      for (final (name, t) in [
        ('light', MicaTokens.light),
        ('dark', MicaTokens.dark_),
      ]) {
        roles(t).forEach((role, colour) {
          expect(
            colour.a,
            greaterThan(0),
            reason: '$name.$role is fully transparent — almost certainly unset',
          );
        });
      }
    });
  });
}
