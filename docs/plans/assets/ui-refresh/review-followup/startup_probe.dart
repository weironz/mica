// Temporary Release measurement overlay; never included in the product.
import 'dart:convert';
import 'dart:io';
import 'package:flutter/widgets.dart';

final startupClock = Stopwatch();
final stages = <String, int>{};

void begin() => startupClock.start();

Future<T> measure<T>(String name, Future<T> Function() action) async {
  final clock = Stopwatch()..start();
  try {
    return await action();
  } finally {
    stages[name] = clock.elapsedMicroseconds;
  }
}

void firstFrameReceipt() {
  WidgetsBinding.instance.waitUntilFirstFrameRasterized.then((_) {
    final micros = startupClock.elapsedMicroseconds;
    final path = Platform.environment['MICA_STARTUP_TIMING_FILE'];
    if (path == null) return;
    File(path).writeAsStringSync(jsonEncode({
      'pid': pid,
      'mainToFirstRasterUs': micros,
      'stagesUs': stages,
      'dpr': WidgetsBinding.instance.platformDispatcher.views.first
          .devicePixelRatio,
    }));
  });
}
