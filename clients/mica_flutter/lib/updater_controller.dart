/// One update operation shared by the sidebar and About dialog.
library;

import 'package:flutter/foundation.dart';

import 'updater.dart' as platform;
import 'updater_common.dart';

enum DesktopUpdateStage {
  idle,
  checking,
  upToDate,
  available,
  downloading,
  error,
}

typedef UpdateCheck = Future<UpdateInfo?> Function(String currentVersion);
typedef UpdateApply =
    Future<void> Function(
      UpdateInfo info, {
      void Function(double progress)? onProgress,
    });

class DesktopUpdateController extends ChangeNotifier {
  DesktopUpdateController({
    required this.currentVersion,
    bool? supported,
    UpdateCheck? check,
    UpdateApply? apply,
  }) : supported = supported ?? platform.updateSupported,
       _check = check ?? platform.checkForUpdate,
       _apply = apply ?? platform.downloadAndApplyUpdate;

  static const checkInterval = Duration(hours: 6);

  final String currentVersion;
  final bool supported;
  final UpdateCheck _check;
  final UpdateApply _apply;
  DesktopUpdateStage _stage = DesktopUpdateStage.idle;
  UpdateInfo? _info;
  Object? _error;
  double _progress = 0;
  DateTime? _lastCheck;
  bool _confirming = false;
  bool _disposed = false;

  DesktopUpdateStage get stage => _stage;
  UpdateInfo? get info => _info;
  Object? get error => _error;
  double get progress => _progress;
  bool get integrityFailed => _error is UpdateIntegrityException;
  bool get isBusy =>
      _confirming ||
      _stage == DesktopUpdateStage.checking ||
      _stage == DesktopUpdateStage.downloading;

  /// Automatic checks are cached across workspace changes and app resumes.
  /// Explicit user checks and the six-hour timer may bypass this cache.
  Future<void> check({bool force = false}) async {
    if (!supported || _disposed || isBusy) return;
    final now = DateTime.now();
    if (!force &&
        _lastCheck != null &&
        now.difference(_lastCheck!) < checkInterval) {
      return;
    }
    _lastCheck = now;
    _stage = DesktopUpdateStage.checking;
    _error = null;
    _notify();
    try {
      final result = await _check(currentVersion);
      if (_disposed) return;
      _info = result;
      _stage = result == null
          ? DesktopUpdateStage.upToDate
          : DesktopUpdateStage.available;
    } catch (error) {
      if (_disposed) return;
      _error = error;
      _stage = DesktopUpdateStage.error;
    }
    _notify();
  }

  /// Hold the lock from the confirmation through installer handoff, so another
  /// entry point cannot open a second prompt or start a second installer.
  Future<void> confirmAndApply(
    Future<bool> Function(UpdateInfo info) confirm,
  ) async {
    final info = _info;
    if (!supported || _disposed || isBusy || info == null) return;
    _confirming = true;
    _notify();
    try {
      if (!await confirm(info) || _disposed) return;
      _stage = DesktopUpdateStage.downloading;
      _error = null;
      _progress = 0;
      _notify();
      // Production saves edits, starts the verified installer, then exits.
      // There is deliberately no second "restart now" gate after confirmation.
      await _apply(
        info,
        onProgress: (value) {
          _progress = value.clamp(0.0, 1.0);
          _notify();
        },
      );
    } catch (error) {
      if (_disposed) return;
      _error = error;
      _stage = DesktopUpdateStage.error;
    } finally {
      _confirming = false;
      _notify();
    }
  }

  void _notify() {
    if (!_disposed) notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }
}
