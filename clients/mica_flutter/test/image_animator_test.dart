import 'dart:convert';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/image_animator.dart';

// A real 3-frame GIF89a (4x4, 80ms a frame, loops forever), 159 bytes.
final kAnimatedGif = base64Decode(
  'R0lGODlhBAAEAIEAAP8AAAAAAAAAAAAAACH/C05FVFNDQVBFMi4wAwEAAAAh+QQACAAAACwAAAAA'
  'BAAEAAAICQABCBxIsCCAgAAh+QQBCAABACwAAAAABAAEAIEA/wAAAAAAAAAAAAAICQABCBxIsCCA'
  'gAAh+QQBCAABACwAAAAABAAEAIEAAP8AAAAAAAAAAAAICQABCBxIsCCAgAA7',
);

// The same, still: one frame, so the editor must NOT animate it.
final kStillPng = base64Decode(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6'
  'kgAAAABJRU5ErkJggg==',
);

// A GIF on the canvas is played by hand (the editor paints raw ui.Images, not
// the Image widget), so the loop's timing, its stop conditions and — above all
// — who disposes which frame are ours to get right.
//
// Driving that off a real ui.Codec is not an option: it is engine backed and
// its frames arrive on futures the test's fake clock never completes, and a
// fake clock is exactly what timing tests need. Hence FrameSource — the loop
// runs on a fake here, and _realCodecTests (bottom) covers the seam to the
// engine that the fake stands in for.

ui.Image _pixel() {
  final recorder = ui.PictureRecorder();
  Canvas(recorder).drawRect(
    const Rect.fromLTWH(0, 0, 1, 1),
    Paint()..color = const Color(0xFF000000),
  );
  return recorder.endRecording().toImageSync(1, 1);
}

class _FakeFrames implements FrameSource {
  _FakeFrames({
    this.frameCount = 3,
    this.repetitionCount = -1,
    this.duration = const Duration(milliseconds: 50),
  });

  @override
  final int frameCount;
  @override
  final int repetitionCount;
  final Duration duration;

  int decoded = 0;
  bool disposed = false;

  @override
  Future<({ui.Image image, Duration duration})> next() async {
    decoded++;
    return (image: _pixel(), duration: duration);
  }

  @override
  void dispose() => disposed = true;
}

