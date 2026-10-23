@TestOn('browser')
library;

import 'dart:html' as html;
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/image_actions_web.dart';

void main() {
  test('invalid image fails promptly and removes fallback DOM', () async {
    final before = html.document
        .querySelectorAll('[contenteditable="true"]')
        .length;
    // Unsupported image MIME makes the modern ClipboardItem path decline;
    // the legacy path must handle the browser's image error event.
    final copied = await copyImageToClipboard(
      Uint8List.fromList([1, 2, 3]),
      'image/invalid',
    ).timeout(const Duration(seconds: 5));
    expect(copied, isFalse);
    expect(
      html.document.querySelectorAll('[contenteditable="true"]').length,
      before,
    );
    expect(html.document.querySelectorAll('img[src^="blob:"]'), isEmpty);
  });

  test('repeated fallback image events settle only once', () async {
    final observer = html.MutationObserver((mutations, _) {
      for (final mutation in mutations) {
        for (final node in mutation.addedNodes) {
          if (node is! html.Element) continue;
          final img = node.querySelector('img');
          if (img == null) continue;
          img.dispatchEvent(html.Event('error'));
          img.dispatchEvent(html.Event('error'));
          img.dispatchEvent(html.Event('load'));
        }
      }
    })..observe(html.document.body!, childList: true);
    addTearDown(observer.disconnect);

    expect(
      await copyImageToClipboard(
        Uint8List.fromList([1, 2, 3]),
        'image/invalid',
      ).timeout(const Duration(seconds: 5)),
      isFalse,
    );
    expect(html.document.querySelectorAll('img[src^="blob:"]'), isEmpty);
  });

  test('fallback times out and cleans up when no image event arrives', () async {
    final before = html.document
        .querySelectorAll('[contenteditable="true"]')
        .length;
    var suppressed = 0;
    void suppressImageEvent(html.Event event) {
      final target = event.target;
      if (target is html.ImageElement &&
          (target.src?.startsWith('blob:') ?? false)) {
        suppressed++;
        event.stopImmediatePropagation();
      }
    }

    // Capture the browser's decode error before it reaches the image listener,
    // reproducing a browser that never delivers a terminal image event.
    html.window.addEventListener('error', suppressImageEvent, true);
    html.window.addEventListener('load', suppressImageEvent, true);
    addTearDown(() {
      html.window.removeEventListener('error', suppressImageEvent, true);
      html.window.removeEventListener('load', suppressImageEvent, true);
    });
    expect(
      await copyImageToClipboard(
        Uint8List.fromList([1, 2, 3]),
        'image/invalid',
      ).timeout(const Duration(seconds: 15)),
      isFalse,
    );
    expect(suppressed, greaterThan(0));
    expect(
      html.document.querySelectorAll('[contenteditable="true"]').length,
      before,
    );
    expect(html.document.querySelectorAll('img[src^="blob:"]'), isEmpty);
  });
}
