// P2-M4 (web→yjs): the web counterpart of the desktop `MicaDocument` (FFI).
//
// Backed by a JS `Y.Doc` (driven over `yjs_interop.dart`) but reading/writing the
// SAME shared-type layout the Rust `mica-core` model uses, so the two are
// wire-compatible:
//   meta:   Map  { root: <block id> }
//   blocks: Map<block_id, Map { ty, text(Y.Text, formatting = inline marks),
//                               props(JSON string of data minus marks),
//                               children(Y.Array<string>) }>
//
// This mirrors `crates/mica-core/src/{doc,marks}.rs`: the block tree DFS, the
// marks ↔ Y.Text-formatting attribute schema (key = mark type; value = `true` or
// `{href,title}`; UTF-16 offsets, which match JS string indices), and the coarse
// editor ops (insert/update/delete/move block). Web-only.
import 'dart:convert';
import 'dart:js_interop';
import 'dart:typed_data';

import 'yjs_interop.dart';

/// Open mark while reconstructing from a delta: (startOffset, href, title).
typedef _Open = (int, String?, String?);

class MicaYDoc {
  MicaYDoc._(this._doc);
  final JSObject _doc;

  /// Last-known `data` per block id — recovers inline marks for the editor's
  /// text-only `update_block` straggler (mirrors `DocOpMirror`).
  final Map<String, Map<String, dynamic>> _dataById = {};

  /// Rebuild a doc from an encoded yrs/yjs v1 update (e.g. the server's base).
  static MicaYDoc fromState(Uint8List bytes) {
    final d = MicaYDoc._(micaYjs.docFromUpdate(bytes.toJS));
    d.seedDataCache();
    return d;
  }

  static MicaYDoc empty() => MicaYDoc._(micaYjs.newDoc());

  // ── sync primitives (mirror MicaDocument) ──
  Uint8List stateVector() => micaYjs.encodeStateVector(_doc).toDart;
  Uint8List encodeState() => micaYjs.encodeState(_doc).toDart;
  Uint8List encodeDiffSince(Uint8List sv) =>
      micaYjs.encodeDiff(_doc, sv.toJS).toDart;
  bool applyUpdate(Uint8List bytes) => micaYjs.applyUpdate(_doc, bytes.toJS);

  // ── shared-type accessors ──
  JSObject _blocksMap() => micaYjs.getMap(_doc, 'blocks');
  JSObject _metaMap() => micaYjs.getMap(_doc, 'meta');

  JSObject? _blockMap(String id) {
    final b = micaYjs.mapGet(_blocksMap(), id);
    return (b != null && micaYjs.isMap(b)) ? b as JSObject : null;
  }

  JSObject? _childrenArray(JSObject bm) {
    final c = micaYjs.mapGet(bm, 'children');
    return (c != null && micaYjs.isArray(c)) ? c as JSObject : null;
  }

  List<String> _arrayStrings(JSObject arr) {
    final out = <String>[];
    for (final c in micaYjs.arrayToList(arr).toDart) {
      if (c != null && c.isA<JSString>()) out.add((c as JSString).toDart);
    }
    return out;
  }

  /// The parent block map of `id` + the child index, scanning all blocks
  /// (mirrors the Rust `find_parent`).
  (JSObject, int)? _findParent(String id) {
    final blocks = _blocksMap();
    for (final k in micaYjs.mapKeys(blocks).toDart) {
      final bm = micaYjs.mapGet(blocks, k.toDart);
      if (bm == null || !micaYjs.isMap(bm)) continue;
      final c = _childrenArray(bm as JSObject);
      if (c == null) continue;
      final idx = _arrayStrings(c).indexOf(id);
      if (idx >= 0) return (bm, idx);
    }
    return null;
  }

  String rootBlockId() {
    final r = micaYjs.mapGet(_metaMap(), 'root');
    return (r != null && r.isA<JSString>()) ? (r as JSString).toDart : '';
  }

  // ── read side ──────────────────────────────────────────────────────────────

