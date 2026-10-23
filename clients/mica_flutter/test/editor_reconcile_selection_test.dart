import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/model.dart';

EditorNode _paragraph(String id, String text, {Map<String, dynamic>? data}) =>
    EditorNode(id: id, kind: 'paragraph', text: text, data: data);

void main() {
  group('reconcile selection (#51)', () {
    late EditorController controller;
    late List<DocOp> sent;

    setUp(() {
      sent = [];
      controller = EditorController(
        rootBlockId: 'root',
        onOps: (ops) async => sent.addAll(ops),
      );
    });
    tearDown(() => controller.dispose());

    for (final reverse in [false, true]) {
      test('preserves ${reverse ? 'backward' : 'forward'} range on echo', () {
        controller.load([_paragraph('a', 'abcdef')]);
        final selection = DocSelection(
          anchor: DocPosition(0, reverse ? 5 : 1),
          focus: DocPosition(0, reverse ? 1 : 5),
        );
        controller.setSelection(selection);

        controller.reconcile([_paragraph('a', 'abcdef')]);

        expect(controller.selection, selection);
        expect(controller.selection!.isCollapsed, isFalse);
        expect(sent, isEmpty);
      });
    }

    test('tracks both block ids across insertions and reordering', () {
      controller.load([
        _paragraph('a', 'alpha'),
        _paragraph('b', 'bravo'),
        _paragraph('c', 'charlie'),
      ]);
      controller.setSelection(
        const DocSelection(anchor: DocPosition(2, 6), focus: DocPosition(0, 2)),
      );

      controller.reconcile([
        _paragraph('new', 'new'),
        _paragraph('c', 'charlie'),
        _paragraph('b', 'bravo'),
        _paragraph('a', 'alpha'),
      ]);

      expect(
        controller.selection,
        const DocSelection(anchor: DocPosition(1, 6), focus: DocPosition(3, 2)),
      );
    });

    final offsetCases =
        <
          ({
            String name,
            String before,
            String after,
            int anchor,
            int focus,
            int expectedAnchor,
            int expectedFocus,
          })
        >[
          (
            name: 'insert before range',
            before: 'abcdef',
            after: 'XXabcdef',
            anchor: 1,
            focus: 5,
            expectedAnchor: 3,
            expectedFocus: 7,
          ),
          (
            name: 'insert inside range',
            before: 'abcdef',
            after: 'abcXXdef',
            anchor: 1,
            focus: 5,
            expectedAnchor: 1,
            expectedFocus: 7,
          ),
          (
            name: 'insert after range',
            before: 'abcdef',
            after: 'abcdefXX',
            anchor: 1,
            focus: 5,
            expectedAnchor: 1,
            expectedFocus: 5,
          ),
          (
            name: 'insertion at caret uses right affinity',
            before: 'abcdef',
            after: 'abcXXdef',
            anchor: 3,
            focus: 3,
            expectedAnchor: 5,
            expectedFocus: 5,
          ),
          (
            name: 'delete before backward range',
            before: 'abcdef',
            after: 'cdef',
            anchor: 5,
            focus: 3,
            expectedAnchor: 3,
            expectedFocus: 1,
          ),
          (
            name: 'delete across endpoint',
            before: 'abcdef',
            after: 'af',
            anchor: 3,
            focus: 6,
            expectedAnchor: 1,
            expectedFocus: 2,
          ),
          (
            name: 'delete entire text',
            before: 'abcdef',
            after: '',
            anchor: 1,
            focus: 5,
            expectedAnchor: 0,
            expectedFocus: 0,
          ),
          (
            name: 'replacement keeps unchanged suffix',
            before: 'abcdef',
            after: 'abXYZef',
            anchor: 1,
            focus: 5,
            expectedAnchor: 1,
            expectedFocus: 6,
          ),
          (
            name: 'UTF-16 insertion',
            before: 'a😀bc',
            after: '中a😀bc',
            anchor: 3,
            focus: 5,
            expectedAnchor: 4,
            expectedFocus: 6,
          ),
          (
            name: 'separate insertions preserve unchanged middle range',
            before: 'abcdefghij',
            after: 'aXXbcdefghiYYj',
            anchor: 3,
            focus: 7,
            expectedAnchor: 5,
            expectedFocus: 9,
          ),
          (
            name: 'separate deletions preserve unchanged middle range',
            before: 'aXXbcdefghiYYj',
            after: 'abcdefghij',
            anchor: 5,
            focus: 9,
            expectedAnchor: 3,
            expectedFocus: 7,
          ),
          (
            name: 'mixed deletion and insertion preserve middle range',
            before: 'aXXbcdefghij',
            after: 'abcdefghYYij',
            anchor: 5,
            focus: 9,
            expectedAnchor: 3,
            expectedFocus: 7,
          ),
          (
            name: 'separate replacements preserve middle range',
            before: 'aaOLDmiddleTAILzz',
            after: 'aaNEWmiddleENDzz',
            anchor: 6,
            focus: 10,
            expectedAnchor: 6,
            expectedFocus: 10,
          ),
          (
            name: 'multiple replacements map interiors to each start',
            before: 'abCDmiddleEFgh',
            after: 'abXYZmiddleQgh',
            anchor: 3,
            focus: 11,
            expectedAnchor: 2,
            expectedFocus: 11,
          ),
          (
            name: 'separate insertions at endpoints use right affinity',
            before: 'abcdefghij',
            after: 'abcXXdefgYYhij',
            anchor: 3,
            focus: 7,
            expectedAnchor: 5,
            expectedFocus: 11,
          ),
          (
            name: 'repeated words retain middle range across insertions',
            before: 'one one middle one one',
            after: 'one XXone middle one YYone',
            anchor: 8,
            focus: 14,
            expectedAnchor: 10,
            expectedFocus: 16,
          ),
          (
            name: 'repeated characters use stable prefix suffix alignment',
            before: 'aaaaMIDDLEaaaa',
            after: 'aaaaaMIDDLEaaa',
            anchor: 4,
            focus: 10,
            expectedAnchor: 5,
            expectedFocus: 11,
          ),
          (
            name: 'multiple UTF-16 edits preserve middle range',
            before: 'a😀middle😃z',
            after: 'a中😀middle😃文z',
            anchor: 3,
            focus: 9,
            expectedAnchor: 4,
            expectedFocus: 10,
          ),
          (
            name: 'large full replacement uses bounded fallback',
            before: 'prefix${'a' * 10000}suffix',
            after: 'prefix${'b' * 10000}suffix',
            anchor: 7,
            focus: 10006,
            expectedAnchor: 6,
            expectedFocus: 10006,
          ),
          (
            name: 'snake work budget falls back for huge interior',
            before: 'a${'x' * 210000}b',
            after: 'c${'x' * 210000}d',
            anchor: 1,
            focus: 210001,
            expectedAnchor: 0,
            expectedFocus: 0,
          ),
          (
            name: 'large length difference uses single replacement fallback',
            before: 'aMIDDLEz',
            after: '${'x' * 1000}MIDDLEy',
            anchor: 2,
            focus: 6,
            expectedAnchor: 0,
            expectedFocus: 0,
          ),
        ];
    for (final value in offsetCases) {
      for (final reverse in [false, true]) {
        test('${value.name}${reverse ? ' (backward)' : ''}', () {
          controller.load([_paragraph('a', value.before)]);
          controller.setSelection(
            DocSelection(
              anchor: DocPosition(0, reverse ? value.focus : value.anchor),
              focus: DocPosition(0, reverse ? value.anchor : value.focus),
            ),
          );

          controller.reconcile([_paragraph('a', value.after)]);

          expect(controller.nodes.single.text, value.after);
          expect(
            controller.selection,
            DocSelection(
              anchor: DocPosition(
                0,
                reverse ? value.expectedFocus : value.expectedAnchor,
              ),
              focus: DocPosition(
                0,
                reverse ? value.expectedAnchor : value.expectedFocus,
              ),
            ),
          );
        });
      }
    }

    test('maps text changes independently across reordered blocks', () {
      controller.load([_paragraph('a', 'abcd'), _paragraph('b', 'wxyz')]);
      controller.setSelection(
        const DocSelection(anchor: DocPosition(1, 3), focus: DocPosition(0, 2)),
      );

      controller.reconcile([_paragraph('b', 'yz'), _paragraph('a', 'XXabcd')]);

      expect(
        controller.selection,
        const DocSelection(anchor: DocPosition(0, 1), focus: DocPosition(1, 4)),
      );
    });

    test(
      'deleted anchor follows surviving old successor despite new blocks',
      () {
        controller.load([
          _paragraph('a', 'alpha'),
          _paragraph('b', 'bravo'),
          _paragraph('c', 'charlie'),
        ]);
        controller.setSelection(
          const DocSelection(
            anchor: DocPosition(1, 3),
            focus: DocPosition(0, 2),
          ),
        );

        controller.reconcile([
          _paragraph('new', 'new'),
          _paragraph('c', 'charlie'),
          _paragraph('a', 'alpha'),
        ]);

        expect(
          controller.selection,
          const DocSelection(
            anchor: DocPosition(1, 0),
            focus: DocPosition(2, 2),
          ),
        );
      },
    );

    test('deleted focus at document end follows predecessor end', () {
      controller.load([_paragraph('a', 'alpha'), _paragraph('b', 'bravo')]);
      controller.setSelection(
        const DocSelection(anchor: DocPosition(0, 1), focus: DocPosition(1, 3)),
      );

      controller.reconcile([_paragraph('a', 'alpha')]);

      expect(
        controller.selection,
        const DocSelection(anchor: DocPosition(0, 1), focus: DocPosition(0, 5)),
      );
    });

    test('complete replacement has valid endpoint fallbacks', () {
      controller.load([_paragraph('a', 'alpha'), _paragraph('b', 'bravo')]);
      controller.setSelection(
        const DocSelection(anchor: DocPosition(0, 4), focus: DocPosition(1, 5)),
      );

      controller.reconcile([_paragraph('new', 'x')]);

      expect(
        controller.selection,
        const DocSelection.collapsed(DocPosition(0, 0)),
      );
    });

    test('empty snapshot clears selection', () {
      controller.load([_paragraph('a', 'alpha')]);
      controller.setSelection(
        const DocSelection(anchor: DocPosition(0, 1), focus: DocPosition(0, 4)),
      );

      controller.reconcile([]);

      expect(controller.selection, isNull);
    });

    test('snapshot does not create an absent selection', () {
      controller.load([_paragraph('a', 'alpha')]);

      controller.reconcile([_paragraph('a', 'XXalpha')]);

      expect(controller.selection, isNull);
    });

    test(
      'dirty text, marks and backward selection survive remote changes',
      () async {
        final marks = [
          {'start': 0, 'end': 3, 'type': 'bold'},
        ];
        controller.load([
          _paragraph('a', 'local', data: {'marks': marks}),
        ]);
        controller.setSelection(
          const DocSelection.collapsed(DocPosition(0, 5)),
        );
        controller.setFocusedText('local!', 6, 2);
        final localMarks = controller.nodes.single.data['marks'];

        controller.reconcile([
          _paragraph('new', 'new'),
          _paragraph(
            'a',
            'REMOTE local',
            data: {
              'marks': [
                {'start': 7, 'end': 12, 'type': 'italic'},
              ],
              'quote': 1,
            },
          ),
        ]);

        expect(controller.nodes[1].text, 'local!');
        expect(controller.nodes[1].data['marks'], localMarks);
        expect(controller.nodes[1].data['quote'], 1);
        expect(
          controller.selection,
          const DocSelection(
            anchor: DocPosition(1, 6),
            focus: DocPosition(1, 2),
          ),
        );
        await controller.flushPending();
        expect(sent.single['text'], 'local!');
        expect((sent.single['data'] as Map)['marks'], localMarks);
      },
    );
  });
}
