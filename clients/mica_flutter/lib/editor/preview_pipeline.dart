part of 'preview_raster.dart';

/// Owns source-keyed rasters. Current paint demand is live; the 48-entry
/// budget evicts only cold entries, so a dense viewport cannot evict itself.
class RasterPreviewPipeline {
  RasterPreviewPipeline({
    required this.previewers,
    required this.requestRebuild,
    MicaTokens tokens = MicaTokens.light,
  }) : _tokens = tokens;

  final List<RasterPreviewer> previewers;
  final void Function(VoidCallback fn) requestRebuild;
  MicaTokens _tokens;
  int _generation = 0;
  bool _disposed = false;
  final Map<String, Map<String, ui.Image>> _cache = {};
  final Map<String, Map<String, double>> _baselines = {};
  final Map<String, Set<String>> _pending = {};
  final Map<String, Map<String, GlobalKey>> _keys = {};
  final Map<String, Map<String, int>> _tries = {};
  final Map<String, Set<String>> _failed = {};
  final Map<PreviewResourceKey, Object> _tickets = {};
  Map<PreviewResourceKey, double>? _demand;
  bool _demandScheduled = false;
  bool _rebuildScheduled = false;
  bool _captureScheduled = false;
  bool _capturing = false;
  bool _captureAgain = false;
  static const _maxPerPreviewer = 48;

  MicaTokens get tokens => _tokens;
  set tokens(MicaTokens value) {
    if (_disposed || _tokens == value) return;
    _tokens = value;
    _generation++;
    for (final map in _cache.values) {
      for (final image in map.values) {
        _disposePostFrame(image);
      }
      map.clear();
    }
    _baselines.clear();
    _pending.clear();
    _keys.clear();
    _tries.clear();
    _tickets.clear();
    _failed.clear();
  }

  Map<String, ui.Image> imagesOf(String id) => _cache[id] ??= {};
  Map<String, Map<String, ui.Image>> get images => _cache;
  Map<String, double> baselinesOf(String id) => _baselines[id] ??= {};
  Map<String, Map<String, double>> get baselines => _baselines;

