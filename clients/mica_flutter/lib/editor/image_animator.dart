import 'dart:async';
import 'dart:ui' as ui;

import 'package:flutter/scheduler.dart';

/// A source of already-composited animation frames, wrapping back to frame 0
/// after the last one.
///
/// This interface exists so [ImageAnimator] can be tested: `ui.Codec` is engine
/// backed and cannot be implemented in Dart, and a real codec's frames only
/// arrive on futures a test's fake clock never completes.
abstract class FrameSource {
  /// Frames in one loop. 1 means a still image — don't animate it.
  int get frameCount;

  /// -1 loops forever, 0 plays the loop once, n repeats it n extra times.
  int get repetitionCount;

  /// The next frame. Every frame is the image's full size, already composited
  /// over the previous one, so callers never deal with GIF sub-rectangles.
  Future<({ui.Image image, Duration duration})> next();

  void dispose();
}

/// The real thing: frames off a decoded GIF / animated WebP.
class CodecFrameSource implements FrameSource {
  CodecFrameSource(this._codec);

  final ui.Codec _codec;

  @override
  int get frameCount => _codec.frameCount;

  @override
  int get repetitionCount => _codec.repetitionCount;

  @override
  Future<({ui.Image image, Duration duration})> next() async {
    final frame = await _codec.getNextFrame();
    return (image: frame.image, duration: frame.duration);
  }

  @override
  void dispose() => _codec.dispose();
}

/// Plays an animated image onto the editor's canvas, one [ui.Image] at a time.
///
/// The canvas paints plain `ui.Image`s, so nothing here can lean on the widget
/// layer's `Image` — this is the same job Flutter's own
/// `MultiFrameImageStreamCompleter` does for that widget, and it is modelled on
/// it, including the part that matters most: a frame is emitted from a
/// **scheduler frame callback**, not straight off a `Timer`. The engine stops
/// producing frames when the window is hidden or minimised, so the animation
/// then stalls by itself instead of burning CPU decoding frames nobody sees.
///
/// **Ownership**: each emitted image is handed to [onFrame] and belongs to the
/// caller from that moment — the animator never touches it again, and the
/// caller must dispose it. Anything still held by the animator when [dispose]
/// is called is disposed here.
class ImageAnimator {
  ImageAnimator(this._source, {required this.onFrame});

  final FrameSource _source;
  final void Function(ui.Image frame) onFrame;

  /// Decoded but not yet shown — it waits until the frame on screen has had
  /// its full duration.
  ({ui.Image image, Duration duration})? _next;

  /// How long the frame currently on screen wants to stay. Null before the
  /// first frame, which is what marks the loop as not-yet-started.
  Duration? _shownFor;
  Duration _shownAt = Duration.zero;

  Timer? _timer;
  int _emitted = 0;
  bool _frameScheduled = false;
  bool _paused = true;
  bool _done = false;
  bool _disposed = false;

  /// A GIF may ask for a 0ms delay, which idiomatically means "unspecified".
  /// Browsers substitute a tenth of a second; so do we, because taking it
  /// literally would spin the loop as fast as the decoder can go.
  static const _unspecifiedDelay = Duration(milliseconds: 100);

  /// Whether frames are still being decoded and emitted. False once the loop
  /// has played out its [FrameSource.repetitionCount], and while paused.
  bool get isPlaying => !_disposed && !_done && !_paused;

  /// Start, or pick a paused loop back up where it left off.
  void start() {
    if (_disposed || _done || !_paused) return;
    _paused = false;
    // A frame decoded before the pause is still good — don't throw it away and
    // skip a frame just because nobody was drawing for a while.
    if (_next != null) {
      _scheduleFrame();
    } else {
      _decodeAndSchedule();
    }
  }

  /// Stop decoding, keeping the codec so [start] can resume the same loop.
  void pause() {
    if (_paused) return;
    _paused = true;
    _timer?.cancel();
    _timer = null;
  }

  void dispose() {
    if (_disposed) return;
    _disposed = true;
    _timer?.cancel();
    _timer = null;
    _next?.image.dispose();
    _next = null;
    _source.dispose();
  }

  Future<void> _decodeAndSchedule() async {
    if (_disposed || _paused) return;
    try {
      _next = await _source.next();
    } catch (_) {
      // A truncated or malformed frame ends the animation on whatever is
      // already on screen, rather than blanking a picture that was fine.
      _done = true;
      return;
    }
    if (_disposed) {
      _next?.image.dispose();
      _next = null;
      return;
    }
    if (_paused) return; // keep _next; start() will show it
    _scheduleFrame();
  }

  void _scheduleFrame() {
    if (_frameScheduled || _disposed || _paused) return;
    _frameScheduled = true;
    SchedulerBinding.instance.scheduleFrameCallback(_onAppFrame);
  }

  void _onAppFrame(Duration timestamp) {
    _frameScheduled = false;
    if (_disposed || _paused) return;
    final next = _next;
    if (next == null) return;

    final elapsed = timestamp - _shownAt;
    if (_shownFor != null && elapsed < _shownFor!) {
      // Woken early (something else drove the frame). Wait out the remainder.
      _timer = Timer(_shownFor! - elapsed, _scheduleFrame);
      return;
    }

    _next = null;
    _shownAt = timestamp;
    _shownFor = next.duration > Duration.zero ? next.duration : _unspecifiedDelay;
    _emitted++;
    onFrame(next.image); // ownership transfers to the host

    // onFrame can pause us (the host stops loops nobody is drawing) or dispose
    // us (the editor went away) — both mean: decode nothing more.
    if (_disposed || _paused) return;

    final cycles = _emitted ~/ _source.frameCount;
    if (_source.repetitionCount == -1 || cycles <= _source.repetitionCount) {
      _decodeAndSchedule();
    } else {
      _done = true;
      _source.dispose();
    }
  }
}