  /// The document as a flat block list in tree order (mirrors `to_blocks`),
  /// including inline marks reconstructed from each text's formatting.
  List<Map<String, dynamic>> toBlocks() {
    final blocks = _blocksMap();
    final byId = <String, Map<String, dynamic>>{};
    for (final k in micaYjs.mapKeys(blocks).toDart) {
      final id = k.toDart;
      final bm = micaYjs.mapGet(blocks, id);
      if (bm == null || !micaYjs.isMap(bm)) continue;
      final block = bm as JSObject;

      final tyAny = micaYjs.mapGet(block, 'ty');
      final ty = (tyAny != null && tyAny.isA<JSString>())
          ? (tyAny as JSString).toDart
          : 'paragraph';

      final textAny = micaYjs.mapGet(block, 'text');
      final hasText = textAny != null && micaYjs.isText(textAny);
      final text = hasText ? micaYjs.textToString(textAny) : '';

      var data = _readProps(block);
      if (hasText) {
        final marks = _marksFromText(textAny as JSObject);
        if (marks.isNotEmpty) data = {...data, 'marks': marks};
      }

      final children = <String>[];
      final childrenAny = micaYjs.mapGet(block, 'children');
      if (childrenAny != null && micaYjs.isArray(childrenAny)) {
        children.addAll(_arrayStrings(childrenAny as JSObject));
      }

      byId[id] = {
        'id': id,
        'type': ty,
        'text': text,
        'data': data,
        'children': children,
      };
    }

    final out = <Map<String, dynamic>>[];
    final seen = <String>{};
    void dfs(String id) {
      if (!seen.add(id)) return;
      final b = byId[id];
      if (b == null) return;
      out.add(b);
      for (final c in (b['children'] as List).cast<String>()) {
        dfs(c);
      }
    }

    final root = rootBlockId();
    if (byId.containsKey(root)) {
      dfs(root);
    } else {
      out.addAll(byId.values);
    }
    return out;
  }

  String toBlocksJson() => jsonEncode(toBlocks());

  /// Reconstruct marks from a Y.Text delta (mirrors `marks_from_runs`): a mark
  /// type stays open across runs that carry it with the same metadata, and
  /// closes when it disappears or its metadata changes.
  List<Map<String, dynamic>> _marksFromText(JSObject text) {
    final delta = jsonDecode(micaYjs.textDeltaJson(text));
    if (delta is! List) return const [];

    final marks = <Map<String, dynamic>>[];
    final open = <String, _Open>{};
    var offset = 0;

    for (final op in delta) {
      if (op is! Map) continue;
      final insert = op['insert'];
      final len = (insert is String) ? insert.length : 0;
      final here = <String, (String?, String?)>{};
      final attrs = op['attributes'];
      if (attrs is Map) {
        attrs.forEach((k, v) {
          if (v == null) return;
          String? href, title;
          if (v is Map) {
            href = v['href'] as String?;
            title = v['title'] as String?;
          }
          here['$k'] = (href, title);
        });
      }

      // Close marks absent here or whose metadata changed.
      final toClose = open.keys.where((ty) {
        final h = here[ty];
        if (h == null) return true;
        final o = open[ty]!;
        return h.$1 != o.$2 || h.$2 != o.$3;
      }).toList();
      for (final ty in toClose) {
        final o = open.remove(ty)!;
        marks.add(_markMap(o.$1, offset, ty, o.$2, o.$3));
      }
      // Open newly present marks.
      here.forEach((ty, meta) {
        open.putIfAbsent(ty, () => (offset, meta.$1, meta.$2));
      });
      offset += len;
    }

    final tail = open.entries.toList()..sort((a, b) => a.key.compareTo(b.key));
    for (final e in tail) {
      marks.add(_markMap(e.value.$1, offset, e.key, e.value.$2, e.value.$3));
    }

    marks.sort((a, b) {
      final c1 = (a['start'] as int).compareTo(b['start'] as int);
      if (c1 != 0) return c1;
      final c2 = (a['end'] as int).compareTo(b['end'] as int);
      if (c2 != 0) return c2;
      return (a['type'] as String).compareTo(b['type'] as String);
    });
    return marks;
  }

  // ── write side (mirrors doc.rs edit ops) ─────────────────────────────────────

