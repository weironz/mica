import 'dart:async';
import 'dart:typed_data';

/// Retains the next local CRDT update until its append succeeds. A failed append
/// must not make the caller re-run a non-idempotent history command or let a
/// later diff overtake the missing dependency.
class PendingDocUpdate {
  PendingDocUpdate({
    required this.persist,
    this.onPendingChanged,
    this.onRetryError,
  });

  final void Function(Uint8List update) persist;
  final void Function()? onPendingChanged;
  final void Function(Object error)? onRetryError;
  Uint8List? _pending;
  Timer? _retry;
  int _attempts = 0;
  bool _disposed = false;

  bool get hasPending => _pending != null;

  void publish(Uint8List update) {
    flush();
    if (update.isEmpty ||
        (update.length == 2 && update[0] == 0 && update[1] == 0)) {
      return;
    }
    _pending = Uint8List.fromList(update);
    onPendingChanged?.call();
    flush();
  }

  /// Call before mutating the engine again, and before close/exit.
  void flush() {
    final update = _pending;
    if (update == null) return;
    try {
      persist(update);
    } catch (_) {
      _scheduleRetry();
      rethrow;
    }
    _pending = null;
    _attempts = 0;
    _retry?.cancel();
    _retry = null;
    onPendingChanged?.call();
  }

  void _scheduleRetry() {
    if (_disposed || _retry != null || _attempts >= 5) return;
    final delay = Duration(milliseconds: 400 * (1 << _attempts++));
    _retry = Timer(delay, () {
      _retry = null;
      try {
        flush();
      } catch (error) {
        onRetryError?.call(error);
      }
    });
  }

  void dispose() {
    _disposed = true;
    _retry?.cancel();
    _retry = null;
  }
}
