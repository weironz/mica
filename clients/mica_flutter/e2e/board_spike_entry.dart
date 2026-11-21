// Standalone Web technical spike entrypoint for Option C (infinite board):
// Verifies Dart deferred chunk splitting, 500-node CanvasKit performance A/B,
// and zoomed-canvas CJK IME editing without touching main.dart or editor.dart.
import 'dart:convert';
import 'dart:js_interop';

import 'package:flutter/material.dart';
import 'package:mica_flutter/board/board_spike_canvas.dart' deferred as board;

@JS('micaBoardSpikeState')
external set _jsState(JSFunction value);

@JS('micaBoardSpikeLoad')
external set _jsLoad(JSFunction value);

@JS('micaBoardSpikeSetMode')
external set _jsSetMode(JSFunction value);

@JS('micaBoardSpikeSetViewport')
external set _jsSetViewport(JSFunction value);

@JS('micaBoardSpikeNodeRect')
external set _jsNodeRect(JSFunction value);

@JS('micaBoardSpikeResetCounters')
external set _jsResetCounters(JSFunction value);

@JS('micaBoardSpikeFrameTimings')
external set _jsFrameTimings(JSFunction value);

@JS('micaBoardSpikeFinishEditing')
external set _jsFinishEditing(JSFunction value);

void main() => runApp(const _BoardSpikeApp());

class _BoardSpikeApp extends StatefulWidget {
  const _BoardSpikeApp();

  @override
  State<_BoardSpikeApp> createState() => _BoardSpikeAppState();
}

class _BoardSpikeAppState extends State<_BoardSpikeApp> {
  bool _loaded = false;
  bool _loading = false;
  dynamic _controller; // board.BoardSpikeController after deferred load.

  @override
  void initState() {
    super.initState();
    _jsState = (() => _encodeState().toJS).toJS;
    _jsLoad = (() {
      _loadBoard();
    }).toJS;
    _jsSetMode = ((JSString mode) {
      if (!_loaded) return;
      final next = mode.toDart == 'naive'
          ? board.BoardRenderMode.naive
          : board.BoardRenderMode.optimized;
      _controller.setMode(next);
    }).toJS;
    _jsSetViewport = ((JSNumber zoom, JSNumber panX, JSNumber panY) {
      if (!_loaded) return;
      _controller.setViewport(
        zoom.toDartDouble,
        Offset(panX.toDartDouble, panY.toDartDouble),
      );
    }).toJS;
    _jsNodeRect = ((JSString nodeId) {
      if (!_loaded) return 'null'.toJS;
      final node = _controller.nodeById[nodeId.toDart];
      if (node == null) return 'null'.toJS;
      final Rect r = _controller.nodeScreenRect(node);
      return jsonEncode({
        'left': r.left,
        'top': r.top,
        'width': r.width,
        'height': r.height,
        'worldW': node.width,
        'worldH': node.height,
        'text': node.text,
      }).toJS;
    }).toJS;
    _jsResetCounters = (() {
      if (!_loaded) return;
      _controller.resetCounters();
    }).toJS;
    _jsFrameTimings = (() {
      if (!_loaded) return '[]'.toJS;
      final List<dynamic> samples = _controller.frameSamples;
      return jsonEncode([for (final s in samples) s.toJson()]).toJS;
    }).toJS;
    _jsFinishEditing = (() {
      if (!_loaded) return;
      _controller.finishEditing();
    }).toJS;
  }

  String _encodeState() {
    if (!_loaded) {
      return jsonEncode({'loaded': false, 'loading': _loading});
    }
    final editId = _controller.editingNodeId as String?;
    final editNode = editId == null ? null : _controller.nodeById[editId];
    return jsonEncode({
      'loaded': true,
      'loading': false,
      'mode': _controller.mode == board.BoardRenderMode.naive
          ? 'naive'
          : 'optimized',
      'zoom': _controller.zoom,
      'panX': _controller.pan.dx,
      'panY': _controller.pan.dy,
      'nodeCount': _controller.nodes.length,
      'edgeCount': _controller.edges.length,
      'editingNodeId': editId,
      'editingText': editNode?.text,
      'staticPaintCount': _controller.staticPaintCount,
      'pictureRebuildCount': _controller.pictureRebuildCount,
      'activePaintCount': _controller.activePaintCount,
      'naivePaintCount': _controller.naivePaintCount,
    });
  }

  Future<void> _loadBoard() async {
    if (_loaded || _loading) return;
    setState(() => _loading = true);
    await board.loadLibrary();
    _controller = board.BoardSpikeController(nodeCount: 500);
    if (mounted) {
      setState(() {
        _loading = false;
        _loaded = true;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      debugShowCheckedModeBanner: false,
      title: 'Mica Board Technical Spike',
      home: _loaded
          ? board.BoardSpikeCanvas(controller: _controller)
          : Scaffold(
              backgroundColor: const Color(0xFFF7F8FA),
              body: Center(
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    const Text(
                      'Mica Board Technical Spike (Deferred Shell)',
                      style: TextStyle(
                        fontSize: 18,
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                    const SizedBox(height: 12),
                    const Text(
                      'Before clicking below, board_spike_canvas.dart has NOT been downloaded.',
                      style: TextStyle(fontSize: 13, color: Color(0xFF646A73)),
                    ),
                    const SizedBox(height: 20),
                    FilledButton(
                      onPressed: _loading ? null : _loadBoard,
                      child: Text(
                        _loading ? 'Loading deferred board…' : 'Open Board Spike',
                      ),
                    ),
                  ],
                ),
              ),
            ),
    );
  }
}