  /// Safe from paint: pin immediately, perform work after the current frame.
  void setDemand(Map<PreviewResourceKey, double> value) {
    if (_disposed) return;
    _demand = Map.of(value);
    for (final key in _tickets.keys.toList()) {
      if (value.containsKey(key)) continue;
      _tickets.remove(key);
      _pending[key.id]?.remove(key.source);
      _keys[key.id]?.remove(key.source);
      _tries[key.id]?.remove(key.source);
    }
    if (_demandScheduled) return;
    _demandScheduled = true;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      _demandScheduled = false;
      if (_disposed) return;
      var trimmed = false;
      for (final id in _cache.keys.toList()) {
        trimmed = _cap(id) || trimmed;
      }
      if (trimmed) _notify();
      for (final entry in _demand!.entries) {
        request(entry.key.id, entry.key.source, entry.value);
      }
    });
    WidgetsBinding.instance.ensureVisualUpdate();
  }

  /// Only actual paint refreshes recency. Layout and cache reads do not.
  void touch(String id, String source) {
    final map = _cache[id];
    final image = map?.remove(source);
    if (image != null) map![source] = image;
  }

  bool _wanted(PreviewResourceKey key) => _demand?.containsKey(key) ?? true;

  bool _cap(String id) {
    final map = imagesOf(id);
    var changed = false;
    for (final source in map.keys.toList()) {
      if (map.length <= _maxPerPreviewer) break;
      if (_demand?.containsKey((id: id, source: source)) ?? false) continue;
      final image = map.remove(source)!;
      _baselines[id]?.remove(source);
      _disposePostFrame(image);
      changed = true;
    }
    final failed = _failed[id];
    if (failed != null) {
      for (final source in failed.toList()) {
        if (failed.length <= 256) break;
        if (_demand?.containsKey((id: id, source: source)) ?? false) continue;
        failed.remove(source);
      }
    }
    return changed;
  }

  void _disposePostFrame(ui.Image image) {
    WidgetsBinding.instance.addPostFrameCallback((_) => image.dispose());
  }

  bool _current(PreviewResourceKey key, Object ticket, int generation) =>
      !_disposed &&
      generation == _generation &&
      identical(_tickets[key], ticket);

  void _finish(
    PreviewResourceKey key,
    Object ticket,
    int generation,
    ui.Image? image,
  ) {
    if (!_current(key, ticket, generation) || !_wanted(key)) {
      image?.dispose();
      return;
    }
    _tickets.remove(key);
    _pending[key.id]?.remove(key.source);
    _keys[key.id]?.remove(key.source);
    _tries[key.id]?.remove(key.source);
    if (image == null) {
      (_failed[key.id] ??= {}).add(key.source);
      _baselines[key.id]?.remove(key.source);
    } else {
      final old = imagesOf(key.id)[key.source];
      imagesOf(key.id)[key.source] = image;
      if (old != null && !identical(old, image)) _disposePostFrame(old);
    }
    _cap(key.id);
    _notify();
  }

  void request(String id, String source, [double targetWidth = 800]) {
    if (_disposed) return;
    final previewer = previewers.where((p) => p.id == id).firstOrNull;
    final key = (id: id, source: source);
    if (previewer == null ||
        !_wanted(key) ||
        imagesOf(id).containsKey(source) ||
        (_failed[id]?.contains(source) ?? false) ||
        _tickets.containsKey(key)) {
      return;
    }
    final generation = _generation;
    final ticket = Object();
    _tickets[key] = ticket;
    (_pending[id] ??= {}).add(source);
    try {
      final direct = previewer.produce(source, targetWidth, _tokens);
      if (direct != null) {
        direct.then(
          (image) => _finish(key, ticket, generation, image),
          onError: (Object error, StackTrace stack) =>
              _finish(key, ticket, generation, null),
        );
      } else if (previewer.buildOffstage(source, _tokens) != null) {
        (_keys[id] ??= {})[source] = GlobalKey();
        _notify();
      } else {
        _tickets.remove(key);
        _pending[id]?.remove(source);
      }
    } catch (_) {
      _finish(key, ticket, generation, null);
    }
  }

  // All arrivals in a frame share one host rebuild and one capture callback.
  // The second callback runs after the rebuilt offstage host has painted.
  void _notify() {
    if (_disposed || _rebuildScheduled) return;
    _rebuildScheduled = true;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      _rebuildScheduled = false;
      if (_disposed) return;
      requestRebuild(() {});
      if (_keys.values.any((keys) => keys.isNotEmpty) && !_captureScheduled) {
        _captureScheduled = true;
        WidgetsBinding.instance.addPostFrameCallback((_) {
          _captureScheduled = false;
          _capture();
        });
      }
    });
    WidgetsBinding.instance.scheduleFrame();
  }

  Widget offstageHost() => Column(
    mainAxisSize: MainAxisSize.min,
    crossAxisAlignment: CrossAxisAlignment.start,
    children: [
      for (final previewer in previewers)
        for (final entry
            in (_keys[previewer.id] ?? const <String, GlobalKey>{}).entries)
          _offstageWidget(previewer, entry.key, entry.value),
    ],
  );

  Widget _offstageWidget(
    RasterPreviewer previewer,
    String source,
    GlobalKey key,
  ) => RepaintBoundary(
    key: key,
    child: _BaselineTap(
      onBaseline: (value) {
        if (value != null &&
            !_disposed &&
            identical(_keys[previewer.id]?[source], key)) {
          baselinesOf(previewer.id)[source] = value;
        }
      },
      child:
          previewer.buildOffstage(source, _tokens) ?? const SizedBox.shrink(),
    ),
  );

  Future<void> _capture() async {
    if (_disposed) return;
    if (_capturing) {
      _captureAgain = true;
      return;
    }
    _capturing = true;
    final generation = _generation;
    try {
      for (final previewer in previewers) {
        final sources = _keys[previewer.id]?.keys.toList() ?? const <String>[];
        for (final source in sources) {
          final key = (id: previewer.id, source: source);
          final ticket = _tickets[key];
          if (ticket == null) continue;
          final boundary = _keys[previewer.id]?[source]?.currentContext
              ?.findRenderObject();
          ui.Image? image;
          var captured = false;
          try {
            if (boundary is RenderRepaintBoundary) {
              image = await boundary.toImage(
                pixelRatio: EditorTheme.mathPixelRatio,
              );
              captured = true;
            }
          } catch (_) {
            /* Not painted yet: retry with a finite budget below. */
          }
          if (!_current(key, ticket, generation)) {
            image?.dispose();
            continue;
          }
          if (captured) {
            _finish(key, ticket, generation, image);
          } else {
            final tries = ((_tries[key.id] ??= {})[source] ?? 0) + 1;
            _tries[key.id]![source] = tries;
            if (tries > 20) _finish(key, ticket, generation, null);
          }
        }
      }
    } finally {
      _capturing = false;
      if (!_disposed &&
          (_captureAgain || _keys.values.any((keys) => keys.isNotEmpty))) {
        _captureAgain = false;
        _notify();
      }
    }
  }

  void dispose() {
    if (_disposed) return;
    _disposed = true;
    _generation++;
    for (final map in _cache.values) {
      for (final image in map.values) {
        image.dispose();
      }
    }
    _cache.clear();
    _baselines.clear();
    _pending.clear();
    _keys.clear();
    _tries.clear();
    _tickets.clear();
    _failed.clear();
    _demand = null;
  }
}