  void seedDataCache() {
    _dataById.clear();
    for (final b in toBlocks()) {
      final d = b['data'];
      if (d is Map<String, dynamic>) _dataById[b['id'] as String] = d;
    }
  }

  /// Replay one editor block-op (mirrors `DocOpMirror.apply`).
  void applyOp(Map<String, dynamic> op) {
    switch (op['type'] as String?) {
      case 'insert_block':
        final block = (op['block'] as Map).cast<String, dynamic>();
        final data = block['data'];
        if (data is Map<String, dynamic>) {
          _dataById[block['id'] as String] = data;
        }
        _insertBlock(op['parent_id'] as String, op['index'] as int, block);
      case 'update_block':
        final id = op['block_id'] as String;
        Map<String, dynamic>? data;
        if (op['data'] is Map) {
          data = (op['data'] as Map).cast<String, dynamic>();
          _dataById[id] = data;
        } else if (op.containsKey('text')) {
          data = _dataById[id];
        }
        _updateBlock(id, op['kind'] as String?, op['text'] as String?, data);
      case 'delete_block':
        _dataById.remove(op['block_id']);
        _deleteBlock(op['block_id'] as String, false);
      case 'move_block':
        _moveBlock(
          op['block_id'] as String,
          op['parent_id'] as String,
          op['index'] as int,
        );
    }
  }

  Map<String, dynamic> _markMap(
      int start, int end, String ty, String? href, String? title) {
    final m = <String, dynamic>{'start': start, 'end': end, 'type': ty};
    if (href != null) m['href'] = href;
    if (title != null) m['title'] = title;
    return m;
  }

  List<Map<String, dynamic>> _marksFromData(Map<String, dynamic> data) {
    final raw = data['marks'];
    if (raw is! List) return const [];
    final out = <Map<String, dynamic>>[];
    for (final m in raw) {
      if (m is! Map) continue;
      final start = m['start'];
      final end = m['end'];
      final type = m['type'];
      if (start is! int || end is! int || type is! String || end <= start) {
        continue;
      }
      final mark = <String, dynamic>{'start': start, 'end': end, 'type': type};
      if (m['href'] is String) mark['href'] = m['href'];
      if (m['title'] is String) mark['title'] = m['title'];
      out.add(mark);
    }
    return out;
  }

  /// Read a block's `props` into a data map — field-level Y.Map (P2-M4.7) or the
  /// legacy JSON-string form (pre-M4.7 data), mirroring the Rust `read_props`.
  Map<String, dynamic> _readProps(JSObject block) {
    final p = micaYjs.mapGet(block, 'props');
    if (p == null) return {};
    if (micaYjs.isMap(p)) {
      final decoded = jsonDecode(micaYjs.mapEntriesJson(p as JSObject));
      return decoded is Map<String, dynamic> ? decoded : {};
    }
    if (p.isA<JSString>()) {
      final s = (p as JSString).toDart;
      if (s.isEmpty || s == 'null') return {};
      final decoded = jsonDecode(s);
      return decoded is Map<String, dynamic> ? decoded : {};
    }
    return {};
  }

  /// Write a block's `data` (minus `marks`) into a nested Y.Map `props`,
  /// reconciling keys in place so concurrent edits to different keys converge
  /// (field-level CRDT, mirrors the Rust `set_props`).
  void _setProps(JSObject bm, Map<String, dynamic> data) {
    final existing = micaYjs.mapGet(bm, 'props');
    JSObject props;
    if (existing != null && micaYjs.isMap(existing)) {
      props = existing as JSObject;
    } else {
      props = micaYjs.newMap();
      micaYjs.mapSet(bm, 'props', props);
    }
    final desired = Map<String, dynamic>.from(data)..remove('marks');
    for (final k in micaYjs.mapKeys(props).toDart) {
      final key = k.toDart;
      if (!desired.containsKey(key)) micaYjs.mapDelete(props, key);
    }
    desired.forEach((k, v) => micaYjs.mapSetJson(props, k, jsonEncode(v)));
  }

