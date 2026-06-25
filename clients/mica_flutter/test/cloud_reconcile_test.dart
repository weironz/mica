// Regression for the cloud page-switch data-loss bug.
//
// A cloud doc whose content lives only in the yrs base (the op snapshot is stale)
// bootstraps EMPTY on a page switch; the yrs session then delivers the real
// content via _applyCloudBlocks -> editor `reconcile`. The bug was that the editor
// never reconciled because _applyCloudBlocks kept the same `versionSeq` (the editor
// only reconciles on a version change), so the content showed as lost. The fix
// bumps the version; this guards the `reconcile` merge it relies on:
//   (1) a stale empty seed is replaced by the real (yrs) content, and
//   (2) the user's in-flight (unsent) local edit is NOT clobbered when reconcile
//       fires (it now fires more often, so this invariant matters more).
@TestOn('vm')
library;

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/model.dart';

EditorNode _para(String id, String text) =>
    EditorNode(id: id, kind: 'paragraph', text: text);

void main() {
  test('reconcile replaces the stale empty seed with the yrs content', () {
    final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
    // Op-snapshot bootstrap for a doc whose content is only in the yrs base:
    // a single empty seed paragraph.
    c.load([_para('seed', '')]);
    expect(c.nodes.map((n) => n.text).toList(), ['']);

    // The yrs session bootstraps and delivers the real content (different block
    // ids). This is what the editor now reconciles to once the version bumps.
    c.reconcile([_para('b1', 'hello cloud'), _para('b2', 'second line')]);

    expect(c.nodes.map((n) => n.id).toList(), ['b1', 'b2']);
    expect(c.nodes.map((n) => n.text).toList(), ['hello cloud', 'second line'],
        reason: 'previously-"lost" yrs content is shown after reconcile');
  });

  test('reconcile does NOT clobber an in-flight (dirty) local edit', () {
    final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
    c.load([_para('b1', '')]);

    // The user just typed into b1 (debounced, not yet sent) — b1 is dirty.
    c.setSelection(const DocSelection.collapsed(DocPosition(0, 0)));
    c.setFocusedText('my unsent text', 14, 14);

    // A reconcile (e.g. from a yrs onReady/remote update carrying older text for
    // b1) must keep the user's in-flight text, not overwrite it.
    c.reconcile([_para('b1', 'stale server text')]);

    expect(c.nodes.first.text, 'my unsent text',
        reason: 'dirty local edit survives a reconcile');
  });

  test('reconcile applies server text to a non-dirty matching block', () {
    final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
    c.load([_para('b1', 'old')]);
    // b1 is clean (no local edit) → reconcile may update its text.
    c.reconcile([_para('b1', 'updated from server')]);
    expect(c.nodes.first.text, 'updated from server');
  });
}
