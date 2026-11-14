import 'package:flutter/material.dart';

import '../prefs.dart';

const double kUiFontScaleSmall = 0.9;
const double kUiFontScaleStandard = 1.0;
const double kUiFontScaleLarge = 1.1;

const List<double> kUiFontScaleOptions = [
  kUiFontScaleSmall,
  kUiFontScaleStandard,
  kUiFontScaleLarge,
];

double uiFontScaleFromPref(String? raw) {
  final value = double.tryParse(raw ?? '');
  if (value == null || !value.isFinite) return kUiFontScaleStandard;
  return kUiFontScaleOptions.reduce(
    (closest, option) =>
        (option - value).abs() <= (closest - value).abs() ? option : closest,
  );
}

/// The UI scale is seeded before runApp, then changed from Settings.
final ValueNotifier<double> uiFontScaleController = ValueNotifier(
  kUiFontScaleStandard,
);

void loadPersistedUiFontScale() {
  uiFontScaleController.value = uiFontScaleFromPref(loadPref('uiFontScale'));
}

/// Multiplies the platform scaler rather than replacing it, so OS accessibility
/// scaling remains effective alongside Mica's interface-size preference.
class MicaInterfaceTextScale extends StatelessWidget {
  const MicaInterfaceTextScale({required this.child, super.key});

  final Widget child;

  @override
  Widget build(BuildContext context) => ValueListenableBuilder<double>(
    valueListenable: uiFontScaleController,
    child: child,
    builder: (context, scale, child) {
      final media = MediaQuery.of(context);
      return MediaQuery(
        data: media.copyWith(
          textScaler: _PreferenceTextScaler(media.textScaler, scale),
        ),
        child: child!,
      );
    },
  );
}

class _PreferenceTextScaler extends TextScaler {
  const _PreferenceTextScaler(this.platform, this.preferenceScale);

  final TextScaler platform;
  final double preferenceScale;

  @override
  double scale(double fontSize) => platform.scale(fontSize) * preferenceScale;

  @override
  @Deprecated('Use scale instead.')
  double get textScaleFactor => scale(1);

  @override
  bool operator ==(Object other) =>
      other is _PreferenceTextScaler &&
      other.platform == platform &&
      other.preferenceScale == preferenceScale;

  @override
  int get hashCode => Object.hash(platform, preferenceScale);
}
