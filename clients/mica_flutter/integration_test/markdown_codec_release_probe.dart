// Standalone release entrypoint, launched with isolated APPDATA by the probe
// driver. Calls the actual application main; no production runner bypass.
import 'dart:convert';
import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:mica_flutter/main.dart' as app;
import 'package:mica_flutter/diagnostics.dart';
import 'package:mica_flutter/markdown/markdown_codec.dart';
import '../test/support/native_build.dart';
import '../test/support/markdown_codec_bench.dart';

void main() {
  final output = Platform.environment['MICA_CODEC_PROBE_RESULT'];
  if (output == null || output.isEmpty) {
    throw StateError('Probe output path is required');
  }
  final expectedUnavailable =
      Platform.environment['MICA_CODEC_PROBE_UNAVAILABLE'] == '1';
  final priorPrint = debugPrint;
  var recorded = false;
  debugPrint = (String? message, {int? wrapWidth}) {
    priorPrint(message, wrapWidth: wrapWidth);
    final match = RegExp(
      r'^markdown codec ready in (\d+) ms$',
    ).firstMatch(message ?? '');
    if (match == null || recorded) return;
    recorded = true;
    final initializationMs = int.parse(match[1]!);
    WidgetsBinding.instance.addPostFrameCallback((_) async {
      final report = <String, Object?>{
        'mode': 'Windows release',
        'init_ms': initializationMs,
        'unavailable': markdownCodec is UnavailableMarkdownCodec,
        'first_frame': true,
      };
      try {
        if (!expectedUnavailable) {
          await initCheckedNativeRust();
          report['fresh_native'] = true;
          report['engine'] = markdownCodec.engineInfo.toString();
          report['inline'] = markdownCodec.parseInline('**release**').text;
          if (Platform.environment['MICA_CODEC_PROBE_BENCH'] == '1') {
            final bench = measureMarkdownCodec(markdownCodec);
            report['bench'] = bench;
            report['bench_passes'] = bench.every(
              (row) => row['passes'] != false,
            );
          }
        }
        if (expectedUnavailable) {
          final crash = File('$diagnosticsDir/crash.log');
          report['initialization_error'] = crash.existsSync()
              ? crash.readAsStringSync()
              : '';
          report['native_failure_logged'] =
              (report['initialization_error'] as String).contains(
                'markdown codec unavailable:',
              );
        }
        report['success'] =
            report['unavailable'] == expectedUnavailable &&
            (expectedUnavailable
                ? report['native_failure_logged'] == true
                : report['inline'] == 'release') &&
            report['bench_passes'] != false;
      } catch (error, stack) {
        report['success'] = false;
        report['error'] = '$error';
        report['stack'] = '$stack';
      }
      try {
        final target = File(output);
        final temporary = File('$output.tmp');
        temporary.writeAsStringSync(
          const JsonEncoder.withIndent(' ').convert(report),
        );
        temporary.renameSync(target.path);
      } catch (error, stack) {
        // The driver must see a failed write, not wait forever for success.
        stderr.writeln(
          jsonEncode({'probe_result_write_error': '$error', 'stack': '$stack'}),
        );
        exitCode = 1;
      }
    });
  };
  app.main();
}
