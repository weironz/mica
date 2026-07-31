import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart' show AuthFormValue, AuthMode;
import 'package:mica_flutter/ui/auth_form.dart';

/// This form existed twice — web's entry screen and desktop's sign-in dialog —
/// so a rule added to one silently missed the other. Now there is one, and these
/// pin the rules that were easiest to get wrong in only one copy.
void main() {
  const strings = AuthFormStrings(
    title: '账户',
    login: '登录',
    register: '注册',
    email: '邮箱',
    displayName: '显示名称',
    password: '密码',
    forgotPassword: '忘记密码?',
  );

  Future<List<(AuthMode, AuthFormValue)>> pump(
    WidgetTester tester, {
    Future<void> Function(String email)? onForgot,
    String? actionLabelOverride,
    String? note,
    String? errorText,
    bool isBusy = false,
    bool allowRegister = true,
  }) async {
    final submitted = <(AuthMode, AuthFormValue)>[];
    await tester.binding.setSurfaceSize(const Size(500, 900));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: AuthFormCard(
            strings: strings,
            isBusy: isBusy,
            note: note,
            errorText: errorText,
            actionLabelOverride: actionLabelOverride,
            onForgotPassword: onForgot,
            allowRegister: allowRegister,
            onSubmit: (m, f) async => submitted.add((m, f)),
          ),
        ),
      ),
    );
    return submitted;
  }

  // Registration being CLOSED is a server fact the form has to be told. It used
  // to be told nothing at all, so a locked-down instance still advertised
  // 「注册」 and answered a filled-in form with a bare 403.
  testWidgets('a closed instance shows no way to register', (tester) async {
    await pump(tester, allowRegister: false);
    expect(find.text('注册'), findsNothing);
    // ...and what remains is a usable sign-in form, not a stump.
    expect(find.text('邮箱'), findsOneWidget);
    expect(find.text('密码'), findsOneWidget);
    expect(find.text('登录'), findsWidgets);
  });

  testWidgets('an open instance still offers both modes', (tester) async {
    // The other direction matters just as much: defaulting to hidden would lock
    // a brand-new instance out of creating its first account.
    await pump(tester);
    expect(find.text('注册'), findsWidgets);
  });

  testWidgets('a form left in register mode submits a LOGIN once the probe '
      'reports registration closed', (tester) async {
    // The real hazard, and the reason the mode is derived instead of synced:
    // the flag arrives ASYNCHRONOUSLY, from a health probe that lands after the
    // form is already on screen. Someone can be sitting in register mode when
    // the answer comes back "closed". Pumping straight to `allowRegister: false`
    // would prove nothing — the form starts in login mode anyway — so this
    // enters register mode FIRST and then flips the flag under it, keeping the
    // same State.
    final submitted = <(AuthMode, AuthFormValue)>[];
    await tester.binding.setSurfaceSize(const Size(500, 900));
    addTearDown(() => tester.binding.setSurfaceSize(null));

    Widget build({required bool allowRegister}) => MaterialApp(
      home: Scaffold(
        body: AuthFormCard(
          strings: strings,
          allowRegister: allowRegister,
          onSubmit: (m, f) async => submitted.add((m, f)),
        ),
      ),
    );

    await tester.pumpWidget(build(allowRegister: true));
    await tester.tap(find.text('注册').last);
    await tester.pumpAndSettle();
    expect(find.text('显示名称'), findsOneWidget, reason: 'now in register mode');

    // The probe comes back: this instance is closed.
    await tester.pumpWidget(build(allowRegister: false));
    await tester.pumpAndSettle();
    expect(find.text('显示名称'), findsNothing, reason: 'fell back to sign-in');

    await tester.enterText(find.byType(TextField).first, 'a@example.com');
    await tester.enterText(find.byType(TextField).last, 'password123');
    await tester.tap(find.byType(FilledButton));
    await tester.pump();
    expect(submitted, hasLength(1));
    expect(submitted.single.$1, AuthMode.login);
  });

  testWidgets('logging in asks for email + password, not a display name', (
    tester,
  ) async {
    // A name is not part of the credential; asking for it to log in implies it
    // is.
    await pump(tester);
    expect(find.text('邮箱'), findsOneWidget);
    expect(find.text('密码'), findsOneWidget);
    expect(find.text('显示名称'), findsNothing);
  });

  testWidgets('registering adds the display name field', (tester) async {
    await pump(tester);
    await tester.tap(find.text('注册'));
    await tester.pumpAndSettle();
    expect(find.text('显示名称'), findsOneWidget);
  });

  testWidgets('submitting reports the mode and the trimmed values', (
    tester,
  ) async {
    final submitted = await pump(tester);
    await tester.enterText(find.byType(TextField).first, '  a@b.dev  ');
    await tester.enterText(find.byType(TextField).last, 'secret123');
    await tester.tap(find.widgetWithText(FilledButton, '登录'));
    await tester.pumpAndSettle();

    expect(submitted.length, 1);
    expect(submitted.single.$1, AuthMode.login);
    // Trimmed: a pasted address with a stray space must not 401.
    expect(submitted.single.$2.email, 'a@b.dev');
    expect(submitted.single.$2.password, 'secret123');
  });

  testWidgets('forgot-password shows only while logging in', (tester) async {
    // Someone registering cannot have forgotten a password yet.
    await pump(tester, onForgot: (_) async {});
    expect(find.text('忘记密码?'), findsOneWidget);
    await tester.tap(find.text('注册'));
    await tester.pumpAndSettle();
    expect(find.text('忘记密码?'), findsNothing);
  });

  testWidgets('no forgot-password callback removes the link, not disables it', (
    tester,
  ) async {
    await pump(tester);
    expect(find.text('忘记密码?'), findsNothing);
  });

  testWidgets('it hands up whatever email is typed, empty included', (
    tester,
  ) async {
    // The "you didn't type one" message is copy and belongs to the caller; the
    // form must not swallow the tap and say nothing.
    final seen = <String>[];
    await pump(tester, onForgot: (e) async => seen.add(e));
    await tester.tap(find.text('忘记密码?'));
    await tester.pumpAndSettle();
    expect(seen, ['']);
  });

  testWidgets('the migrate flow renames the action and explains itself', (
    tester,
  ) async {
    await pump(tester, actionLabelOverride: '迁移', note: '把「笔记」搬到云端');
    expect(find.widgetWithText(FilledButton, '迁移'), findsOneWidget);
    expect(find.text('把「笔记」搬到云端'), findsOneWidget);
  });

  testWidgets('Enter submits from the EMAIL field, not just the password', (
    tester,
  ) async {
    // The email box has focus when the screen opens, so this is the field people
    // actually press Enter in. It used to do nothing there.
    final submitted = await pump(tester);
    await tester.enterText(find.byType(TextField).first, 'a@b.dev');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();
    expect(submitted, hasLength(1));
  });

  testWidgets('Enter submits from the password field too', (tester) async {
    final submitted = await pump(tester);
    await tester.enterText(find.byType(TextField).first, 'a@b.dev');
    await tester.enterText(find.byType(TextField).last, 'secret123');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();
    expect(submitted, hasLength(1));
    expect(submitted.single.$2.password, 'secret123');
  });

  testWidgets('a failed attempt is shown BY THE FORM, not left invisible', (
    tester,
  ) async {
    // The app parks failures in a shell-level banner, and the sign-in screen is
    // not the shell: a wrong password used to produce no visible reaction at all.
    await pump(tester, errorText: '邮箱或密码不正确');
    expect(find.text('邮箱或密码不正确'), findsOneWidget);
    expect(find.byIcon(Icons.error_outline), findsOneWidget);
  });

  testWidgets('no error means no empty red row', (tester) async {
    await pump(tester);
    expect(find.byIcon(Icons.error_outline), findsNothing);
  });

  testWidgets('busy disables submit instead of queueing a second sign-in', (
    tester,
  ) async {
    await pump(tester, isBusy: true);
    final button = tester.widget<FilledButton>(find.byType(FilledButton));
    expect(button.onPressed, isNull);
  });
}
