// Where a page's title comes from on the CLIENT, and why it is not just
// `view.name`.
//
// Since P2 the document is the authority (`root.data['title']`) and the
// `views.name` column is its projection — see `docs/page-title-plan.md`. There
// is deliberately NO backfill migration, so both halves are live at once: a page
// renamed since P2 carries a title in its document, every other page still
// answers from the column. `pageTitle` is the one rule that reads both, and it
// has to match the server's (`mica_markdown::document_title`, then `views.name`)
// exactly — a client that disagreed would show one name and export another.
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/main.dart';

DocumentBootstrap bootstrapWith({
  required String viewName,
  Map<String, dynamic>? rootData,
}) {
  return DocumentBootstrap.fromJson({
    'document': {
      'id': '11111111-1111-4111-8111-111111111111',
      'workspace_id': '22222222-2222-4222-8222-222222222222',
      'root_block_id': 'root',
      'current_seq': 0,
      'created_by': '33333333-3333-4333-8333-333333333333',
      'created_at': '2026-08-27T00:00:00Z',
      'updated_at': '2026-08-27T00:00:00Z',
    },
    'view': {
      'id': '44444444-4444-4444-8444-444444444444',
      'parent_view_id': null,
      'object_id': '11111111-1111-4111-8111-111111111111',
      'object_type': 'document',
      'name': viewName,
      'position': '0',
    },
    'snapshot': {
      'version_seq': 0,
      'schema_version': 1,
      'payload': {
        'schema_version': 1,
        'root_block_id': 'root',
        'blocks': [
          {
            'id': 'root',
            'type': 'page',
            'text': '',
            if (rootData != null) 'data': rootData,
            'children': ['b1'],
          },
          {'id': 'b1', 'type': 'paragraph', 'text': '正文。', 'children': []},
        ],
      },
    },
  });
}

void main() {
  group('DocumentBootstrap.pageTitle', () {
    test('a document that carries a title wins over the column', () {
      final b = bootstrapWith(
        viewName: '列里的旧名字',
        rootData: {'title': '文档里的新名字'},
      );
      expect(b.pageTitle, '文档里的新名字');
    });

    // The ordinary case, not an edge one: no backfill means most pages have no
    // title in their document at all. If this regressed, every page never
    // renamed since P2 would lose its name in the UI.
    test('a document with no title falls back to the column', () {
      expect(bootstrapWith(viewName: '只有列有名字').pageTitle, '只有列有名字');
      expect(
        bootstrapWith(viewName: '只有列有名字', rootData: {}).pageTitle,
        '只有列有名字',
      );
    });

    // Front matter lives on the same root block. Reading one must not depend on
    // or disturb the other.
    test('a title and front matter coexist on the root block', () {
      final b = bootstrapWith(
        viewName: '列名',
        rootData: {'title': '文档标题', 'front_matter': 'tags: [a]'},
      );
      expect(b.pageTitle, '文档标题');
      expect(b.rootFrontMatter, 'tags: [a]');
    });

    // Matches the server's rule (`document_title` filters blank after trim).
    // Diverging here would show an empty title while the export showed a name.
    test('a blank title falls back rather than rendering as nothing', () {
      expect(
        bootstrapWith(viewName: '列名', rootData: {'title': ''}).pageTitle,
        '列名',
      );
      expect(
        bootstrapWith(viewName: '列名', rootData: {'title': '   '}).pageTitle,
        '列名',
      );
    });

    test('a non-string title is ignored', () {
      // Nothing writes this today; the point is that a surprising value falls
      // back to a real name instead of throwing inside a title bar.
      expect(
        bootstrapWith(viewName: '列名', rootData: {'title': 42}).pageTitle,
        '列名',
      );
    });

    test('surrounding whitespace is trimmed, as the server trims it', () {
      expect(
        bootstrapWith(viewName: '列名', rootData: {'title': '  有空格  '}).pageTitle,
        '有空格',
      );
    });
  });
}
