// Open tabs survive a restart (lib/doc_tab_persistence.dart): what is saved,
// how the strip is rebuilt around the page startup already opened, and that
// startup frames cannot overwrite the saved strip.
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';
import 'package:mica_flutter/doc_tab.dart';
import 'package:mica_flutter/doc_tab_persistence.dart';

DocumentView _view(String id, {String type = 'document'}) => DocumentView(
  id: id,
  parentViewId: null,
  objectId: 'doc-$id',
  objectType: type,
  name: 'Page $id',
  position: '0000000010',
);

DocTab _tab(String id, String ws) => DocTab(view: _view(id))..workspaceId = ws;

void main() {
  final trees = {
    'w1': [_view('a'), _view('b'), _view('f', type: 'folder')],
    'w2': [_view('c')],
  };
  DocumentView? lookup(String ws, String id) =>
      trees[ws]?.where((v) => v.id == id).firstOrNull;

  test('a single tab, or tabs with no page, are not worth saving', () {
    expect(encodeOpenTabs([_tab('a', 'w1')], 0), isNull);
    expect(encodeOpenTabs([_tab('a', 'w1'), DocTab()], 0), isNull);
  });

  test('round trip keeps order, workspaces and the active tab', () {
    final saved = encodeOpenTabs([
      _tab('a', 'w1'),
      _tab('c', 'w2'),
      _tab('b', 'w1'),
    ], 2)!;
    // Startup reopened the last viewed page, `b`, in a fresh tab.
    final current = _tab('b', 'w1');
    final restored = restoreOpenTabs(saved, current, lookup)!;
    expect(restored.tabs.map((t) => t.view!.id), ['a', 'c', 'b']);
    expect(restored.tabs.map((t) => t.workspaceId), ['w1', 'w2', 'w1']);
    expect(restored.active, 2);
    expect(identical(restored.tabs[2], current), isTrue);
    // The others are labels: no document until first activated.
    expect(restored.tabs[0].bootstrap, isNull);
    expect(restored.tabs[1].bootstrap, isNull);
  });

  test('gone pages, folders and unknown workspaces are dropped', () {
    final saved = encodeOpenTabs([
      _tab('a', 'w1'),
      _tab('gone', 'w1'),
      _tab('f', 'w1'),
      _tab('c', 'w-left'),
    ], 0)!;
    final restored = restoreOpenTabs(saved, _tab('a', 'w1'), lookup);
    expect(restored, isNull, reason: 'only the current tab survived');
  });

  test('a current page not in the saved strip goes first', () {
    final saved = encodeOpenTabs([_tab('a', 'w1'), _tab('c', 'w2')], 1)!;
    final current = _tab('b', 'w1');
    final restored = restoreOpenTabs(saved, current, lookup)!;
    expect(restored.tabs.map((t) => t.view!.id), ['b', 'a', 'c']);
    expect(restored.active, 0);
  });

  test('garbage in the pref restores nothing', () {
    for (final bad in [null, '', 'not json', '[]', '{"tabs":3}']) {
      expect(restoreOpenTabs(bad, _tab('a', 'w1'), lookup), isNull, reason: bad);
    }
  });

  test('the saver writes only after priming, and only on change', () {
    final writes = <String>[];
    final saver = OpenTabsSaver();
    final tabs = [_tab('a', 'w1'), _tab('b', 'w1')];
    // Startup frames before the restore: one empty tab. Must not erase.
    saver.maybeSave([DocTab()], 0, writes.add);
    expect(writes, isEmpty);
    saver.prime(tabs, 0);
    saver.maybeSave(tabs, 0, writes.add);
    expect(writes, isEmpty, reason: 'nothing changed since the restore');
    saver.maybeSave(tabs, 1, writes.add);
    expect(writes, hasLength(1));
    saver.maybeSave(tabs, 1, writes.add);
    expect(writes, hasLength(1));
    // Back to one tab: the saved strip is cleared, not left stale.
    saver.maybeSave([tabs.first], 0, writes.add);
    expect(writes.last, '');
  });
}