void main() {
  // The animator emits from a scheduler frame callback, so a test drives it the
  // same way the engine does: by pumping frames.
  ({ImageAnimator anim, List<ui.Image> frames}) play(FrameSource source) {
    final frames = <ui.Image>[];
    final anim = ImageAnimator(source, onFrame: frames.add);
    // The net for a failing expect (which skips the body's dispose); it runs
    // after flutter_test's pending-timer check, so it cannot replace it.
    addTearDown(() {
      anim.dispose();
      for (final f in frames) {
        if (!f.debugDisposed) f.dispose();
      }
    });
    anim.start();
    return (anim: anim, frames: frames);
  }

  testWidgets('plays frame after frame, one per frame duration', (tester) async {
    final src = _FakeFrames(duration: const Duration(milliseconds: 50));
    final p = play(src);

    await tester.pump(); // decode lands
    await tester.pump(); // first frame is emitted immediately
    expect(p.frames.length, 1);

    await tester.pump(const Duration(milliseconds: 50));
    await tester.pump(const Duration(milliseconds: 50));
    expect(p.frames.length, greaterThan(1),
        reason: 'the loop must keep running on its own');
    p.anim.dispose();
  });

  testWidgets('every frame is a fresh image the host owns', (tester) async {
    final p = play(_FakeFrames());
    await tester.pump();
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));
    await tester.pump(const Duration(milliseconds: 50));

    expect(p.frames.length, greaterThan(1));
    // Handed over, not recycled: the animator must never dispose these behind
    // the host's back, or the canvas paints a dead image.
    for (final f in p.frames) {
      expect(f.debugDisposed, isFalse);
    }
    expect(p.frames.toSet().length, p.frames.length);
    p.anim.dispose();
  });

  testWidgets('a 0ms delay does not spin the loop', (tester) async {
    // A GIF asking for 0ms means "unspecified", not "as fast as you can" —
    // taking it literally would decode flat out and peg a core.
    final src = _FakeFrames(duration: Duration.zero);
    final p = play(src);
    await tester.pump();
    await tester.pump();
    expect(p.frames.length, 1);

    await tester.pump(const Duration(milliseconds: 30));
    expect(p.frames.length, 1, reason: '30ms is inside the 100ms substitute');

    await tester.pump(const Duration(milliseconds: 100));
    expect(p.frames.length, 2, reason: 'and it does advance once 100ms is up');
    p.anim.dispose();
  });

  testWidgets('a play-once loop stops after one pass and frees the codec',
      (tester) async {
    final src = _FakeFrames(frameCount: 2, repetitionCount: 0);
    final p = play(src);
    for (var i = 0; i < 8; i++) {
      await tester.pump(const Duration(milliseconds: 50));
    }
    expect(p.frames.length, 2, reason: 'repetitionCount 0 = play the loop once');
    expect(p.anim.isPlaying, isFalse);
    expect(src.disposed, isTrue, reason: 'a finished loop holds no codec');
    p.anim.dispose();
  });

  testWidgets('pause stops decoding; start picks the loop back up',
      (tester) async {
    final src = _FakeFrames();
    final p = play(src);
    await tester.pump();
    await tester.pump();
    p.anim.pause();
    final decodedAtPause = src.decoded;

    for (var i = 0; i < 5; i++) {
      await tester.pump(const Duration(milliseconds: 50));
    }
    expect(src.decoded, decodedAtPause,
        reason: 'a paused loop must not decode — that is the whole point');
    expect(p.anim.isPlaying, isFalse);

    p.anim.start();
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));
    await tester.pump(const Duration(milliseconds: 50));
    expect(src.decoded, greaterThan(decodedAtPause));
    p.anim.dispose();
  });

  testWidgets('a frame decoded before a pause is shown, not skipped',
      (tester) async {
    final src = _FakeFrames();
    final p = play(src);
    await tester.pump();
    await tester.pump();
    final shown = p.frames.length;
    p.anim.pause(); // a frame is already decoded and waiting
    final decodedAtPause = src.decoded;

    p.anim.start();
    await tester.pump(const Duration(milliseconds: 50));
    expect(p.frames.length, shown + 1);
    expect(src.decoded, decodedAtPause + 1,
        reason: 'the waiting frame is shown, then one more is decoded');
    p.anim.dispose();
  });

  testWidgets('dispose frees the codec and stops the loop dead', (tester) async {
    final src = _FakeFrames();
    final p = play(src);
    await tester.pump();
    await tester.pump();
    p.anim.dispose();
    final decodedAtDispose = src.decoded;
    expect(src.disposed, isTrue);

    for (var i = 0; i < 5; i++) {
      await tester.pump(const Duration(milliseconds: 50));
    }
    expect(src.decoded, decodedAtDispose);
    expect(p.anim.isPlaying, isFalse);
  });

  testWidgets('a decode error ends the loop instead of blanking the picture',
      (tester) async {
    final p = play(_ThrowingFrames());
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));
    expect(p.frames, isEmpty);
    expect(p.anim.isPlaying, isFalse);
  });

  _realCodecTests();
}

// Everything above runs on a fake FrameSource. These two run the real engine
// codec — inside runAsync, the one place a test can await genuinely async work
// — because the whole feature rests on one premise worth pinning down: that the
// engine reports a GIF's frames at all. If frameCount came back 1, every test
// above would still pass and no GIF would ever move.
void _realCodecTests() {
  testWidgets('the engine reports a real GIF as multi-frame and looping',
      (tester) async {
    await tester.runAsync(() async {
      final codec = await ui.instantiateImageCodec(kAnimatedGif);
      addTearDown(codec.dispose);
      final source = CodecFrameSource(codec);
      expect(source.frameCount, 3, reason: 'this is what routes it to the animator');
      expect(source.repetitionCount, -1, reason: 'the GIF says loop forever');

      final first = await source.next();
      expect(first.duration, const Duration(milliseconds: 80));
      expect(first.image.width, 4);
      first.image.dispose();

      // Frames composite to the full size, which is what lets a new frame be
      // swapped in with a repaint instead of a relayout.
      final second = await source.next();
      expect(second.image.width, 4);
      expect(second.image.height, 4);
      second.image.dispose();
    });
  });

  testWidgets('a still image reports one frame, so it is never animated',
      (tester) async {
    await tester.runAsync(() async {
      final codec = await ui.instantiateImageCodec(kStillPng);
      addTearDown(codec.dispose);
      expect(codec.frameCount, 1);
    });
  });
}

class _ThrowingFrames implements FrameSource {
  @override
  int get frameCount => 3;
  @override
  int get repetitionCount => -1;
  @override
  Future<({ui.Image image, Duration duration})> next() async =>
      throw StateError('truncated');
  @override
  void dispose() {}
}
