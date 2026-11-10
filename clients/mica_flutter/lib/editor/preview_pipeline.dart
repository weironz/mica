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
  final Map<String, Set<String>> _failed = {};
  final Map<PreviewResourceKey, Object> _tickets = {};
  Map<PreviewResourceKey, double>? _demand;
  bool _demandScheduled = false;
  bool _rebuildScheduled = false;
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
    RasterPreview? preview,
  ) {
    final image = preview?.image;
    if (!_current(key, ticket, generation) || !_wanted(key)) {
      image?.dispose();
      return;
    }
    _tickets.remove(key);
    if (image == null) {
      (_failed[key.id] ??= {}).add(key.source);
      _baselines[key.id]?.remove(key.source);
    } else {
      final old = imagesOf(key.id)[key.source];
      imagesOf(key.id)[key.source] = image;
      final baseline = preview?.baseline;
      if (baseline != null) baselinesOf(key.id)[key.source] = baseline;
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
    try {
      previewer
          .produce(source, targetWidth, _tokens)
          .then(
            (preview) => _finish(key, ticket, generation, preview),
            onError: (Object error, StackTrace stack) =>
                _finish(key, ticket, generation, null),
          );
    } catch (_) {
      _finish(key, ticket, generation, null);
    }
  }

  // All arrivals in a frame share one rebuild.
  void _notify() {
    if (_disposed || _rebuildScheduled) return;
    _rebuildScheduled = true;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      _rebuildScheduled = false;
      if (!_disposed) requestRebuild(() {});
    });
    WidgetsBinding.instance.scheduleFrame();
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
    _tickets.clear();
    _failed.clear();
    _demand = null;
  }
}
