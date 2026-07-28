import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/ui/destructive_confirm.dart';
import 'package:mica_flutter/ui/theme_tokens.dart';

/// The behaviour under test is "the irreversible thing does not happen unless
/// the user says yes". Purging from the recycle bin and revoking an API token
/// both shipped without this gate: one tap on an icon button destroyed a whole
/// page subtree (views + backing documents, cascading version history and
/// comments) or 401'd every script holding a token, with nothing to undo it.
void main() {
  /// Pumps a button that runs the gate and records what it answered.
  Future<List<bool>> pumpGate(WidgetTester tester) async {
    final answers = <bool>[];
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: Builder(
            builder: (context) => TextButton(
              onPressed: () async {
                answers.add(
                  await showDestructiveConfirm(
                    context,
                    title: '永久删除「留存」?',
                    body: '它和它的所有子页面都会被立即删除,无法恢复。',
                    confirmLabel: '永久删除',
                    cancelLabel: '取消',
                  ),
                );
              },
              child: const Text('open'),
            ),
          ),
        ),
      ),
    );
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();
    return answers;
  }

  testWidgets('shows the title, the consequence and both actions', (
    tester,
  ) async {
    await pumpGate(tester);

    expect(find.text('永久删除「留存」?'), findsOneWidget);
    // The body must state the consequence, not merely re-ask the question —
    // that is what makes the gate worth stopping for.
    expect(find.text('它和它的所有子页面都会被立即删除,无法恢复。'), findsOneWidget);
    expect(find.text('永久删除'), findsOneWidget);
    expect(find.text('取消'), findsOneWidget);
  });

  testWidgets('confirming answers true', (tester) async {
    final answers = await pumpGate(tester);

    await tester.tap(find.text('永久删除'));
    await tester.pumpAndSettle();

    expect(answers, [true]);
  });

  testWidgets('cancelling answers false', (tester) async {
    final answers = await pumpGate(tester);

    await tester.tap(find.text('取消'));
    await tester.pumpAndSettle();

    expect(answers, [false]);
  });

  testWidgets('dismissing without choosing answers false, never true', (
    tester,
  ) async {
    final answers = await pumpGate(tester);

    // A barrier tap pops with null. Null has to read as "no": if it ever fell
    // through as "yes", losing focus would delete your pages.
    await tester.tapAt(const Offset(10, 10));
    await tester.pumpAndSettle();

    expect(answers, [false]);
  });

  testWidgets('the confirm button is the destructive red, the cancel is not', (
    tester,
  ) async {
    await pumpGate(tester);

    final confirm = tester.widget<FilledButton>(
      find.ancestor(of: find.text('永久删除'), matching: find.byType(FilledButton)),
    );
    // The ROLE, not a hex: which red `status.danger` is depends on the palette,
    // and the gate is wrapped in no MicaTheme here, so it resolves to light.
    expect(
      confirm.style?.backgroundColor?.resolve(const <WidgetState>{}),
      MicaTokens.light.status.danger,
    );
    // Cancel stays a plain TextButton: only one of the two should look like the
    // thing you meant to press.
    expect(
      find.ancestor(of: find.text('取消'), matching: find.byType(TextButton)),
      findsOneWidget,
    );
  });

  testWidgets('a recoverable action still asks, but without the red', (
    tester,
  ) async {
    // Clearing the re-downloadable cache costs offline access until you
    // reconnect — nothing more. Spending the same red on it would leave the red
    // meaning "some action", and the gate in front of the truly irreversible
    // ones would stop reading as a warning at all.
    var answered = false;
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: Builder(
            builder: (context) => TextButton(
              onPressed: () async {
                answered = await showDestructiveConfirm(
                  context,
                  title: '清理云端镜像?',
                  body: '重新联网会再次缓存。本地独有的内容不动。',
                  confirmLabel: '清理',
                  cancelLabel: '取消',
                  destructive: false,
                );
              },
              child: const Text('open'),
            ),
          ),
        ),
      ),
    );
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();

    final confirm = tester.widget<FilledButton>(
      find.ancestor(of: find.text('清理'), matching: find.byType(FilledButton)),
    );
    // `isNot(kDestructiveRed)` would pass here no matter what — `kDestructiveRed`
    // is a function now, and a Color is never equal to a closure. Name the value.
    expect(
      confirm.style?.backgroundColor?.resolve(const <WidgetState>{}),
      isNot(MicaTokens.light.status.danger),
    );

    // Still a gate: the action must not run on its own.
    expect(answered, isFalse);
    await tester.tap(find.text('清理'));
    await tester.pumpAndSettle();
    expect(answered, isTrue);
  });
}
