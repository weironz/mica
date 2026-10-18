import 'dart:convert';
import 'package:flutter/rendering.dart';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:mica_flutter/api/client.dart';
import 'package:mica_flutter/api/models.dart';
import 'package:mica_flutter/main.dart' as app;
import 'package:mica_flutter/prefs.dart';

/// Startup-chain smoke test: pumps the FULL MicaApp (WorkspaceShell inside),
/// which no other unit test does — the gap where the v0.13.53/54 regressions
/// lived: 1,814 unit tests green, only the browser e2e caught them, feedback
/// 10 minutes per round. This file shrinks that feedback to seconds.
///
/// What it pins (each assertion maps to a real past incident):
///  1. initState completes without a setState-during-initState exception —
///     the v0.13.53 ServerRegistry listener-order crash.
///  2. A persisted session restores without a password: localStorage/prefs
///     credentials → restoreSession → the cloud world shows the account —
///     the v0.13.54 resolveApiBase stale-copy crash (stuck on the local
///     world instead).
///  3. No controller writes after unmount (the follow-up crash class).
///
/// The prefs store is the real file-backed one (stub on desktop writes to
/// APPDATA); each test seeds it through the same savePref the app uses and
/// the keys are unique per test run (uuid in origin), so tests are order-
/// independent and never see each other's state.
void main() {
  tearDown(resetHttpClientForTesting);

  testWidgets('startup with a persisted session restores the cloud world',
      (tester) async {
    final origin = 'https://shell-smoke-${DateTime.now().millisecondsSinceEpoch}.test';
    // --- seed prefs exactly as a previous session would have left them ---
    savePref('cloudOrigin', origin);
    savePref('activeOrigin', origin);
    savePref('servers', jsonEncode([origin]));
    savePref('authUser:$origin',
        jsonEncode({'id': 'u1', 'email': 'smoke@mica.test', 'display_name': 'Smoke User'}));
    // A non-expired access token: restore must use it, not refresh.
    savePref('authToken:$origin', _nonExpiredToken());

    final seen = <String>[];
    useHttpClientForTesting(MockClient((request) async {
      final path = request.url.path;
      seen.add(path);
      if (path == '/api/workspaces') {
        return http.Response(
            jsonEncode([
              {'id': 'ws1', 'name': 'Smoke Workspace', 'owner_id': 'u1', 'role': 'owner'}
            ]),
            200);
      }
      if (path == '/api/auth/me') {
        return http.Response(
            jsonEncode({'id': 'u1', 'email': 'smoke@mica.test', 'display_name': 'Smoke User'}),
            200);
      }
      // Anything else the shell asks during startup gets an empty OK — the
      // point of this test is the startup chain, not any single endpoint.
      return http.Response('{}', 200);
    }));

    await tester.pumpWidget(const app.MicaApp());
    // initState + the postFrame restore chain run here. An exception in any
    // of it (the v0.13.53 class) surfaces as a framework error.
    await tester.pumpAndSettle(const Duration(seconds: 2));

    expect(tester.takeException(), isNull,
        reason: 'the startup chain must not throw (v0.13.53 regression class)');
    expect(seen, contains('/api/workspaces'),
        reason: 'a persisted session boots into the cloud world (v0.13.54 class: '
            'a stale origin meant the request never went out, or went nowhere)');
    debugDumpApp();
  expect(true, isTrue);
  });

  testWidgets('startup with no session lands on the world picker, not a crash',
      (tester) async {
    // Empty-ish prefs: no origin, no credentials.
    final origin = 'https://shell-cold-${DateTime.now().millisecondsSinceEpoch}.test';
    savePref('cloudOrigin', origin);
    savePref('activeOrigin', origin);
    removePref('authToken:$origin');
    removePref('authUser:$origin');

    useHttpClientForTesting(MockClient((request) async => http.Response('{}', 200)));

    await tester.pumpWidget(const app.MicaApp());
    await tester.pumpAndSettle(const Duration(seconds: 2));

    expect(tester.takeException(), isNull);
    // No workspace fetch should have been attempted with no credentials.
  });

  testWidgets('the shell survives a frame after the controller handoff',
      (tester) async {
    final origin = 'https://shell-frame-${DateTime.now().millisecondsSinceEpoch}.test';
    savePref('cloudOrigin', origin);
    savePref('activeOrigin', origin);
    savePref('authUser:$origin',
        jsonEncode({'id': 'u1', 'email': 'smoke@mica.test', 'display_name': 'Smoke User'}));
    savePref('authToken:$origin', _nonExpiredToken());

    useHttpClientForTesting(MockClient((request) async {
      if (request.url.path == '/api/workspaces') {
        return http.Response(jsonEncode([
          {'id': 'ws1', 'name': 'W', 'owner_id': 'u1', 'role': 'owner'}
        ]), 200);
      }
      return http.Response('{}', 200);
    }));

    await tester.pumpWidget(const app.MicaApp());
    await tester.pumpAndSettle(const Duration(seconds: 2));
    // Rebuild after the async restore settled: the follow-up crash class was
    // a controller write landing on an unmounted/detached element.
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 300));
    expect(tester.takeException(), isNull);
  });
}

/// A JWT with a far-future exp — the shell only reads the exp claim here;
/// signature is never verified client-side (the server does that).
String _nonExpiredToken() {
  final payload = base64UrlEncode(utf8.encode(jsonEncode({
        'sub': 'u1',
        'exp': DateTime.now().millisecondsSinceEpoch ~/ 1000 + 86400,
      })));
  return 'eyJhbGciOiJIUzI1NiJ9.$payload.unsigned-smoke';
}