  void _applyMarks(JSObject text, List<Map<String, dynamic>> marks) {
    for (final m in marks) {
      final start = m['start'] as int;
      final end = m['end'] as int;
      if (end <= start) continue;
      final href = m['href'] as String?;
      final title = m['title'] as String?;
      final Object value = (href != null || title != null)
          ? <String, String>{'href': ?href, 'title': ?title}
          : true;
      micaYjs.textFormatJson(
        text,
        start,
        end - start,
        jsonEncode({m['type'] as String: value}),
      );
    }
  }

  /// Replace a block's whole text + marks (mirrors `set_text_and_marks`).
  void _setTextAndMarks(JSObject bm, String text, List<Map<String, dynamic>> marks) {
    final t = micaYjs.mapGet(bm, 'text');
    if (t == null || !micaYjs.isText(t)) {
      final nt = micaYjs.newText(text);
      micaYjs.mapSet(bm, 'text', nt);
      _applyMarks(nt, marks);
      return;
    }
    final text0 = t as JSObject;
    final len = micaYjs.textLength(text0);
    if (len > 0) micaYjs.textDelete(text0, 0, len);
    micaYjs.textInsert(text0, 0, text);
    _applyMarks(text0, marks);
  }

  void _insertBlock(String parentId, int index, Map<String, dynamic> block) {
    final blocks = _blocksMap();
    final id = block['id'] as String;
    final bm = micaYjs.newMap();
    micaYjs.mapSet(blocks, id, bm);
    micaYjs.mapSet(bm, 'ty', ((block['type'] as String?) ?? 'paragraph').toJS);
    final text = micaYjs.newText((block['text'] as String?) ?? '');
    micaYjs.mapSet(bm, 'text', text);
    final data = (block['data'] as Map?)?.cast<String, dynamic>() ?? const {};
    _applyMarks(text, _marksFromData(data));
    _setProps(bm, data);
    final children = micaYjs.newArray();
    micaYjs.mapSet(bm, 'children', children);
    final childIds = (block['children'] as List?)?.cast<String>() ?? const [];
    if (childIds.isNotEmpty) {
      micaYjs.arrayInsertJson(children, 0, jsonEncode(childIds));
    }
    final parent = _blockMap(parentId);
    if (parent != null) {
      final pc = _childrenArray(parent);
      if (pc != null) {
        final i = index.clamp(0, micaYjs.arrayLength(pc));
        micaYjs.arrayInsertJson(pc, i, jsonEncode([id]));
      }
    }
  }

  void _updateBlock(String id, String? kind, String? text, Map<String, dynamic>? data) {
    final bm = _blockMap(id);
    if (bm == null) return;
    if (kind != null) micaYjs.mapSet(bm, 'ty', kind.toJS);
    if (text != null) {
      final marks = data != null ? _marksFromData(data) : const <Map<String, dynamic>>[];
      _setTextAndMarks(bm, text, marks);
    }
    if (data != null) {
      _setProps(bm, data);
    }
  }

  void _deleteBlock(String id, bool bringChildren) {
    final blocks = _blocksMap();
    var childIds = const <String>[];
    final bm = _blockMap(id);
    if (bm != null) {
      final c = _childrenArray(bm);
      if (c != null) childIds = _arrayStrings(c);
    }
    final pp = _findParent(id);
    if (pp != null) {
      final pc = _childrenArray(pp.$1);
      if (pc != null) {
        micaYjs.arrayDelete(pc, pp.$2, 1);
        if (bringChildren && childIds.isNotEmpty) {
          micaYjs.arrayInsertJson(pc, pp.$2, jsonEncode(childIds));
        }
      }
    }
    micaYjs.mapDelete(blocks, id);
  }

  void _moveBlock(String id, String newParent, int index) {
    final pp = _findParent(id);
    if (pp != null) {
      final oc = _childrenArray(pp.$1);
      if (oc != null) micaYjs.arrayDelete(oc, pp.$2, 1);
    }
    final np = _blockMap(newParent);
    if (np != null) {
      final nc = _childrenArray(np);
      if (nc != null) {
        final i = index.clamp(0, micaYjs.arrayLength(nc));
        micaYjs.arrayInsertJson(nc, i, jsonEncode([id]));
      }
    }
  }
}
