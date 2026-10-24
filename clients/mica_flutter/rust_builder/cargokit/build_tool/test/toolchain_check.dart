// Run with `dart run test/toolchain_check.dart`; no test-runner SDK dependency.
import 'dart:async';

import 'package:build_tool/src/builder.dart';
import 'package:build_tool/src/cargo.dart';
import 'package:build_tool/src/options.dart';
import 'package:build_tool/src/rustup.dart';
import 'package:build_tool/src/target.dart';
import 'package:build_tool/src/util.dart';

void check(bool condition, String message) {
  if (!condition) throw StateError(message);
}

Future<void> main() async {
  const pinned = '1.98.0-x86_64-pc-windows-msvc';
  const manifestDir = 'fixture/project/native';
  late List<TestRunCommandArgs> calls;
  late String active;

  RustBuilder builder({CargoBuildOptions? options}) => RustBuilder(
        target: Target(rust: 'x86_64-pc-windows-msvc'),
        environment: BuildEnvironment(
          configuration: BuildConfiguration.release,
          crateOptions: CargokitCrateOptions(cargo: {
            if (options != null) BuildConfiguration.release: options,
          }),
          targetTempDir: 'fixture/output',
          manifestDir: manifestDir,
          crateInfo: CrateInfo(packageName: 'fixture'),
          isAndroid: false,
        ),
      );

  Future<void> run(String name, FutureOr<void> Function() body) async {
    calls = [];
    active = '$pinned (overridden by fixture/rust-toolchain.toml)\r\n';
    testRunCommandOverride = (args) {
      calls.add(args);
      final command = args.arguments.join(' ');
      return TestRunCommandResult(
          stdout: switch (command) {
        'toolchain list' =>
          'stable-x86_64-pc-windows-msvc (default)\r\n$pinned\r\nnightly-x86_64-pc-windows-msvc\r\n',
        'show active-toolchain' => active,
        _ when command.startsWith('target list ') =>
          'x86_64-pc-windows-msvc\r\n',
        _ => '',
      });
    };
    try {
      await body();
      print('PASS: $name');
    } finally {
      testRunCommandOverride = null;
    }
  }

  await run('prepare/build honor the crate pin without reinstalling it',
      () async {
    final native = builder();
    native.prepare(Rustup());
    await native.build();
    final build = calls.singleWhere((c) => c.arguments.contains('cargo'));
    check(build.arguments.take(3).join(' ') == 'run $pinned cargo',
        'Cargokit must run the active repository pin, not floating stable');
    final activeCalls =
        calls.where((c) => c.arguments.join(' ') == 'show active-toolchain');
    check(activeCalls.length == 1, 'Resolve the active toolchain once');
    check(activeCalls.single.workingDirectory == manifestDir,
        'Resolve in the crate directory');
    check(
        calls
            .every((c) => c.arguments.take(2).join(' ') != 'toolchain install'),
        'An installed version pin must not be treated as absent');
  });

  await run('explicit Cargokit toolchain and flags remain authoritative',
      () async {
    final native = builder(
        options: CargoBuildOptions(
      toolchain: Toolchain.nightly,
      flags: ['--locked'],
    ));
    native.prepare(Rustup());
    await native.build();
    final build = calls.singleWhere((c) => c.arguments.contains('cargo'));
    check(build.arguments.take(3).join(' ') == 'run nightly cargo',
        'Preserve explicit toolchain');
    check(build.arguments.contains('--locked'), 'Preserve configured flags');
    check(calls.every((c) => !c.arguments.contains('active-toolchain')),
        'No implicit override');
  });

  await run('missing active toolchain fails without falling back to stable',
      () {
    active = ' \r\n';
    var rejected = false;
    try {
      builder().prepare(Rustup());
    } on FormatException {
      rejected = true;
    }
    check(rejected, 'Missing active toolchain must fail');
    check(calls.every((c) => !c.arguments.contains('cargo')),
        'Do not build after resolution failure');
  });
}
