// Manual/native-UIA probe. Run only with an isolated prefs profile beneath output.
import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:tray_manager/tray_manager.dart';
import 'package:window_manager/window_manager.dart';
import 'package:mica_flutter/editor/pick_file_stub.dart';
import 'package:mica_flutter/prefs.dart';
import 'package:mica_flutter/window_setup.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  final path = Platform.environment['MICA_PLUGIN_PROBE_OUTPUT'];
  if (path == null || path.isEmpty) {
    throw StateError('missing output directory');
  }
  final output = Directory(path)..createSync(recursive: true);
  if (!configDir().startsWith(output.absolute.path)) {
    throw StateError('APPDATA/XDG_CONFIG_HOME must be isolated beneath output');
  }
  Future<void> record(
    String phase, [
    Map<String, dynamic> extra = const {},
  ]) async {
    final b = await windowManager.getBounds();
    final data = <String, dynamic>{
      'phase': phase,
      'dpr': WidgetsBinding
          .instance
          .platformDispatcher
          .views
          .first
          .devicePixelRatio,
      'pid': pid,
      'bounds': [b.left, b.top, b.width, b.height],
      'visible': await windowManager.isVisible(),
      'saved': [
        loadPref('windowX'),
        loadPref('windowY'),
        loadPref('windowWidth'),
        loadPref('windowHeight'),
      ],
      'maximized': await windowManager.isMaximized(),
      ...extra,
    };
    final tmp = File('${output.path}/status.tmp');
    tmp.writeAsStringSync(jsonEncode(data));
    final status = File('${output.path}/status.json');
    if (status.existsSync()) status.deleteSync();
    tmp.renameSync(status.path);
  }

  try {
    saveCloseBehavior(kCloseQuit);
    await initDesktopWindow();
    await windowManager.setTitle('Mica isolated plugin QA');
    runApp(
      MaterialApp(
        navigatorKey: appNavigatorKey,
        home: const Scaffold(
          body: Center(child: Text('Mica isolated plugin QA')),
        ),
      ),
    );
    await WidgetsBinding.instance.endOfFrame;
    await record('ready');
    appExitFlush = () async {
      File(
        '${output.path}/flush.json',
      ).writeAsStringSync(jsonEncode({'flushed': true, 'pid': pid}));
    };
    while (true) {
      final command = File('${output.path}/command.txt');
      if (!command.existsSync()) {
        await Future<void>.delayed(const Duration(milliseconds: 50));
        continue;
      }
      final action = command.readAsStringSync().trim();
      command.deleteSync();
      switch (action) {
        case 'inspect':
          final trayBounds = trayIsSupported
              ? await trayManager.getBounds()
              : null;
          await record('inspected', {
            'trayBounds': trayBounds == null
                ? null
                : [
                    trayBounds.left,
                    trayBounds.top,
                    trayBounds.width,
                    trayBounds.height,
                  ],
          });
        case 'tray':
          if (!await ensureTray()) throw StateError('tray registration failed');
          final r = await trayManager.getBounds();
          await applyCloseBehavior(kCloseTray);
          await record('tray-hidden', {
            'trayBounds': r == null ? null : [r.left, r.top, r.width, r.height],
          });
        case 'pick':
          await record('picker-opening');
          final file = await pickTextFile();
          if (file == null) {
            throw StateError('native file selection was cancelled');
          }
          final expected = File(
            '${output.path}/fixture/note.md',
          ).readAsStringSync();
          if (file.name != 'note.md' || file.text != expected) {
            throw StateError('native selected bytes differ');
          }
          await record('picker-read', {
            'fileName': file.name,
            'byteCount': utf8.encode(file.text).length,
          });
        case 'close':
          await windowManager.close();
        default:
          throw StateError('unknown probe command');
      }
    }
  } catch (error, stack) {
    File('${output.path}/failure.txt').writeAsStringSync('$error\n$stack');
    exit(1);
  }
}
