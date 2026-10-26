@TestOn('browser')
library;

import 'dart:js_interop';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/image_actions_web.dart';
import 'package:web/web.dart' as web;

void main() {
  test('invalid image fails promptly and removes fallback DOM', () async {
    final before = web.document
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
      web.document.querySelectorAll('[contenteditable="true"]').length,
      before,
    );
    expect(web.document.querySelectorAll('img[src^="blob:"]').length, 0);
  });

  test('repeated fallback image events settle only once', () async {
    void onMutation(
      JSArray<web.MutationRecord> mutations,
      web.MutationObserver _,
    ) {
      for (final mutation in mutations.toDart) {
        final added = mutation.addedNodes;
        for (var i = 0; i < added.length; i++) {
          final node = added.item(i);
          if (node == null || !node.isA<web.Element>()) continue;
          final img = (node as web.Element).querySelector('img');
          if (img == null) continue;
          img.dispatchEvent(web.Event('error'));
          img.dispatchEvent(web.Event('error'));
          img.dispatchEvent(web.Event('load'));
        }
      }
    }

    final observer = web.MutationObserver(onMutation.toJS)
      ..observe(
        web.document.body!,
        web.MutationObserverInit(childList: true),
      );
    addTearDown(() => observer.disconnect());

    expect(
      await copyImageToClipboard(
        Uint8List.fromList([1, 2, 3]),
        'image/invalid',
      ).timeout(const Duration(seconds: 5)),
      isFalse,
    );
    expect(web.document.querySelectorAll('img[src^="blob:"]').length, 0);
  });

  test('fallback times out and cleans up when no image event arrives', () async {
    final before = web.document
        .querySelectorAll('[contenteditable="true"]')
        .length;
    var suppressed = 0;
    void suppressImageEvent(web.Event event) {
      final target = event.target;
      if (target != null &&
          target.isA<web.HTMLImageElement>() &&
          (target as web.HTMLImageElement).src.startsWith('blob:')) {
        suppressed++;
        event.stopImmediatePropagation();
      }
    }

    // Capture the browser's decode error before it reaches the image listener,
    // reproducing a browser that never delivers a terminal image event.
    final listener = suppressImageEvent.toJS;
    web.window.addEventListener('error', listener, web.EventListenerOptions(capture: true));
    web.window.addEventListener('load', listener, web.EventListenerOptions(capture: true));
    addTearDown(() {
      web.window.removeEventListener('error', listener, web.EventListenerOptions(capture: true));
      web.window.removeEventListener('load', listener, web.EventListenerOptions(capture: true));
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
      web.document.querySelectorAll('[contenteditable="true"]').length,
      before,
    );
    expect(web.document.querySelectorAll('img[src^="blob:"]').length, 0);
  });
}
