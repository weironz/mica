// Pure-Flutter desktop/multiplatform entrypoint for the Option C board spike.
// Run on Windows to test real native CJK IME candidate positioning & feel:
//   cd clients/mica_flutter
//   flutter run -d windows --no-pub -t e2e/board_spike_desktop_entry.dart
import 'package:flutter/material.dart';
import 'package:mica_flutter/board/board_spike_canvas.dart';

void main() {
  runApp(const _BoardSpikeDesktopApp());
}

class _BoardSpikeDesktopApp extends StatefulWidget {
  const _BoardSpikeDesktopApp();

  @override
  State<_BoardSpikeDesktopApp> createState() => _BoardSpikeDesktopAppState();
}

class _BoardSpikeDesktopAppState extends State<_BoardSpikeDesktopApp> {
  late final BoardSpikeController _controller = BoardSpikeController(
    nodeCount: 500,
  );

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      debugShowCheckedModeBanner: false,
      title: 'Mica Board Spike (Desktop)',
      home: BoardSpikeCanvas(controller: _controller),
    );
  }
}
