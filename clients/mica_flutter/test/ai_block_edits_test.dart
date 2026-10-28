import 'dart:async';
import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/ai_edits.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/markdown/markdown_codec.dart';
import 'package:mica_flutter/editor/model.dart';
import 'support/markdown_codec.dart';

EditorNode _node(
  String id,
  String text, {
  String kind = 'paragraph',
  Map<String, dynamic>? data,
}) => EditorNode(id: id, kind: kind, text: text, data: data);

BlockSpec _spec(
  String text, {
  String kind = 'paragraph',
  Map<String, dynamic> data = const {},
}) => (kind: kind, text: text, data: data);

AiBlockEdit _edit(List<String> ids, List<BlockSpec> replacement) => AiBlockEdit(
  blockIds: ids,
  before: 'preview before',
  after: 'preview after',
  replacement: replacement,
);

List<Map<String, dynamic>> _state(EditorController controller) => [
  for (final node in controller.nodes)
    {
      'id': node.id,
      'kind': node.kind,
      'text': node.text,
      'data': jsonDecode(jsonEncode(node.data)),
    },
];

Future<void> _drain() => Future<void>.delayed(Duration.zero);


void main() {
  setUpAll(useNativeMarkdownCodec);
  late EditorController controller;
  late List<List<Map<String, dynamic>>> writes;

  setUp(() {
    writes = [];
    controller = EditorController(
      rootBlockId: 'root',
      onOps: (ops) async {
        writes.add([
          for (final op in ops)
            Map<String, dynamic>.from(jsonDecode(jsonEncode(op)) as Map),
        ]);
      },
    );
    controller.load([
      _node(
        'a',
        'keep before',
        data: {
          'custom': {'value': 1},
        },
      ),
      _node('b', 'target one'),
      _node('c', 'keep middle'),
      _node('d', 'target two'),
      _node('e', 'keep after'),
    ]);
  });
  tearDown(() => controller.dispose());

  test(
    'AI apply does not report completion before its write completes',
    () async {
      controller.dispose();
      final committed = Completer<void>();
      controller = EditorController(
        rootBlockId: 'root',
        onOps: (ops) {
          writes.add(ops);
          return committed.future;
        },
      );
      controller.load([_node('a', 'original'), _node('b', 'untouched')]);
      final source = controller.captureAiSnapshot();
      var completed = false;
      final applied = controller
          .applyAiEdits(
            source,
            AiEditPreview([
              _edit(['a'], [_spec('optimized')]),
            ]),
          )
          .then((_) => completed = true);
      await _drain();
      expect(writes, hasLength(1));
      expect(completed, isFalse);
      committed.complete();
      await applied;
      expect(completed, isTrue);
    },
  );

  test(
    'AI write failure is reported while local content and undo are retained',
    () async {
      controller.dispose();
      var fail = true;
      controller = EditorController(
        rootBlockId: 'root',
        onOps: (ops) async {
          writes.add(ops);
          if (fail) throw StateError('outbox unavailable');
        },
      );
      controller.load([_node('a', 'original'), _node('b', 'untouched')]);
      final source = controller.captureAiSnapshot();
      final before = _state(controller);
      await expectLater(
        controller.applyAiEdits(
          source,
          AiEditPreview([
            _edit(['a'], [_spec('optimized')]),
          ]),
        ),
        throwsA(isA<AiEditSaveFailure>()),
      );
      expect(controller.nodes.map((node) => node.text), [
        'optimized',
        'untouched',
      ]);
      expect(controller.canUndo, isTrue);
      expect(controller.opFaultCount, 1);
      fail = false;
      controller.undo();
      await _drain();
      expect(_state(controller), before);
      expect(writes, hasLength(3), reason: 'save the failed batch before undo');
      expect(identical(writes[0], writes[1]), isTrue);
      expect(writes.last.single['text'], 'original');
    },
  );

  test(
    'failed pending user text flush prevents any AI mutation or AI write',
    () async {
      controller.dispose();
      controller = EditorController(
        rootBlockId: 'root',
        onOps: (ops) async {
          writes.add(ops);
          throw StateError('pending user edit not saved');
        },
      );
      controller.load([_node('a', 'target'), _node('b', 'other')]);
      final source = controller.captureAiSnapshot();
      controller.setSelection(const DocSelection.collapsed(DocPosition(1, 0)));
      controller.setFocusedText('pending user text', 17, 17);
      final before = _state(controller);
      await expectLater(
        controller.applyAiEdits(
          source,
          AiEditPreview([
            _edit(['a'], [_spec('must not apply')]),
          ]),
        ),
        throwsA(isA<AiEditSaveFailure>()),
      );
      expect(_state(controller), before);
      expect(writes, hasLength(1));
      expect(writes.single.single['block_id'], 'b');
      expect(writes.single.single['text'], 'pending user text');
    },
  );

  test(
    'canApply revoked during pending flush prevents AI changes and write',
    () async {
      controller.dispose();
      final flushed = Completer<void>();
      var allowed = true;
      controller = EditorController(
        rootBlockId: 'root',
        onOps: (ops) {
          writes.add(ops);
          return flushed.future;
        },
      );
      controller.load([_node('a', 'target'), _node('b', 'other')]);
      final source = controller.captureAiSnapshot();
      controller.setSelection(const DocSelection.collapsed(DocPosition(1, 0)));
      controller.setFocusedText('pending user text', 17, 17);
      final before = _state(controller);
      final applied = controller.applyAiEdits(
        source,
        AiEditPreview([
          _edit(['a'], [_spec('must not apply')]),
        ]),
        canApply: () => allowed,
      );
      final rejected = expectLater(applied, throwsA(isA<AiEditConflict>()));
      await _drain();
      expect(writes, hasLength(1));
      allowed = false;
      flushed.complete();
      await rejected;
      expect(_state(controller), before);
      expect(writes.single.single['block_id'], 'b');
    },
  );

  test(
    'canApply revoked while queued prevents dispatch of AI operations',
    () async {
      controller.dispose();
      final earlierWrite = Completer<void>();
      var allowed = true;
      controller = EditorController(
        rootBlockId: 'root',
        onOps: (ops) {
          writes.add(ops);
          return earlierWrite.future;
        },
      );
      controller.load([_node('a', 'target'), _node('b', 'other')]);
      final source = controller.captureAiSnapshot();
      // A structural send is in flight while _dirty remains empty, so the AI
      // passes flushPending but waits on the controller's existing send queue.
      controller.setBlockText('b', 'earlier committed user change');
      await _drain();
      final applied = controller.applyAiEdits(
        source,
        AiEditPreview([
          _edit(['a'], [_spec('queued AI proposal')]),
        ]),
        canApply: () => allowed,
      );
      final rejected = expectLater(applied, throwsA(isA<AiEditConflict>()));
      await _drain();
      expect(writes, hasLength(1));
      allowed = false;
      earlierWrite.complete();
      await rejected;
      expect(
        writes,
        hasLength(1),
        reason: 'the queued AI callback was never invoked',
      );
      expect(writes.single.single['block_id'], 'b');
      expect(
        controller.opFaultCount,
        0,
        reason: 'revocation is not a storage fault',
      );
    },
  );

  test('snapshot freezes nested data independently of later edits', () {
    final source = controller.captureAiSnapshot();
    (controller.nodes.first.data['custom'] as Map)['value'] = 9;
    controller.nodes[1].text = 'later text';
    expect(source.rootBlockId, 'root');
    expect(source.nodes.first.data['custom'], {'value': 1});
    expect(source.nodes[1].text, 'target one');
    expect(source.blocks[1]['id'], 'b');
    expect(source.blocks[1]['type'], 'paragraph');
  });

  test(
    'only target changes and unrelated edits made after generation survive undo',
    () async {
      final source = controller.captureAiSnapshot();
      controller.setBlockText('c', 'user edit after generation');
      await controller.flushPending();
      await _drain();
      final beforeAi = _state(controller);
      writes.clear();
      await controller.applyAiEdits(
        source,
        AiEditPreview([
          _edit(
            ['b'],
            [
              _spec('optimized', kind: 'heading', data: {'level': 2}),
            ],
          ),
        ]),
      );
      await _drain();
      expect(controller.nodes.map((n) => n.id), ['a', 'b', 'c', 'd', 'e']);
      expect(controller.nodes[1].kind, 'heading');
      expect(controller.nodes[1].text, 'optimized');
      expect(controller.nodes[1].data, {'level': 2});
      for (final index in [0, 2, 3, 4]) {
        expect(_state(controller)[index], beforeAi[index]);
      }
      expect(writes, hasLength(1));
      expect(writes.single.map((op) => op['block_id']), ['b']);
      controller.undo();
      await _drain();
      expect(_state(controller), beforeAi);
    },
  );

  test('multiple nonadjacent ranges are a single undo and redo step', () async {
    final before = _state(controller);
    final source = controller.captureAiSnapshot();
    await controller.applyAiEdits(
      source,
      AiEditPreview([
        _edit(['d'], [_spec('D revised')]),
        _edit(['b'], [_spec('B revised')]),
      ]),
    );
    await _drain();
    final after = _state(controller);
    expect(controller.nodes.map((n) => n.text), [
      'keep before',
      'B revised',
      'keep middle',
      'D revised',
      'keep after',
    ]);
    expect(writes, hasLength(1));
    controller.undo();
    await _drain();
    expect(_state(controller), before);
    expect(controller.canUndo, isFalse);
    controller.redo();
    await _drain();
    expect(_state(controller), after);
  });

  test(
    'replacement retains existing IDs and allocates IDs only for added blocks',
    () async {
      final source = controller.captureAiSnapshot();
      await controller.applyAiEdits(
        source,
        AiEditPreview([
          _edit(['b', 'c'], [_spec('first'), _spec('second'), _spec('added')]),
        ]),
      );
      await _drain();
      expect(controller.nodes.map((n) => n.text), [
        'keep before',
        'first',
        'second',
        'added',
        'target two',
        'keep after',
      ]);
      expect(controller.nodes.take(3).map((n) => n.id), ['a', 'b', 'c']);
      expect(
        controller.nodes[3].id,
        isNot(isIn(source.nodes.map((n) => n.id))),
      );
      expect(controller.nodes.map((n) => n.id).toSet(), hasLength(6));
      final inserts = writes.single.where((op) => op['type'] == 'insert_block');
      expect(inserts, hasLength(1));
      expect(inserts.single['index'], 3);
      expect((inserts.single['block'] as Map)['id'], controller.nodes[3].id);
    },
  );

  test(
    'partial deletion preserves surrounding blocks and can be undone',
    () async {
      final source = controller.captureAiSnapshot();
      final before = _state(controller);
      await controller.applyAiEdits(
        source,
        AiEditPreview([
          _edit(['b', 'c'], []),
        ]),
      );
      await _drain();
      expect(controller.nodes.map((n) => n.id), ['a', 'd', 'e']);
      expect(writes.single.map((op) => op['block_id']), ['b', 'c']);
      expect(writes.single.every((op) => op['type'] == 'delete_block'), isTrue);
      controller.undo();
      await _drain();
      expect(_state(controller), before);
    },
  );

  for (final changed in [
    'text',
    'data',
    'kind',
    'removed',
    'inserted',
    'reordered',
  ]) {
    test(
      'target $changed conflict rejects all ranges without any AI writes',
      () async {
        final source = controller.captureAiSnapshot();
        // Simulate collaborator changes already present in the live model.
        switch (changed) {
          case 'text':
            controller.nodes[3].text = 'collaborator text';
          case 'data':
            controller.nodes[3].data = {'custom': 1};
          case 'kind':
            controller.nodes[3].kind = 'quote';
          case 'removed':
            controller.nodes.removeAt(3);
          case 'inserted':
            controller.nodes.insert(3, _node('new', 'inside target'));
          case 'reordered':
            final node = controller.nodes.removeAt(3);
            controller.nodes.insert(2, node);
        }
        final liveBefore = _state(controller);
        final selectionBefore = controller.selection;
        await expectLater(
          controller.applyAiEdits(
            source,
            AiEditPreview([
              _edit(['b'], [_spec('must never apply')]),
              _edit(['c', 'd'], [_spec('also rejected')]),
            ]),
          ),
          throwsA(isA<AiEditConflict>()),
        );
        await _drain();
        expect(_state(controller), liveBefore);
        expect(controller.selection, selectionBefore);
        expect(writes, isEmpty);
        expect(controller.canUndo, isFalse);
      },
    );
  }

  test(
    'deep target data mutation is detected by the frozen snapshot',
    () async {
      final source = controller.captureAiSnapshot();
      (controller.nodes.first.data['custom'] as Map)['value'] = 2;
      final liveBefore = _state(controller);
      await expectLater(
        controller.applyAiEdits(
          source,
          AiEditPreview([
            _edit(['a'], [_spec('rejected')]),
          ]),
        ),
        throwsA(isA<AiEditConflict>()),
      );
      await _drain();
      expect(_state(controller), liveBefore);
      expect(writes, isEmpty);
    },
  );

  test(
    'whole-page deletion is rejected before any mutation or write',
    () async {
      final source = controller.captureAiSnapshot();
      final before = _state(controller);
      await expectLater(
        controller.applyAiEdits(
          source,
          AiEditPreview([
            _edit(['a', 'b', 'c', 'd', 'e'], []),
          ]),
        ),
        throwsA(isA<AiEditConflict>()),
      );
      await _drain();
      expect(_state(controller), before);
      expect(writes, isEmpty);
      expect(controller.canUndo, isFalse);
    },
  );

  for (final bad in [
    'duplicate',
    'unknown',
    'noncontiguous',
    'empty',
    'root',
  ]) {
    test('$bad edit plan is rejected without partial writes', () async {
      final source = bad == 'root'
          ? EditorAiSnapshot('another-root', controller.nodes)
          : controller.captureAiSnapshot();
      final before = _state(controller);
      final edits = switch (bad) {
        'duplicate' => [
          _edit(['b'], [_spec('one')]),
          _edit(['b'], [_spec('two')]),
        ],
        'unknown' => [
          _edit(['missing'], [_spec('one')]),
        ],
        'noncontiguous' => [
          _edit(['b', 'd'], [_spec('one')]),
        ],
        'empty' => <AiBlockEdit>[],
        _ => [
          _edit(['b'], [_spec('one')]),
        ],
      };
      await expectLater(
        controller.applyAiEdits(source, AiEditPreview(edits)),
        throwsA(isA<AiEditConflict>()),
      );
      await _drain();
      expect(_state(controller), before);
      expect(writes, isEmpty);
    });
  }
}
