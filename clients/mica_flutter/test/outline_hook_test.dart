import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/editor.dart';

// The outline used to read a frozen bootstrap snapshot, so local edits didn't
// show until you navigated away and back. It now listens to EditorOutlineHook,
// which the editor republishes on every edit. These lock in the hook's
// contract: it notifies when the heading list actually changes, and stays quiet
// (no outline rebuild churn) when a republish is identical.
void main() {
  group('EditorOutlineHook', () {
    test('notifies + stores headings on first publish', () {
      final hook = EditorOutlineHook();
      var n = 0;
      hook.addListener(() => n++);
      hook.publish([const OutlineEntry(id: 'a', text: 'One', level: 1)]);
      expect(n, 1);
      expect(hook.headings.single.text, 'One');
    });

    test('identical republish does NOT notify (dedupes rebuild churn)', () {
      final hook = EditorOutlineHook();
      var n = 0;
      hook.addListener(() => n++);
      hook.publish([const OutlineEntry(id: 'a', text: 'One', level: 1)]);
      hook.publish([const OutlineEntry(id: 'a', text: 'One', level: 1)]);
      expect(n, 1);
    });

    test('notifies when a heading is retitled (live typing)', () {
      final hook = EditorOutlineHook();
      var n = 0;
      hook.addListener(() => n++);
      hook.publish([const OutlineEntry(id: 'a', text: 'One', level: 1)]);
      hook.publish([const OutlineEntry(id: 'a', text: 'One edited', level: 1)]);
      expect(n, 2);
      expect(hook.headings.single.text, 'One edited');
    });

    test('notifies on level change and on count change', () {
      final hook = EditorOutlineHook();
      var n = 0;
      hook.addListener(() => n++);
      hook.publish([const OutlineEntry(id: 'a', text: 'One', level: 1)]);
      hook.publish([const OutlineEntry(id: 'a', text: 'One', level: 2)]);
      hook.publish([
        const OutlineEntry(id: 'a', text: 'One', level: 2),
        const OutlineEntry(id: 'b', text: 'Two', level: 1),
      ]);
      expect(n, 3);
    });

    test('OutlineEntry has value equality', () {
      expect(
        const OutlineEntry(id: 'a', text: 't', level: 1),
        const OutlineEntry(id: 'a', text: 't', level: 1),
      );
      expect(
        const OutlineEntry(id: 'a', text: 't', level: 1),
        isNot(const OutlineEntry(id: 'a', text: 't', level: 2)),
      );
    });
  });
}
