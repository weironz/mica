// Why the page-title field does NOT simply mirror the document's title.
//
// The title lives in the document (P2; `views.name` is its projection), the
// field saves on a 700ms debounce, and the save comes back as a CRDT update. So
// the field and the document are routinely out of step in both directions, and
// the version of this code that just assigned the document's title into the
// field ATE INPUT: type a title, the debounce commits the first half, and that
// half-title echoes back mid-word and replaces everything in the field.
//
// Reported as "editing the title auto-refreshes and part of what I typed is
// lost". These cases pin which side wins in each state — the whole fix is that
// an edit in flight outranks an incoming title on the SAME page.
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/ui/rename.dart';

void main() {
  group('titleFieldSync', () {
    test(
      'normalized acknowledgement settles without overwriting whitespace',
      () {
        expect(
          titleFieldSync(
            field: ' 新标题 ',
            documentTitle: '新标题',
            pageChanged: false,
            editPending: true,
          ),
          TitleFieldSync.settled,
        );
        expect(
          titleFieldSync(
            field: ' 新标题 ',
            documentTitle: '新标题',
            pageChanged: true,
            editPending: true,
          ),
          TitleFieldSync.takeDocument,
        );
      },
    );
    // THE BUG. The field holds "长标题的全部", the echo of the debounced save
    // carries only "长标题的" — assigning it would delete the rest.
    test('an echo half a title behind does not overwrite the field', () {
      expect(
        titleFieldSync(
          field: '长标题的全部',
          documentTitle: '长标题的',
          pageChanged: false,
          editPending: true,
        ),
        TitleFieldSync.keepField,
      );
    });

    // The echo that finally catches up: nothing to write, and the latch must
    // open so later renames are not refused for the rest of the page's life.
    test('an echo that agrees with the field settles it', () {
      expect(
        titleFieldSync(
          field: '长标题的全部',
          documentTitle: '长标题的全部',
          pageChanged: false,
          editPending: true,
        ),
        TitleFieldSync.settled,
      );
    });

    // Equal text is `settled` even with nothing pending: assigning anyway
    // resets the selection, which the web engine renders as select-all (one
    // backspace would then wipe the name) and which drops a live IME
    // composition.
    test('equal text is never written back, pending or not', () {
      expect(
        titleFieldSync(
          field: '同一个名字',
          documentTitle: '同一个名字',
          pageChanged: false,
          editPending: false,
        ),
        TitleFieldSync.settled,
      );
    });

    // The other half of the feature this guard sits inside: a rename made on
    // another device (or via the sidebar row / breadcrumb) still lands, because
    // the field has nothing newer to lose.
    test('a rename from elsewhere takes the field when nothing is pending', () {
      expect(
        titleFieldSync(
          field: '旧名字',
          documentTitle: '别处改的新名字',
          pageChanged: false,
          editPending: false,
        ),
        TitleFieldSync.takeDocument,
      );
    });

    // A pending edit belongs to the page that is LEAVING — its own debounce
    // saves it. Holding the field here would show the previous page's title on
    // the new page, which is worse than anything the latch protects.
    test('opening another page always takes the document, even mid-edit', () {
      expect(
        titleFieldSync(
          field: '上一页刚打的字',
          documentTitle: '这一页的标题',
          pageChanged: true,
          editPending: true,
        ),
        TitleFieldSync.takeDocument,
      );
    });

    // An untitled page is shown as an EMPTY field (the placeholder), so '' is a
    // real value on both sides, not a missing one.
    test('the untitled placeholder is a value like any other', () {
      // Field cleared by the user, document still named: nothing pending means
      // the name comes back rather than the field sitting blank forever.
      expect(
        titleFieldSync(
          field: '',
          documentTitle: '有名字',
          pageChanged: false,
          editPending: false,
        ),
        TitleFieldSync.takeDocument,
      );
      // Same state mid-edit: the user is clearing it on purpose, let them.
      expect(
        titleFieldSync(
          field: '',
          documentTitle: '有名字',
          pageChanged: false,
          editPending: true,
        ),
        TitleFieldSync.keepField,
      );
      // First keystroke on a fresh page: the document still says untitled ('').
      expect(
        titleFieldSync(
          field: '第',
          documentTitle: '',
          pageChanged: false,
          editPending: true,
        ),
        TitleFieldSync.keepField,
      );
    });
  });
}
