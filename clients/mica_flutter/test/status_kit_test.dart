import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/ui/status_kit.dart';

// What these pin is the design's one rule for status surfaces (`19 空状态与故障
// 态`): say what happened, give one next step. Concretely — the copy the caller
// passes is the copy that shows; an action that exists reaches the host; an
// action that doesn't exist renders nothing (no dead control); and a
// self-healing toast never grows a button.

Widget host(Widget child) => MaterialApp(home: Scaffold(body: child));

void main() {
  group('MicaEmptyState', () {
    testWidgets('shows title + body and its action reaches the host', (
      tester,
    ) async {
      var taps = 0;
      await tester.pumpWidget(
        host(
          MicaEmptyState(
            icon: Icons.description_outlined,
            title: 'No pages yet',
            body: 'Create a page to start writing.',
            actionLabel: 'New page',
            onAction: () => taps++,
          ),
        ),
      );

      expect(find.text('No pages yet'), findsOneWidget);
      expect(find.text('Create a page to start writing.'), findsOneWidget);
      expect(find.byKey(MicaEmptyState.actionKey), findsOneWidget);

      await tester.tap(find.byKey(MicaEmptyState.actionKey));
      expect(taps, 1);
    });

    testWidgets('with no action, renders no button at all', (tester) async {
      await tester.pumpWidget(
        host(
          const MicaEmptyState(
            icon: Icons.search,
            title: 'Nothing found',
            body: 'Try another word, or check the workspace you searched in.',
          ),
        ),
      );

      expect(find.text('Nothing found'), findsOneWidget);
      expect(find.byKey(MicaEmptyState.actionKey), findsNothing);
    });
  });

  group('MicaFailureCard', () {
    testWidgets('both actions reach the host', (tester) async {
      var primary = 0;
      var secondary = 0;
      await tester.pumpWidget(
        host(
          MicaFailureCard(
            severity: MicaFailureSeverity.warning,
            title: 'Could not open this page',
            body: 'The local copy looks damaged. Your other pages are fine.',
            primaryLabel: 'Restore checkpoint',
            onPrimary: () => primary++,
            secondaryLabel: 'Version history',
            onSecondary: () => secondary++,
          ),
        ),
      );

      await tester.tap(find.byKey(MicaFailureCard.primaryKey));
      await tester.tap(find.byKey(MicaFailureCard.secondaryKey));
      expect(primary, 1);
      expect(secondary, 1);
    });

    testWidgets('no secondary label → no secondary button', (tester) async {
      await tester.pumpWidget(
        host(
          MicaFailureCard(
            title: 'Update aborted',
            body: 'The download failed its integrity check and was deleted.',
            primaryLabel: 'Retry',
            onPrimary: () {},
          ),
        ),
      );

      expect(find.byKey(MicaFailureCard.primaryKey), findsOneWidget);
      expect(find.byKey(MicaFailureCard.secondaryKey), findsNothing);
    });

    testWidgets('severity switches the tile tint and glyph colour', (
      tester,
    ) async {
      Future<void> pump(MicaFailureSeverity severity) => tester.pumpWidget(
        host(
          MicaFailureCard(
            severity: severity,
            title: 'Title',
            body: 'Body',
            primaryLabel: 'Retry',
            onPrimary: () {},
          ),
        ),
      );

      Color tileColor() {
        final tile = tester.widget<Container>(
          find.byKey(MicaFailureCard.tileKey),
        );
        return (tile.decoration! as BoxDecoration).color!;
      }

      Color glyphColor() => tester
          .widget<Icon>(
            find.descendant(
              of: find.byKey(MicaFailureCard.tileKey),
              matching: find.byType(Icon),
            ),
          )
          .color!;

      await pump(MicaFailureSeverity.warning);
      expect(tileColor(), const Color(0xFFFEF3C7));
      expect(glyphColor(), const Color(0xFFB45309));

      await pump(MicaFailureSeverity.error);
      expect(tileColor(), const Color(0xFFFEF2F2));
      expect(glyphColor(), const Color(0xFFDC2626));
    });
  });

  group('MicaStatusToast', () {
    testWidgets('self-healing carries no action', (tester) async {
      await tester.pumpWidget(
        host(
          const MicaStatusToast(
            kind: MicaToastKind.selfHealing,
            icon: Icons.cloud_off_outlined,
            message: 'Offline · edits are saved locally and will sync later.',
          ),
        ),
      );

      expect(
        find.text('Offline · edits are saved locally and will sync later.'),
        findsOneWidget,
      );
      expect(find.byKey(MicaStatusToast.actionKey), findsNothing);
    });

    testWidgets('needs-decision carries one action that reaches the host', (
      tester,
    ) async {
      var taps = 0;
      await tester.pumpWidget(
        host(
          MicaStatusToast(
            kind: MicaToastKind.needsDecision,
            icon: Icons.error_outline,
            message: 'Your session expired. Sign in again to keep syncing.',
            actionLabel: 'Sign in',
            onAction: () => taps++,
          ),
        ),
      );

      expect(find.byKey(MicaStatusToast.actionKey), findsOneWidget);
      await tester.tap(find.byKey(MicaStatusToast.actionKey));
      expect(taps, 1);
    });

    testWidgets('a self-healing toast with an action is rejected', (
      tester,
    ) async {
      expect(
        () => MicaStatusToast(
          kind: MicaToastKind.selfHealing,
          icon: Icons.cloud_off_outlined,
          message: 'Offline',
          actionLabel: 'Retry',
          onAction: () {},
        ),
        throwsAssertionError,
      );
    });
  });

  group('MicaStatusToastHost', () {
    testWidgets('stacks toasts bottom-right, 12px apart', (tester) async {
      await tester.pumpWidget(
        host(
          const MicaStatusToastHost(
            toasts: [
              MicaStatusToast(
                kind: MicaToastKind.selfHealing,
                icon: Icons.cloud_off_outlined,
                message: 'Offline',
                key: Key('first'),
              ),
              MicaStatusToast(
                kind: MicaToastKind.selfHealing,
                icon: Icons.sync,
                message: 'Reconnecting…',
                key: Key('second'),
              ),
            ],
          ),
        ),
      );

      final screen = tester.getRect(find.byType(MicaStatusToastHost));
      final first = tester.getRect(find.byKey(const Key('first')));
      final second = tester.getRect(find.byKey(const Key('second')));

      // Order preserved top-to-bottom, one 12px gap between them.
      expect(second.top - first.bottom, 12);
      // Pinned to the bottom-right corner (20px default inset).
      expect(first.right, second.right);
      expect(screen.right - second.right, 20);
      expect(screen.bottom - second.bottom, 20);
    });

    testWidgets('no toasts → nothing occupying the corner', (tester) async {
      await tester.pumpWidget(host(const MicaStatusToastHost(toasts: [])));
      expect(find.byType(MicaStatusToast), findsNothing);
    });
  });
}
