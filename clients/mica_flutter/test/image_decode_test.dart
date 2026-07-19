import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/image_decode.dart';

/// A 1x1 RGBA PNG.
final _png = Uint8List.fromList([
  0x89,0x50,0x4E,0x47,0x0D,0x0A,0x1A,0x0A,0x00,0x00,0x00,0x0D,0x49,0x48,0x44,0x52,
  0x00,0x00,0x00,0x01,0x00,0x00,0x00,0x01,0x08,0x06,0x00,0x00,0x00,0x1F,0x15,0xC4,
  0x89,0x00,0x00,0x00,0x0A,0x49,0x44,0x41,0x54,0x78,0x9C,0x63,0x00,0x01,0x00,0x00,
  0x05,0x00,0x01,0x0D,0x0A,0x2D,0xB4,0x00,0x00,0x00,0x00,0x49,0x45,0x4E,0x44,0xAE,
  0x42,0x60,0x82,
]);

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  /// Regression (v0.12.5): the display-cap decode disposed the ImageDescriptor
  /// before pulling a frame, which invalidates the encoded data the codec still
  /// needs — EVERY image in the app became a broken-image placeholder while the
  /// URL itself still loaded fine. Pins that a frame actually comes out.
  test('decodeCapped produces a frame (descriptor must outlive the codec)', () async {
    final codec = await decodeCapped(_png, 2048);
    final frame = await codec.getNextFrame();
    expect(frame.image.width, 1);
    expect(frame.image.height, 1);
    frame.image.dispose();
    codec.dispose();
  });

  test('decodeCapped never upscales below the cap', () async {
    final codec = await decodeCapped(_png, 4096);
    final frame = await codec.getNextFrame();
    expect(frame.image.width, 1, reason: 'a 1px image must not be blown up to the cap');
    frame.image.dispose();
    codec.dispose();
  });

  /// The failure mode itself, pinned so nobody "tidies up" by disposing the
  /// descriptor again: doing so makes frame production fail.
  test('disposing the descriptor early is what breaks decoding', () async {
    final buffer = await ui.ImmutableBuffer.fromUint8List(_png);
    final descriptor = await ui.ImageDescriptor.encoded(buffer);
    final codec = await descriptor.instantiateCodec();
    descriptor.dispose();
    await expectLater(codec.getNextFrame(), throwsA(isA<Exception>()));
    buffer.dispose();
  });
}
