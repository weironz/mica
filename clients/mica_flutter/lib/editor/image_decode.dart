import 'dart:typed_data';
import 'dart:ui' as ui;

/// Decode [bytes] into a codec whose pixels are capped at [capPx] wide.
///
/// Why a cap: a 4K screenshot otherwise becomes a ~33MB full-resolution
/// texture even when the page draws it ~800px wide, and every image in an open
/// document stays resident (the GPU-load audit). The cap is the machine's
/// physical screen width, so nothing visibly degrades — the fullscreen viewer
/// is bounded by the same screen. `targetWidth` is passed ONLY when the native
/// width exceeds the cap: never upscale. It is applied at the CODEC level, so
/// animated GIF/WebP frames decode capped too.
///
/// The dispose order here is load-bearing, and got shipped wrong once
/// (v0.12.5: every image in the app turned into a broken-image placeholder).
/// `ImageDescriptor.dispose()` invalidates the encoded data the codec still
/// needs to produce frames — calling it before `getNextFrame()` makes every
/// decode fail with "Codec failed to produce an image". Flutter's own
/// `instantiateImageCodecWithSize` disposes the BUFFER after instantiating and
/// never disposes the descriptor; this mirrors that exactly. The descriptor is
/// reclaimed by GC. See `test/image_decode_test.dart`, which pins each variant.
Future<ui.Codec> decodeCapped(Uint8List bytes, int capPx) async {
  final buffer = await ui.ImmutableBuffer.fromUint8List(bytes);
  final descriptor = await ui.ImageDescriptor.encoded(buffer);
  final codec = await descriptor.instantiateCodec(
    targetWidth: descriptor.width > capPx ? capPx : null,
  );
  // Buffer only — NOT the descriptor (see above).
  buffer.dispose();
  return codec;
}
