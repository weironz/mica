import 'dart:async';
import 'package:flutter/foundation.dart';
import '../api/models.dart';
import '../editor/render.dart' show EditorAppearance;
import '../ui/page_width_frame.dart';
import '../ui/settings_dialog.dart'
    show kFontPxMin, kFontPxMax, fontScaleFromPx;
import '../ui/settings_sync.dart';
import '../ui/interface_text_scale.dart';

/// Local appearance preferences and their account-scoped cloud write-through.
/// The shell owns theme/locale UI notifications; this controller owns the
/// mutable values, apply guard, debounce and disposal of pending writes.
class ShellPreferencesController {
  ShellPreferencesController({
    required this.readPref,
    required this.writePref,
    required this.session,
    required this.owner,
    required this.isLocal,
    required this.isMounted,
    required this.fetch,
    required this.push,
    required this.onCloudApplied,
  });

  final String? Function(String) readPref;
  final void Function(String, String) writePref;
  final AuthSession? Function() session;
  final Object? Function() owner;
  final bool Function() isLocal;
  final bool Function() isMounted;
  final Future<Map<String, dynamic>> Function(String token) fetch;
  final Future<void> Function(String token, Map<String, String> settings) push;
  final VoidCallback onCloudApplied;
  EditorAppearance appearance = const EditorAppearance();
  double pageWidth = kPageWidthDefault;
  double uiFontScale = kUiFontScaleStandard;
  bool reHostImages = true;
  bool showFormatBar = false;
  bool showPageTitle = true;
  bool aiEnabled = false;
  Timer? _timer;
  bool _applyingCloud = false;
  bool _disposed = false;

  void loadAppearance() {
    final fontScale = double.tryParse(readPref('fontScale') ?? '');
    uiFontScale = uiFontScaleFromPref(readPref('uiFontScale'));
    uiFontScaleController.value = uiFontScale;
    final fontFamily = readPref('fontFamily');
    appearance = EditorAppearance(
      // Bounds derived from the settings control's px detents (see kFontBasePx
      // in dialogs.dart) rather than restated as literals here — restated
      // bounds are how a widened slider and a load-time clamp drift apart.
      fontScale: (fontScale ?? 1.0).clamp(
        fontScaleFromPx(kFontPxMin),
        fontScaleFromPx(kFontPxMax),
      ),
      fontFamily: (fontFamily == null || fontFamily.isEmpty)
          ? null
          : fontFamily,
    );
    // Clamp a stored value into the discrete-step range; absent → the readable
    // default. The editor still caps the *render* at the window width, so a wide
    // saved value on a small window simply fills it.
    pageWidth =
        (double.tryParse(readPref('pageWidth') ?? '') ?? kPageWidthDefault)
            .clamp(kPageWidthMin, kPageWidthMax);
    reHostImages = readPref('reHostImages') != 'false';
    showFormatBar = readPref('showFormatBar') == 'true';
    showPageTitle = readPref('showPageTitle') != 'false';
    aiEnabled = readPref('aiEnabled') == 'true';
  }

  void save() {
    writePref('uiFontScale', uiFontScale.toString());
    writePref('fontScale', appearance.fontScale.toString());
    writePref('fontFamily', appearance.fontFamily ?? '');
    writePref('pageWidth', pageWidth.toString());
    writePref('reHostImages', reHostImages.toString());
    writePref('showFormatBar', showFormatBar.toString());
    writePref('showPageTitle', showPageTitle.toString());
    writePref('aiEnabled', aiEnabled.toString());
    schedulePush();
  }

  void schedulePush() {
    if (_disposed || _applyingCloud) return;
    _timer?.cancel();
    final current = session();
    if (current == null || isLocal()) return;
    final claim = owner();
    // Capture values at enqueue time, so a later account/server transition
    // cannot make an old timer submit the next account's preferences.
    final payload = settingsPayload(readPref);
    _timer = Timer(const Duration(seconds: 1), () async {
      if (!_current(claim) || isLocal()) return;
      try {
        await push(current.accessToken, payload);
      } catch (error) {
        if (_current(claim)) debugPrint('settings push failed: $error');
      }
    });
  }

  bool _current(Object? claim) => !_disposed && isMounted() && owner() == claim;

  Future<void> syncFromCloud(AuthSession current) async {
    final claim = owner();
    try {
      final cloud = await fetch(current.accessToken);
      if (!_current(claim) || session()?.accessToken != current.accessToken) {
        return;
      }
      final apply = cloudSettingsToApply(cloud);
      if (apply.isEmpty) {
        schedulePush();
        return;
      }
      _applyingCloud = true;
      try {
        apply.forEach(writePref);
        loadAppearance();
        onCloudApplied();
      } finally {
        _applyingCloud = false;
      }
      schedulePush();
    } catch (error) {
      if (_current(claim)) debugPrint('settings sync skipped: $error');
    }
  }

  void dispose() {
    _disposed = true;
    _timer?.cancel();
  }
}
